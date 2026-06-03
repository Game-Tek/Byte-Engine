use math::Vector3;

#[derive(Debug)]
pub struct Contact {
	pub(crate) a: Side,
	pub(crate) b: Side,
	pub(crate) normal: Vector3,
	pub(crate) depth: f32,
	pub(crate) toi: f32,
}

impl PartialEq for Contact {
	fn eq(&self, other: &Self) -> bool {
		self.toi == other.toi
	}
}

impl Eq for Contact {}

impl PartialOrd for Contact {
	fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
		self.toi.partial_cmp(&other.toi)
	}
}

impl Ord for Contact {
	fn cmp(&self, other: &Self) -> std::cmp::Ordering {
		self.toi.partial_cmp(&other.toi).unwrap_or(std::cmp::Ordering::Equal)
	}
}

#[derive(Debug)]
pub struct Side {
	/// The object handle for this side of the contact.
	pub(crate) object: usize,
	/// The point in the world where the contact ocurred.
	pub(crate) point: Vector3,
}
