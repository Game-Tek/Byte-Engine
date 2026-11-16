//! A level editor that runs inside the game it edits.
//!
//! The editor is not a separate program: the game installs it with [`install`] during setup and steps it with
//! [`Editor::frame`] every tick. It draws its own UI layer over the game's frame, so the game keeps running and
//! rendering under it. It reads and changes the scene through the inspector protocol, the same one the console and
//! the HTTP inspector speak.
//!
//! F5 opens and closes it. Its modes stack as layers: the scene at the bottom, then a material graph, the palette,
//! a setting being typed, or a carried load on top. The top layer takes input, and Escape pops it, so Escape steps
//! back one mode at a time and closes the editor from the scene. While it's open, it frees the cursor and takes all
//! of the game's input. Clicking the game
//! view selects what it shows, and the gizmo, drawn in its own render pass, moves, turns, or scales the selection, by
//! the tool the toolbar picks.
//! The command palette opens, as VS Code's does, on a scope picked by its shortcut: Cmd+A on what can be placed, a
//! mesh, prefab, or light to drag into the scene or pin to the pointer with Enter until a click places it, and Cmd+K
//! on that and the material graphs to edit on a node canvas. Cmd+E opens it on the resources to explore: material
//! graphs, which open on the canvas, and textures, which open in a preview panel decoded from their source images.
//! While a graph is open, any of them finds a node kind to add to it. A prefab spawns where it's placed, with its
//! parameters' defaults, through the spawner the game gave the inspector, so it knows the game's own kinds. Cmd+. opens it on the game's runtime settings, which Cmd+K finds too:
//! Enter on one shows its value in the search field, and Enter again sets it to what the field says, as the console's
//! `set` does.
//!
//! Cmd+K also finds the New level command, which lists the folders under the asset directory and writes an empty
//! level into the one picked.
//!
//! While it's open, the warnings and errors the game logs show as toasts in the bottom-right corner.
//!
//! See [Edit a level in the running game](/docs/use/editor) for how to install it and what it edits.

mod carry;
mod fields;
mod gizmo;
mod graph;
mod input;
mod layers;
mod palette;
mod scene;
mod shell;
mod toasts;

use std::cell::RefCell;
use std::rc::Rc;
use std::time::{Duration, Instant};

use shell::Shell;

use crate::application::graphics::GraphicsApplication;
use crate::configuration::{ConfigurationEventId, ConfigurationUpdateState, ConfigurationValue};
use crate::core::Creator as _;
use crate::core::channel::{Channel as _, DefaultChannel};
use crate::core::factory::{CreateMessage, Factory, Handle};
use crate::core::listener::{DefaultListener, Listener as _};
use crate::gameplay::Transform;
use crate::gameplay::level::level_entity_handle_from_root;
use crate::ghi::window::{Event, Events};
use crate::editor::layers::Layer;
use crate::input::{ActionPhase, ResolvedAction, SeatHandle, Value};
use crate::inspector::{
	CAMERA_UPDATE_MESSAGE_TYPE, DELETE_MESSAGE_TYPE, InspectedEntity, InspectedValue, TRANSFORMATION_UPDATE_MESSAGE_TYPE,
};
use crate::math::Point;
use crate::physics::{RayQuery, RayQueryResponse};
use crate::rendering::{Features, ScreenSceneHit, ScreenSceneQuery, SetWindowFeatures, Window};
use crate::ui::layout::engine::Engine;
use crate::ui::{Key, Render, RenderRevision, Size, UiPoint, UiVector};

/// How often the editor asks the inspector for the scene, the same rate the console refreshes at.
const SCENE_POLL: Duration = Duration::from_millis(250);

/// The `Editor` struct is the editor layer a game installs over itself, which the game steps from its tick.
///
/// Create it with [`install`] before the game opens its first window, then call [`Editor::frame`] once per tick.
pub struct Editor {
	/// Lays out the editor's panels from the [`Shell`] state.
	engine: Engine<Shell>,
	/// Where the editor's renders go to its own UI render pass.
	renders: Factory<Render>,
	/// The revision of the last render sent, so an unchanged editor is not sent again.
	published: Option<RenderRevision>,
	/// The editor's actions, which its input layer resolves before the game's.
	actions: input::Actions,
	/// What the input layer resolved, and whether it takes the game's input.
	layer: Rc<RefCell<input::Layer>>,
	/// The resolved actions this frame reads, swapped with the layer's so neither list allocates again.
	resolved: Vec<ResolvedAction>,
	/// The game's windows as they open, so the editor learns which window to free the cursor of.
	windows: DefaultListener<CreateMessage<Window>>,
	/// The game's window and the features it opened with, once it opened.
	window: Option<(Handle, Features)>,
	window_features: DefaultChannel<SetWindowFeatures>,
	/// Resizes, so the editor lays out at the window's size.
	window_events: DefaultListener<Event>,
	/// When the editor last read the scene from the inspector, or `None` before the first read.
	polled: Option<Instant>,
	/// Every entity the inspector reported in that read, which the scene search runs over.
	entities: Vec<InspectedEntity>,
	/// The search the overview's rows were last found for.
	searched: String,
	/// The query of the last click on the game view, whose hit becomes the selection.
	picking: Option<Handle>,
	/// The renderer's answers to screen scene queries.
	hits: DefaultListener<CreateMessage<ScreenSceneHit>>,
	/// What the gizmo's render pass draws, and the view it drew through.
	gizmo: Rc<RefCell<gizmo::State>>,
	/// The entity being dragged with the gizmo, and the drag.
	drag: Option<(Handle, gizmo::Drag)>,
	/// Where the pointer last was, in normalized window coordinates.
	pointer: UiPoint,
	/// Whether a system key, Command or Super, is held.
	meta: bool,
	/// The palette result the pointer pressed, until it leaves the palette or is released.
	palette_press: Option<usize>,
	/// What the palette was last used for, newest first, by label and kind, which it lists first when it opens.
	recent: Vec<(String, palette::Kind)>,
	/// The settings the palette asked the game to change, by event, until their systems answer.
	settings: Vec<(ConfigurationEventId, String)>,
	/// The physics world's answers to the carried mesh's ray queries.
	rays: DefaultListener<CreateMessage<RayQueryResponse>>,
	/// The number of the last log record the editor read, so it reads each warning and error once.
	log_seen: u64,
}

