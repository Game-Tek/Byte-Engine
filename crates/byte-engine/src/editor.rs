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
//! of the game's input. It shows the game's camera until W, A, S, D, Space, Shift, or a right-button drag flies the
//! view, which switches the game window to the editor's own camera, standing where the game's was; closing the editor
//! gives the window the game's camera back. Clicking the game view selects what it shows, and the gizmo, drawn in its
//! own render pass, moves, turns, or scales the selection, by the tool the toolbar picks.
//! The command palette opens, as VS Code's does, on a scope picked by its shortcut: Cmd+A on what can be placed, a
//! mesh, prefab, or light to drag into the scene or pin to the pointer with Enter until a click places it, and Cmd+K
//! on that and the material graphs to edit on a node canvas. Cmd+E opens it on the resources to explore: material
//! graphs, material variants, whose parameters it changes, meshes, and textures, decoded from their source images. Each
//! opens in a window of its own, with its own layers and UI engine, which takes the input of that window; there, any of
//! the shortcuts finds a node kind to add to the graph. A variant's window shows it on a sphere, and a mesh's window
//! shows the mesh, both drawn on the editor's preview stage, a scene of its own; see [`preview`].
//! A prefab spawns where it's placed, with its parameters' defaults, through the spawner the game gave the inspector,
//! so it knows the game's own kinds. Cmd+. opens it on the game's runtime settings, which Cmd+K finds too: Enter on one
//! shows its value in the search field, and Enter again sets it to what the field says, as the console's `set` does.
//!
//! Cmd+K also finds the New level command, which lists the folders under the asset directory and writes an empty
//! level into the one picked.
//!
//! While it's open, the warnings and errors the game logs show as toasts in the bottom-right corner.
//!
//! See [Edit a level in the running game](/docs/use/editor) for how to install it and what it edits.

mod bindings;
mod carry;
mod create;
mod drawer;
mod fields;
mod fly;
mod gizmo;
mod graph;
mod history;
mod input;
mod layers;
mod palette;
mod panels;
mod preview;
mod scene;
mod shell;
mod toasts;
mod variant;

pub use panels::{Button, Label, Panel};
pub use shell::Shell;

use std::cell::RefCell;
use std::rc::Rc;
use std::time::{Duration, Instant};


use crate::application::graphics::GraphicsApplication;
use crate::configuration::{ConfigurationEventId, ConfigurationUpdateState, ConfigurationValue};
use crate::core::Creator as _;
use crate::core::channel::{Channel as _, DefaultChannel};
use crate::core::factory::{CreateMessage, Factory, Handle};
use crate::core::listener::{DefaultListener, Listener as _};
use crate::gameplay::Transform;
use crate::gameplay::level::level_entity_handle_from_root;
use crate::gameplay::transform::TransformationUpdate;
use crate::ghi::window::{Event, Events};
use crate::editor::layers::Layer;
use crate::input::{ActionPhase, ResolvedAction, SeatHandle, Value};
use crate::inspector::{
	CAMERA_UPDATE_MESSAGE_TYPE, DELETE_MESSAGE_TYPE, InspectedEntity, TRANSFORMATION_UPDATE_MESSAGE_TYPE,
};
use crate::math::{Point, Vector};
use crate::physics::{RayQuery, RayQueryResponse};
use crate::rendering::{
	Camera, CloseWindow, Features, ScreenSceneHit, ScreenSceneQuery, SetWindowCamera, SetWindowFeatures, Window,
};
use crate::ui::layout::engine::Engine;
use crate::ui::{Key, Render, RenderRevision, Size, UiPoint, UiVector};

/// The size a resource window opens at, in points.
const RESOURCE_WINDOW: [u32; 2] = [1100, 720];

/// The `ResourceWindow` struct is a window of its own where the editor shows one material graph or texture, so it can
/// be edited beside the game instead of over it.
struct ResourceWindow {
	/// The handle the window was created with.
	window: Handle,
	/// The window's native id, which its events carry, once the renderer opened it.
	id: Option<crate::ghi::window::WindowId>,
	/// The asset the window shows, such as `materials/floor.bemx`.
	asset: String,
	/// Lays out the window's interface, from its own [`Shell`].
	engine: Engine<Shell>,
	/// Where the window's renders go to its own UI render pass.
	renders: Factory<Render>,
	/// The window size of the last evaluation and the revision of the last render sent; see [`present`].
	published: Option<(Size, RenderRevision)>,
	/// The stage slot the window's main panel shows, while it previews a mesh or material.
	preview: Option<usize>,
}

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
	/// The window size of the last evaluation and the revision of the last render sent, so an idle editor isn't laid
	/// out and an unchanged one isn't sent again; see [`present`].
	published: Option<(Size, RenderRevision)>,
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
	/// The keys held down, which every key's press and release updates through [`Self::press`].
	held: bindings::Held,
	/// Whether the editor closed and its pass keeps drawing for the scene drawer to leave.
	closing: bool,
	/// The operations an undo or redo is applying, kept to reuse their storage.
	applying: Vec<history::Operation>,
	/// What the editor changed in the game, to undo and redo.
	history: history::History,
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
	/// The material graphs and textures open in windows of their own.
	resources: Vec<ResourceWindow>,
	/// Closes resource windows.
	close_windows: DefaultChannel<CloseWindow>,
	/// The camera the editor flies, once it first flew; it stays out of the scene list.
	camera: Option<Handle>,
	/// The editor camera's flight, and the game's camera it took over from, while the game window shows the editor's.
	flight: Option<(fly::Fly, Handle)>,
	/// The push the fly keys held this frame: right, up, and forward.
	thrust: [f32; 3],
	/// How far the right-button drag moved this frame, in normalized window coordinates, which turns the camera.
	turned: [f32; 2],
	/// Where the right-button drag last was, while it turns the camera.
	look: Option<UiPoint>,
	/// When the editor camera last moved, to move it on by the time since.
	flown: Option<Instant>,
	/// Switches the game window between the game's camera and the editor's.
	window_cameras: DefaultChannel<SetWindowCamera>,
	/// Where resource windows preview meshes and materials, apart from the game's world.
	stage: preview::Stage,
}

