//! The editor's share of the game's input.
//!
//! The editor adds an input layer to the game's collector, so it pulls before the game's actions resolve. While the
//! editor is open, the layer captures everything, so the game sees no input at all; while it's closed, it passes
//! everything, and only watches for the key that opens it.

use std::cell::RefCell;
use std::rc::Rc;

use crate::application::graphics::GraphicsApplication;
use crate::input::{ActionBindingDescription, ActionHandle, Capture, InputSink, ResolvedAction, TickPolicy, Types};

/// The `Actions` struct names the editor's actions, so [`crate::editor::Editor::frame`] can tell the resolved ones apart.
pub(crate) struct Actions {
	pub(crate) pointer: ActionHandle,
	pub(crate) click: ActionHandle,
	pub(crate) scroll: ActionHandle,
	/// Keeps press, motion, and release in order for one pointer.
	pub(crate) drag: ActionHandle,
	/// Opens and closes the editor.
	pub(crate) toggle: ActionHandle,
	/// Typed text, for the focused text field.
	pub(crate) character: ActionHandle,
	/// Erases backward in the focused text field.
	pub(crate) backspace: ActionHandle,
	/// Either system key: Command on macOS, Windows or Super elsewhere. With K, it opens the palette on everything;
	/// with A, on what can be placed; with the period, on the runtime settings; with S, it saves the open material
	/// graph.
	pub(crate) meta: ActionHandle,
	pub(crate) k: ActionHandle,
	pub(crate) a: ActionHandle,
	pub(crate) period: ActionHandle,
	pub(crate) s: ActionHandle,
	pub(crate) up: ActionHandle,
	pub(crate) down: ActionHandle,
	pub(crate) enter: ActionHandle,
	pub(crate) escape: ActionHandle,
}

/// The `Layer` struct is the state the editor's input layer shares with the editor, which reads what the layer
/// resolved and decides whether it's open.
#[derive(Default)]
pub(crate) struct Layer {
	/// Whether the layer captures the game's input.
	pub(crate) open: bool,
	/// The actions the layer resolved since the editor last read them, in arrival order.
	pub(crate) resolved: Vec<ResolvedAction>,
}

/// Adds the editor's input layer to `app` and returns its actions and the state it shares.
pub(crate) fn install(app: &mut GraphicsApplication) -> (Actions, Rc<RefCell<Layer>>) {
	// The editor's actions are its own, so nothing else listens for them.
	let channel = app.messages().channel();
	let (collector, _) = app.actions_mut();
	let mut sink = InputSink::new(collector.add_sink(), channel);
	let mut action = |name: &'static str, bindings: &[ActionBindingDescription], kind: Types| {
		sink.create_action(collector, name, kind, bindings, TickPolicy::OnChange)
	};
	let actions = Actions {
		pointer: action(
			"EditorPointer",
			&[ActionBindingDescription::new("Pointer.Position")],
			Types::Vector2,
		),
		click: action(
			"EditorClick",
			&[ActionBindingDescription::new("Pointer.LeftButton")],
			Types::Boolean,
		),
		scroll: action(
			"EditorScroll",
			&[ActionBindingDescription::new("Pointer.Scroll")],
			Types::Float,
		),
		drag: action(
			"EditorDrag",
			&[ActionBindingDescription::new("Pointer.Position").dragged_by("Pointer.LeftButton")],
			Types::Vector2,
		),
		toggle: action(
			"EditorToggle",
			&[ActionBindingDescription::new("Keyboard.F5")],
			Types::Boolean,
		),
		character: action(
			"EditorCharacter",
			&[ActionBindingDescription::new("Keyboard.Character")],
			Types::Unicode,
		),
		backspace: action(
			"EditorBackspace",
			&[ActionBindingDescription::new("Keyboard.Backspace")],
			Types::Boolean,
		),
		meta: action(
			"EditorMeta",
			&[
				ActionBindingDescription::new("Keyboard.LeftMeta"),
				ActionBindingDescription::new("Keyboard.RightMeta"),
			],
			Types::Boolean,
		),
		k: action("EditorK", &[ActionBindingDescription::new("Keyboard.K")], Types::Boolean),
		a: action("EditorA", &[ActionBindingDescription::new("Keyboard.A")], Types::Boolean),
		period: action(
			"EditorPeriod",
			&[ActionBindingDescription::new("Keyboard.Period")],
			Types::Boolean,
		),
		s: action("EditorS", &[ActionBindingDescription::new("Keyboard.S")], Types::Boolean),
		up: action("EditorUp", &[ActionBindingDescription::new("Keyboard.Up")], Types::Boolean),
		down: action(
			"EditorDown",
			&[ActionBindingDescription::new("Keyboard.Down")],
			Types::Boolean,
		),
		enter: action(
			"EditorEnter",
			&[ActionBindingDescription::new("Keyboard.Enter")],
			Types::Boolean,
		),
		escape: action(
			"EditorEscape",
			&[ActionBindingDescription::new("Keyboard.Escape")],
			Types::Boolean,
		),
	};

	let layer = Rc::new(RefCell::new(Layer::default()));
	let shared = Rc::clone(&layer);
	app.add_input_layer(move |collector| {
		let mut layer = shared.borrow_mut();
		let Layer { open, resolved } = &mut *layer;
		sink.pull(collector, |action| {
			resolved.push(*action);
			Capture::when(*open)
		});
		// While open, the game sees nothing, not even keys the editor has no action for.
		if *open {
			sink.capture_pending(collector);
		}
	});
	(actions, layer)
}
