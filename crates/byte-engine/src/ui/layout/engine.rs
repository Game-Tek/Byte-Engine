use std::marker::PhantomData;

use math::{Base as _, Vector2};
use utils::RGBA;

use crate::ui::{
	flow::Location,
	intersection::{build_mouse_click_acceleration, MouseClickAcceleration},
	layout::{IdedElement, RenderElement},
	primitive::{Events, Primitives},
	style::{self, Color, ConcreteStyle},
	Container,
};

use super::{
	element::{ElementHandle, Id},
	flow::Size,
	layout_elements,
	query::Fetcher,
	ConcreteElement, Element, LayoutElement,
};

pub struct Engine {
	viewports: Vec<VirtualViewport>,
	cursor_position: Vector2,
	is_clicking: bool,
	clicks: Vec<bool>,
}

impl Engine {
	pub fn new() -> Self {
		Self {
			viewports: Vec::new(),
			cursor_position: Vector2::zero(),
			is_clicking: false,
			clicks: Vec::new(),
		}
	}

	pub fn add_viewport(&mut self, viewport: VirtualViewport) {
		self.viewports.push(viewport);
	}

	/// Evaluates the layout of the given root component and returns a snapshot of the resulting layout.
	pub fn evaluate<'a>(&'a mut self, root: &impl Component, size: Size) -> Snapshot {
		struct State<'a> {
			id: Id,
			counter: &'a mut u32,
			elements: &'a mut Vec<IdedElement>,
			relations: &'a mut Vec<(Id, Id)>,
		}

		impl<'state> Context for State<'state> {
			type Child<'a>
				= State<'a>
			where
				Self: 'a;

			fn container<'a>(&'a mut self, element: Container) -> Self::Child<'a> {
				let id = Id::new(*self.counter).unwrap();

				*self.counter += 1;

				self.elements.push(IdedElement {
					id,
					element: ConcreteElement::container(element),
				});

				if id != self.id {
					self.relations.push((self.id, id));
				}

				State {
					id,
					counter: &mut *self.counter,
					elements: &mut *self.elements,
					relations: &mut *self.relations,
				}
			}

			fn component(&mut self, component: &impl Component) {
				component.render(self);
			}
		}

		let mut elements = Vec::new();
		let mut relations = Vec::new();

		let mut counter = 1;

		let mut state = State {
			id: Id::new(counter).unwrap(),
			counter: &mut counter,
			elements: &mut elements,
			relations: &mut relations,
		};

		root.render(&mut state);

		let elements = layout_elements(elements, &relations, size);

		let acc = build_mouse_click_acceleration(&elements);

		let mut snapshot = Snapshot {
			elements,
			relations,
			acceleration: acc,
			cursor: None,
			size,
		};

		while let Some(click) = self.clicks.pop() {
			if click {
				let _ = snapshot.click(self.cursor_position);
			}
		}

		snapshot
	}

	/// Renders the given snapshot into a [`Render`] object.
	pub fn render<'a>(&'a mut self, snapshot: Snapshot) -> Render {
		let size = snapshot.size;

		let mouse_pos = (self.cursor_position + 1.0) * 0.5;
		let mouse_pos = mouse_pos * Vector2::new(size.x() as f32, size.y() as f32);
		let mouse_pos = Vector2::new(mouse_pos.x, size.y() as f32 - mouse_pos.y);

		let hovered_element_id = snapshot
			.acceleration
			.query(Location::new(mouse_pos.x as u32, mouse_pos.y as u32));
		let hovered_element = hovered_element_id.map(|id| Id::new(id).unwrap());

		let elements = snapshot
			.elements
			.iter()
			.map(|e| {
				let style = match &e.element.element.primitive {
					Primitives::Container(c) => {
						if let Some(styler) = c.styler.as_ref() {
							let state = style::StyleState {
								is_hovered: hovered_element == Some(e.element.id),
							};

							styler.call(state)
						} else {
							ConcreteStyle::default()
						}
					}
					_ => ConcreteStyle::default(),
				};

				let layer = &style.layers[0];

				let color = match layer.color {
					Color::Value(rgba) => rgba,
					Color::Sample(_) => todo!(),
				};

				RenderElement {
					id: e.element.id.get(),
					position: e.position,
					size: e.size,
					color,
				}
			})
			.collect::<Vec<_>>();

		Render {
			elements,
			relations: snapshot.relations.clone(),
		}
	}

	pub fn set_cursor_position(&mut self, v: Vector2) {
		self.cursor_position = v;
	}

	pub fn update_click_state(&mut self, v: bool) {
		self.is_clicking = v;
		self.clicks.push(v);
	}
}