/// Installs the editor over `app` and returns it, so the game can step it with [`Editor::frame`].
///
/// Call it during setup, after the game's own render passes, before the first window, and before
/// [`crate::application::graphics::defaults::launch_deferred_tasks_thread`]: render passes register before the first
/// window, the editor's pass must come last to draw over everything else, the game's own UI included, and the pipeline
/// previews draw with loads on the loading thread. The game must use the default input, from `setup_default_input`.
/// `viewport` is the window size the editor lays out at until the first resize. The editor starts closed.
pub fn install(app: &mut GraphicsApplication, viewport: Size) -> Editor {
	let renders = Factory::new();
	// The gizmo draws over the game's frame, and the editor's panels over the gizmo.
	let gizmo = Rc::new(RefCell::new(gizmo::State::default()));
	gizmo::register(app.renderer_mut(), &gizmo);
	crate::application::graphics::setup_ui_render_pass(app, None, "editor", &renders, &[]);
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
		held: bindings::Held::default(),
		closing: false,
		applying: Vec::new(),
		history: history::History::default(),
		palette_press: None,
		recent: Vec::new(),
		settings: Vec::new(),
		rays: app.world().factory::<RayQueryResponse>().listener(),
		// What was logged before the editor existed isn't news.
		log_seen: crate::application::log_capture::revision(),
		resources: Vec::new(),
		close_windows: app.messages().channel(),
		camera: None,
		flight: None,
		thrust: [0.0; 3],
		turned: [0.0; 2],
		look: None,
		flown: None,
		window_cameras: app.messages().channel(),
		stage: preview::Stage::install(app),
	}
}

/// Lays `engine` out for its window's size and sends its render to `renders` when it changed since `published`, the
/// size it was last laid out for and the revision last sent.
///
/// It skips the evaluation while the engine asks for none, through [`Engine::next_tick`], and the window keeps its
/// size, so an idle interface costs nothing.
fn present(
	engine: &mut Engine<Shell>,
	renders: &mut Factory<Render>,
	published: &mut Option<(Size, RenderRevision)>,
	frame_allocator: &bumpalo::Bump,
) {
	let viewport = engine.ctx().viewport;
	let due = engine.next_tick().is_some_and(|tick| tick <= Instant::now());
	if !due && published.is_some_and(|(size, _)| size == viewport) {
		return;
	}
	engine.evaluate(viewport, frame_allocator);
	let render = engine.render();
	if published.is_none_or(|(_, revision)| revision != render.revision()) {
		renders.create(render.clone());
	}
	*published = Some((viewport, render.revision()));
}

impl Editor {
	/// Shows `panel` at the bottom of the entity panel while an entity with a `T` component is selected, under a header
	/// that reads `title`, in a column as tall as what the panel adds.
	///
	/// The editor runs `panel` with a [`Panel`] when such an entity is selected, and drops it when the selection
	/// changes, so it can loop for as long as it shows: add its controls, then wait for clicks and frames. Register the
	/// messages it posts and the properties it reads on the inspector, with
	/// [`DefaultInspector::register_message`](crate::inspector::DefaultInspector::register_message) and
	/// [`DefaultInspector::watch_property`](crate::inspector::DefaultInspector::watch_property). Panels show in the
	/// order they're registered.
	pub fn panel<T: 'static>(&mut self, title: &'static str, panel: impl AsyncFn(&mut Panel<'_>) + 'static) {
		let panel = Rc::new(panel);
		let mount: panels::Mount = Rc::new(move |ctx, entity| {
			let panel = Rc::clone(&panel);
			Box::pin(async move { panel(&mut Panel::new(ctx, entity)).await })
		});
		self.engine.ctx_mut().panels.push(panels::Registration {
			component: std::any::type_name::<T>(),
			title,
			mount,
		});
	}

