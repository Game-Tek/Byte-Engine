//! The command palette: a floating search over what the editor can do, as VS Code's is.
//!
//! Each shortcut opens it on a [`Scope`]: Cmd+K on everything, Cmd+A on what can be placed in the scene, Cmd+E on the
//! resources to explore, Cmd+. on the game's runtime settings, and any of them on the node kinds while a material graph
//! is open. The Browse gym command opens it on the levels, and picking one opens it as a gym to pick an asset from;
//! see [`crate::editor::gym`]. A variant's texture parameter opens it on the textures, and picking one gives the
//! parameter that texture.
//! A placeable [`Entry`] is dragged into the scene or pinned to the pointer until a click places it; a
//! material graph opens on the canvas, a texture in a preview panel, and a node kind is added to the open graph. A setting opens a [`Setting`] over the palette, which shows its value in
//! the search field, where a typed value replaces it.
//!
//! This module holds what the palette shows and where it is on screen. The palette is a layer of the editor's
//! [`Layers`](crate::editor::layers::Layers), which the shell draws it from; the editor turns presses, keys, and
//! typing into changes of it, and acting on the chosen entry is the editor's job.

use crate::configuration::{Accepts, Parameter};
use crate::editor::carry::Light;
use crate::inspector::fuzzy_find;
use crate::ui::Size;
use crate::ui::TextEdit;

/// The palette's width, in pixels.
pub(crate) const WIDTH: f32 = 520.0;
/// The gap between the window's top edge and the palette.
pub(crate) const TOP: f32 = 120.0;
/// The gap between the panel's edge and its content.
pub(crate) const INSET: f32 = 8.0;
/// The search field's height.
pub(crate) const FIELD_HEIGHT: f32 = 32.0;
/// The gap between the field and the first row.
pub(crate) const GAP: f32 = 6.0;
/// One result's height.
pub(crate) const ROW_HEIGHT: f32 = 30.0;
/// The most results the palette shows at once.
pub(crate) const MAX_ROWS: usize = 8;

/// The `Scope` enum is which of the palette's versions is open, picked by the shortcut that opened it.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) enum Scope {
	/// Everything the palette finds, opened with Cmd+K.
	#[default]
	All,
	/// What can be placed in the scene, opened with Cmd+A.
	Place,
	/// The resources to open and look at, material graphs, variants, meshes, and textures, opened with Cmd+E.
	Explore,
	/// The game's runtime settings, opened with Cmd+.
	Configure,
	/// The node kinds to add to the open material graph.
	Nodes,
	/// The root materials a new variant can specialize, opened by the New variant command.
	Materials,
	/// The folders what's being created can go in, opened by the New level, New material, and New variant commands.
	Folders(Create),
	/// The textures the open variant's `Texture2D` parameter this names can show, opened by clicking its value.
	Textures(String),
	/// The levels to browse as a gym, opened by the Browse gym command.
	Gyms,
}

/// The `Create` enum is what the folders palette creates in the folder it finds.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Create {
	/// An empty level.
	Level,
	/// A root material with an empty graph of its own.
	Material,
	/// A variant of the root material this names, which starts with the material's values.
	Variant(String),
}

impl Scope {
	/// What the search field says while it's empty.
	pub(crate) fn prompt(&self) -> &'static str {
		match self {
			Self::All => "Find something to place, a resource to open, or a setting",
			Self::Place => "Find a mesh, prefab, or light to place",
			Self::Explore => "Find a material, variant, or texture to open",
			Self::Configure => "Find a setting to show or change",
			Self::Nodes => "Find a node to add",
			Self::Materials => "Find the material the new variant specializes",
			Self::Folders(Create::Level) => "Find the folder the new level goes in, or type a new one",
			Self::Folders(Create::Material) => "Find the folder the new material goes in, or type a new one",
			Self::Folders(Create::Variant(_)) => "Find the folder the new variant goes in, or type a new one",
			Self::Textures(_) => "Find the texture the parameter shows",
			Self::Gyms => "Find the level to pick an asset from",
		}
	}
}

