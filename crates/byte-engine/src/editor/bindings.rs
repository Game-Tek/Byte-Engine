//! Every key the editor answers, in one table: what each press runs, which keys change a drag while held, and what the
//! hint bar offers, as Blender's status bar does.
//!
//! The editor tracks the keys held in [`Held`], runs a pressed key through [`pressed`], asks [`holding`] whether a
//! drag's keys are down, and asks [`offered`] what would finish an action left partway, which the hint bar shows once
//! the user stops there. Adding a key is adding a row to [`BINDINGS`], and the hint bar offers it from then on.

use crate::application::graphics::GraphicsApplication;
use crate::editor::Editor;
use crate::editor::gizmo::Tool;
use crate::editor::layers::{Layer, Layers};
use crate::editor::palette::Scope;

/// The `Key` enum names what a binding is pressed with: the keys the editor listens for, and the release that ends a
/// drag, which the hint bar names but the pointer's path answers.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Key {
	/// The shortcut modifier, which starts shortcuts and snaps a drag: the platform's own, Command on macOS and
	/// Control elsewhere; see [`ghi::window::input::Keys::ShortcutLeft`].
	Cmd,
	Shift,
	/// Option on macOS.
	Alt,
	K,
	A,
	E,
	F,
	G,
	Period,
	S,
	Z,
	Up,
	Down,
	Enter,
	Escape,
	/// The typed slash, which no physical key stands for: it sits on different keys in different layouts.
	Slash,
	Release,
}

impl Key {
	/// Every key the editor makes an action for, the ones with [`Self::inputs`].
	pub(crate) const LISTENED: [Key; 15] = [
		Key::Cmd,
		Key::Shift,
		Key::Alt,
		Key::K,
		Key::A,
		Key::E,
		Key::F,
		Key::G,
		Key::Period,
		Key::S,
		Key::Z,
		Key::Up,
		Key::Down,
		Key::Enter,
		Key::Escape,
	];

	/// The input sources the key's action binds, or none for a press another path answers: the release, or a typed
	/// character, which [`Self::typed`] names.
	pub(crate) fn inputs(self) -> &'static [&'static str] {
		match self {
			Key::Cmd => &["Keyboard.LeftShortcut", "Keyboard.RightShortcut"],
			Key::Shift => &["Keyboard.LeftShift", "Keyboard.RightShift"],
			Key::Alt => &["Keyboard.LeftAlt", "Keyboard.RightAlt"],
			Key::K => &["Keyboard.K"],
			Key::A => &["Keyboard.A"],
			Key::E => &["Keyboard.E"],
			Key::F => &["Keyboard.F"],
			Key::G => &["Keyboard.G"],
			Key::Period => &["Keyboard.Period"],
			Key::S => &["Keyboard.S"],
			Key::Z => &["Keyboard.Z"],
			Key::Up => &["Keyboard.Up"],
			Key::Down => &["Keyboard.Down"],
			Key::Enter => &["Keyboard.Enter"],
			Key::Escape => &["Keyboard.Escape"],
			Key::Slash | Key::Release => &[],
		}
	}

	/// The key's name on its keycap in the hint bar.
	pub(crate) fn label(self) -> &'static str {
		match self {
			Key::Cmd => ghi::window::input::SHORTCUT_NAME,
			Key::Shift => "Shift",
			Key::Alt => "Alt",
			Key::K => "K",
			Key::A => "A",
			Key::E => "E",
			Key::F => "F",
			Key::G => "G",
			Key::Period => ".",
			Key::S => "S",
			Key::Z => "Z",
			Key::Up => "Up",
			Key::Down => "Down",
			Key::Enter => "Enter",
			Key::Escape => "Esc",
			Key::Slash => "/",
			Key::Release => "Release",
		}
	}

	/// The key a typed `character` presses, for the keys the layout decides, or `None` for text that only types.
	pub(crate) fn typed(character: char) -> Option<Self> {
		match character {
			'/' => Some(Key::Slash),
			_ => None,
		}
	}

	/// Whether the key changes what others do, rather than doing something itself.
	pub(crate) fn modifier(self) -> bool {
		matches!(self, Key::Cmd | Key::Shift | Key::Alt)
	}

	fn bit(self) -> u32 {
		1 << self as u32
	}
}