/// Installs the editor over `app` and returns it, so the game can step it with [`Editor::frame`].
///
/// Call it during setup, after the game's own render passes and before the first window: render passes register
/// before the first window, and the editor's pass must come last to draw over everything else, the game's own UI
/// included. The game must use the default input, from `setup_default_input`. `viewport` is the window size the
/// editor lays out at until the first resize. The editor starts closed.
pub fn install(app: &mut GraphicsApplication, viewport: Size) -> Editor {
	let renders = Factory::new();
	// The gizmo draws over the game's frame, and the editor's panels over the gizmo.
	let gizmo = Rc::new(RefCell::new(gizmo::State::default()));
	gizmo::register(app.renderer_mut(), &gizmo);
	crate::application::graphics::setup_ui_render_pass(app, "editor", &renders, &[]);
	// A closed editor costs nothing: its pass forwards the frame unchanged.
	app.inspector()
		.update_configuration("render.pass.editor", ConfigurationValue::from("bypassed"));
	let (actions, layer) = input::install(app);

	let mut engine = Engine::with_context(Shell::new(viewport));
	engine.set_waker(app.waker().into());
	engine.mount(async |root| shell::shell(root).await);

	Editor {
		engine,
		renders,
		published: None,
		actions,
		layer,
		resolved: Vec::new(),
		windows: app.window_factory().listener(),
		window: None,
		window_features: app.messages().channel(),
		window_events: app.messages().channel::<Event>().listener(),
		polled: None,
		entities: Vec::new(),
		searched: String::new(),
		picking: None,
		hits: app.world().factory::<ScreenSceneHit>().listener(),
		gizmo,
		drag: None,
		pointer: UiPoint::new(0.0, 0.0),
		meta: false,
		palette_press: None,
		recent: Vec::new(),
		settings: Vec::new(),
		rays: app.world().factory::<RayQueryResponse>().listener(),
		// What was logged before the editor existed isn't news.
		log_seen: crate::application::log_capture::revision(),
	}
}

impl Editor {
	/// Opens or closes the editor on F5 and Escape, feeds this tick's pointer input and resizes into the editor's UI,
	/// lays it out, and sends the render when it changed.
	pub fn frame(&mut self, app: &mut GraphicsApplication) {
		while let Some(message) = self.windows.read() {
			// The editor frees the cursor of the game's first window.
			if self.window.is_none() {
				self.window = Some((message.handle(), message.data().features()));
			}
		}
		while let Some(event) = self.window_events.read() {
			if let Event::Window {
				event: Events::Resize { width, height },
				..
			} = event && width > 0
				&& height > 0
			{
				self.engine.ctx_mut().viewport = Size::new(width, height);
			}
		}

		// The list is taken out while it's read, since a toggle needs the whole editor, and put back to keep its storage.
		let mut resolved = std::mem::take(&mut self.resolved);
		std::mem::swap(&mut resolved, &mut self.layer.borrow_mut().resolved);
		let open = self.layer.borrow().open;
		for action in resolved.drain(..) {
			if action.action == self.actions.toggle {
				if action.value == Value::Bool(true) {
					self.toggle(app);
				}
				continue;
			}
			// A closed editor shows nothing, so it ignores the pointer.
			if !open {
				continue;
			}
			let handle = action.action;
			match action.value {
				Value::Vector2(v) if handle == self.actions.drag => {
					let pointer = UiPoint::new(v.x, v.y);
					self.pointer = pointer;
					// A palette or a carried mesh on top of the scene sees the pointer first.
					if self.layer_takes(app, action.phase, pointer) {
						continue;
					}
					match action.phase {
						ActionPhase::Started => {
							// A press the editor's panels don't take lands on the game view, where it grabs the gizmo
							// or picks a mesh.
							if !self.engine.press(pointer) && !self.grab(pointer) {
								self.pick(app, pointer);
							}
						}
						ActionPhase::Updated if self.drag.is_some() => self.drag_to(app, pointer),
						ActionPhase::Updated => {
							self.engine.drag_to(pointer);
						}
						ActionPhase::Ended | ActionPhase::Cancelled if self.drag.is_some() => self.release_gizmo(),
						ActionPhase::Ended => {
							self.engine.release(pointer);
						}
						ActionPhase::Cancelled => {
							self.engine.cancel();
						}
					}
				}
				Value::Vector2(v) if handle == self.actions.pointer => {
					self.pointer = UiPoint::new(v.x, v.y);
					self.engine.set_cursor_position(self.pointer);
					self.hover(self.pointer);
				}
				Value::Bool(down) if handle == self.actions.click => self.engine.update_click_state(down),
				Value::Float(v) if handle == self.actions.scroll => self.engine.update_scroll_state(UiVector::new(0.0, v)),
				Value::Unicode(character) if handle == self.actions.character => self.engine.input_character(character),
				Value::Bool(down) if handle == self.actions.backspace => {
					self.engine.update_key_state(Key::Backspace, down);
					if down {
						self.engine.delete_text_backward();
					}
				}
				Value::Bool(down) if handle == self.actions.meta => self.meta = down,
				// A focused field, such as a camera setting taking typed text, hears Enter and Escape too.
				Value::Bool(down) if handle == self.actions.enter || handle == self.actions.escape => {
					let key = if handle == self.actions.enter {
						Key::Enter
					} else {
						Key::Escape
					};
					self.engine.update_key_state(key, down);
					if down {
						self.key(app, handle);
					}
				}
				Value::Bool(true) => self.key(app, handle),
				_ => {}
			}
		}

		self.resolved = resolved;
		self.sync_open(app);

		let polled = self.poll_scene(app);
		self.find_shown(polled);
		self.adopt_hits();
		self.report_settings(app);
		self.toast_log();
		self.move_carry(app);
		self.post_edit(app);
		self.save_graph(app);
		self.edit_texture(app);
		self.place_gizmo();

		let viewport = self.engine.ctx_mut().viewport;
		self.engine.evaluate(viewport, app.frame_allocator());
		let render = self.engine.render();
		if self.published != Some(render.revision()) {
			self.published = Some(render.revision());
			self.renders.create(render.clone());
		}
	}