/// The `Snapshot` struct preserves a laid out UI tree together with the interaction state needed to drive it by mouse or spatial cursor.
pub struct Snapshot {
	elements: Vec<LayoutElement>,
	relations: Vec<(Id, Id)>,
	acceleration: MouseClickAcceleration,
	cursor: Option<Id>,
	size: Size,
}

impl Snapshot {
	pub fn cursor(&self) -> Option<Id> {
		self.cursor
	}

	/// Updates the snapshot cursor to a specific element when that element still exists in the snapshot.
	pub fn set_cursor(&mut self, cursor: Option<Id>) -> Option<Id> {
		self.cursor = cursor.filter(|id| self.element(*id).is_some());
		self.cursor
	}

	pub fn clear_cursor(&mut self) {
		self.cursor = None;
	}

	/// Moves the snapshot cursor from a joystick-like axis by following the dominant axis.
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

	/// Moves the snapshot cursor horizontally, with positive values going right and negative values going left.
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

	/// Moves the snapshot cursor vertically, with positive values going up and negative values going down.
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

	/// Resolves a mouse click against the snapshot and stores the clicked element in the snapshot cursor.
	pub fn click(&mut self, mouse_pos: Vector2) -> Option<Id> {
		let size = self.size;

		let mouse_pos = (mouse_pos + 1f32) * 0.5;
		let mouse_pos = mouse_pos * Vector2::new(size.x() as f32, size.y() as f32);
		let mouse_pos = Vector2::new(mouse_pos.x, size.y() as f32 - mouse_pos.y);

		if let Some(id) = self
			.acceleration
			.query(Location::new(mouse_pos.x as u32, mouse_pos.y as u32))
			.and_then(Id::new)
		{
			self.cursor = Some(id);
			return self.actuate_element(id);
		}

		None
	}

	/// Activates the element currently stored in the snapshot cursor.
	pub fn click_cursor(&self) -> Option<Id> {
		self.cursor.and_then(|id| self.actuate_element(id))
	}

	// Moves the snapshot cursor toward the best element in the requested cardinal direction.
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

			// Scores candidates by lane alignment first and travel distance second.
			let Some(score) = direction_score(direction, origin, candidate, layout_index) else {
				continue;
			};

			match best_candidate {
				Some((_, best_score)) if !score.is_better_than(&best_score) => {}
				_ => best_candidate = Some((candidate_id, score)),
			}
		}

		if let Some((candidate_id, _)) = best_candidate {
			self.cursor = Some(candidate_id);
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

	fn actuate_element(&self, id: Id) -> Option<Id> {
		let element = self.element(id)?;

		match &element.element.element.primitive {
			Primitives::Container(c) => {
				if let Some(on_event) = &c.on_event {
					on_event.call(Events::Actuate {});
				}
			}
			_ => {}
		}

		Some(id)
	}

	fn element(&self, id: Id) -> Option<&LayoutElement> {
		self.elements.iter().find(|element| element.element.id == id)
	}

	fn is_cursor_related(&self, cursor: Id, candidate: Id) -> bool {
		self.is_ancestor_of(cursor, candidate) || self.is_ancestor_of(candidate, cursor)
	}

	// Walks the snapshot tree so spatial cursor moves stay between nearby peers instead of climbing to ancestors.
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

// Converts a candidate into a directional score so cursor moves prefer aligned neighbors before longer diagonal jumps.
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
			let forward = -dx;
			let forward_gap = (origin.left - candidate.right).max(0.0);
			let (alignment_rank, orthogonal_gap) = interval_gap(origin.top, origin.bottom, candidate.top, candidate.bottom);
			(forward, forward_gap, alignment_rank, orthogonal_gap)
		}
		SpatialCursorDirection::Right => {
			let forward = dx;
			let forward_gap = (candidate.left - origin.right).max(0.0);
			let (alignment_rank, orthogonal_gap) = interval_gap(origin.top, origin.bottom, candidate.top, candidate.bottom);
			(forward, forward_gap, alignment_rank, orthogonal_gap)
		}
		SpatialCursorDirection::Up => {
			let forward = -dy;
			let forward_gap = (origin.top - candidate.bottom).max(0.0);
			let (alignment_rank, orthogonal_gap) = interval_gap(origin.left, origin.right, candidate.left, candidate.right);
			(forward, forward_gap, alignment_rank, orthogonal_gap)
		}
		SpatialCursorDirection::Down => {
			let forward = dy;
			let forward_gap = (candidate.top - origin.bottom).max(0.0);
			let (alignment_rank, orthogonal_gap) = interval_gap(origin.left, origin.right, candidate.left, candidate.right);
			(forward, forward_gap, alignment_rank, orthogonal_gap)
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

/// The `Render` struct preserves the visual data derived from a snapshot so UI rectangles can be submitted to the renderer.
#[derive(Clone)]
pub struct Render {
	elements: Vec<RenderElement>,
	relations: Vec<(Id, Id)>,
}

impl Render {
	pub fn root(&self) -> &RenderElement {
		self.elements.iter().find(|e| e.id == 1).unwrap()
	}

	pub fn size(&self) -> usize {
		self.elements.len()
	}

	pub fn elements(&self) -> impl Iterator<Item = &RenderElement> {
		self.elements.iter()
	}
}

pub trait Context: Sized {
	type Child<'a>: Context
	where
		Self: 'a;

	fn container<'a>(&'a mut self, element: Container) -> Self::Child<'a>;
	fn component(&mut self, component: &impl Component);
}