/// The `Held` struct is the set of keys held down, which the editor keeps as their actions report presses and
/// releases.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct Held(u32);

impl Held {
	pub(crate) fn of(keys: &[Key]) -> Self {
		Self(keys.iter().fold(0, |bits, key| bits | key.bit()))
	}

	pub(crate) fn set(&mut self, key: Key, down: bool) {
		if down {
			self.0 |= key.bit();
		} else {
			self.0 &= !key.bit();
		}
	}

	pub(crate) fn contains(self, key: Key) -> bool {
		self.0 & key.bit() != 0
	}

	/// Only the held modifiers.
	pub(crate) fn modifiers(self) -> Self {
		Self(self.0 & Held::of(&[Key::Cmd, Key::Shift, Key::Alt]).0)
	}

	fn includes(self, other: Self) -> bool {
		self.0 & other.0 == other.0
	}
}

/// The `Situation` struct is what the editor is doing, which decides the bindings that apply.
#[derive(Clone, Copy)]
pub(crate) struct Situation<'a> {
	/// The layers of the window the keys reach.
	pub(crate) layers: &'a Layers,
	/// Whether that window is the main one, which shows the game.
	pub(crate) scene: bool,
	/// Whether a gizmo drag is under way.
	pub(crate) dragging: bool,
	/// The gizmo's tool.
	pub(crate) tool: Tool,
}

impl Situation<'_> {
	fn top(&self) -> Option<&Layer> {
		self.layers.top()
	}

	/// Whether the main window's scene takes the keys.
	fn on_scene(&self) -> bool {
		self.scene && matches!(self.top(), Some(Layer::Scene))
	}
}

/// What a binding runs on the editor when pressed.
type Run = fn(&mut Editor, &mut GraphicsApplication);

/// The `Binding` struct is one row of [`BINDINGS`]: keys held together, what they do, when, and what a press runs.
pub(crate) struct Binding {
	/// The keys, modifiers first, as the hint bar shows them.
	pub(crate) keys: &'static [Key],
	/// What the keys do, as the hint bar names it.
	pub(crate) label: &'static str,
	/// Whether the binding applies.
	when: fn(&Situation) -> bool,
	/// What a press of its last key runs, with exactly its modifiers held; `None` for keys a drag reads while held, and
	/// for gestures other paths answer.
	run: Option<Run>,
	/// Whether holding the key runs it again on each key repeat, as stepping through a list does.
	repeats: bool,
}

impl Binding {
	/// The binding's modifiers.
	fn modifiers(&self) -> Held {
		Held::of(self.keys).modifiers()
	}

	/// Whether `modifiers`, held, are a start of the binding's keys that its other keys finish.
	fn started_by(&self, modifiers: Held) -> bool {
		modifiers != Held::default() && (1..self.keys.len()).any(|count| Held::of(&self.keys[..count]) == modifiers)
	}
}

const fn shows(keys: &'static [Key], label: &'static str, when: fn(&Situation) -> bool) -> Binding {
	Binding {
		keys,
		label,
		when,
		run: None,
		repeats: false,
	}
}

const fn runs(keys: &'static [Key], label: &'static str, when: fn(&Situation) -> bool, run: Run) -> Binding {
	Binding {
		keys,
		label,
		when,
		run: Some(run),
		repeats: false,
	}
}

/// A binding that runs again on each key repeat while its key is held, for keys that step through a list.
const fn repeating(keys: &'static [Key], label: &'static str, when: fn(&Situation) -> bool, run: Run) -> Binding {
	Binding {
		repeats: true,
		..runs(keys, label, when, run)
	}
}

/// Snaps a gizmo drag to whole steps while held, as in Unity, which snaps with the platform's shortcut modifier.
pub(crate) static SNAP: Binding = shows(&[Key::Cmd], "Snap", dragging);
/// Snaps a gizmo drag to steps a tenth that size while held.
pub(crate) static SNAP_FINELY: Binding = shows(&[Key::Cmd, Key::Shift], "Snap finely", dragging);
/// Moves the camera along with the entity a Move drag carries while held.
pub(crate) static CARRY: Binding = shows(&[Key::Alt], "Bring the camera", |s| dragging(s) && s.tool == Tool::Move);