	/// Opens the editor on the scene when it's closed, or pops every layer when it's open, which closes it.
	fn toggle(&mut self, app: &GraphicsApplication) {
		if !self.engine.ctx_mut().layers.open() {
			self.engine.ctx_mut().layers.push(Layer::Scene);
			return;
		}
		while self.engine.ctx_mut().layers.open() {
			self.pop(app);
		}
	}

	/// Pops the top layer and undoes what it left in the game: a carried mesh or light is removed.
	fn pop(&mut self, app: &GraphicsApplication) {
		if let Some(Layer::Carry(carry)) = self.engine.ctx_mut().layers.pop() {
			remove_carried(app, carry);
		}
	}

	/// Opens or closes the editor to match its layers: it's open while any layer is.
	///
	/// An open editor shows its pass, frees the game window's cursor, and takes all of the game's input, so the game's
	/// held actions end. A closed one bypasses its pass and gives the window its own cursor capture back.
	fn sync_open(&mut self, app: &mut GraphicsApplication) {
		let open = self.engine.ctx_mut().layers.open();
		if std::mem::replace(&mut self.layer.borrow_mut().open, open) == open {
			return;
		}
		let pass = if open { "enabled" } else { "bypassed" };
		app.inspector()
			.update_configuration("render.pass.editor", ConfigurationValue::from(pass));
		if let Some((window, features)) = self.window {
			let features = if open { features - Features::CAPTURE_CURSOR } else { features };
			self.window_features.send(SetWindowFeatures::new(window, features));
		}
		if open {
			// The editor captures the releases of keys the game holds, so the game's actions end here instead.
			app.actions_mut().1.cancel_seat(SeatHandle::stub());
		} else {
			self.engine.cancel();
			self.release_gizmo();
		}
	}

	/// Asks the renderer which mesh the game's camera shows at `pointer`, in normalized window coordinates.
	///
	/// The answer arrives a few frames later; [`Self::adopt_hits`] selects it.
	fn pick(&mut self, app: &GraphicsApplication, pointer: UiPoint) {
		let shell = self.engine.ctx_mut();
		let Some(camera) = shell.camera() else {
			return;
		};
		// The pointer runs from -1 at the left and bottom edges to 1 at the right and top; the query counts pixels
		// from the top-left one.
		let pixel = |normalized: f32, extent: f32| ((normalized + 1.0) * 0.5 * extent).clamp(0.0, extent - 1.0) as u32;
		let point = [pixel(pointer.x, shell.viewport.x()), pixel(-pointer.y, shell.viewport.y())];
		self.picking = Some(app.world().create(ScreenSceneQuery::new(camera, point)).handle());
	}

	/// Selects the mesh the last pick found, or clears the selection when it found none.
	fn adopt_hits(&mut self) {
		while let Some(message) = self.hits.read() {
			let hit = message.data();
			if self.picking == Some(hit.query()) {
				self.picking = None;
				self.engine.ctx_mut().selected = hit.entity();
			}
		}
	}

	/// Answers a pressed key: Cmd+K opens the palette on everything, Cmd+A on what can be placed, Cmd+E on the
	/// resources to explore, and Cmd+. on the runtime settings, Cmd+S saves the open material graph, the arrows move
	/// the palette's highlight, Enter picks the highlighted entry or sets the typed setting, and Escape pops the top
	/// layer, which closes the editor from the scene. A property field taking typed text answers Enter and Escape
	/// itself.
	fn key(&mut self, app: &GraphicsApplication, key: crate::input::ActionHandle) {
		let scope = if key == self.actions.k {
			Some(palette::Scope::All)
		} else if key == self.actions.a {
			Some(palette::Scope::Place)
		} else if key == self.actions.e {
			Some(palette::Scope::Explore)
		} else if key == self.actions.period {
			Some(palette::Scope::Configure)
		} else {
			None
		};
		if let Some(scope) = scope
			&& self.meta
		{
			self.toggle_palette(app, scope);
			return;
		}
		if key == self.actions.s && self.meta {
			let shell = self.engine.ctx_mut();
			shell.save_graph = shell.layers.graph().is_some();
			return;
		}
		match self.engine.ctx_mut().layers.top_mut() {
			Some(Layer::Typing) => {}
			Some(Layer::Palette(palette)) if key == self.actions.up => palette.move_highlight(-1),
			Some(Layer::Palette(palette)) if key == self.actions.down => palette.move_highlight(1),
			Some(Layer::Palette(palette)) if key == self.actions.enter => {
				if let Some(entry) = palette.highlighted().cloned() {
					self.pick_entry(app, &entry);
				}
			}
			Some(Layer::Setting(setting)) if key == self.actions.up => setting.move_highlight(-1),
			Some(Layer::Setting(setting)) if key == self.actions.down => setting.move_highlight(1),
			Some(Layer::Setting(_)) if key == self.actions.enter => self.set_setting(app),
			Some(_) if key == self.actions.escape => self.pop(app),
			_ => {}
		}
	}

