use math::Vector3;

pub struct Contact {
	pub(crate) a: Side,
	pub(crate) b: Side,
	pub(crate) normal: Vector3,
	pub(crate) depth: f32,
}

pub struct Side {
	/// The object handle for this side of the contact.
	pub(crate) object: usize,
	/// The point in the world where the contact ocurred.
	pub(crate) point: Vector3,
}
