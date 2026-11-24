use std::ops::Add;

/// The `Offset` struct stores signed screen-space movement during UI layout.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct Offset(f32, f32);

/// The `Location` struct stores an absolute two-dimensional UI position.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct Location(f32, f32);

/// The `Location3` struct stores an absolute UI position with depth ordering.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct Location3(f32, f32, u32);

/// The `Size` struct stores a two-dimensional UI extent in logical pixels.
#[derive(Clone, Copy, PartialEq, Debug, Default)]
pub struct Size(f32, f32);

/// The `FlowInput` struct passes parent space, cursor, child size, and the child's visual center
/// into reusable layout flow functions.
///
/// A flow that centers its children should center [`Self::child_center`] rather than the middle of the child's box, as
/// the built-in centering flows do, so vector icons look centered; see
/// [Visual center](/docs/reference/gui#visual-center).
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct FlowInput {
	parent_size: Size,
	cursor: Offset,
	child_size: Size,
	child_center: Offset,
}

/// The `FlowOutput` struct returns a child's offset and the next cursor
/// from reusable layout flow functions.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct FlowOutput {
	child_offset: Offset,
	next_cursor: Offset,
}

impl Offset {
	pub fn new(x: impl Into<f64>, y: impl Into<f64>) -> Self {
		Self(x.into() as f32, y.into() as f32)
	}

	#[inline]
	pub fn x(&self) -> f32 {
		self.0
	}

	#[inline]
	pub fn y(&self) -> f32 {
		self.1
	}
}

impl Add for Offset {
	type Output = Self;

	fn add(self, rhs: Self) -> Self::Output {
		Self(self.0 + rhs.0, self.1 + rhs.1)
	}
}

impl From<Offset> for Location {
	fn from(val: Offset) -> Self {
		Location(val.0, val.1)
	}
}

impl Location {
	pub fn new(x: impl Into<f64>, y: impl Into<f64>) -> Self {
		Self(x.into() as f32, y.into() as f32)
	}

	#[inline]
	pub fn x(&self) -> f32 {
		self.0
	}

	#[inline]
	pub fn y(&self) -> f32 {
		self.1
	}
}

impl Add for Location {
	type Output = Self;

	fn add(self, rhs: Self) -> Self::Output {
		Self(self.0 + rhs.0, self.1 + rhs.1)
	}
}

impl From<Location> for Offset {
	fn from(val: Location) -> Self {
		Offset(val.0, val.1)
	}
}

impl From<Location> for (f32, f32) {
	fn from(val: Location) -> Self {
		(val.0, val.1)
	}
}

impl Size {
	pub fn new(width: impl Into<f64>, height: impl Into<f64>) -> Self {
		Self(width.into() as f32, height.into() as f32)
	}

	#[inline]
	pub fn x(&self) -> f32 {
		self.0
	}

	#[inline]
	pub fn y(&self) -> f32 {
		self.1
	}
}

impl Add for Size {
	type Output = Self;

	fn add(self, rhs: Self) -> Self::Output {
		Self(self.0 + rhs.0, self.1 + rhs.1)
	}
}

impl FlowInput {
	/// Creates the input for one child. `child_center` is the point in the child's box that looks like its middle, in
	/// layout units from the box's top-left corner; it is the middle of the box for everything but vector icons.
	pub fn new(parent_size: Size, cursor: Offset, child_size: Size, child_center: Offset) -> Self {
		Self {
			parent_size,
			cursor,
			child_size,
			child_center,
		}
	}

	#[inline]
	pub fn parent_size(&self) -> Size {
		self.parent_size
	}

	#[inline]
	pub fn cursor(&self) -> Offset {
		self.cursor
	}

	#[inline]
	pub fn child_size(&self) -> Size {
		self.child_size
	}

	/// The point in the child's box that looks like its middle, in layout units from the box's top-left corner.
	#[inline]
	pub fn child_center(&self) -> Offset {
		self.child_center
	}
}

/// Returns the offset on one axis that centers a child's visual center in the parent. The box is centered first, never
/// starting before the parent's edge, then moved by how far the visual center sits from the box's middle.
fn centered(parent: f32, child: f32, center: f32) -> f32 {
	((parent - child) * 0.5).max(0.0) + child * 0.5 - center
}