/// Every binding, in the order a press tries them and the hint bar lists them.
pub(crate) static BINDINGS: &[&Binding] = &[
	// A gizmo drag, and the keys that change it while held.
	&SNAP,
	&SNAP_FINELY,
	&CARRY,
	&shows(&[Key::Release], "Finish", dragging),
	&runs(
		&[Key::Escape],
		"Close the editor",
		|s| s.on_scene() && !s.dragging,
		|editor, app| editor.pop(app),
	),
	// The shortcuts Cmd starts; the palette refuses to open over a field taking text or a carried load.
	&runs(&[Key::Cmd, Key::K], "Search everything", palette_opens, |editor, app| {
		editor.toggle_palette(app, Scope::All)
	}),
	&runs(
		&[Key::Cmd, Key::A],
		"Add a node",
		|s| palette_opens(s) && s.layers.graph().is_some(),
		|editor, app| editor.toggle_palette(app, Scope::Place),
	),
	&runs(&[Key::Cmd, Key::A], "Place", palette_opens, |editor, app| {
		editor.toggle_palette(app, Scope::Place)
	}),
	&runs(&[Key::Cmd, Key::E], "Explore", palette_opens, |editor, app| {
		editor.toggle_palette(app, Scope::Explore)
	}),
	&runs(&[Key::Cmd, Key::F], "Find", palette_opens, |editor, app| {
		editor.toggle_palette(app, Scope::Entities)
	}),
	// A gym's waypoints are found over the gym, so Go to opens over one too.
	&runs(
		&[Key::Cmd, Key::G],
		"Go to",
		|s| palette_opens(s) || matches!(s.top(), Some(Layer::Gym(_))),
		|editor, app| editor.toggle_palette(app, Scope::Waypoints),
	),
	&runs(&[Key::Cmd, Key::Period], "Settings", palette_opens, |editor, app| {
		editor.toggle_palette(app, Scope::Configure)
	}),
	&runs(&[Key::Cmd, Key::Z], "Undo", undoes, |editor, app| {
		editor.step_history(app, false)
	}),
	&runs(&[Key::Cmd, Key::Shift, Key::Z], "Redo", undoes, |editor, app| {
		editor.step_history(app, true)
	}),
	&runs(
		&[Key::Cmd, Key::S],
		"Save",
		|s| s.layers.graph().is_some(),
		|editor, _| editor.engine.ctx_mut().save_graph = true,
	),
	// The scene list, whose selection steps through its rows as the palette's highlight does.
	&repeating(
		&[Key::Up],
		"Select previous",
		|s| s.on_scene(),
		|editor, _| editor.step_selection(-1),
	),
	&repeating(
		&[Key::Down],
		"Select next",
		|s| s.on_scene(),
		|editor, _| editor.step_selection(1),
	),
	&runs(
		&[Key::Slash],
		"Search the scene",
		|s| s.on_scene() && !s.dragging,
		|editor, _| editor.engine.ctx_mut().search_scene = true,
	),
	// The palette, and the setting it found.
	&repeating(&[Key::Up], "Previous", chooses, |editor, _| editor.move_highlight(-1)),
	&repeating(&[Key::Down], "Next", chooses, |editor, _| editor.move_highlight(1)),
	&runs(
		&[Key::Enter],
		"Pick",
		|s| matches!(s.top(), Some(Layer::Palette(_))),
		|editor, app| editor.pick_highlighted(app),
	),
	&runs(
		&[Key::Enter],
		"Set",
		|s| matches!(s.top(), Some(Layer::Setting(_))),
		|editor, app| editor.set_setting(app),
	),
	&runs(
		&[Key::Escape],
		"Close",
		|s| matches!(s.top(), Some(Layer::Palette(_))),
		|editor, app| editor.pop(app),
	),
	&runs(
		&[Key::Escape],
		"Back",
		|s| matches!(s.top(), Some(Layer::Setting(_))),
		|editor, app| editor.pop(app),
	),
	// A carried load, until a click places it; a field taking text answers Enter and Escape itself.
	&runs(
		&[Key::Escape],
		"Cancel",
		|s| matches!(s.top(), Some(Layer::Carry(_))),
		|editor, app| editor.pop(app),
	),
	// A gym being browsed for an asset.
	&runs(
		&[Key::Enter],
		"Place",
		|s| matches!(s.top(), Some(Layer::Gym(_))),
		|editor, app| editor.place_from_gym(app),
	),
	&runs(
		&[Key::Escape],
		"Back",
		|s| matches!(s.top(), Some(Layer::Gym(_))),
		|editor, app| editor.leave_gym(app),
	),
	// A resource window's own view.
	&runs(&[Key::Escape], "Close the window", resource_view, |editor, app| {
		editor.pop(app)
	}),
];