	/// Opens the palette on `asked`, closes it when it's already open on that scope, or switches it to that scope, as
	/// VS Code's shortcuts do. What the palette opens over picks the scope it opens on: over a material graph, it finds
	/// the graph's node kinds. A carried load or a field taking text keeps the palette closed.
	fn toggle_palette(&mut self, app: &GraphicsApplication, asked: palette::Scope) {
		let layers = &mut self.engine.ctx_mut().layers;
		if matches!(layers.top(), None | Some(Layer::Carry(_) | Layer::Typing)) {
			return;
		}
		let scope = layers.palette_scope(asked);
		// A setting being typed goes back to the palette that found it, which the new palette replaces.
		if matches!(layers.top(), Some(Layer::Setting(_))) {
			layers.pop();
		}
		let same = match layers.top() {
			Some(Layer::Palette(palette)) => {
				let same = palette.scope == scope;
				layers.pop();
				same
			}
			_ => false,
		};
		self.palette_press = None;
		if !same {
			let entries = palette_entries(app, scope);
			self.engine
				.ctx_mut()
				.layers
				.push(Layer::Palette(palette::State::new(scope, entries, &self.recent)));
		}
	}

	/// Routes the pointer's press, motion, and release to the top layer when it's a carried load or the palette, and
	/// returns whether it took them.
	///
	/// A pinned load is placed by the next press, a dragged one by its release. A press on a palette result picks it: it
	/// spawns once the pointer leaves the palette, and follows the pointer until the release, or, released on the same
	/// result, it's pinned. A press elsewhere closes the palette and goes on to the scene.
	fn layer_takes(&mut self, app: &GraphicsApplication, phase: ActionPhase, pointer: UiPoint) -> bool {
		let shell = self.engine.ctx_mut();
		match shell.layers.top() {
			Some(Layer::Carry(carry)) => {
				match (phase, carry.held) {
					(ActionPhase::Started, false) | (ActionPhase::Ended, true) => self.place_carry(app),
					(ActionPhase::Cancelled, true) => self.pop(app),
					_ => {}
				}
				return true;
			}
			Some(Layer::Palette(_) | Layer::Setting(_)) => {}
			_ => return false,
		}
		let viewport = shell.viewport;
		let pixel = [(pointer.x + 1.0) * 0.5 * viewport.x(), (1.0 - pointer.y) * 0.5 * viewport.y()];
		// A setting being typed shows its own rows in the palette's place.
		let entry_at = |layers: &layers::Layers, row: usize| match layers.setting() {
			Some(setting) => setting.rows.get(row).cloned(),
			None => layers.palette().and_then(|palette| palette.results.get(row).cloned()),
		};
		let rows = match (shell.layers.setting(), shell.layers.palette()) {
			(Some(setting), _) => setting.rows.len(),
			(None, Some(palette)) => palette.results.len(),
			(None, None) => 0,
		};
		match phase {
			ActionPhase::Started => {
				if !palette::contains(viewport, rows, pixel) {
					shell.layers.close(|layer| matches!(layer, Layer::Palette(_)));
					return false;
				}
				self.palette_press = palette::row_at(viewport, rows, pixel);
			}
			ActionPhase::Updated => {
				if let Some(row) = self.palette_press
					&& !palette::contains(viewport, rows, pixel)
					&& let Some(entry) = entry_at(&shell.layers, row)
				{
					self.palette_press = None;
					// Only what's placed can be dragged into the scene; a material graph opens, and a node kind is
					// added, when its row is clicked.
					if entry.kind.places() {
						shell.layers.pop();
						self.remember(&entry);
						self.spawn_carry(app, &entry, true);
					}
				}
			}
			ActionPhase::Ended => {
				if let Some(row) = self.palette_press.take()
					&& palette::row_at(viewport, rows, pixel) == Some(row)
					&& let Some(entry) = entry_at(&shell.layers, row)
				{
					if let Some(setting) = shell.layers.setting_mut() {
						setting.pick(row);
						self.set_setting(app);
					} else {
						self.pick_entry(app, &entry);
					}
				}
			}
			ActionPhase::Cancelled => self.palette_press = None,
		}
		true
	}

	/// Returns the world ray under the pointer, through the game camera's view the gizmo was last drawn through.
	fn pointer_ray(&self) -> Option<crate::math::Ray> {
		let (view, _) = self.gizmo.borrow().view?;
		Some(view.ray([self.pointer.x, self.pointer.y]))
	}

	/// Answers Enter, or a click, on the palette's `entry`: opens a setting's value over the palette to type a new one,
	/// or closes the palette and [`Self::choose`]s the entry.
	fn pick_entry(&mut self, app: &GraphicsApplication, entry: &palette::Entry) {
		self.remember(entry);
		let layers = &mut self.engine.ctx_mut().layers;
		if let Some(setting) = palette::Setting::new(entry) {
			layers.push(Layer::Setting(setting));
			return;
		}
		// The palette gives way to what it opens.
		layers.pop();
		self.choose(app, entry, false);
	}

	/// Puts `entry` first among what the palette was used for.
	fn remember(&mut self, entry: &palette::Entry) {
		// Every scope shares the list, so it keeps enough that one scope's use doesn't push out another's.
		const KEPT: usize = 32;
		self.recent
			.retain(|(label, kind)| !(*label == entry.label && *kind == entry.kind));
		self.recent.insert(0, (entry.label.clone(), entry.kind));
		self.recent.truncate(KEPT);
	}