	/// Opens or closes the editor on F5 and Escape, feeds this tick's pointer input and resizes into the editor's UI,
	/// lays it out, and sends the render when it changed.
	pub fn frame(&mut self, app: &mut GraphicsApplication) {
		while let Some(message) = self.windows.read() {
			// The editor frees the cursor of the game's first window.
			if self.window.is_none() {
				self.window = Some((message.handle(), message.data().features()));
			}
		}
		// A resource window's native id is known once the renderer opened it, which window events name it by.
		for resource in self.resources.iter_mut().filter(|resource| resource.id.is_none()) {
			resource.id = app.window_id(resource.window);
		}
		while let Some(event) = self.window_events.read() {
			let Event::Window { window: id, event } = event else {
				continue;
			};
			let resource = self.resources.iter().position(|resource| resource.id == Some(id));
			match (event, resource) {
				(Events::Resize { width, height }, resource) if width > 0 && height > 0 => {
					let engine = resource.map_or(&mut self.engine, |index| &mut self.resources[index].engine);
					engine.ctx_mut().viewport = Size::new(width, height);
				}
				// The application closed the window already; the editor lets go of what drew in it.
				(Events::Close, Some(index)) => {
					let resource = self.resources.remove(index);
					self.release(&resource);
				}
				_ => {}
			}
		}

		// What the shell edited in the last evaluation is posted before this frame's input, so a release ends the gesture
		// only after the gesture's last edit joined its step.
		self.post_edit(app);
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
			// A resource window's input acts on its own engine, which takes the main engine's place while it's handled.
			match self.resources.iter().position(|resource| Some(resource.window) == action.window) {
				Some(index) => {
					std::mem::swap(&mut self.engine, &mut self.resources[index].engine);
					self.act(app, action, false);
					std::mem::swap(&mut self.engine, &mut self.resources[index].engine);
				}
				None => self.act(app, action, true),
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
		self.fly_camera(app);
		self.place_gizmo();
		self.publish_drag();

		present(&mut self.engine, &mut self.renders, &mut self.published, app.frame_allocator());
		self.step_resources(app);
	}

	/// Writes what each resource window changed, lays it out, and sends its render when it changed. Closes a window
	/// whose last layer Escape popped, and every one once the editor closed.
	fn step_resources(&mut self, app: &mut GraphicsApplication) {
		self.stage.adopt();
		let open = self.engine.ctx().layers.open();
		let mut index = 0;
		while index < self.resources.len() {
			if !open || !self.resources[index].engine.ctx().layers.open() {
				let resource = self.resources.remove(index);
				self.close_windows.send(CloseWindow::new(resource.window));
				self.release(&resource);
				continue;
			}
			// Saving reads the window's own shell, which takes the main engine's place while it's written.
			std::mem::swap(&mut self.engine, &mut self.resources[index].engine);
			self.save_graph(app);
			self.edit_texture(app);
			self.edit_variant(app);
			std::mem::swap(&mut self.engine, &mut self.resources[index].engine);

			let resource = &mut self.resources[index];
			if resource.engine.ctx().copy.is_some()
				&& let Some(text) = resource.engine.ctx_mut().copy.take()
				&& let Err(error) = app.renderer_mut().set_clipboard_text(&text)
			{
				log::warn!("{error}");
			}
			// The preview follows the drag and scroll the panel took, fills the room the panel leaves it, and shows once
			// the renderer named its image.
			// The shell is written only when the panel took a drag or a scroll, or what the stage reports changed.
			if let Some(slot) = resource.preview {
				let shell = resource.engine.ctx();
				let (turn, zoom) = if shell.turn.is_some() || shell.zoom != 0.0 {
					let shell = resource.engine.ctx_mut();
					(shell.turn.take(), std::mem::take(&mut shell.zoom))
				} else {
					(None, 0.0)
				};
				let shell = resource.engine.ctx();
				self.stage.steer(slot, turn, zoom);
				self.stage.show_level(slot, shell.detail_level);
				let viewport = [shell.viewport.x(), shell.viewport.y()];
				let room = shell::open_room(viewport[0], shell.drawer_shown, shell.drawers);
				// The drawers make room while the subject is zoomed wider than the room between them.
				let width = self.stage.present(slot, viewport, (room[0] + room[1]) * 0.5);
				let open = shell::fits(width, viewport[0], shell.drawers);
				let image = self.stage.image(slot);
				if (open, image) != (shell.drawer_open, shell.preview) {
					let shell = resource.engine.ctx_mut();
					(shell.drawer_open, shell.preview) = (open, image);
				}
			}
			present(
				&mut resource.engine,
				&mut resource.renders,
				&mut resource.published,
				app.frame_allocator(),
			);
			index += 1;
		}
	}

	/// Frees the stage slot a closed resource window previewed in, for the next preview.
	fn release(&mut self, resource: &ResourceWindow) {
		if let Some(slot) = resource.preview {
			self.stage.close(slot);
		}
	}

	/// Opens `layer`, such as a material graph or a texture, in a window of its own titled `id`, the asset it shows,
	/// with `subject` on the stage for its main panel to show. The window draws only its own interface, which its
	/// engine lays out with `layer` at the bottom of its stack. An asset already open in a window doesn't open again.
	fn open_resource(&mut self, app: &mut GraphicsApplication, id: &str, layer: Layer, subject: Option<preview::Subject>) {
		if self.resources.iter().any(|resource| resource.asset == id) {
			return;
		}
		let extent = utils::Extent::rectangle(RESOURCE_WINDOW[0], RESOURCE_WINDOW[1]);
		let window = app
			.window_factory()
			.create(Window::new(id, extent).with_feature(Features::DECORATIONS));
		let renders = Factory::new();
		crate::application::graphics::setup_ui_render_pass(app, Some(window), "editor window", &renders, &[]);
		// The window reports its size in pixels once it opens, which replaces this first guess.
		let mut shell = Shell::new(Size::new(RESOURCE_WINDOW[0], RESOURCE_WINDOW[1]));
		shell.layers.push(layer);
		let mut engine = Engine::with_context(shell);
		engine.set_waker(app.waker().into());
		engine.mount(async |root| shell::window(root).await);
		let preview = subject.map(|subject| self.stage.open(app, subject, RESOURCE_WINDOW));
		self.resources.push(ResourceWindow {
			window,
			id: None,
			asset: id.to_owned(),
			engine,
			renders,
			published: None,
			preview,
		});
	}

	/// Answers one resolved action on `self.engine`: the main window's, or a resource window's while it takes its place.
	/// Only the main window, the `scene`, shows the game, so only its presses grab the gizmo or pick what they land on.
	fn act(&mut self, app: &mut GraphicsApplication, action: ResolvedAction, scene: bool) {
		let handle = action.action;
		if let (Value::Bool(down), Some(key)) = (action.value, self.actions.key(handle)) {
			self.press(app, key, down, scene);
			return;
		}
		match action.value {
			// The keys and the right-button drag fly the editor's camera while nothing over the scene takes them.
			Value::Vector3(push) if handle == self.actions.fly => {
				if scene && self.flies() {
					self.thrust = [push.x, push.y, push.z];
				}
			}
			Value::Vector2(v) if handle == self.actions.look => {
				let pointer = UiPoint::new(v.x, v.y);
				match action.phase {
					ActionPhase::Started => self.look = (scene && self.flies()).then_some(pointer),
					ActionPhase::Updated => {
						if let Some(last) = self.look.replace(pointer) {
							self.turned[0] += pointer.x - last.x;
							self.turned[1] += pointer.y - last.y;
						}
					}
					ActionPhase::Ended | ActionPhase::Cancelled => self.look = None,
				}
			}
			Value::Vector2(v) if handle == self.actions.drag => {
				let pointer = UiPoint::new(v.x, v.y);
				if scene {
					// What one press changes, until its release, is one step to undo.
					match action.phase {
						ActionPhase::Started => self.history.begin_gesture(),
						ActionPhase::Ended | ActionPhase::Cancelled => self.history.end_gesture(),
						ActionPhase::Updated => {}
					}
					self.pointer = pointer;
				}
				// A palette or a carried mesh on top of the scene sees the pointer first.
				if self.layer_takes(app, action.phase, pointer) {
					return;
				}
				match action.phase {
					ActionPhase::Started => {
						// A press the editor's panels don't take lands on the game view, where it grabs the gizmo
						// or picks a mesh.
						if !self.engine.press(pointer) && scene && !self.grab(pointer) {
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
				let pointer = UiPoint::new(v.x, v.y);
				self.engine.set_cursor_position(pointer);
				if scene {
					self.pointer = pointer;
					self.hover(pointer);
				}
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
			_ => {}
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
	/// held actions end. A closed one gives the window its own cursor capture back, and bypasses its pass once the
	/// scene drawer has slid out.
	fn sync_open(&mut self, app: &mut GraphicsApplication) {
		let open = self.engine.ctx().layers.open();
		// A closed editor's pass draws until the scene drawer has left, and then forwards the frame unchanged.
		if self.closing && self.engine.ctx().drawer_gone {
			self.closing = false;
			app.inspector()
				.update_configuration("render.pass.editor", ConfigurationValue::from("bypassed"));
		}
		if std::mem::replace(&mut self.layer.borrow_mut().open, open) == open {
			return;
		}
		if open {
			self.closing = false;
			app.inspector()
				.update_configuration("render.pass.editor", ConfigurationValue::from("enabled"));
		} else {
			self.closing = true;
		}
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
			self.land();
		}
	}

	/// Whether the fly keys and the right-button drag fly the camera: while the scene is the top layer, so a field
	/// taking text or the palette keeps the keys, no system key is held, so Cmd+A and Cmd+S stay shortcuts, and no
	/// gizmo drag is held, so Shift refines its snap instead of lowering the camera.
	fn flies(&mut self) -> bool {
		!self.held.contains(bindings::Key::Cmd)
			&& self.drag.is_none()
			&& matches!(self.engine.ctx().layers.top(), Some(Layer::Scene))
	}

	/// Moves the editor's camera on by this frame's keys and drag, switching the game window to it on its first move,
	/// and lets it glide to a stop once they're released.
	fn fly_camera(&mut self, app: &GraphicsApplication) {
		let now = Instant::now();
		// A long frame, such as the first after a pause, moves the camera no further than a short one.
		let dt = self.flown.map_or(0.0, |flown| now.duration_since(flown).as_secs_f32()).min(0.1);
		self.flown = Some(now);
		let thrust = std::mem::take(&mut self.thrust);
		let turned = std::mem::take(&mut self.turned);
		let steered = thrust != [0.0; 3] || turned != [0.0; 2];
		if steered {
			self.take_over(app);
		}
		let (Some((fly, _)), Some(camera)) = (&mut self.flight, self.camera) else {
			return;
		};
		if !steered && !fly.moving() {
			return;
		}
		fly.turn(turned);
		let transform = fly.step(thrust, dt);
		app.world()
			.channel::<TransformationUpdate>()
			.send(TransformationUpdate::new(camera, transform));
	}

	/// Switches the game window to the editor's camera, with the game camera's settings and standing where it stands, so
	/// the view doesn't jump. Does nothing while the window shows the editor's camera already, or before the scene lists
	/// a camera.
	fn take_over(&mut self, app: &GraphicsApplication) {
		let Some((window, _)) = self.window else {
			return;
		};
		if self.flight.is_some() {
			return;
		}
		let shell = self.engine.ctx_mut();
		let Some((game, settings)) = shell
			.scene
			.iter()
			.find_map(|entity| Some((entity, entity.camera.clone()?)))
		else {
			return;
		};
		// The editor's camera is made once and placed again each time it takes over.
		let camera = match self.camera {
			Some(camera) => {
				app.world().factory::<Camera>().derive(camera, settings);
				camera
			}
			None => {
				let camera = app.world().create(settings).handle();
				self.camera = Some(camera);
				camera
			}
		};
		let fly = fly::Fly::from_transform(&game.transform);
		app.world()
			.channel::<TransformationUpdate>()
			.send(TransformationUpdate::new(camera, fly.transform()));
		self.window_cameras.send(SetWindowCamera::new(window, camera));
		shell.view_camera = Some(camera);
		self.flight = Some((fly, game.handle));
	}

	/// Gives the game window its own camera back, so the editor opens on it next time.
	fn land(&mut self) {
		if let Some((_, game)) = self.flight.take()
			&& let Some((window, _)) = self.window
		{
			self.window_cameras.send(SetWindowCamera::new(window, game));
		}
		self.engine.ctx_mut().view_camera = None;
		self.look = None;
	}

	/// Asks the renderer which mesh the game's camera shows at `pointer`, in normalized window coordinates.
	///
	/// The answer arrives a few frames later; [`Self::adopt_hits`] selects it.
	fn pick(&mut self, app: &GraphicsApplication, pointer: UiPoint) {
		let shell = self.engine.ctx();
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

	/// Moves the highlight of the palette, or of the setting it found, by `delta` rows.
	fn move_highlight(&mut self, delta: isize) {
		match self.engine.ctx_mut().layers.top_mut() {
			Some(Layer::Palette(palette)) => palette.move_highlight(delta),
			Some(Layer::Setting(setting)) => setting.move_highlight(delta),
			_ => {}
		}
	}

	/// Picks the palette's highlighted entry, as a click on it does.
	fn pick_highlighted(&mut self, app: &mut GraphicsApplication) {
		let entry = self.engine.ctx_mut().layers.palette().and_then(|palette| palette.highlighted().cloned());
		if let Some(entry) = entry {
			self.pick_entry(app, &entry);
		}
	}

	/// What the editor is doing in the window whose engine is `self.engine`, the main one when `scene`, for the
	/// bindings to decide which apply.
	fn situation(&mut self, scene: bool) -> bindings::Situation<'_> {
		let dragging = self.drag.is_some();
		let shell = self.engine.ctx_mut();
		bindings::Situation {
			layers: &shell.layers,
			scene,
			dragging,
			tool: shell.tool,
		}
	}

	/// The one path every key the bindings listen for takes: it keeps [`Self::held`], lets a focused field hear Enter
	/// and Escape, measures a drag again when a held key changes how it snaps or carries, and runs what a press binds.
	fn press(&mut self, app: &mut GraphicsApplication, key: bindings::Key, down: bool, scene: bool) {
		self.held.set(key, down);
		// A resource window's engine stands in the main one's place while its input is handled, so the window the key
		// reached reads it too, as a mesh window's material names do Alt.
		let shell = self.engine.ctx_mut();
		shell.held = self.held;
		// Letting go of the modifiers ends the shortcut they started, run or not.
		if self.held.modifiers() == bindings::Held::default() {
			shell.finished = false;
		}
		let field_key = match key {
			bindings::Key::Enter => Some(Key::Enter),
			bindings::Key::Escape => Some(Key::Escape),
			_ => None,
		};
		if let Some(field_key) = field_key {
			self.engine.update_key_state(field_key, down);
		}
		if key.modifier() {
			// The dragged entity moves to the new snap at once, without waiting for the pointer to move.
			let pointer = self.pointer;
			self.drag_to(app, pointer);
			return;
		}
		if !down {
			return;
		}
		let held = self.held;
		if let Some(run) = bindings::pressed(&self.situation(scene), held, key) {
			self.engine.ctx_mut().finished = true;
			run(self, app);
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
			self.open_palette(app, scope);
		}
	}

	/// Routes the pointer's press, motion, and release to the top layer when it's a carried load or the palette, and
	/// returns whether it took them.
	///
	/// A pinned load is placed by the next press, a dragged one by its release. A press on a palette result picks it: it
	/// spawns once the pointer leaves the palette, and follows the pointer until the release, or, released on the same
	/// result, it's pinned. A press elsewhere closes the palette and goes on to the scene.
	fn layer_takes(&mut self, app: &mut GraphicsApplication, phase: ActionPhase, pointer: UiPoint) -> bool {
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
				if !palette::contains(viewport, shell.palette_height, pixel) {
					shell.layers.close(|layer| matches!(layer, Layer::Palette(_)));
					return false;
				}
				self.palette_press = palette::row_at(viewport, rows, pixel);
			}
			ActionPhase::Updated => {
				if let Some(row) = self.palette_press
					&& !palette::contains(viewport, shell.palette_height, pixel)
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
	fn pick_entry(&mut self, app: &mut GraphicsApplication, entry: &palette::Entry) {
		self.remember(entry);
		let layers = &mut self.engine.ctx_mut().layers;
		if let Some(setting) = palette::Setting::new(entry) {
			layers.push(Layer::Setting(setting));
			return;
		}
		// The palette gives way to what it opens.
		if let Some(Layer::Palette(palette)) = layers.pop()
			&& let palette::Scope::Folders(what) = &palette.scope
		{
			// A new material opens its graph on the canvas, to build its surface, and a new variant opens to change its
			// parameters.
			if let Some(next) = create::create(app, &entry.label, what) {
				self.choose(app, &next, false);
			}
			return;
		}
		self.choose(app, entry, false);
	}

	/// Opens the palette on `scope` over what's open, which offers new variants of the open graph's materials.
	fn open_palette(&mut self, app: &GraphicsApplication, scope: palette::Scope) {
		let layers = &mut self.engine.ctx_mut().layers;
		let entries = palette_entries(app, &scope, layers.graph().map(|graph| graph.id.as_str()));
		layers.push(Layer::Palette(palette::State::new(scope, entries, &self.recent)));
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
	fn choose(&mut self, app: &mut GraphicsApplication, entry: &palette::Entry, held: bool) {
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
			palette::Kind::Variant => {
				self.open_variant(app, id);
				return;
			}
			palette::Kind::MeshPreview => {
				self.open_mesh(app, id);
				return;
			}
			// A command's palette gives way to one on what it asks for next: the folder what it creates goes in, or
			// the material a variant specializes.
			palette::Kind::NewLevel | palette::Kind::NewMaterial | palette::Kind::NewVariant | palette::Kind::Material => {
				let scope = match entry.kind {
					palette::Kind::NewLevel => palette::Scope::Folders(palette::Create::Level),
					palette::Kind::NewMaterial => palette::Scope::Folders(palette::Create::Material),
					palette::Kind::NewVariant => palette::Scope::Materials,
					_ => palette::Scope::Folders(palette::Create::Variant(id.to_owned())),
				};
				return self.open_palette(app, scope);
			}
			// Only the folders palette lists folders, which picking an entry answers.
			palette::Kind::Folder => return,
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
			Ok(graph) => self.open_resource(app, id, Layer::Graph(graph), None),
			Err(error) => log::warn!("The editor couldn't open the material graph. {error}"),
		}
	}

	/// Writes the setting the texture's properties panel changed to the texture's sidecar, and reads back what the
	/// texture now bakes with, for the panel to show.
	fn edit_texture(&mut self, app: &GraphicsApplication) {
		if self.engine.ctx().texture_edit.is_none() {
			return;
		}
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

	/// Opens the material variant `id` in a window of its own, with its parent's parameters and the values it gives
	/// them.
	fn open_variant(&mut self, app: &mut GraphicsApplication, id: &str) {
		let opened = create::read_material(app, id)
			.and_then(|source| variant::Variant::open(id, source, |parent| create::read_material(app, parent)));
		match opened {
			Ok(variant) => self.open_resource(app, id, Layer::Variant(variant), Some(preview::Subject::Material(id))),
			Err(error) => log::warn!("The editor couldn't open the variant. {error}"),
		}
	}

	/// Writes the parameter the variant window changed to the variant's file, which the game then bakes again.
	fn edit_variant(&mut self, app: &GraphicsApplication) {
		if self.engine.ctx().variant_edit.is_none() {
			return;
		}
		let shell = self.engine.ctx_mut();
		let Some((name, value)) = shell.variant_edit.take() else {
			return;
		};
		let Some(variant) = shell.layers.variant_mut() else {
			return;
		};
		let source = variant.set(&name, value);
		if let Err(error) = app.inspector().write_source_asset(&variant.id, &source) {
			log::warn!("The editor couldn't save '{}'. {error}", variant.id);
		}
	}

	/// Opens the stored mesh `id` in a window of its own, on the stage, framed by the box that holds it.
	fn open_mesh(&mut self, app: &mut GraphicsApplication, id: &str) {
		let mesh = app.inspector().mesh(id).and_then(|resource| {
			let bounds = resource.bounding_box().ok_or_else(|| {
				"It has no primitives. The most likely cause is a model without geometry. Bake it again from a model that has some.".to_owned()
			})?;
			Ok(layers::Mesh { id: id.to_owned(), bounds, resource })
		});
		match mesh {
			Ok(mesh) => {
				let bounds = mesh.bounds;
				self.open_resource(app, id, Layer::Mesh(mesh), Some(preview::Subject::Mesh(id, bounds)));
			}
			Err(error) => log::warn!("The editor couldn't preview '{id}'. {error}"),
		}
	}

	/// Opens the texture source image `id` in the preview panel, in place of any texture it showed.
	///
	/// The image decodes on this thread, which holds the frame for as long as a large image takes to decode; it's
	/// what a click asked for, once.
	fn open_texture(&mut self, app: &mut GraphicsApplication, id: &str) {
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
				let texture = layers::Texture {
					id: id.to_owned(),
					preview,
					settings,
				};
				self.open_resource(app, id, Layer::Texture(texture), None);
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
		if !matches!(self.engine.ctx().layers.top(), Some(Layer::Carry(_))) {
			while self.rays.read().is_some() {}
			return;
		}
		let Some(carry) = self.engine.ctx_mut().layers.carry_mut() else {
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
			let payload = serde_json::to_string(&transform).expect(
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
		// The shell is read first and written only when it holds something to post, so a quiet frame leaves the
		// components that follow it asleep.
		let shell = self.engine.ctx();
		if shell.delete.is_none() && shell.posts.is_empty() && shell.edit.is_none() {
			return;
		}
		if let Some(handle) = self.engine.ctx_mut().delete.take() {
			// The deleted entity can't be dragged on.
			if self.drag.as_ref().is_some_and(|(dragged, _)| *dragged == handle) {
				self.release_gizmo();
			}
			if let Err(error) = app.inspector().post_message(DELETE_MESSAGE_TYPE, handle, "null") {
				log::warn!("The editor couldn't delete the entity. {error} Check that the game registers inspector messages.");
			}
		}
		// The panels' messages are taken out while they're posted, and put back to keep their storage.
		let mut posts = std::mem::take(&mut self.engine.ctx_mut().posts);
		for (entity, message_type, payload) in posts.drain(..) {
			if let Err(error) = app.inspector().post_message(message_type, entity, &payload) {
				log::warn!(
					"A panel's message didn't reach the game. {error} Register it with `DefaultInspector::register_message` before the panel posts it."
				);
			}
		}
		self.engine.ctx_mut().posts = posts;
		let shell = self.engine.ctx_mut();
		let Some((handle, change)) = shell.edit.take() else {
			return;
		};
		let Some(entity) = shell.scene.iter_mut().find(|entity| entity.handle == handle) else {
			return;
		};
		let (message_type, payload) = match change {
			fields::Change::Transform(transform) => {
				let delta = history::Delta::between(&entity.transform, &transform);
				self.edit(app, history::Operation::Transform { entity: handle, delta });
				return;
			}
			fields::Change::Camera(camera) => {
				let payload = serde_json::to_string(&camera);
				entity.camera = Some(camera);
				(CAMERA_UPDATE_MESSAGE_TYPE, payload)
			}
			fields::Change::Light(light) => {
				let payload = serde_json::to_string(&light.payload);
				let message_type = light.kind.update_message();
				entity.light = Some(light);
				(message_type, payload)
			}
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
		if !self.engine.ctx().save_graph {
			return;
		}
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
		// The shell is written only when a toast is posted.
		let engine = &mut self.engine;
		self.log_seen = log_capture::entries_since(self.log_seen, |entry| {
			if let LineKind::Log(level @ (log::Level::Warn | log::Level::Error)) = entry.kind
				&& open
			{
				engine.ctx_mut().toasts.post(level, entry.message());
			}
		});
	}


	/// Stands the gizmo at the selection while the editor is open, in the game camera's view, and hides it otherwise.
	fn place_gizmo(&mut self) {
		let open = self.layer.borrow().open;
		let shell = self.engine.ctx();
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
		let shell = self.engine.ctx();
		// Only the scene's own layer shows the gizmo to point at; anything over it takes the pointer.
		if self.drag.is_some() || !matches!(shell.layers.top(), Some(Layer::Scene)) {
			self.gizmo.borrow_mut().hovered = None;
			return;
		}
		let target = self.engine.ctx().selection().map(|entity| entity.transform.clone());
		let mut state = self.gizmo.borrow_mut();
		state.hovered = target
			.and_then(|target| state.placement(&target))
			.and_then(|placement| placement.pick(placement.pixel([pointer.x, pointer.y])));
	}

	/// Begins dragging the selection by the gizmo part under `pointer`, and returns whether the pointer was over one.
	fn grab(&mut self, pointer: UiPoint) -> bool {
		let Some(entity) = self.engine.ctx().selection() else {
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
		let held = self.held;
		let situation = self.situation(true);
		let holding = |binding| bindings::holding(binding, &situation, held);
		let snap = if holding(&bindings::SNAP_FINELY) {
			gizmo::Snap::Fine
		} else if holding(&bindings::SNAP) {
			gizmo::Snap::Steps
		} else {
			gizmo::Snap::Free
		};
		let carry = holding(&bindings::CARRY);
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
		let Some(transform) = drag.transform(&placement, pixel, snap) else {
			return;
		};
		state.stretch = drag.stretch(pixel, snap);
		let operation = history::Operation::Transform {
			entity: *handle,
			delta: history::Delta::between(&entity.transform, &transform),
		};
		let moved = transform.get_position() - entity.transform.get_position();
		drop(state);
		self.edit(app, operation);
		if carry {
			self.carry_camera(app, moved);
		}
	}

	/// Fills the hint bar with what would finish the action the main window's user left partway.
	///
	/// It writes the shell only when the drag moved, started, or ended, so components that follow it stay asleep
	/// while nothing changes.
	fn publish_drag(&mut self) {
		let dragged = self.drag.is_some().then_some([self.pointer.x, self.pointer.y]);
		if self.engine.ctx().dragged != dragged {
			self.engine.ctx_mut().dragged = dragged;
		}
	}

	/// Moves the editor's camera by `offset`, the step a gizmo drag just moved its entity, so the entity stays where
	/// it is on screen. Takes the game window over first, so the game's own camera never moves.
	fn carry_camera(&mut self, app: &GraphicsApplication, offset: Vector) {
		if offset == Vector::new(0.0, 0.0, 0.0) {
			return;
		}
		self.take_over(app);
		let (Some((fly, _)), Some(camera)) = (&mut self.flight, self.camera) else {
			return;
		};
		let transform = fly.carry(offset);
		app.world()
			.channel::<TransformationUpdate>()
			.send(TransformationUpdate::new(camera, transform));
	}

	/// Applies `operation` and records it, so it can be undone.
	fn edit(&mut self, app: &GraphicsApplication, operation: history::Operation) {
		self.history.record(operation);
		self.apply(app, operation);
	}

	/// Undoes the last step, or redoes the last undone one when `redo`, applying its operations as an edit does.
	fn step_history(&mut self, app: &GraphicsApplication, redo: bool) {
		// An undo mid-drag would fight the drag, which moves from where it started.
		self.release_gizmo();
		let mut operations = std::mem::take(&mut self.applying);
		if redo {
			operations.extend(self.history.redo());
		} else {
			operations.extend(self.history.undo());
		}
		for operation in operations.drain(..) {
			self.apply(app, operation);
		}
		self.applying = operations;
	}

	/// Applies `operation` to the editor's copy of its entity and posts it to the game: the one path edits, undo, and
	/// redo all change the game through. The copy changes at once, so the gizmo and the properties panel follow without
	/// waiting for the next scene poll.
	fn apply(&mut self, app: &GraphicsApplication, operation: history::Operation) {
		let history::Operation::Transform { entity: handle, delta } = operation;
		let shell = self.engine.ctx_mut();
		let Some(entity) = shell.scene.iter_mut().find(|entity| entity.handle == handle) else {
			log::warn!(
				"The editor couldn't change an entity it no longer lists. The most likely cause is that the game removed it. Undo or redo the next step instead."
			);
			return;
		};
		entity.transform = delta.applied_to(&entity.transform);
		let payload = serde_json::to_string(&entity.transform).expect(
			"Transform failed to serialize. The most likely cause is a non-finite component. Check the edited transform.",
		);
		if let Err(error) = app
			.inspector()
			.post_message(TRANSFORMATION_UPDATE_MESSAGE_TYPE, handle, &payload)
		{
			log::warn!("The editor couldn't move the entity. {error} Check that the game registers inspector messages.");
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
		// The editor's own camera isn't part of the scene.
		if let Some(camera) = self.camera {
			self.entities.retain(|entity| entity.handle() != camera);
		}
		let scene = scene::collect(&self.entities);
		if scene != self.engine.ctx().scene {
			let shell = self.engine.ctx_mut();
			shell.scene_revision += u64::from(scene::relisted(&shell.scene, &scene));
			shell.scene = scene;
		}
		true
	}

	/// Finds the rows the overview lists, when the scene was just read or the search changed, and has the list rebuilt
	/// when they changed.
	fn find_shown(&mut self, polled: bool) {
		let shell = self.engine.ctx();
		if !polled && shell.search == self.searched {
			return;
		}
		self.searched.clone_from(&shell.search);
		let shown = scene::shown(&self.entities, &shell.scene, &shell.search);
		if shown != shell.shown {
			let shell = self.engine.ctx_mut();
			shell.shown = shown;
			shell.scene_revision += 1;
		}
	}
}

/// Removes a carried mesh or light from the scene. A carried prefab hasn't spawned, so it only stops being carried.
fn remove_carried(app: &GraphicsApplication, carry: carry::Carry) {
	if let carry::Load::Spawned { root, .. } = carry.load
		&& let Err(error) = app.inspector().post_message(DELETE_MESSAGE_TYPE, root, "null")
	{
		log::warn!("The editor couldn't remove the carried mesh. {error} Check that the game registers inspector messages.");
	}
}

/// Gathers what the palette's `scope` finds: the node kinds for a graph, led by a new variant of each root material
/// the open `graph` computes; the runtime settings for settings; the resources to explore for exploring; the root
/// materials for a new variant; the source folders for what's being created; otherwise the stored meshes, the
/// prefabs, and every kind of light, and for everything, also the New level, New material, and New variant commands,
/// the resources to explore, and the runtime settings.
fn palette_entries(app: &GraphicsApplication, scope: &palette::Scope, graph: Option<&str>) -> Vec<palette::Entry> {
	use palette::{Entry, Kind, Scope};
	let settings = || app.inspector().configuration_parameters().into_iter().map(Entry::setting);
	let mut entries = Vec::new();
	match scope {
		Scope::Nodes => {
			if let Some(graph) = graph {
				let materials = create::root_materials(app).into_iter();
				entries.extend(materials.filter(|(_, of)| of == graph).map(|(material, _)| Entry {
					detail: "New variant".to_owned(),
					..Entry::new(material, Kind::Material)
				}));
			}
			let kinds = resource_management::bemx::kinds::KINDS.iter();
			entries.extend(kinds.map(|kind| Entry::new(kind.name, Kind::Node)));
			return entries;
		}
		Scope::Configure => return settings().collect(),
		Scope::Explore => {
			explorable(app, &mut entries);
			// Only the explorer opens meshes: everywhere else, a mesh is placed.
			match app.inspector().resources("Mesh") {
				Ok(meshes) => entries.extend(meshes.into_iter().map(|id| Entry::new(id, Kind::MeshPreview))),
				Err(error) => log::warn!("The palette couldn't list the game's meshes. {error}"),
			}
			return entries;
		}
		Scope::Materials => {
			let materials = create::root_materials(app).into_iter();
			return materials.map(|(material, _)| Entry::new(material, Kind::Material)).collect();
		}
		Scope::Folders(_) => {
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
	if *scope == Scope::All {
		entries.push(Entry::new("New level", Kind::NewLevel));
		entries.push(Entry::new("New material", Kind::NewMaterial));
		entries.push(Entry::new("New variant", Kind::NewVariant));
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
	entries.extend(create::variants(app).into_iter().map(|id| Entry::new(id, Kind::Variant)));
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
