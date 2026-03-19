use std::ops::Add;

#[derive(Clone, Copy)]
pub struct Offset(i32, i32);
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Location(u32, u32);
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Location3(u32, u32, u32);
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Size(u32, u32);

/// The `FlowInput` struct carries the parent space and cursor needed to place children.
#[derive(Clone, Copy)]
pub struct FlowInput {
	parent_size: Size,
	cursor: Offset,
	child_size: Size,
}

/// The `FlowOutput` struct carries the positioned child offset and the next cursor.
#[derive(Clone, Copy)]
pub struct FlowOutput {
	child_offset: Offset,
	next_cursor: Offset,
}

impl Offset {
	pub fn new(x: i32, y: i32) -> Self {
		Self(x, y)
	}

	#[inline]
	pub fn x(&self) -> i32 {
		self.0
	}

	#[inline]
	pub fn y(&self) -> i32 {
		self.1
	}
}

impl Add for Offset {
	type Output = Self;

	fn add(self, rhs: Self) -> Self::Output {
		Self(self.0 + rhs.0, self.1 + rhs.1)
	}
}

impl Into<Location> for Offset {
	fn into(self) -> Location {
		Location(self.0 as u32, self.1 as u32)
	}
}

impl Location {
	pub fn new(x: u32, y: u32) -> Self {
		Self(x, y)
	}

	#[inline]
	pub fn x(&self) -> u32 {
		self.0
	}

	#[inline]
	pub fn y(&self) -> u32 {
		self.1
	}
}

impl Add for Location {
	type Output = Self;

	fn add(self, rhs: Self) -> Self::Output {
		Self(self.0 + rhs.0, self.1 + rhs.1)
	}
}

impl Into<Offset> for Location {
	fn into(self) -> Offset {
		Offset(self.0 as i32, self.1 as i32)
	}
}

impl Into<(u32, u32)> for Location {
	fn into(self) -> (u32, u32) {
		(self.0, self.1)
	}
}

impl Size {
	pub fn new(width: u32, height: u32) -> Self {
		Self(width, height)
	}

	#[inline]
	pub fn x(&self) -> u32 {
		self.0
	}

	#[inline]
	pub fn y(&self) -> u32 {
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
		Self::new(offset, Offset(offset.0 + size.0 as i32, offset.1))
	}
}

pub fn row(input: FlowInput) -> FlowOutput {
	let offset = input.cursor;
	let size = input.child_size;
	FlowOutput::new(offset, Offset(offset.0 + size.0 as i32, offset.1))
}

pub fn column(input: FlowInput) -> FlowOutput {
	let offset = input.cursor;
	let size = input.child_size;
	FlowOutput::new(offset, Offset(offset.0, offset.1 + size.1 as i32))
}

pub fn grid(input: FlowInput) -> FlowOutput {
	let offset = input.cursor;
	let size = input.child_size;
	FlowOutput::new(offset, Offset(offset.0 + size.0 as i32, offset.1 + size.1 as i32))
}

pub fn row_with_gap(gap: u32) -> impl FlowFunction {
	move |input| {
		let offset = input.cursor;
		let size = input.child_size;
		FlowOutput::new(offset, Offset(offset.0 + size.0 as i32 + gap as i32, offset.1))
	}
}

pub fn column_with_gap(gap: u32) -> impl FlowFunction {
	move |input| {
		let offset = input.cursor;
		let size = input.child_size;
		FlowOutput::new(offset, Offset(offset.0, offset.1 + size.1 as i32 + gap as i32))
	}
}

pub fn centered_row(input: FlowInput) -> FlowOutput {
	let offset = Offset(
		input.cursor.0,
		input.cursor.1 + ((input.parent_size.1 as i32 - input.child_size.1 as i32) / 2).max(0),
	);

	FlowOutput::new(offset, Offset(offset.0 + input.child_size.0 as i32, offset.1))
}

pub fn centered_column(input: FlowInput) -> FlowOutput {
	let offset = Offset(
		input.cursor.0 + ((input.parent_size.0 as i32 - input.child_size.0 as i32) / 2).max(0),
		input.cursor.1,
	);

	FlowOutput::new(offset, Offset(input.cursor.0, input.cursor.1 + input.child_size.1 as i32))
}

pub fn center(input: FlowInput) -> FlowOutput {
	let offset = Offset(
		input.cursor.0 + ((input.parent_size.0 as i32 - input.child_size.0 as i32) / 2).max(0),
		input.cursor.1 + ((input.parent_size.1 as i32 - input.child_size.1 as i32) / 2).max(0),
	);

	FlowOutput::new(offset, input.cursor)
}

pub trait FlowFunction = Fn(FlowInput) -> FlowOutput + Copy;

impl Location3 {
	pub fn new(x: u32, y: u32, z: u32) -> Self {
		Self(x, y, z)
	}

	pub fn x(&self) -> u32 {
		self.0
	}

	pub fn y(&self) -> u32 {
		self.1
	}

	pub fn z(&self) -> u32 {
		self.2
	}
}

impl From<(Location, u32)> for Location3 {
	fn from(value: (Location, u32)) -> Self {
		Location3(value.0 .0, value.0 .1, value.1)
	}
}

impl From<Location3> for Location {
	fn from(value: Location3) -> Self {
		Location(value.0, value.1)
	}
}