	/// Sets the setting being typed to the picked or typed value, and closes it with the palette that found it.
	fn set_setting(&mut self, app: &GraphicsApplication) {
		let layers = &mut self.engine.ctx_mut().layers;
		let Some(setting) = layers.setting_mut() else {
			return;
		};
		// Read as the console reads a `set`, so `true`, `2`, and `off` reach the setting as their own kinds.
		let value = ConfigurationValue::from_typed(setting.chosen());
		let name = std::mem::take(&mut setting.name);
		layers.close(|layer| matches!(layer, Layer::Palette(_)));
		let event = app.inspector().update_configuration(&name, value);
		self.settings.push((event.id(), name));
	}

	/// Toasts each setting the palette changed that its system refused, with the system's reason, and forgets those
	/// it answered.
	fn report_settings(&mut self, app: &GraphicsApplication) {
		self.settings.retain(|(id, name)| match app.inspector().configuration_event(*id).map(|event| event.state().clone()) {
			Some(ConfigurationUpdateState::Pending) => true,
			Some(ConfigurationUpdateState::NotSet { reason }) => {
				// The toast names the setting, as some systems' reasons don't; a reason that does opens with the same
				// sentence, which would show twice.
				let lead = format!("`{name}` was not set. ");
				let cause = reason.strip_prefix(lead.as_str()).unwrap_or(&reason);
				log::warn!("{lead}{cause}");
				false
			}
			Some(ConfigurationUpdateState::Set { .. }) | None => false,
		});
	}

	/// Acts on the palette's `entry`: adds a node of its kind to the open graph, opens a material graph on the canvas
	/// or a texture in the preview panel, or carries what it places, `held` when dragged out of the palette.
	fn choose(&mut self, app: &GraphicsApplication, entry: &palette::Entry, held: bool) {
		let id = entry.label.as_str();
		match entry.kind {
			palette::Kind::Node => {
				if let Some(graph) = self.engine.ctx_mut().layers.graph_mut()
					&& let Some(kind) = resource_management::bemx::kinds::kind(id)
				{
					graph.add(kind);
				}
				return;
			}
			palette::Kind::Graph => {}
			palette::Kind::Texture => {
				self.open_texture(app, id);
				return;
			}
			palette::Kind::NewLevel => {
				// The command's palette gives way to one on the folders the level can go in.
				let entries = palette_entries(app, palette::Scope::Folders);
				let palette = palette::State::new(palette::Scope::Folders, entries, &self.recent);
				self.engine.ctx_mut().layers.push(Layer::Palette(palette));
				return;
			}
			palette::Kind::Folder => {
				create_level(app, id);
				return;
			}
			_ => {
				self.spawn_carry(app, entry, held);
				return;
			}
		}
		let opened = app.inspector().read_source_asset(id).and_then(|bytes| {
			let text = String::from_utf8(bytes).map_err(|_| {
				format!("'{id}' isn't text. The most likely cause is a binary file named as a graph. Open a `.bemx` file saved as UTF-8 text.")
			})?;
			graph::Graph::open(id, &text)
		});
		match opened {
			Ok(graph) => {
				let shell = self.engine.ctx_mut();
				// What the explorer opens replaces the texture it showed.
				shell.layers.close(|layer| matches!(layer, Layer::Texture(_)));
				shell.layers.push(Layer::Graph(graph));
				shell.graph_revision += 1;
			}
			Err(error) => log::warn!("The editor couldn't open the material graph. {error}"),
		}
	}

	/// Writes the setting the texture's properties panel changed to the texture's sidecar, and reads back what the
	/// texture now bakes with, for the panel to show.
	fn edit_texture(&mut self, app: &GraphicsApplication) {
		let shell = self.engine.ctx_mut();
		let Some(setting) = shell.texture_edit.take() else {
			return;
		};
		let Some(texture) = shell.layers.texture_mut() else {
			return;
		};
		let inspector = app.inspector();
		match inspector.set_image_setting(&texture.id, setting).and_then(|()| inspector.image_settings(&texture.id)) {
			Ok(settings) => texture.settings = Some(settings),
			Err(error) => log::warn!("The editor couldn't change the settings of '{}'. {error}", texture.id),
		}
	}

	/// Opens the texture source image `id` in the preview panel, in place of any texture it showed.
	///
	/// The image decodes on this thread, which holds the frame for as long as a large image takes to decode; it's
	/// what a click asked for, once.
	fn open_texture(&mut self, app: &GraphicsApplication, id: &str) {
		// The panel never shows a texture larger than this, so a larger one is scaled down before it's uploaded.
		const LONGEST: u32 = 1024;
		let preview = app
			.inspector()
			.read_source_asset(id)
			.and_then(|bytes| resource_management::processors::preview_rgba8(&bytes, LONGEST));
		match preview {
			Ok(preview) => {
				// The panel shows the picture even when the settings can't be read, as from a malformed sidecar.
				let settings = app
					.inspector()
					.image_settings(id)
					.inspect_err(|error| log::warn!("The editor couldn't show the settings of '{id}'. {error}"))
					.ok();
				let layers = &mut self.engine.ctx_mut().layers;
				layers.close(|layer| matches!(layer, Layer::Texture(_)));
				layers.push(Layer::Texture(layers::Texture {
					id: id.to_owned(),
					preview,
					settings,
				}));
			}
			Err(error) => log::warn!("The editor couldn't preview '{id}'. {error}"),
		}
	}

