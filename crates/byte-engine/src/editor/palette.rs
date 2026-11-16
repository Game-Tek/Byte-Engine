//! The command palette: a floating search over the game's mesh resources, from which a mesh is dragged into the scene
//! or pinned to the pointer until a click places it.
//!
//! This module holds what the palette shows and where it is on screen. The shell draws it from [`State`], and the
//! editor turns presses, keys, and typing into changes of it; placing the chosen mesh is the editor's job.

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

/// The `State` struct is what the palette shows, which the editor changes and the shell draws.
#[derive(Default)]
pub(crate) struct State {
	/// Whether the palette is showing.
	pub(crate) open: bool,
	/// What the search field holds.
	pub(crate) search: String,
	/// Every mesh resource the game has, as the inspector listed them when the palette opened.
	pub(crate) resources: Vec<String>,
	/// The resources the search finds, best first, at most [`MAX_ROWS`].
	pub(crate) results: Vec<String>,
	/// The result Enter picks.
	pub(crate) highlight: usize,
	/// Advanced whenever the results or the highlight change, so the shell redraws the rows.
	pub(crate) revision: u64,
}

impl State {
	/// Shows the palette over `resources`, with an empty search.
	pub(crate) fn open(&mut self, resources: Vec<String>) {
		self.open = true;
		self.search.clear();
		self.resources = resources;
		self.find();
	}

	/// Hides the palette.
	pub(crate) fn close(&mut self) {
		self.open = false;
		self.revision += 1;
	}

	/// Finds the results for the search, as the console finds entities by name, and highlights the best one.
	pub(crate) fn find(&mut self) {
		let ids: Vec<&str> = self.resources.iter().map(String::as_str).collect();
		self.results = fuzzy_find(&ids, &self.search)
			.into_iter()
			.take(MAX_ROWS)
			.map(|index| self.resources[index].clone())
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
	pub(crate) fn highlighted(&self) -> Option<&str> {
		self.results.get(self.highlight).map(String::as_str)
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

/// The name a mesh's entity gets: its resource's file name without its extension, such as `MotusMan_v55`.
pub(crate) fn entity_name(resource: &str) -> &str {
	let file = resource.rsplit('/').next().unwrap_or(resource);
	file.rsplit_once('.').map_or(file, |(stem, _)| stem)
}

#[cfg(test)]
mod tests {
	use super::*;

	fn palette(resources: &[&str]) -> State {
		let mut state = State::default();
		state.open(resources.iter().map(|id| id.to_string()).collect());
		state
	}

	#[test]
	fn the_search_finds_resources_best_first_and_highlights_the_best() {
		let mut state = palette(&["props/barrel.glb", "props/crate.glb", "crate.glb"]);
		assert_eq!(state.results.len(), 3);

		state.search = "crate".to_string();
		state.find();
		assert_eq!(state.results, ["crate.glb", "props/crate.glb"]);
		assert_eq!(state.highlighted(), Some("crate.glb"));
	}

	#[test]
	fn the_highlight_stops_at_the_first_and_last_result() {
		let mut state = palette(&["a", "b", "c"]);
		state.move_highlight(-1);
		assert_eq!(state.highlight, 0);
		state.move_highlight(5);
		assert_eq!(state.highlighted(), Some("c"));
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
