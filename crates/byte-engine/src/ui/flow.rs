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
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct Size(f32, f32);

/// The `FlowInput` struct passes parent space, cursor, and child size
/// into reusable layout flow functions.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct FlowInput {
	parent_size: Size,
	cursor: Offset,
	child_size: Size,
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
	pub fn new(parent_size: Size, cursor: Offset, child_size: Size) -> Self {
		Self {
			parent_size,
			cursor,
			child_size,
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
		input.cursor.1 + ((input.parent_size.1 - input.child_size.1) * 0.5).max(0.0),
	);

	FlowOutput::new(offset, Offset(input.cursor.0 + input.child_size.0, input.cursor.1))
}

pub fn centered_column(input: FlowInput) -> FlowOutput {
	let offset = Offset(
		input.cursor.0 + ((input.parent_size.0 - input.child_size.0) * 0.5).max(0.0),
		input.cursor.1,
	);

	FlowOutput::new(offset, Offset(input.cursor.0, input.cursor.1 + input.child_size.1))
}

pub fn center(input: FlowInput) -> FlowOutput {
	let offset = Offset(
		input.cursor.0 + ((input.parent_size.0 - input.child_size.0) * 0.5).max(0.0),
		input.cursor.1 + ((input.parent_size.1 - input.child_size.1) * 0.5).max(0.0),
	);

	FlowOutput::new(offset, input.cursor)
}

pub trait FlowFunction = Fn(FlowInput) -> FlowOutput + Copy;

/// Identifies pure built-in flows and the gap captured by their closure variants.
/// Custom callables, including erased function pointers, must replay placement.
pub(crate) fn placement_key(flow: &utils::InlineCopyFn<fn(FlowInput) -> FlowOutput>) -> Option<(std::any::TypeId, FlowOutput)> {
	use std::any::Any;
	let kind = flow.callable_type_id();
	let builtin = [
		row.type_id(),
		column.type_id(),
		grid.type_id(),
		centered_row.type_id(),
		centered_column.type_id(),
		center.type_id(),
		row_with_gap(0.0).type_id(),
		column_with_gap(0.0).type_id(),
	]
	.contains(&kind);
	// The gap is the only captured input in built-in flows. Zero sizes and cursor
	// expose it without rounding it against a nonzero size or copying closure bytes.
	builtin.then(|| {
		(
			kind,
			flow.call(FlowInput::new(Size::new(0, 0), Offset::new(0, 0), Size::new(0, 0))),
		)
	})
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