	/// Carries what `entry` places from the ground under the pointer: `held` when dragged out of the palette, pinned
	/// otherwise. A mesh or light spawns now, to follow the pointer; a prefab spawns where it's placed.
	fn spawn_carry(&mut self, app: &GraphicsApplication, entry: &palette::Entry, held: bool) {
		let position = self.pointer_ray().and_then(carry::ground_point).unwrap_or(Point::origin());
		let (light, kind, settings) = match entry.kind {
			palette::Kind::Prefab => {
				let carry = carry::Carry::prefab(&entry.label, position, held);
				self.engine.ctx_mut().layers.push(Layer::Carry(carry));
				return;
			}
			palette::Kind::Light(light) => {
				let settings = serde_json::from_str(light.settings).expect(
					"A light's settings failed to parse. The most likely cause is a typo in the editor's light presets. Check `carry::LIGHTS`.",
				);
				(Some(light), light.kind, settings)
			}
			_ => (None, "mesh", serde_json::json!({ "resource": entry.label })),
		};
		let source = carry::level_source(entry.name(), &carry::placed(position, light), kind, settings);
		match app.inspector().spawn_level(&source) {
			Ok(root) => {
				let carry = carry::Carry::spawned(root, position, light, held);
				self.engine.ctx_mut().layers.push(Layer::Carry(carry));
			}
			Err(error) => log::warn!("The editor couldn't place '{}'. {error}", entry.label),
		}
	}

	/// Moves the carried mesh to the surface under the pointer: the first body the pointer's ray meets, as the physics
	/// world answers, or the ground when it meets none.
	///
	/// One query waits at a time, so the mesh follows the pointer a physics step behind.
	fn move_carry(&mut self, app: &GraphicsApplication) {
		let ray = self.pointer_ray();
		let Some(carry) = self.engine.ctx_mut().layers.carry_mut() else {
			while self.rays.read().is_some() {}
			return;
		};
		let Some(ray) = ray else {
			return;
		};
		let mut target = None;
		while let Some(answer) = self.rays.read() {
			let answer = answer.data();
			if carry.query == Some(answer.query()) {
				carry.query = None;
				target = Some(answer.hit().map(|hit| hit.point()).or_else(|| carry::ground_point(ray)));
			}
		}
		if carry.query.is_none() {
			carry.query = Some(app.world().create(RayQuery::new(ray, f32::INFINITY)).handle());
		}
		if let Some(Some(position)) = target
			&& position != carry.position
		{
			carry.position = position;
			// A prefab spawns only where it's placed, so only a spawned mesh or light moves along.
			let carry::Load::Spawned { entity, .. } = carry.load else {
				return;
			};
			let transform = carry::placed(position, carry.light);
			let payload = serde_json::to_string(&InspectedValue::Transform(&transform)).expect(
				"Transform failed to serialize. The most likely cause is a non-finite component. Check the pointer's ray.",
			);
			if let Err(error) = app
				.inspector()
				.post_message(TRANSFORMATION_UPDATE_MESSAGE_TYPE, entity, &payload)
			{
				log::warn!(
					"The editor couldn't move the carried mesh. {error} Check that the game registers inspector messages."
				);
			}
		}
	}

	/// Posts the deletion and the change the properties panel made to the game: a deletion as a `Delete`, a transform
	/// as a `TransformationUpdate`, camera settings as a `CameraUpdate`, and light settings as the light's own update.
	fn post_edit(&mut self, app: &GraphicsApplication) {
		if let Some(handle) = self.engine.ctx_mut().delete.take() {
			// The deleted entity can't be dragged on.
			if self.drag.as_ref().is_some_and(|(dragged, _)| *dragged == handle) {
				self.release_gizmo();
			}
			if let Err(error) = app.inspector().post_message(DELETE_MESSAGE_TYPE, handle, "null") {
				log::warn!("The editor couldn't delete the entity. {error} Check that the game registers inspector messages.");
			}
		}
		let Some((handle, change)) = self.engine.ctx_mut().edit.take() else {
			return;
		};
		let (message_type, payload) = match &change {
			fields::Change::Transform(transform) => (
				TRANSFORMATION_UPDATE_MESSAGE_TYPE,
				serde_json::to_string(&InspectedValue::Transform(transform)),
			),
			fields::Change::Camera(camera) => (CAMERA_UPDATE_MESSAGE_TYPE, serde_json::to_string(camera)),
			fields::Change::Light(light) => (light.kind.update_message(), serde_json::to_string(&light.payload)),
		};
		let payload = payload.expect(
			"The edited value failed to serialize. The most likely cause is a non-finite value. Check the edited field.",
		);
		if let Err(error) = app.inspector().post_message(message_type, handle, &payload) {
			log::warn!("The editor couldn't change the entity. {error} Check that the game registers inspector messages.");
		}
	}

	/// Writes the open material graph to its source file through the inspector when the canvas asked to save it.
	///
	/// A graph that breaks the format's rules isn't written; the canvas shows why. A written graph is baked again, as
	/// after any source change, so the game's materials show the edit.
	fn save_graph(&mut self, app: &GraphicsApplication) {
		let shell = self.engine.ctx_mut();
		if !std::mem::take(&mut shell.save_graph) {
			return;
		}
		let Some(graph) = shell.layers.graph_mut() else {
			return;
		};
		match graph
			.save_text()
			.and_then(|text| app.inspector().write_source_asset(&graph.id, &text))
		{
			Ok(()) => graph.saved(),
			Err(error) => {
				log::warn!("The editor couldn't save material graph '{}'. {error}", graph.id);
				graph.fail(error);
			}
		}
	}

