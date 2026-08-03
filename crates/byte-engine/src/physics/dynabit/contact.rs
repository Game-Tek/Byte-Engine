use math::{collision::DynamicIntersection, collision::Intersection, Point, UnitVector};

/// The `Contact` struct stores the world-space data required to resolve one collision.
#[derive(Debug)]
pub struct Contact {
	pub(crate) a: Side,
	pub(crate) b: Side,
	pub(crate) normal: UnitVector,
	pub(crate) depth: f32,
	pub(crate) toi: f32,
}

impl Contact {
	/// Converts a static math intersection into a physics contact.
	pub(crate) fn from_static(a: usize, b: usize, intersection: Intersection, toi: f32) -> Self {
		Self {
			a: Side {
				object: a,
				point: intersection.point_on_a(),
			},
			b: Side {
				object: b,
				point: intersection.point_on_b(),
			},
			normal: intersection.normal(),
			depth: intersection.depth(),
			toi,
		}
	}

	/// Converts a dynamic math intersection into a physics contact.
	pub(crate) fn from_dynamic(a: usize, b: usize, intersection: DynamicIntersection) -> Self {
		let toi = intersection.toi();
		Self::from_static(a, b, intersection.into_contact(), toi)
	}
}

impl PartialEq for Contact {
	fn eq(&self, other: &Self) -> bool {
		self.toi == other.toi
	}
}

impl Eq for Contact {}

impl PartialOrd for Contact {
	fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
		Some(self.cmp(other))
	}
}

impl Ord for Contact {
	fn cmp(&self, other: &Self) -> std::cmp::Ordering {
		self.toi.total_cmp(&other.toi)
	}
}

/// The `Side` struct identifies one body and its world-space contact point.
#[derive(Debug)]
pub struct Side {
	pub(crate) object: usize,
	pub(crate) point: Point,
}

/// The `Pair` struct identifies two broad-phase body indices.
#[derive(Debug, Clone, Copy)]
pub struct Pair {
	pub a: usize,
	pub b: usize,
}

impl Pair {
	/// Creates a pair from two body indices.
	pub fn new(a: usize, b: usize) -> Self {
		Self { a, b }
	}
}

impl Eq for Pair {}

impl PartialEq for Pair {
	fn eq(&self, other: &Self) -> bool {
		(self.a == other.a && self.b == other.b) || (self.a == other.b && self.b == other.a)
	}
}