/// The `Entry` struct is one thing the palette finds: what its row says, which the search matches, and what it is.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Entry {
	/// A resource's or source's ID, a node kind's name, a light's name, or a setting's name.
	pub(crate) label: String,
	pub(crate) kind: Kind,
	/// What the row says after the name, such as a setting's value; empty for most entries.
	pub(crate) detail: String,
}

/// The `Kind` enum is what choosing an entry does.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum Kind {
	/// Places the mesh resource the label names.
	Mesh,
	/// Places the prefab the label names.
	Prefab,
	/// Places a light of this kind.
	Light(&'static Light),
	/// Opens the material graph source the label names on the canvas.
	Graph,
	/// Previews the texture source image the label names.
	Texture,
	/// Previews the stored mesh the label names, which the explorer opens instead of placing it.
	MeshPreview,
	/// Opens the material variant the label names, to change its parameters.
	Variant,
	/// Adds a node of the kind the label names to the open graph.
	Node,
	/// Shows the runtime setting the label names, which takes these values, to change it.
	Setting(Accepts),
	/// Lists the folders a new level can go in.
	NewLevel,
	/// Lists the folders a new material can go in.
	NewMaterial,
	/// Lists the root materials a new variant can specialize.
	NewVariant,
	/// Lists the folders a new variant of the root material the label names can go in.
	Material,
	/// Writes what the folders palette creates into the folder the label names, creating the folder when it doesn't
	/// exist.
	Folder,
	/// Lists the levels to browse as a gym.
	BrowseGym,
	/// Opens the level the label names as a gym, to pick an asset from.
	Gym,
}

impl Kind {
	/// Whether choosing the entry places something in the scene, so it can be dragged out of the palette.
	pub(crate) fn places(self) -> bool {
		matches!(self, Self::Mesh | Self::Prefab | Self::Light(_))
	}
}

impl Entry {
	pub(crate) fn new(label: impl Into<String>, kind: Kind) -> Self {
		Self {
			label: label.into(),
			kind,
			detail: String::new(),
		}
	}

	/// The entry of a runtime setting, which shows its value.
	pub(crate) fn setting(parameter: Parameter) -> Self {
		Self {
			detail: parameter.value.to_string(),
			label: parameter.name,
			kind: Kind::Setting(parameter.accepts),
		}
	}

	/// The name the row leads with: a resource's file name without its extension, or the whole label otherwise.
	pub(crate) fn name(&self) -> &str {
		match self.kind {
			Kind::Mesh
			| Kind::MeshPreview
			| Kind::Prefab
			| Kind::Graph
			| Kind::Texture
			| Kind::Variant
			| Kind::Material
			| Kind::Gym => entity_name(&self.label),
			Kind::Light(_)
			| Kind::Node
			| Kind::Setting(_)
			| Kind::NewLevel
			| Kind::NewMaterial
			| Kind::NewVariant
			| Kind::Folder
			| Kind::BrowseGym => &self.label,
		}
	}
}

/// The `State` struct is what the palette shows, which the editor changes and the shell draws.
#[derive(Default)]
pub(crate) struct State {
	/// Which version of the palette is open.
	pub(crate) scope: Scope,
	/// What the search field holds.
	pub(crate) search: String,
	/// Everything the open scope finds, gathered when it opened.
	pub(crate) entries: Vec<Entry>,
	/// The entries used most recently, newest first, as indices into `entries`; an empty search lists them first.
	recent: Vec<usize>,
	/// The entries the search finds, best first, at most [`MAX_ROWS`].
	pub(crate) results: Vec<Entry>,
	/// The result Enter picks.
	pub(crate) highlight: usize,
	/// Advanced whenever the results or the highlight change, so the shell redraws the rows.
	pub(crate) revision: u64,
}

impl State {
	/// The palette's `scope` over `entries`, with an empty search, which lists those of `recent` it finds first.
	/// `recent` is what was used most recently in any scope, newest first, by label and kind.
	pub(crate) fn new(scope: Scope, entries: Vec<Entry>, recent: &[(String, Kind)]) -> Self {
		// Only what this scope finds is listed: a recent mesh isn't a setting's recent.
		let recent = recent
			.iter()
			.filter_map(|(label, kind)| entries.iter().position(|entry| entry.label == *label && entry.kind == *kind))
			.collect();
		let mut state = Self {
			scope,
			entries,
			recent,
			..Self::default()
		};
		state.find();
		state
	}