	/// Leaves the carried mesh where it is, or spawns the carried prefab there, and selects it, so the gizmo can move it
	/// on. A prefab's first entity is selected.
	fn place_carry(&mut self, app: &GraphicsApplication) {
		let layers = &mut self.engine.ctx_mut().layers;
		if layers.carry_mut().is_none() {
			return;
		}
		let Some(Layer::Carry(carry)) = layers.pop() else {
			return;
		};
		let entity = match carry.load {
			carry::Load::Spawned { entity, .. } => entity,
			carry::Load::Prefab(id) => match app.inspector().spawn_resource(&id, Transform::from_position(carry.position)) {
				Ok(root) => level_entity_handle_from_root(root, 0),
				Err(error) => {
					log::warn!("The editor couldn't place prefab '{id}'. {error}");
					return;
				}
			},
		};
		self.engine.ctx_mut().selected = Some(entity);
	}

	/// Shows the warnings and errors logged since the last frame as toasts, while the editor is open. Those logged
	/// while it's closed are passed over.
	fn toast_log(&mut self) {
		use crate::application::log_capture::{self, LineKind};
		let open = self.layer.borrow().open;
		let toasts = &mut self.engine.ctx_mut().toasts;
		self.log_seen = log_capture::entries_since(self.log_seen, |entry| {
			if let LineKind::Log(level @ (log::Level::Warn | log::Level::Error)) = entry.kind
				&& open
			{
				toasts.post(level, entry.message());
			}
		});
	}


	/// Stands the gizmo at the selection while the editor is open, in the game camera's view, and hides it otherwise.
	fn place_gizmo(&mut self) {
		let open = self.layer.borrow().open;
		let shell = self.engine.ctx_mut();
		let mut state = self.gizmo.borrow_mut();
		state.target = shell.selection().filter(|_| open).map(|entity| entity.transform.clone());
		state.camera = shell.camera();
		if state.target.is_none() || state.tool != shell.tool {
			state.hovered = None;
		}
		state.tool = shell.tool;
	}

	/// Lights the gizmo part under `pointer`, in normalized window coordinates, unless a part is being dragged.
	fn hover(&mut self, pointer: UiPoint) {
		let shell = self.engine.ctx_mut();
		// Only the scene's own layer shows the gizmo to point at; anything over it takes the pointer.
		if self.drag.is_some() || !matches!(shell.layers.top(), Some(Layer::Scene)) {
			self.gizmo.borrow_mut().hovered = None;
			return;
		}
		let target = self.engine.ctx_mut().selection().map(|entity| entity.transform.clone());
		let mut state = self.gizmo.borrow_mut();
		state.hovered = target
			.and_then(|target| state.placement(&target))
			.and_then(|placement| placement.pick(placement.pixel([pointer.x, pointer.y])));
	}

	/// Begins dragging the selection by the gizmo part under `pointer`, and returns whether the pointer was over one.
	fn grab(&mut self, pointer: UiPoint) -> bool {
		let Some(entity) = self.engine.ctx_mut().selection() else {
			return false;
		};
		let mut state = self.gizmo.borrow_mut();
		let target = &entity.transform;
		let Some(drag) = state
			.placement(target)
			.and_then(|placement| gizmo::grab(&placement, target, placement.pixel([pointer.x, pointer.y])))
		else {
			return false;
		};
		state.active = Some(drag.part);
		self.drag = Some((entity.handle, drag));
		true
	}

	/// Moves, turns, or scales the dragged entity to follow `pointer` along the grabbed part, and posts its new
	/// transform to the game through the inspector.
	fn drag_to(&mut self, app: &GraphicsApplication, pointer: UiPoint) {
		let Some((handle, drag)) = &self.drag else {
			return;
		};
		let shell = self.engine.ctx_mut();
		let Some(entity) = shell.scene.iter_mut().find(|entity| entity.handle == *handle) else {
			return;
		};
		let mut state = self.gizmo.borrow_mut();
		let Some(placement) = state.placement(&entity.transform) else {
			return;
		};
		let pixel = placement.pixel([pointer.x, pointer.y]);
		let Some(transform) = drag.transform(&placement, pixel) else {
			return;
		};
		state.stretch = drag.stretch(pixel);
		// The editor's copy changes at once, so the gizmo and the properties panel follow the pointer without waiting
		// for the next scene poll.
		entity.transform = transform;
		let payload = serde_json::to_string(&InspectedValue::Transform(&entity.transform)).expect(
			"Transform failed to serialize. The most likely cause is a non-finite component. Check the dragged entity's transform.",
		);
		if let Err(error) = app
			.inspector()
			.post_message(TRANSFORMATION_UPDATE_MESSAGE_TYPE, *handle, &payload)
		{
			log::warn!(
				"The editor couldn't move the dragged entity. {error} Check that the game registers inspector messages."
			);
		}
	}

	/// Ends a gizmo drag, leaving the entity as the last drag motion left it.
	fn release_gizmo(&mut self) {
		self.drag = None;
		let mut state = self.gizmo.borrow_mut();
		state.active = None;
		state.stretch = None;
	}

	/// Reads the scene through the inspector, as the console and HTTP inspector do, at most every [`SCENE_POLL`], and
	/// returns whether it read.
	///
	/// It doesn't read during a gizmo drag, when the inspector may still report where the entity was.
	fn poll_scene(&mut self, app: &GraphicsApplication) -> bool {
		if self.drag.is_some() {
			return false;
		}
		let now = Instant::now();
		if self.polled.is_some_and(|polled| now.duration_since(polled) < SCENE_POLL) {
			return false;
		}
		self.polled = Some(now);
		self.entities = app.inspector().entities(None, None);
		let shell = self.engine.ctx_mut();
		if scene::collect(&self.entities, &mut shell.scene) {
			shell.scene_revision += 1;
		}
		true
	}

	/// Finds the rows the overview lists, when the scene was just read or the search changed, and has the list rebuilt
	/// when they changed.
	fn find_shown(&mut self, polled: bool) {
		let shell = self.engine.ctx_mut();
		if !polled && shell.search == self.searched {
			return;
		}
		self.searched.clone_from(&shell.search);
		let shown = scene::shown(&self.entities, &shell.scene, &shell.search);
		if shown != shell.shown {
			shell.shown = shown;
			shell.scene_revision += 1;
		}
	}
}

