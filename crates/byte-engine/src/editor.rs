//! A level editor that runs inside the game it edits.
//!
//! The editor is not a separate program: the game installs it with [`install`] during setup and steps it with
//! [`Editor::frame`] every tick. It draws its own UI layer over the game's frame, so the game keeps running and
//! rendering under it. It reads and changes the scene through the inspector protocol, the same one the console and
//! the HTTP inspector speak.
//!
//! F5 opens and closes it. While it's open, it frees the cursor and takes all of the game's input. Clicking the game
//! view selects what it shows, and the gizmo, drawn in its own render pass, moves, turns, or scales the selection, by
//! the tool the toolbar picks.
//! The command palette opens, as VS Code's does, on a scope picked by its shortcut: Cmd+A on what can be placed, a
//! mesh, prefab, or light to drag into the scene or pin to the pointer with Enter until a click places it, and Cmd+K
//! on that and the material graphs to edit on a node canvas. While a graph is open, either finds a node kind to add to
//! it. A prefab spawns where it's placed, with its parameters' defaults, through the spawner the game gave the
//! inspector, so it knows the game's own kinds.
//!
//! While it's open, the warnings and errors the game logs show as toasts in the bottom-right corner.
//!
//! See [Edit a level in the running game](/docs/use/editor) for how to install it and what it edits.

mod carry;
mod fields;
mod gizmo;
mod graph;
mod input;
mod palette;
mod scene;
mod shell;
mod toasts;

use std::cell::RefCell;
use std::rc::Rc;
use std::time::{Duration, Instant};

use shell::Shell;

use crate::application::graphics::GraphicsApplication;
use crate::configuration::ConfigurationValue;
use crate::core::Creator as _;
use crate::core::channel::{Channel as _, DefaultChannel};
use crate::core::factory::{CreateMessage, Factory, Handle};
use crate::core::listener::{DefaultListener, Listener as _};
use crate::gameplay::Transform;
use crate::gameplay::level::level_entity_handle_from_root;
use crate::ghi::window::{Event, Events};
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
	/// The mesh the pointer carries into the scene.
	carry: Option<carry::Carry>,
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
		carry: None,
		rays: app.world().factory::<RayQueryResponse>().listener(),
		// What was logged before the editor existed isn't news.
		log_seen: crate::application::log_capture::revision(),
	}
}

impl Editor {
	/// Opens or closes the editor on F5, feeds this tick's pointer input and resizes into the editor's UI, lays it
	/// out, and sends the render when it changed.
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
					// The palette and a carried mesh see the pointer first.
					if self.carry_or_palette_takes(app, action.phase, pointer) {
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

		let polled = self.poll_scene(app);
		self.find_shown(polled);
		self.adopt_hits();
		self.toast_log();
		self.move_carry(app);
		self.post_edit(app);
		self.save_graph(app);
		self.place_gizmo();

		let viewport = self.engine.ctx_mut().viewport;
		self.engine.evaluate(viewport, app.frame_allocator());
		let render = self.engine.render();
		if self.published != Some(render.revision()) {
			self.published = Some(render.revision());
			self.renders.create(render.clone());
		}
	}