	/// Finds the results for the search, as the console finds entities by name, and highlights the best one. An
	/// empty search lists the recently used entries first, then the rest in order.
	pub(crate) fn find(&mut self) {
		let found: Vec<usize> = if self.search.trim().is_empty() {
			let rest = (0..self.entries.len()).filter(|index| !self.recent.contains(index));
			self.recent.iter().copied().chain(rest).collect()
		} else {
			let labels: Vec<&str> = self.entries.iter().map(|entry| entry.label.as_str()).collect();
			fuzzy_find(&labels, &self.search)
		};
		self.results = found
			.into_iter()
			.take(MAX_ROWS)
			.map(|index| self.entries[index].clone())
			.collect();
		if matches!(self.scope, Scope::Folders(_))
			&& let Some(folder) = typed_folder(&self.search)
			&& !self.entries.iter().any(|entry| entry.label == folder)
		{
			// A folder that doesn't exist yet is offered too; writing the level creates it. It leads while what's
			// typed can't still grow into a listed folder's name, so Enter takes it, and trails the matches otherwise.
			let leads = !self
				.entries
				.iter()
				.any(|entry| entry.label.starts_with(folder.trim_end_matches('/')));
			let entry = Entry {
				detail: "New folder".to_owned(),
				..Entry::new(folder, Kind::Folder)
			};
			self.results.truncate(MAX_ROWS - 1);
			let at = if leads { 0 } else { self.results.len() };
			self.results.insert(at, entry);
		}
		self.highlight = 0;
		self.revision += 1;
	}

	/// Moves the highlight by `step` results, turning around past the first and the last.
	pub(crate) fn move_highlight(&mut self, step: isize) {
		self.highlight = wrapped(self.highlight, step, self.results.len());
		self.revision += 1;
	}

	/// The highlighted result, if there is any.
	pub(crate) fn highlighted(&self) -> Option<&Entry> {
		self.results.get(self.highlight)
	}
}

/// The row `step` rows from `row` in a list of `len` rows, going on from the other end past either one; 0 for an
/// empty list.
fn wrapped(row: usize, step: isize, len: usize) -> usize {
	if len == 0 {
		return 0;
	}
	// A list in memory holds fewer rows than `isize::MAX`.
	(row as isize + step).rem_euclid(len as isize) as usize
}

/// The folder ID that `search` names, such as `levels/maps/` for ` /levels/maps `, or `None` when it names none:
/// when it's empty or the asset folder itself, or has an empty, `.`, or `..` part.
fn typed_folder(search: &str) -> Option<String> {
	let path = search.trim().trim_matches('/');
	let valid = !path.is_empty()
		&& path
			.split('/')
			.all(|part| !part.trim().is_empty() && part != "." && part != "..");
	valid.then(|| format!("{path}/"))
}

/// The `Setting` struct is a runtime setting being given a new value, over the palette that found it: the field
/// holds the value, and the rows below it list the values a setting with a closed set takes, or say what an open one
/// takes. Enter sets the highlighted value, or the typed one.
pub(crate) struct Setting {
	/// The setting's full name.
	pub(crate) name: String,
	/// What the field holds: the setting's value when it opened, then what's typed.
	pub(crate) value: String,
	/// The rows under the field: the values that start with what's typed, for a setting that takes a closed set of
	/// words, or one row saying what the setting takes otherwise.
	pub(crate) rows: Vec<Entry>,
	/// The listed value Enter picks, among `rows`; `None` when the setting takes typed values.
	pub(crate) highlight: Option<usize>,
	/// Advanced whenever the rows or the highlight change, so the shell redraws them.
	pub(crate) revision: u64,
	accepts: Accepts,
	/// Whether anything was typed yet; until then, the value shows as if selected, and typing replaces it.
	typed: bool,
}

