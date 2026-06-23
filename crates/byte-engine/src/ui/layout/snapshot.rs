use std::{cell::RefCell, rc::Rc};

use math::{Base as _, Vector2};

use super::{
	element::Id,
	engine::EngineState,
	flow::{Location, Size},
	LayoutElement,
};
use crate::ui::intersection::MouseClickAcceleration;

/// Preserves a laid-out UI tree together with interaction state.
pub struct Snapshot {
	pub(super) elements: Vec<LayoutElement>,
	pub(super) relations: Vec<(Id, Id)>,
	pub(super) acceleration: MouseClickAcceleration,
	pub(super) cursor: Option<Id>,
	pub(super) engine_state: Rc<RefCell<EngineState>>,
	pub(super) size: Size,
}

impl Snapshot {
	pub fn cursor(&self) -> Option<Id> {
		self.cursor
	}

	pub fn set_cursor(&mut self, cursor: Option<Id>) -> Option<Id> {
		self.cursor = self
			.engine_state
			.borrow_mut()
			.set_cursor(cursor.filter(|id| self.element(*id).is_some()));
		self.cursor
	}

	pub fn clear_cursor(&mut self) {
		let _ = self.set_cursor(None);
	}

	pub fn move_cursor(&mut self, axis: Vector2) -> Option<Id> {
		if axis.x.abs() < SPATIAL_CURSOR_DEADZONE && axis.y.abs() < SPATIAL_CURSOR_DEADZONE {
			return self.cursor;
		}

		if axis.x.abs() >= axis.y.abs() {
			self.move_cursor_sideways(axis.x)
		} else {
			self.move_cursor_longitudinally(axis.y)
		}
	}

	pub fn move_cursor_sideways(&mut self, axis: f32) -> Option<Id> {
		if axis.abs() < SPATIAL_CURSOR_DEADZONE {
			return self.cursor;
		}

		let direction = if axis.is_sign_positive() {
			SpatialCursorDirection::Right
		} else {
			SpatialCursorDirection::Left
		};

		self.move_cursor_in_direction(direction)
	}

	pub fn move_cursor_longitudinally(&mut self, axis: f32) -> Option<Id> {
		if axis.abs() < SPATIAL_CURSOR_DEADZONE {
			return self.cursor;
		}

		let direction = if axis.is_sign_positive() {
			SpatialCursorDirection::Up
		} else {
			SpatialCursorDirection::Down
		};

		self.move_cursor_in_direction(direction)
	}

	pub fn click(&mut self, mouse_pos: Vector2) -> Option<Id> {
		let size = self.size;

		let mouse_pos = (mouse_pos + 1f32) * 0.5;
		let mouse_pos = mouse_pos * Vector2::new(size.x() as f32, size.y() as f32);
		let mouse_pos = Vector2::new(mouse_pos.x, size.y() as f32 - mouse_pos.y);

		let id = self
			.acceleration
			.query(Location::new(mouse_pos.x as u32, mouse_pos.y as u32))
			.and_then(Id::new)?;

		self.set_cursor(Some(id));
		Some(id)
	}

	pub fn click_cursor(&self) -> Option<Id> {
		self.cursor.filter(|id| self.element(*id).is_some())
	}

	fn move_cursor_in_direction(&mut self, direction: SpatialCursorDirection) -> Option<Id> {
		let current_cursor = self.cursor;
		let origin = current_cursor
			.and_then(|id| self.element(id))
			.map(NavigationFrame::from_element)
			.unwrap_or_else(|| self.snapshot_frame());

		let mut best_candidate: Option<(Id, CandidateScore)> = None;

		for (layout_index, element) in self.elements.iter().enumerate() {
			let candidate_id = element.element.id;

			if Some(candidate_id) == current_cursor {
				continue;
			}

			if let Some(cursor_id) = current_cursor {
				if self.is_cursor_related(cursor_id, candidate_id) {
					continue;
				}
			}

			let candidate = NavigationFrame::from_element(element);
			let Some(score) = direction_score(direction, origin, candidate, layout_index) else {
				continue;
			};

			match best_candidate {
				Some((_, best_score)) if !score.is_better_than(&best_score) => {}
				_ => best_candidate = Some((candidate_id, score)),
			}
		}

		if let Some((candidate_id, _)) = best_candidate {
			let _ = self.set_cursor(Some(candidate_id));
		}

		self.cursor
	}

	fn snapshot_frame(&self) -> NavigationFrame {
		let mut right: f32 = 1.0;
		let mut bottom: f32 = 1.0;

		for element in &self.elements {
			let frame = NavigationFrame::from_element(element);
			right = right.max(frame.right);
			bottom = bottom.max(frame.bottom);
		}

		NavigationFrame::from_point(Vector2::new(right * 0.5, bottom * 0.5))
	}

	fn element(&self, id: Id) -> Option<&LayoutElement> {
		self.elements.iter().find(|element| element.element.id == id)
	}

	fn is_cursor_related(&self, cursor: Id, candidate: Id) -> bool {
		self.is_ancestor_of(cursor, candidate) || self.is_ancestor_of(candidate, cursor)
	}

