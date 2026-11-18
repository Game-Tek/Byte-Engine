//! The editor's share of the game's input.
//!
//! The editor adds an input layer to the game's collector, so it pulls before the game's actions resolve. While the
//! editor is open, the layer captures everything, so the game sees no input at all; while it's closed, it passes
//! everything, and only watches for the key that opens it.

use std::cell::RefCell;
use std::rc::Rc;

use crate::application::graphics::GraphicsApplication;
use crate::constants::{forward, right, up};
use crate::input::{ActionBindingDescription, ActionHandle, Axis3, Capture, InputSink, ResolvedAction, TickPolicy, Types};

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
	/// with A, on what can be placed; with E, on the resources to explore; with the period, on the runtime settings;
	/// with S, it saves the open material graph; with Z, it undoes, and with Shift and Z, it redoes.
	pub(crate) meta: ActionHandle,
	/// Either Shift key, which turns Cmd+Z's undo into redo, and a snapped gizmo drag's steps into fine ones.
	pub(crate) shift: ActionHandle,
	/// Either Control key, which snaps a gizmo drag to whole steps while held, as in Blender, on every platform.
	pub(crate) control: ActionHandle,
	/// Either Alt key, Option on macOS, which turns a click on a material's name into copying its resource id.
	pub(crate) alt: ActionHandle,
	pub(crate) k: ActionHandle,
	pub(crate) a: ActionHandle,
	pub(crate) e: ActionHandle,
	pub(crate) period: ActionHandle,
	pub(crate) s: ActionHandle,
	pub(crate) z: ActionHandle,
	pub(crate) up: ActionHandle,
	pub(crate) down: ActionHandle,
	pub(crate) enter: ActionHandle,
	pub(crate) escape: ActionHandle,
	/// W, A, S, D, Space, and the left Shift held, as a push right, up, and forward along the editor camera's view, the
	/// model viewer's keys. It repeats every tick while a key is held.
	pub(crate) fly: ActionHandle,
	/// The pointer dragged with the right button, which turns the editor camera.
	pub(crate) look: ActionHandle,
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
	let push = |key: &'static str, direction: crate::math::UnitVector| {
		ActionBindingDescription::new(key).mapped(Axis3::from(direction).into())
	};
	let fly = sink.create_action(
		collector,
		"EditorFly",
		Types::Vector3,
		&[
			push("Keyboard.W", forward()),
			push("Keyboard.S", -forward()),
			push("Keyboard.A", -right()),
			push("Keyboard.D", right()),
			push("Keyboard.Space", up()),
			push("Keyboard.LeftShift", -up()),
		],
		TickPolicy::WhileActive,
	);
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
		shift: action(
			"EditorShift",
			&[
				ActionBindingDescription::new("Keyboard.LeftShift"),
				ActionBindingDescription::new("Keyboard.RightShift"),
			],
			Types::Boolean,
		),
		control: action(
			"EditorControl",
			&[
				ActionBindingDescription::new("Keyboard.LeftControl"),
				ActionBindingDescription::new("Keyboard.RightControl"),
			],
			Types::Boolean,
		),
		alt: action(
			"EditorAlt",
			&[
				ActionBindingDescription::new("Keyboard.LeftAlt"),
				ActionBindingDescription::new("Keyboard.RightAlt"),
			],
			Types::Boolean,
		),
		k: action("EditorK", &[ActionBindingDescription::new("Keyboard.K")], Types::Boolean),
		a: action("EditorA", &[ActionBindingDescription::new("Keyboard.A")], Types::Boolean),
		e: action("EditorE", &[ActionBindingDescription::new("Keyboard.E")], Types::Boolean),
		period: action(
			"EditorPeriod",
			&[ActionBindingDescription::new("Keyboard.Period")],
			Types::Boolean,
		),
		s: action("EditorS", &[ActionBindingDescription::new("Keyboard.S")], Types::Boolean),
		z: action("EditorZ", &[ActionBindingDescription::new("Keyboard.Z")], Types::Boolean),
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
		fly,
		look: action(
			"EditorLook",
			&[ActionBindingDescription::new("Pointer.Position").dragged_by("Pointer.RightButton")],
			Types::Vector2,
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