impl Setting {
	/// The setting `entry` names, its value typed in to be replaced and its current value highlighted when it takes a
	/// closed set, or `None` when the entry isn't a setting.
	pub(crate) fn new(entry: &Entry) -> Option<Self> {
		let Kind::Setting(accepts) = entry.kind else {
			return None;
		};
		let mut setting = Self {
			name: entry.label.clone(),
			value: entry.detail.clone(),
			rows: Vec::new(),
			highlight: None,
			revision: 0,
			accepts,
			typed: false,
		};
		setting.list();
		// The value it opened with is the one highlighted, so Enter alone keeps it.
		let current = setting
			.rows
			.iter()
			.position(|row| row.label.eq_ignore_ascii_case(&setting.value));
		setting.highlight = setting.highlight.and(current.or(Some(0)));
		Some(setting)
	}

	/// Applies a keystroke to the value. The first one replaces the value it opened with, as typing over a selection
	/// does: a character starts a new value, and erasing clears it. A closed set lists the values that start with it.
	pub(crate) fn type_edit(&mut self, edit: TextEdit) {
		if !std::mem::replace(&mut self.typed, true) {
			self.value.clear();
		}
		if !matches!(edit, TextEdit::Deleted(_)) || !self.value.is_empty() {
			edit.apply_to(&mut self.value);
		}
		self.list();
	}

	/// Moves the highlight by `step` listed values, turning around past the first and the last.
	pub(crate) fn move_highlight(&mut self, step: isize) {
		if let Some(highlight) = &mut self.highlight {
			*highlight = wrapped(*highlight, step, self.rows.len());
			self.revision += 1;
		}
	}

	/// Picks the listed value at `row`, as a click on it does.
	pub(crate) fn pick(&mut self, row: usize) {
		if self.highlight.is_some() && row < self.rows.len() {
			self.highlight = Some(row);
		}
	}

	/// The value Enter sets: the highlighted listed value, or what's typed when none is listed.
	pub(crate) fn chosen(&self) -> &str {
		match self.highlight.and_then(|row| self.rows.get(row)) {
			Some(row) => &row.label,
			None => self.value.trim(),
		}
	}

	/// Lists the rows for the field's value: the closed set's words that start with it, the first highlighted, or the
	/// row that says what an open setting takes.
	fn list(&mut self) {
		self.revision += 1;
		let words = match self.accepts {
			// Only a setting whose every value is a word has a closed set; one that also takes numbers is typed.
			Accepts::Bool | Accepts::Choice(_) => self.accepts.words(),
			_ => {
				self.rows = vec![Entry {
					detail: format!("takes {}", self.accepts.describe()),
					..Entry::new(self.name.clone(), Kind::Setting(self.accepts))
				}];
				return;
			}
		};
		// Before anything is typed, every value is listed; the field then holds the value it opened with.
		let prefix = if self.typed {
			self.value.trim().to_ascii_lowercase()
		} else {
			String::new()
		};
		self.rows = words
			.iter()
			.filter(|word| word.to_ascii_lowercase().starts_with(&prefix))
			.map(|word| Entry::new(*word, Kind::Setting(self.accepts)))
			.collect();
		self.highlight = Some(0);
	}
}

/// How long the palette takes to come in, and to go out, in seconds.
pub(crate) const ENTER: f32 = 0.15;
pub(crate) const EXIT: f32 = 0.09;

/// The palette's rectangle in a window of `viewport` pixels while it's `height` tall, as the shell measured it: left,
/// top, right, bottom.
pub(crate) fn rect(viewport: Size, height: f32) -> [f32; 4] {
	let left = (viewport.x() - WIDTH) * 0.5;
	[left, TOP, left + WIDTH, TOP + height]
}

/// Whether the pixel `point` is on the palette while it's `height` tall.
pub(crate) fn contains(viewport: Size, height: f32, point: [f32; 2]) -> bool {
	let [left, top, right, bottom] = rect(viewport, height);
	(left..right).contains(&point[0]) && (top..bottom).contains(&point[1])
}

/// The result under the pixel `point` while the palette shows `rows` results, if any.
pub(crate) fn row_at(viewport: Size, rows: usize, point: [f32; 2]) -> Option<usize> {
	let left = (viewport.x() - WIDTH) * 0.5;
	let first = TOP + INSET + FIELD_HEIGHT + GAP;
	if !(left..left + WIDTH).contains(&point[0]) || point[1] < first {
		return None;
	}
	let row = ((point[1] - first) / ROW_HEIGHT) as usize;
	(row < rows.min(MAX_ROWS)).then_some(row)
}

