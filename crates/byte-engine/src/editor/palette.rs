//! The command palette: a floating search over what the editor can do, as VS Code's is.
//!
//! Each shortcut opens it on a [`Scope`]: Cmd+K on everything, Cmd+A on what can be placed in the scene, and either
//! on the node kinds while a material graph is open. A placeable [`Entry`] is dragged into the scene or pinned to the
//! pointer until a click places it; a material graph opens on the canvas, and a node kind is added to the open graph.
//!
//! This module holds what the palette shows and where it is on screen. The shell draws it from [`State`], and the
//! editor turns presses, keys, and typing into changes of it; acting on the chosen entry is the editor's job.

use crate::editor::carry::Light;
use crate::inspector::fuzzy_find;
use crate::ui::Size;

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
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) enum Scope {
	/// Everything the palette finds, opened with Cmd+K.
	#[default]
	All,
	/// What can be placed in the scene, opened with Cmd+A.
	Place,
	/// The node kinds to add to the open material graph.
	Nodes,
}

impl Scope {
	/// What the search field says while it's empty.
	pub(crate) fn prompt(self) -> &'static str {
		match self {
			Self::All => "Find a mesh, prefab, or light to place, or a material to edit",
			Self::Place => "Find a mesh, prefab, or light to place",
			Self::Nodes => "Find a node to add",
		}
	}
}

/// The `Entry` struct is one thing the palette finds: what its row says, which the search matches, and what it is.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Entry {
	/// A resource's or source's ID, a node kind's name, or a light's name.
	pub(crate) label: String,
	pub(crate) kind: Kind,
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
	/// Adds a node of the kind the label names to the open graph.
	Node,
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
		}
	}

	/// The name the row leads with: a resource's file name without its extension, or the whole label otherwise.
	pub(crate) fn name(&self) -> &str {
		match self.kind {
			Kind::Mesh | Kind::Prefab | Kind::Graph => entity_name(&self.label),
			Kind::Light(_) | Kind::Node => &self.label,
		}
	}
}

/// The `State` struct is what the palette shows, which the editor changes and the shell draws.
#[derive(Default)]
pub(crate) struct State {
	/// Whether the palette is showing.
	pub(crate) open: bool,
	/// Which version of the palette is open.
	pub(crate) scope: Scope,
	/// What the search field holds.
	pub(crate) search: String,
	/// Everything the open scope finds, gathered when it opened.
	pub(crate) entries: Vec<Entry>,
	/// The entries the search finds, best first, at most [`MAX_ROWS`].
	pub(crate) results: Vec<Entry>,
	/// The result Enter picks.
	pub(crate) highlight: usize,
	/// Advanced whenever the results or the highlight change, so the shell redraws the rows.
	pub(crate) revision: u64,
}

impl State {
	/// Shows the palette's `scope` over `entries`, with an empty search.
	pub(crate) fn open(&mut self, scope: Scope, entries: Vec<Entry>) {
		self.open = true;
		self.scope = scope;
		self.search.clear();
		self.entries = entries;
		self.find();
	}

	/// Hides the palette.
	pub(crate) fn close(&mut self) {
		self.open = false;
		self.revision += 1;
	}

	/// Finds the results for the search, as the console finds entities by name, and highlights the best one.
	pub(crate) fn find(&mut self) {
		let labels: Vec<&str> = self.entries.iter().map(|entry| entry.label.as_str()).collect();
		self.results = fuzzy_find(&labels, &self.search)
			.into_iter()
			.take(MAX_ROWS)
			.map(|index| self.entries[index].clone())
			.collect();
		self.highlight = 0;
		self.revision += 1;
	}

	/// Moves the highlight by `step` results, stopping at the first and the last.
	pub(crate) fn move_highlight(&mut self, step: isize) {
		let last = self.results.len().saturating_sub(1);
		self.highlight = self.highlight.saturating_add_signed(step).min(last);
		self.revision += 1;
	}

	/// The highlighted result, if there is any.
	pub(crate) fn highlighted(&self) -> Option<&Entry> {
		self.results.get(self.highlight)
	}
}

/// The palette's rectangle in a window of `viewport` pixels while it shows `rows` results: left, top, right, bottom.
pub(crate) fn rect(viewport: Size, rows: usize) -> [f32; 4] {
	let left = (viewport.x() - WIDTH) * 0.5;
	[left, TOP, left + WIDTH, TOP + height(rows)]
}

/// The palette's height while it shows `rows` results.
pub(crate) fn height(rows: usize) -> f32 {
	let rows = rows.min(MAX_ROWS) as f32;
	INSET * 2.0 + FIELD_HEIGHT + if rows > 0.0 { GAP + rows * ROW_HEIGHT } else { 0.0 }
}

/// Whether the pixel `point` is on the palette while it shows `rows` results.
pub(crate) fn contains(viewport: Size, rows: usize, point: [f32; 2]) -> bool {
	let [left, top, right, bottom] = rect(viewport, rows);
	(left..right).contains(&point[0]) && (top..bottom).contains(&point[1])
}

/// The result under the pixel `point` while the palette shows `rows` results, if any.
pub(crate) fn row_at(viewport: Size, rows: usize, point: [f32; 2]) -> Option<usize> {
	let [left, top, right, _] = rect(viewport, rows);
	let first = top + INSET + FIELD_HEIGHT + GAP;
	if !(left..right).contains(&point[0]) || point[1] < first {
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
		let mut state = State::default();
		let entries = resources.iter().map(|id| Entry::new(*id, Kind::Mesh)).collect();
		state.open(Scope::Place, entries);
		state
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
	fn the_highlight_stops_at_the_first_and_last_result() {
		let mut state = palette(&["a", "b", "c"]);
		state.move_highlight(-1);
		assert_eq!(state.highlight, 0);
		state.move_highlight(5);
		assert_eq!(state.highlighted().map(|entry| entry.label.as_str()), Some("c"));
	}

	#[test]
	fn rows_are_found_under_the_pointer_below_the_field() {
		let viewport = Size::new(1000, 800);
		let [left, top, ..] = rect(viewport, 3);
		let first_row = top + INSET + FIELD_HEIGHT + GAP;

		assert_eq!(row_at(viewport, 3, [left + 10.0, first_row + 1.0]), Some(0));
		assert_eq!(row_at(viewport, 3, [left + 10.0, first_row + ROW_HEIGHT * 2.5]), Some(2));
		assert_eq!(row_at(viewport, 3, [left + 10.0, first_row + ROW_HEIGHT * 3.5]), None);
		assert_eq!(
			row_at(viewport, 3, [left + 10.0, top + INSET + 1.0]),
			None,
			"the field isn't a row"
		);
		assert!(contains(viewport, 3, [left + 1.0, top + 1.0]));
		assert!(!contains(viewport, 3, [left - 1.0, top + 1.0]));
	}

	#[test]
	fn entities_are_named_after_their_mesh_file() {
		assert_eq!(entity_name("MotusMan/MotusMan_v55.fbx"), "MotusMan_v55");
		assert_eq!(entity_name("crate"), "crate");
	}
}