impl FlowOutput {
	pub fn new(child_offset: Offset, next_cursor: Offset) -> Self {
		Self {
			child_offset,
			next_cursor,
		}
	}

	#[inline]
	pub fn child_offset(&self) -> Offset {
		self.child_offset
	}

	#[inline]
	pub fn next_cursor(&self) -> Offset {
		self.next_cursor
	}

	#[inline]
	pub fn anchored(offset: Offset, size: Size) -> Self {
		Self::new(offset, Offset(offset.0 + size.0, offset.1))
	}
}

pub fn row(input: FlowInput) -> FlowOutput {
	let offset = input.cursor;
	let size = input.child_size;
	FlowOutput::new(offset, Offset(offset.0 + size.0, offset.1))
}

pub fn column(input: FlowInput) -> FlowOutput {
	let offset = input.cursor;
	let size = input.child_size;
	FlowOutput::new(offset, Offset(offset.0, offset.1 + size.1))
}

pub fn grid(input: FlowInput) -> FlowOutput {
	let offset = input.cursor;
	let size = input.child_size;
	FlowOutput::new(offset, Offset(offset.0 + size.0, offset.1 + size.1))
}

/// Places children in a row with a fixed gap after each child.
pub fn row_with_gap(gap: impl Into<f64>) -> impl FlowFunction {
	// Keep the callable type independent of the caller's numeric type.
	fn with_gap(gap: f32) -> impl FlowFunction {
		move |input| {
			let offset = input.cursor;
			let size = input.child_size;
			FlowOutput::new(offset, Offset(offset.0 + size.0 + gap, offset.1))
		}
	}
	with_gap(gap.into() as f32)
}

/// Places children in a column with a fixed gap after each child.
pub fn column_with_gap(gap: impl Into<f64>) -> impl FlowFunction {
	// Keep the callable type independent of the caller's numeric type.
	fn with_gap(gap: f32) -> impl FlowFunction {
		move |input| {
			let offset = input.cursor;
			let size = input.child_size;
			FlowOutput::new(offset, Offset(offset.0, offset.1 + size.1 + gap))
		}
	}
	with_gap(gap.into() as f32)
}

pub fn centered_row(input: FlowInput) -> FlowOutput {
	let offset = Offset(
		input.cursor.0,
		input.cursor.1 + centered(input.parent_size.1, input.child_size.1, input.child_center.1),
	);

	FlowOutput::new(offset, Offset(input.cursor.0 + input.child_size.0, input.cursor.1))
}

/// Places children in a row, each centered vertically in the parent, with a fixed gap after each child.
pub fn centered_row_with_gap(gap: impl Into<f64>) -> impl FlowFunction {
	// Keep the callable type independent of the caller's numeric type.
	fn with_gap(gap: f32) -> impl FlowFunction {
		move |input| {
			let output = centered_row(input);
			FlowOutput::new(
				output.child_offset(),
				Offset(output.next_cursor().0 + gap, output.next_cursor().1),
			)
		}
	}
	with_gap(gap.into() as f32)
}

pub fn centered_column(input: FlowInput) -> FlowOutput {
	let offset = Offset(
		input.cursor.0 + centered(input.parent_size.0, input.child_size.0, input.child_center.0),
		input.cursor.1,
	);

	FlowOutput::new(offset, Offset(input.cursor.0, input.cursor.1 + input.child_size.1))
}

/// Places each child against the end of the row, such as a value aligned to the right edge of a field.
///
/// The cursor does not move, so every child sits at the end; give each its own container to line up several.
pub fn end_row(input: FlowInput) -> FlowOutput {
	let offset = Offset(input.cursor.0 + input.parent_size.0 - input.child_size.0, input.cursor.1);
	FlowOutput::new(offset, input.cursor)
}

/// Places each child against the end of the row and centered vertically, such as a right-aligned label in a button
/// or a row.
///
/// Like [`end_row`], the cursor does not move.
pub fn centered_end_row(input: FlowInput) -> FlowOutput {
	let offset = Offset(
		input.cursor.0 + input.parent_size.0 - input.child_size.0,
		input.cursor.1 + centered(input.parent_size.1, input.child_size.1, input.child_center.1),
	);
	FlowOutput::new(offset, input.cursor)
}

