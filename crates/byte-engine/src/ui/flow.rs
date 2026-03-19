use std::ops::Add;

#[derive(Clone, Copy)]
pub struct Offset(i32, i32);
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Location(u32, u32);
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Location3(u32, u32, u32);
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Size(u32, u32);

impl Offset {
	pub fn new(x: i32, y: i32) -> Self {
		Self(x, y)
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

pub fn row(offset: Offset, size: Size) -> Offset {
	Offset(offset.0 + size.0 as i32, offset.1)
}

pub fn column(offset: Offset, size: Size) -> Offset {
	Offset(offset.0, offset.1 + size.1 as i32)
}

pub fn grid(offset: Offset, size: Size) -> Offset {
	Offset(offset.0 + size.0 as i32, offset.1 + size.1 as i32)
}

pub fn row_with_gap(gap: u32) -> impl FlowFunction {
	move |offset, size| Offset(offset.0 + size.0 as i32 + gap as i32, offset.1)
}

pub fn column_with_gap(gap: u32) -> impl FlowFunction {
	move |offset, size| Offset(offset.0, offset.1 + size.1 as i32 + gap as i32)
}

pub trait FlowFunction = Fn(Offset, Size) -> Offset + Copy;

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