struct VirtualViewport;

impl ElementHandle for VirtualViewport {
	fn id(&self) -> Id {
		unimplemented!()
	}
}

pub trait Component {
	fn render(&self, ctx: &mut impl Context);
}

#[cfg(test)]
mod tests {
	use std::sync::{
		atomic::{AtomicUsize, Ordering},
		Arc,
	};

	use math::{Base, Vector2};
	use utils::{Box, RGBA};

	use crate::ui::{
		components::container::OnEventFunction,
		flow::Offset,
		intersection::build_mouse_click_acceleration,
		primitive::Events,
		style::{ConcreteLayer, StyleState},
	};

	use super::super::super::{
		components::container::{Container, ContainerSettings},
		element::Id,
		flow::{self, Location3, Size},
		layout::engine::{Context, Engine, VirtualViewport},
		layout::{ConcreteElement, IdedElement, LayoutElement, Sizing},
		primitive::Shapes,
		Component,
	};

	use super::Snapshot;

	struct Bar {
		options: Vec<BarOption>,
	}

	impl Bar {
		pub fn new(options: Vec<BarOption>) -> Self {
			Self { options }
		}
	}

	impl Component for Bar {
		fn render(&self, ctx: &mut impl Context) {
			let mut ctx = ctx.container(Container::new(ContainerSettings::default().height(32.into())));

			for option in &self.options {
				option.render(&mut ctx);
			}
		}
	}

	struct BarOption {
		label: String,
	}

	impl BarOption {
		pub fn new(label: String) -> Self {
			Self { label }
		}
	}

	impl Component for BarOption {
		fn render(&self, ctx: &mut impl Context) {
			ctx.container(Container::new(ContainerSettings::default()));
		}
	}

	struct BarList {}

	impl BarList {
		pub fn new() -> Self {
			Self {}
		}
	}

	struct BarListOption {}

	impl BarListOption {
		pub fn new() -> Self {
			Self {}
		}
	}

	struct Application {
		bar: Bar,
	}

	impl Application {
		pub fn new() -> Self {
			let options = vec![
				BarOption::new("File".to_string()),
				BarOption::new("Edit".to_string()),
				BarOption::new("View".to_string()),
			];

			Self { bar: Bar::new(options) }
		}
	}

	impl Component for Application {
		fn render(&self, ctx: &mut impl Context) {
			let mut ctx = ctx.container(Container::new(ContainerSettings::default()));
			self.bar.render(&mut ctx);
		}
	}

	fn test_snapshot(
		elements: Vec<(u32, Location3, Size, Option<utils::InlineCopyFn<OnEventFunction>>)>,
		relations: &[(u32, u32)],
	) -> Snapshot {
		let elements = elements
			.into_iter()
			.map(|(id, position, size, on_event)| LayoutElement {
				position,
				size,
				element: IdedElement {
					id: Id::new(id).unwrap(),
					element: ConcreteElement {
						primitive: Container::new(
							ContainerSettings::default()
								.width(Sizing::Absolute(size.x()))
								.height(Sizing::Absolute(size.y())),
						)
						.into(),
					},
				},
			})
			.collect::<Vec<_>>();

		Snapshot {
			acceleration: build_mouse_click_acceleration(&elements),
			elements,
			relations: relations
				.iter()
				.map(|&(parent, child)| (Id::new(parent).unwrap(), Id::new(child).unwrap()))
				.collect(),
			cursor: None,
			size: Size::new(1024, 1024),
		}
	}

	#[test]
	fn snapshot_moves_cursor_sideways_between_physical_neighbors() {
		let mut snapshot = test_snapshot(
			vec![
				(1, Location3::new(0, 0, 0), Size::new(400, 120), None),
				(2, Location3::new(40, 40, 1), Size::new(80, 40), None),
				(3, Location3::new(160, 40, 1), Size::new(80, 40), None),
				(4, Location3::new(280, 40, 1), Size::new(80, 40), None),
			],
			&[(1, 2), (1, 3), (1, 4)],
		);

		assert_eq!(snapshot.move_cursor_sideways(1.0), Id::new(4));
		assert_eq!(snapshot.move_cursor_sideways(-1.0), Id::new(3));
		assert_eq!(snapshot.move_cursor_sideways(-1.0), Id::new(2));
	}