pub fn center(input: FlowInput) -> FlowOutput {
	let offset = Offset(
		input.cursor.0 + centered(input.parent_size.0, input.child_size.0, input.child_center.0),
		input.cursor.1 + centered(input.parent_size.1, input.child_size.1, input.child_center.1),
	);

	FlowOutput::new(offset, input.cursor)
}

pub trait FlowFunction = Fn(FlowInput) -> FlowOutput + Copy;

/// The `FlowIdentity` enum lets the retained tree tell whether an edit changed a container's flow, so unchanged
/// placement is reused. See [Flows](/docs/reference/gui#flows).
#[derive(Clone, Copy, PartialEq)]
pub(crate) enum FlowIdentity {
	/// A built-in flow and the output that exposes its captured gap.
	BuiltIn(std::any::TypeId, FlowOutput),
	/// Any other callable, known only by how many times its container's flow was set, because the values a closure
	/// captured can't be compared.
	Custom { writes: u32 },
}

/// Identifies `flow`, set `writes` times on its container. Built-in flows compare by kind and gap, so setting the
/// same one again is no change; any other callable changes with every write.
pub(crate) fn identity(flow: &utils::InlineCopyFn<FlowInput, FlowOutput>, writes: u32) -> FlowIdentity {
	use std::any::Any;
	let kind = flow.callable_type_id();
	let builtin = [
		row.type_id(),
		column.type_id(),
		grid.type_id(),
		centered_row.type_id(),
		centered_column.type_id(),
		center.type_id(),
		end_row.type_id(),
		centered_end_row.type_id(),
		row_with_gap(0.0).type_id(),
		column_with_gap(0.0).type_id(),
		centered_row_with_gap(0.0).type_id(),
	]
	.contains(&kind);
	if !builtin {
		return FlowIdentity::Custom { writes };
	}
	// The gap is the only captured input in built-in flows. Zero sizes and cursor
	// expose it without rounding it against a nonzero size or copying closure bytes.
	FlowIdentity::BuiltIn(
		kind,
		flow.call(FlowInput::new(
			Size::new(0, 0),
			Offset::new(0, 0),
			Size::new(0, 0),
			Offset::new(0, 0),
		)),
	)
}

impl Location3 {
	pub fn new(x: impl Into<f64>, y: impl Into<f64>, z: u32) -> Self {
		Self(x.into() as f32, y.into() as f32, z)
	}

	pub fn x(&self) -> f32 {
		self.0
	}

	pub fn y(&self) -> f32 {
		self.1
	}

	pub fn z(&self) -> u32 {
		self.2
	}
}

impl From<(Location, u32)> for Location3 {
	fn from(value: (Location, u32)) -> Self {
		Location3(value.0.0, value.0.1, value.1)
	}
}

impl From<Location3> for Location {
	fn from(value: Location3) -> Self {
		Location(value.0, value.1)
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn end_rows_place_children_against_the_parent_end() {
		let input = FlowInput::new(Size::new(200, 40), Offset::new(10, 5), Size::new(50, 20), Offset::new(25, 10));

		let end = end_row(input);
		assert_eq!(end.child_offset(), Offset::new(160, 5));
		assert_eq!(end.next_cursor(), Offset::new(10, 5));

		let centered = centered_end_row(input);
		assert_eq!(centered.child_offset(), Offset::new(160, 15));
		assert_eq!(centered.next_cursor(), Offset::new(10, 5));
	}

	#[test]
	fn centering_flows_center_the_childs_visual_center() {
		// A 20 by 10 child that looks centered 2 units left of and 1 unit above its box's middle.
		let input = FlowInput::new(Size::new(100, 80), Offset::new(0, 0), Size::new(20, 10), Offset::new(8, 4));

		assert_eq!(center(input).child_offset(), Offset::new(42, 36));
		assert_eq!(centered_row(input).child_offset(), Offset::new(0, 36));
		assert_eq!(centered_column(input).child_offset(), Offset::new(42, 0));
		assert_eq!(centered_end_row(input).child_offset(), Offset::new(80, 36));
	}

	#[test]
	fn a_child_larger_than_the_parent_starts_at_its_edge_then_shifts_by_its_visual_center() {
		let input = FlowInput::new(Size::new(10, 10), Offset::new(0, 0), Size::new(20, 20), Offset::new(9, 11));

		assert_eq!(center(input).child_offset(), Offset::new(1, -1));
	}
}