/// The name a resource's entity gets: its file name without its extension, such as `MotusMan_v55`.
pub(crate) fn entity_name(resource: &str) -> &str {
	let file = resource.rsplit('/').next().unwrap_or(resource);
	file.rsplit_once('.').map_or(file, |(stem, _)| stem)
}

#[cfg(test)]
mod tests {
	use super::*;

	fn palette(resources: &[&str]) -> State {
		State::new(
			Scope::Place,
			resources.iter().map(|id| Entry::new(*id, Kind::Mesh)).collect(),
			&[],
		)
	}

	fn labels(state: &State) -> Vec<&str> {
		state.results.iter().map(|entry| entry.label.as_str()).collect()
	}

	#[test]
	fn the_search_finds_resources_best_first_and_highlights_the_best() {
		let mut state = palette(&["props/barrel.glb", "props/crate.glb", "crate.glb"]);
		assert_eq!(state.results.len(), 3);

		state.search = "crate".to_string();
		state.find();
		assert_eq!(labels(&state), ["crate.glb", "props/crate.glb"]);
		assert_eq!(state.highlighted().map(|entry| entry.label.as_str()), Some("crate.glb"));
	}

	#[test]
	fn an_empty_search_lists_the_scopes_recently_used_entries_first() {
		let entries = ["a.glb", "b.glb", "c.glb"].map(|id| Entry::new(id, Kind::Mesh)).to_vec();
		// A setting used last belongs to another scope, so it isn't listed here.
		let recent = [
			(
				"render.lod.bias".to_string(),
				Kind::Setting(crate::configuration::Accepts::Bool),
			),
			("c.glb".to_string(), Kind::Mesh),
			("b.glb".to_string(), Kind::Mesh),
		];
		let mut state = State::new(Scope::Place, entries, &recent);
		assert_eq!(labels(&state), ["c.glb", "b.glb", "a.glb"]);

		state.search = "a".to_string();
		state.find();
		assert_eq!(labels(&state), ["a.glb"], "a search finds by name alone");
	}

	#[test]
	fn rows_are_found_under_the_pointer_below_the_field() {
		let viewport = Size::new(1000, 800);
		let [left, top, ..] = rect(viewport, 200.0);
		let first_row = top + INSET + FIELD_HEIGHT + GAP;

		assert_eq!(row_at(viewport, 3, [left + 10.0, first_row + 1.0]), Some(0));
		assert_eq!(row_at(viewport, 3, [left + 10.0, first_row + ROW_HEIGHT * 2.5]), Some(2));
		assert_eq!(row_at(viewport, 3, [left + 10.0, first_row + ROW_HEIGHT * 3.5]), None);
		assert_eq!(
			row_at(viewport, 3, [left + 10.0, top + INSET + 1.0]),
			None,
			"the field isn't a row"
		);
		assert!(contains(viewport, 200.0, [left + 1.0, top + 199.0]));
		assert!(!contains(viewport, 200.0, [left + 1.0, top + 201.0]));
		assert!(!contains(viewport, 200.0, [left - 1.0, top + 1.0]));
	}

	#[test]
	fn the_highlight_turns_around_past_the_first_and_last_result() {
		let mut state = palette(&["a", "b", "c"]);
		let label = |state: &State| state.highlighted().map(|entry| entry.label.clone());
		state.move_highlight(-1);
		assert_eq!(label(&state).as_deref(), Some("c"), "up from the first goes to the last");
		state.move_highlight(1);
		assert_eq!(label(&state).as_deref(), Some("a"), "down from the last goes to the first");
	}

	#[test]
	fn a_picked_setting_puts_its_value_in_the_field_and_says_what_it_takes() {
		let bias = Parameter {
			name: "render.lod.bias".to_string(),
			accepts: Accepts::Number { min: 0.0 },
			value: crate::configuration::ConfigurationValue::Float(1.5),
		};
		let state = State::new(Scope::Configure, vec![Entry::setting(bias)], &[]);
		assert_eq!(state.results[0].detail, "1.5");

		let setting = Setting::new(state.highlighted().unwrap()).unwrap();
		assert_eq!(setting.value, "1.5");
		assert_eq!(setting.rows[0].label, "render.lod.bias");
		assert_eq!(setting.rows[0].detail, "takes a number of 0 or more");
		assert_eq!(setting.highlight, None, "a number is typed, not picked");
		assert!(Setting::new(&Entry::new("crate.glb", Kind::Mesh)).is_none());
	}