	fn is_ancestor_of(&self, ancestor: Id, descendant: Id) -> bool {
		let mut stack = vec![ancestor];

		while let Some(parent) = stack.pop() {
			for &(candidate_parent, candidate_child) in &self.relations {
				if candidate_parent != parent {
					continue;
				}

				if candidate_child == descendant {
					return true;
				}

				stack.push(candidate_child);
			}
		}

		false
	}

	pub fn size(&self) -> Size {
		self.size
	}
}

#[derive(Clone, Copy)]
enum SpatialCursorDirection {
	Left,
	Right,
	Up,
	Down,
}

#[derive(Clone, Copy)]
struct NavigationFrame {
	left: f32,
	right: f32,
	top: f32,
	bottom: f32,
	center: Vector2,
	depth: u32,
}

impl NavigationFrame {
	fn from_element(element: &LayoutElement) -> Self {
		let left = element.position.x() as f32;
		let top = element.position.y() as f32;
		let right = left + element.size.x() as f32;
		let bottom = top + element.size.y() as f32;

		Self {
			left,
			right,
			top,
			bottom,
			center: Vector2::new((left + right) * 0.5, (top + bottom) * 0.5),
			depth: element.position.z(),
		}
	}

	fn from_point(point: Vector2) -> Self {
		Self {
			left: point.x,
			right: point.x,
			top: point.y,
			bottom: point.y,
			center: point,
			depth: 0,
		}
	}
}

#[derive(Clone, Copy)]
struct CandidateScore {
	alignment_rank: u8,
	orthogonal_gap: f32,
	forward_gap: f32,
	center_distance_squared: f32,
	depth: u32,
	layout_index: usize,
}

impl CandidateScore {
	fn is_better_than(&self, other: &Self) -> bool {
		self.alignment_rank < other.alignment_rank
			|| (self.alignment_rank == other.alignment_rank && self.orthogonal_gap < other.orthogonal_gap)
			|| (self.alignment_rank == other.alignment_rank
				&& self.orthogonal_gap == other.orthogonal_gap
				&& self.forward_gap < other.forward_gap)
			|| (self.alignment_rank == other.alignment_rank
				&& self.orthogonal_gap == other.orthogonal_gap
				&& self.forward_gap == other.forward_gap
				&& self.center_distance_squared < other.center_distance_squared)
			|| (self.alignment_rank == other.alignment_rank
				&& self.orthogonal_gap == other.orthogonal_gap
				&& self.forward_gap == other.forward_gap
				&& self.center_distance_squared == other.center_distance_squared
				&& self.depth > other.depth)
			|| (self.alignment_rank == other.alignment_rank
				&& self.orthogonal_gap == other.orthogonal_gap
				&& self.forward_gap == other.forward_gap
				&& self.center_distance_squared == other.center_distance_squared
				&& self.depth == other.depth
				&& self.layout_index > other.layout_index)
	}
}

const SPATIAL_CURSOR_DEADZONE: f32 = 0.35;

fn direction_score(
	direction: SpatialCursorDirection,
	origin: NavigationFrame,
	candidate: NavigationFrame,
	layout_index: usize,
) -> Option<CandidateScore> {
	let dx = candidate.center.x - origin.center.x;
	let dy = candidate.center.y - origin.center.y;
	let center_distance_squared = dx * dx + dy * dy;

	let (forward, forward_gap, alignment_rank, orthogonal_gap) = match direction {
		SpatialCursorDirection::Left => {
			let (alignment_rank, orthogonal_gap) = interval_gap(origin.top, origin.bottom, candidate.top, candidate.bottom);
			(-dx, (origin.left - candidate.right).max(0.0), alignment_rank, orthogonal_gap)
		}
		SpatialCursorDirection::Right => {
			let (alignment_rank, orthogonal_gap) = interval_gap(origin.top, origin.bottom, candidate.top, candidate.bottom);
			(dx, (candidate.left - origin.right).max(0.0), alignment_rank, orthogonal_gap)
		}
		SpatialCursorDirection::Up => {
			let (alignment_rank, orthogonal_gap) = interval_gap(origin.left, origin.right, candidate.left, candidate.right);
			(-dy, (origin.top - candidate.bottom).max(0.0), alignment_rank, orthogonal_gap)
		}
		SpatialCursorDirection::Down => {
			let (alignment_rank, orthogonal_gap) = interval_gap(origin.left, origin.right, candidate.left, candidate.right);
			(dy, (candidate.top - origin.bottom).max(0.0), alignment_rank, orthogonal_gap)
		}
	};

	if forward <= 0.0 {
		return None;
	}

	Some(CandidateScore {
		alignment_rank,
		orthogonal_gap,
		forward_gap,
		center_distance_squared,
		depth: candidate.depth,
		layout_index,
	})
}

fn interval_gap(start_a: f32, end_a: f32, start_b: f32, end_b: f32) -> (u8, f32) {
	if start_a <= end_b && start_b <= end_a {
		(0, 0.0)
	} else if end_a < start_b {
		(1, start_b - end_a)
	} else {
		(1, start_a - end_b)
	}
}