	/// Opens the editor when it's closed, or closes it when it's open.
	///
	/// An open editor shows its pass, frees the game window's cursor, and takes all of the game's input, so the game's
	/// held actions end. A closed one bypasses its pass and gives the window its own cursor capture back.
	fn toggle(&mut self, app: &mut GraphicsApplication) {
		let open = {
			let mut layer = self.layer.borrow_mut();
			layer.open = !layer.open;
			layer.open
		};
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

	/// Answers a pressed key: Cmd+K opens the palette on everything and Cmd+A on what can be placed, Cmd+S saves the
	/// open material graph, the arrows move the palette's highlight, Enter chooses the highlighted entry, and Escape
	/// cancels a carried load or closes the palette or the graph.
	fn key(&mut self, app: &GraphicsApplication, key: crate::input::ActionHandle) {
		let open = self.engine.ctx_mut().palette.open;
		let scope = if key == self.actions.k {
			Some(palette::Scope::All)
		} else if key == self.actions.a {
			Some(palette::Scope::Place)
		} else {
			None
		};
		if let Some(scope) = scope
			&& self.meta
			&& self.carry.is_none()
		{
			self.toggle_palette(app, scope);
		} else if key == self.actions.s && self.meta {
			let shell = self.engine.ctx_mut();
			shell.save_graph = shell.graph.is_some();
		} else if key == self.actions.up && open {
			self.engine.ctx_mut().palette.move_highlight(-1);
		} else if key == self.actions.down && open {
			self.engine.ctx_mut().palette.move_highlight(1);
		} else if key == self.actions.enter && open {
			let palette = &mut self.engine.ctx_mut().palette;
			if let Some(entry) = palette.highlighted().cloned() {
				palette.close();
				self.choose(app, &entry, false);
			}
		} else if key == self.actions.escape {
			let shell = self.engine.ctx_mut();
			if self.carry.is_some() {
				self.cancel_carry(app);
			} else if open {
				shell.palette.close();
			} else if shell.graph.is_some() {
				shell.graph = None;
			}
		}
	}

	/// Opens the palette on `scope`, closes it when it's already open on `scope`, or switches it to `scope`, as VS
	/// Code's shortcuts do. While a material graph is open, every scope is its node kinds.
	fn toggle_palette(&mut self, app: &GraphicsApplication, scope: palette::Scope) {
		let shell = self.engine.ctx_mut();
		let scope = if shell.graph.is_some() { palette::Scope::Nodes } else { scope };
		if shell.palette.open && shell.palette.scope == scope {
			shell.palette.close();
			return;
		}
		self.palette_press = None;
		let entries = palette_entries(app, scope);
		self.engine.ctx_mut().palette.open(scope, entries);
	}

	/// Routes the pointer's press, motion, and release to a carried mesh or the open palette, and returns whether they
	/// took it.
	///
	/// A pinned mesh is placed by the next press, a dragged one by its release. A press on a palette result picks it: it
	/// spawns once the pointer leaves the palette, and follows the pointer until the release, or, released on the same
	/// result, it's pinned. A press elsewhere closes the palette and goes on to the scene.
	fn carry_or_palette_takes(&mut self, app: &GraphicsApplication, phase: ActionPhase, pointer: UiPoint) -> bool {
		if let Some(held) = self.carry.as_ref().map(|carry| carry.held) {
			match phase {
				ActionPhase::Started if !held => self.place_carry(app),
				ActionPhase::Ended if held => self.place_carry(app),
				ActionPhase::Cancelled if held => self.cancel_carry(app),
				_ => {}
			}
			return true;
		}
		let shell = self.engine.ctx_mut();
		if !shell.palette.open && self.palette_press.is_none() {
			return false;
		}
		let viewport = shell.viewport;
		let pixel = [(pointer.x + 1.0) * 0.5 * viewport.x(), (1.0 - pointer.y) * 0.5 * viewport.y()];
		let rows = shell.palette.results.len();
		let entry_at = |row: usize| shell.palette.results.get(row).cloned();
		match phase {
			ActionPhase::Started => {
				if !palette::contains(viewport, rows, pixel) {
					shell.palette.close();
					return false;
				}
				self.palette_press = palette::row_at(viewport, rows, pixel);
			}
			ActionPhase::Updated => {
				if let Some(row) = self.palette_press
					&& !palette::contains(viewport, rows, pixel)
					&& let Some(entry) = entry_at(row)
				{
					self.palette_press = None;
					// Only what's placed can be dragged into the scene; a material graph opens, and a node kind is
					// added, when its row is clicked.
					if entry.kind.places() {
						shell.palette.close();
						self.spawn_carry(app, &entry, true);
					}
				}
			}
			ActionPhase::Ended => {
				if let Some(row) = self.palette_press.take()
					&& palette::row_at(viewport, rows, pixel) == Some(row)
					&& let Some(entry) = entry_at(row)
				{
					shell.palette.close();
					self.choose(app, &entry, false);
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

	/// Acts on the palette's `entry`: adds a node of its kind to the open graph, opens a material graph on the canvas,
	/// or carries what it places, `held` when dragged out of the palette.
	fn choose(&mut self, app: &GraphicsApplication, entry: &palette::Entry, held: bool) {
		let id = entry.label.as_str();
		match entry.kind {
			palette::Kind::Node => {
				if let Some(graph) = self.engine.ctx_mut().graph.as_mut()
					&& let Some(kind) = resource_management::bemx::kinds::kind(id)
				{
					graph.add(kind);
				}
				return;
			}
			palette::Kind::Graph => {}
			_ => {
				self.spawn_carry(app, entry, held);
				return;
			}
		}
		let opened = app
			.inspector()
			.read_source_asset(id)
			.and_then(|text| graph::Graph::open(id, &text));
		match opened {
			Ok(graph) => {
				let shell = self.engine.ctx_mut();
				shell.graph = Some(graph);
				shell.graph_revision += 1;
			}
			Err(error) => log::warn!("The editor couldn't open the material graph. {error}"),
		}
	}

	/// Carries what `entry` places from the ground under the pointer: `held` when dragged out of the palette, pinned
	/// otherwise. A mesh or light spawns now, to follow the pointer; a prefab spawns where it's placed.
	fn spawn_carry(&mut self, app: &GraphicsApplication, entry: &palette::Entry, held: bool) {
		let position = self.pointer_ray().and_then(carry::ground_point).unwrap_or(Point::origin());
		let (light, kind, settings) = match entry.kind {
			palette::Kind::Prefab => {
				self.carry = Some(carry::Carry::prefab(&entry.label, position, held));
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
			Ok(root) => self.carry = Some(carry::Carry::spawned(root, position, light, held)),
			Err(error) => log::warn!("The editor couldn't place '{}'. {error}", entry.label),
		}
	}

	/// Moves the carried mesh to the surface under the pointer: the first body the pointer's ray meets, as the physics
	/// world answers, or the ground when it meets none.
	///
	/// One query waits at a time, so the mesh follows the pointer a physics step behind.
	fn move_carry(&mut self, app: &GraphicsApplication) {
		if self.carry.is_none() {
			while self.rays.read().is_some() {}
			return;
		}
		let Some(ray) = self.pointer_ray() else {
			return;
		};
		let Some(carry) = self.carry.as_mut() else {
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
	/// as a `TransformationUpdate`, and camera settings as a `CameraUpdate`.
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
		let Some(graph) = shell.graph.as_mut() else {
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
		let Some(carry) = self.carry.take() else {
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

	/// Removes the carried mesh from the scene. A carried prefab hasn't spawned, so it only stops being carried.
	fn cancel_carry(&mut self, app: &GraphicsApplication) {
		if let Some(carry) = self.carry.take()
			&& let carry::Load::Spawned { root, .. } = carry.load
			&& let Err(error) = app.inspector().post_message(DELETE_MESSAGE_TYPE, root, "null")
		{
			log::warn!(
				"The editor couldn't remove the carried mesh. {error} Check that the game registers inspector messages."
			);
		}
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
		if self.drag.is_some() || self.carry.is_some() || shell.palette.open || shell.graph.is_some() {
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

/// Gathers what the palette's `scope` finds: the node kinds for a graph; otherwise the stored meshes, the prefabs,
/// and every kind of light, and for everything, also the `.bemx` sources a development build can open.
fn palette_entries(app: &GraphicsApplication, scope: palette::Scope) -> Vec<palette::Entry> {
	use palette::{Entry, Kind, Scope};
	if scope == Scope::Nodes {
		return resource_management::bemx::kinds::KINDS
			.iter()
			.map(|kind| Entry::new(kind.name, Kind::Node))
			.collect();
	}
	let mut entries: Vec<Entry> = app
		.inspector()
		.resources("Mesh")
		.unwrap_or_else(|error| {
			log::warn!("The palette couldn't list the game's meshes. {error}");
			Vec::new()
		})
		.into_iter()
		.map(|id| Entry::new(id, Kind::Mesh))
		.collect();
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
		match app.inspector().source_assets("bemx") {
			Ok(graphs) => entries.extend(graphs.into_iter().map(|id| Entry::new(id, Kind::Graph))),
			Err(error) => log::warn!("The palette couldn't list the game's material graphs. {error}"),
		}
	}
	entries
}