	#[test]
	fn typing_over_a_setting_replaces_its_value_then_edits_what_was_typed() {
		let entry = Entry::setting(Parameter {
			name: "render.lod.bias".to_string(),
			accepts: Accepts::Number { min: 0.0 },
			value: crate::configuration::ConfigurationValue::Float(1.5),
		});
		let mut setting = Setting::new(&entry).unwrap();
		setting.type_edit(TextEdit::Inserted('3'));
		setting.type_edit(TextEdit::Inserted('2'));
		assert_eq!(setting.value, "32");
		setting.type_edit(TextEdit::Deleted('2'));
		assert_eq!(setting.value, "3");

		let mut erased = Setting::new(&entry).unwrap();
		erased.type_edit(TextEdit::Deleted('5'));
		assert_eq!(erased.value, "", "erasing first clears the value");
		assert_eq!(setting.chosen(), "3");
	}

	#[test]
	fn a_closed_set_lists_its_values_from_the_current_one_and_typing_narrows_them() {
		let entry = Entry::setting(Parameter {
			name: "render.gi.technique".to_string(),
			accepts: Accepts::Choice(&["ssgi", "surfels", "probes"]),
			value: "probes".into(),
		});
		let mut setting = Setting::new(&entry).unwrap();
		let labels = |setting: &Setting| setting.rows.iter().map(|row| row.label.clone()).collect::<Vec<_>>();
		assert_eq!(labels(&setting), ["ssgi", "surfels", "probes"]);
		assert_eq!(setting.chosen(), "probes", "Enter alone keeps the current value");

		setting.move_highlight(-1);
		assert_eq!(setting.chosen(), "surfels");
		setting.move_highlight(2);
		assert_eq!(setting.chosen(), "ssgi", "down from the last goes to the first");
		setting.move_highlight(1);

		setting.type_edit(TextEdit::Inserted('s'));
		assert_eq!(labels(&setting), ["ssgi", "surfels"]);
		setting.type_edit(TextEdit::Inserted('u'));
		assert_eq!(setting.chosen(), "surfels");

		setting.pick(0);
		assert_eq!(setting.chosen(), "surfels", "the only listed value");
	}

	#[test]
	fn a_typed_folder_that_isnt_listed_is_offered_as_a_new_one() {
		let folders = ["/", "levels/", "textures/"]
			.map(|folder| Entry::new(folder, Kind::Folder))
			.to_vec();
		let mut state = State::new(Scope::Folders(Create::Level), folders, &[]);
		let typed = |state: &mut State, search: &str| {
			state.search = search.to_owned();
			state.find();
			state
				.results
				.iter()
				.map(|entry| (entry.label.clone(), entry.detail.clone()))
				.collect::<Vec<_>>()
		};
		let new = |label: &str| (label.to_owned(), "New folder".to_owned());

		// A name no listed folder starts with leads, so Enter creates it.
		assert_eq!(typed(&mut state, "/maps/arena"), [new("maps/arena/")]);
		// One that could still become a listed folder trails the matches.
		assert_eq!(typed(&mut state, "lev").first().map(|row| row.0.as_str()), Some("levels/"));
		assert_eq!(typed(&mut state, "lev").last(), Some(&new("lev/")));
		// A listed folder, the asset folder, and paths that leave it aren't offered.
		for search in ["levels", "levels/", "/", "../maps", "a//b"] {
			assert!(!typed(&mut state, search).iter().any(|row| row.1 == "New folder"), "{search}");
		}
	}

	#[test]
	fn entities_are_named_after_their_mesh_file() {
		assert_eq!(entity_name("MotusMan/MotusMan_v55.fbx"), "MotusMan_v55");
		assert_eq!(entity_name("crate"), "crate");
	}
}