/// Writes a new, empty level source into `folder`, as `/` or a folder ID such as `levels/`, under the first
/// `untitled` name no level there has.
fn create_level(app: &GraphicsApplication, folder: &str) {
	let created = app.inspector().source_assets("level").and_then(|levels| {
		let id = new_level_id(folder, &levels);
		app.inspector().write_source_asset(&id, EMPTY_LEVEL).map(|()| id)
	});
	match created {
		Ok(id) => log::info!("The editor created the level '{id}'."),
		Err(error) => log::warn!("The editor couldn't create the level. {error}"),
	}
}

/// The source of a level with no entities, which a new level starts as.
const EMPTY_LEVEL: &str = "{\n\t\"entities\": []\n}\n";

/// The ID a new level in `folder` takes: `untitled.level`, or `untitled 2.level` and so on when `levels`, the IDs of
/// the levels there are, has it.
fn new_level_id(folder: &str, levels: &[String]) -> String {
	// The asset directory's own folder adds nothing to the ID.
	let folder = folder.strip_prefix('/').unwrap_or(folder);
	(1..)
		.map(|n| match n {
			1 => format!("{folder}untitled.level"),
			n => format!("{folder}untitled {n}.level"),
		})
		.find(|id| !levels.contains(id))
		.expect("An unbounded count always finds a free name.")
}

/// Removes a carried mesh or light from the scene. A carried prefab hasn't spawned, so it only stops being carried.
fn remove_carried(app: &GraphicsApplication, carry: carry::Carry) {
	if let carry::Load::Spawned { root, .. } = carry.load
		&& let Err(error) = app.inspector().post_message(DELETE_MESSAGE_TYPE, root, "null")
	{
		log::warn!("The editor couldn't remove the carried mesh. {error} Check that the game registers inspector messages.");
	}
}

/// Gathers what the palette's `scope` finds: the node kinds for a graph, the runtime settings for settings, the
/// resources to explore for exploring, the source folders for a new level; otherwise the stored meshes, the prefabs,
/// and every kind of light, and for everything, also the New level command, the resources to explore, and the runtime
/// settings.
fn palette_entries(app: &GraphicsApplication, scope: palette::Scope) -> Vec<palette::Entry> {
	use palette::{Entry, Kind, Scope};
	let settings = || app.inspector().configuration_parameters().into_iter().map(Entry::setting);
	let mut entries = Vec::new();
	match scope {
		Scope::Nodes => {
			return resource_management::bemx::kinds::KINDS
				.iter()
				.map(|kind| Entry::new(kind.name, Kind::Node))
				.collect();
		}
		Scope::Configure => return settings().collect(),
		Scope::Explore => {
			explorable(app, &mut entries);
			return entries;
		}
		Scope::Folders => {
			return app.inspector().source_folders().map_or_else(
				|error| {
					log::warn!("The palette couldn't list the game's folders. {error}");
					Vec::new()
				},
				|folders| folders.into_iter().map(|folder| Entry::new(folder, Kind::Folder)).collect(),
			);
		}
		Scope::All | Scope::Place => {}
	}
	match app.inspector().resources("Mesh") {
		Ok(meshes) => entries.extend(meshes.into_iter().map(|id| Entry::new(id, Kind::Mesh))),
		Err(error) => log::warn!("The palette couldn't list the game's meshes. {error}"),
	}
	// A development build bakes a prefab from its source when it spawns, so its sources are what can spawn: the
	// store may still hold prefabs whose sources were deleted, and lacks those not baked yet. A build without
	// sources spawns what the store holds.
	match app
		.inspector()
		.source_assets("prefab")
		.or_else(|_| app.inspector().resources("Prefab"))
	{
		Ok(prefabs) => entries.extend(prefabs.into_iter().map(|id| Entry::new(id, Kind::Prefab))),
		Err(error) => log::warn!("The palette couldn't list the game's prefabs. {error}"),
	}
	entries.extend(carry::LIGHTS.iter().map(|light| Entry::new(light.name, Kind::Light(light))));
	if scope == Scope::All {
		entries.push(Entry::new("New level", Kind::NewLevel));
		explorable(app, &mut entries);
		entries.extend(settings());
	}
	entries
}

/// Adds the resources the explorer opens to `entries`: the `.bemx` material graphs and the PNG and JPEG texture
/// sources a development build can read.
fn explorable(app: &GraphicsApplication, entries: &mut Vec<palette::Entry>) {
	use palette::{Entry, Kind};
	match app.inspector().source_assets("bemx") {
		Ok(graphs) => entries.extend(graphs.into_iter().map(|id| Entry::new(id, Kind::Graph))),
		Err(error) => log::warn!("The palette couldn't list the game's material graphs. {error}"),
	}
	for extension in ["png", "jpg", "jpeg"] {
		match app.inspector().source_assets(extension) {
			Ok(textures) => entries.extend(textures.into_iter().map(|id| Entry::new(id, Kind::Texture))),
			Err(error) => {
				log::warn!("The palette couldn't list the game's textures. {error}");
				// Every extension fails alike, so one warning is enough.
				break;
			}
		}
	}
}

#[cfg(test)]
mod tests {
	use super::new_level_id;

	#[test]
	fn a_new_level_takes_the_first_untitled_name_free_in_its_folder() {
		assert_eq!(new_level_id("/", &[]), "untitled.level");
		let taken = ["levels/untitled.level", "levels/untitled 2.level", "untitled 3.level"].map(String::from);
		assert_eq!(new_level_id("levels/", &taken), "levels/untitled 3.level");
	}
}