/// Whether a gizmo drag is under way in the scene, the only time its held keys mean anything.
fn dragging(s: &Situation) -> bool {
	s.on_scene() && s.dragging
}

/// Whether a Cmd shortcut can open the palette: over anything but a field taking text, a carried load, or a gym, and
/// not during a drag, whose held Cmd snaps.
fn palette_opens(s: &Situation) -> bool {
	!s.dragging && !matches!(s.top(), None | Some(Layer::Carry(_) | Layer::Typing | Layer::Gym(_)))
}

/// Whether the palette, or the setting it found, shows rows to choose between.
fn chooses(s: &Situation) -> bool {
	matches!(s.top(), Some(Layer::Palette(_) | Layer::Setting(_)))
}

/// Whether undo and redo apply: in the main window, while no field takes text and no drag is under way, whose held
/// Cmd snaps and whose release records it in the history.
fn undoes(s: &Situation) -> bool {
	s.scene && s.layers.open() && !s.dragging && !matches!(s.top(), Some(Layer::Typing))
}

/// Whether a resource window shows its own view, at the bottom of its layers.
fn resource_view(s: &Situation) -> bool {
	matches!(
		s.top(),
		Some(Layer::Graph(_) | Layer::Texture(_) | Layer::Variant(_) | Layer::Mesh(_))
	)
}

/// Returns what pressing `key` with `held` runs in `situation`: the first applying binding that ends in `key` and
/// whose modifiers are exactly the ones held, so Cmd+Shift+Z never also undoes. A key `repeat` runs it only when the
/// binding repeats.
pub(crate) fn pressed(situation: &Situation, held: Held, key: Key, repeat: bool) -> Option<Run> {
	BINDINGS.iter().find_map(|binding| {
		let fits = binding.keys.last() == Some(&key) && binding.modifiers() == held.modifiers() && (binding.when)(situation);
		binding.run.filter(|_| fits && (!repeat || binding.repeats))
	})
}

/// Whether the keys of `binding`, one a drag reads while held, are all down, and it applies in `situation`.
pub(crate) fn holding(binding: &Binding, situation: &Situation, held: Held) -> bool {
	held.includes(Held::of(binding.keys)) && (binding.when)(situation)
}