	#[test]
	fn snapshot_moves_cursor_longitudinally_with_positive_input_going_up() {
		let mut snapshot = test_snapshot(
			vec![
				(1, Location3::new(0, 0, 0), Size::new(240, 300), None),
				(2, Location3::new(80, 20, 1), Size::new(80, 40), None),
				(3, Location3::new(80, 130, 1), Size::new(80, 40), None),
				(4, Location3::new(80, 240, 1), Size::new(80, 40), None),
			],
			&[(1, 2), (1, 3), (1, 4)],
		);

		assert_eq!(snapshot.set_cursor(Some(Id::new(3).unwrap())), Id::new(3));
		assert_eq!(snapshot.move_cursor_longitudinally(1.0), Id::new(2));
		assert_eq!(snapshot.move_cursor_longitudinally(-1.0), Id::new(3));
		assert_eq!(snapshot.move_cursor(Vector2::new(0.2, -1.0)), Id::new(4));
	}

	#[test]
	fn snapshot_keeps_the_current_cursor_when_only_an_ancestor_exists_in_that_direction() {
		let mut snapshot = test_snapshot(
			vec![
				(1, Location3::new(0, 0, 0), Size::new(400, 160), None),
				(2, Location3::new(260, 60, 1), Size::new(80, 40), None),
			],
			&[(1, 2)],
		);

		assert_eq!(snapshot.set_cursor(Some(Id::new(2).unwrap())), Id::new(2));
		assert_eq!(snapshot.move_cursor_sideways(-1.0), Id::new(2));
	}

	#[test]
	fn render_applies_container_stylers_to_render_elements() {
		struct StyledColumn;

		impl Component for StyledColumn {
			fn render(&self, ctx: &mut impl Context) {
				let styler = |state: StyleState| {
					if state.is_hovered {
						ConcreteLayer::new().color(RGBA::new(1.0, 0.0, 0.0, 1.0).into()).into()
					} else {
						ConcreteLayer::new().color(RGBA::new(0.0, 1.0, 0.0, 1.0).into()).into()
					}
				};

				let mut ctx = ctx.container(Container::new(ContainerSettings::default().flow(flow::column)));

				ctx.container(Container::new(ContainerSettings::default().size(Sizing::Absolute(20))).styler(styler));
				ctx.container(Container::new(ContainerSettings::default().size(Sizing::Absolute(20))).styler(styler));
			}
		}

		let mut engine = Engine::new();
		engine.set_cursor_position(Vector2::new(-0.8, 0.8));

		let snapshot = engine.evaluate(&StyledColumn, Size::new(100, 100));
		let render = engine.render(snapshot);

		assert_eq!(render.size(), 3);

		let root = render.elements().find(|element| element.id == 1).unwrap();
		assert_eq!(root.position, Location3::new(0, 0, 0));
		assert_eq!(root.size, Size::new(100, 100));
		assert_eq!(root.color, RGBA::white());

		let first_child = render.elements().find(|element| element.id == 2).unwrap();
		assert_eq!(first_child.position, Location3::new(0, 0, 1));
		assert_eq!(first_child.size, Size::new(20, 20));
		assert_eq!(first_child.color, RGBA::new(1.0, 0.0, 0.0, 1.0));

		let second_child = render.elements().find(|element| element.id == 3).unwrap();
		assert_eq!(second_child.position, Location3::new(0, 20, 1));
		assert_eq!(second_child.size, Size::new(20, 20));
		assert_eq!(second_child.color, RGBA::new(0.0, 1.0, 0.0, 1.0));
	}

	// 	#[test]
	// 	fn snapshot_click_cursor_activates_the_focused_element() {
	// 		let click_count = Arc::new(AtomicUsize::new(0));
	// 		let on_event = {
	// 			let click_count = Arc::clone(&click_count);
	// 			utils::InlineCopyFn::<OnEventFunction>::new(move |e| {
	// 				match e {
	// 					Events::Actuate {  } => {
	// 						click_count.fetch_add(1, Ordering::SeqCst);
	// 					}
	// 				}
	// 			})
	// 		};

	// 		let mut snapshot = test_snapshot(
	// 			vec![
	// 				(1, Location3::new(0, 0, 0), Size::new(200, 120), None),
	// 				(2, Location3::new(60, 40, 1), Size::new(80, 40), Some(on_event)),
	// 			],
	// 			&[(1, 2)],
	// 		);

	// 		assert_eq!(snapshot.set_cursor(Some(Id::new(2).unwrap())), Id::new(2));
	// 		assert_eq!(snapshot.click_cursor(), Id::new(2));
	// 		assert_eq!(click_count.load(Ordering::SeqCst), 1);
	// 	}
}