/// Fills `offered` with what would finish the action left partway in `situation` with `held`, which the hint bar shows
/// once the user stops there; empty when nothing is partway. A gizmo drag offers its held keys and its release, only
/// those that include the modifiers held once any are; held modifiers alone offer the shortcuts they start, so holding
/// Cmd lists what Cmd starts.
pub(crate) fn offered(situation: &Situation, held: Held, offered: &mut Vec<&'static Binding>) {
	let applying = BINDINGS.iter().copied().filter(|binding| (binding.when)(situation));
	let modifiers = held.modifiers();
	offered.clear();
	if dragging(situation) {
		offered.extend(applying.filter(|binding| binding.run.is_none() && binding.modifiers().includes(modifiers)));
	} else {
		offered.extend(applying.filter(|binding| binding.run.is_some() && binding.started_by(modifiers)));
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	fn layers(layer: Layer) -> Layers {
		let mut layers = Layers::default();
		layers.push(Layer::Scene);
		if !matches!(layer, Layer::Scene) {
			layers.push(layer);
		}
		layers
	}

	fn situation(layers: &Layers) -> Situation<'_> {
		Situation {
			layers,
			scene: true,
			dragging: false,
			tool: Tool::Move,
		}
	}

	fn labels(situation: &Situation, held: &[Key]) -> Vec<&'static str> {
		let mut found = Vec::new();
		offered(situation, Held::of(held), &mut found);
		found.iter().map(|binding| binding.label).collect()
	}

	#[test]
	fn a_press_runs_the_binding_with_exactly_its_modifiers() {
		let layers = layers(Layer::Scene);
		let s = situation(&layers);
		let undo = pressed(&s, Held::of(&[Key::Cmd]), Key::Z, false);
		let redo = pressed(&s, Held::of(&[Key::Cmd, Key::Shift]), Key::Z, false);
		assert!(undo.is_some() && redo.is_some());
		assert_ne!(undo.map(|run| run as usize), redo.map(|run| run as usize));
		assert!(pressed(&s, Held::default(), Key::Z, false).is_none());
	}

	#[test]
	fn a_held_key_repeats_only_the_bindings_that_step_through_a_list() {
		let scene = layers(Layer::Scene);
		let s = situation(&scene);
		assert!(pressed(&s, Held::default(), Key::Down, true).is_some(), "Select next repeats");
		assert!(pressed(&s, Held::of(&[Key::Cmd]), Key::Z, true).is_none(), "Undo doesn't");
		let palette = layers(Layer::Palette(Default::default()));
		let s = situation(&palette);
		assert!(pressed(&s, Held::default(), Key::Up, true).is_some(), "Previous repeats");
		assert!(pressed(&s, Held::default(), Key::Enter, true).is_none(), "Pick doesn't");
		assert!(pressed(&s, Held::default(), Key::Enter, false).is_some());
	}

	#[test]
	fn a_typed_slash_searches_the_scene_only_while_nothing_takes_text() {
		assert_eq!(Key::typed('/'), Some(Key::Slash));
		assert_eq!(Key::typed('a'), None, "other text only types");
		let scene = layers(Layer::Scene);
		assert!(pressed(&situation(&scene), Held::default(), Key::Slash, false).is_some());
		for layer in [Layer::Typing, Layer::Palette(Default::default())] {
			let over = layers(layer);
			assert!(
				pressed(&situation(&over), Held::default(), Key::Slash, false).is_none(),
				"a field taking text types the slash"
			);
		}
	}

	#[test]
	fn a_drag_reads_its_held_keys() {
		let layers = layers(Layer::Scene);
		let dragging = Situation {
			dragging: true,
			..situation(&layers)
		};
		assert!(holding(&SNAP, &dragging, Held::of(&[Key::Cmd])));
		assert!(!holding(&SNAP_FINELY, &dragging, Held::of(&[Key::Cmd])));
		assert!(holding(&SNAP_FINELY, &dragging, Held::of(&[Key::Cmd, Key::Shift])));
		let turning = Situation {
			tool: Tool::Rotate,
			..dragging
		};
		assert!(!holding(&CARRY, &turning, Held::of(&[Key::Alt])));
	}

	#[test]
	fn only_an_action_left_partway_offers_keys() {
		let layers = layers(Layer::Scene);
		let s = situation(&layers);
		assert!(labels(&s, &[]).is_empty(), "nothing is partway at rest");
		// Shift alone starts nothing; it only finishes Cmd+Shift+Z.
		assert!(labels(&s, &[Key::Shift]).is_empty());
		assert!(!labels(&s, &[Key::Cmd]).contains(&"Snap"), "Cmd snaps only during a drag");

		let shortcuts = labels(&s, &[Key::Cmd]);
		assert!(
			shortcuts.contains(&"Search everything") && shortcuts.contains(&"Redo"),
			"{shortcuts:?}"
		);
		assert_eq!(labels(&s, &[Key::Cmd, Key::Shift]), ["Redo"]);

		let dragging = Situation { dragging: true, ..s };
		assert_eq!(labels(&dragging, &[]), ["Snap", "Snap finely", "Bring the camera", "Finish"]);
		assert_eq!(labels(&dragging, &[Key::Cmd, Key::Shift]), ["Snap finely"]);
	}

	#[test]
	fn a_field_taking_text_keeps_the_shortcuts() {
		let layers = layers(Layer::Typing);
		let s = situation(&layers);
		assert!(pressed(&s, Held::of(&[Key::Cmd]), Key::K, false).is_none());
		assert!(labels(&s, &[Key::Cmd]).is_empty());
	}
}
