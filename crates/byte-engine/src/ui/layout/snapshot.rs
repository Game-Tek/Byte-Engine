use super::{
	element::Id,
	flow::{Location, Size},
};
use crate::ui::{UiPoint, intersection::MouseClickAcceleration, point::normalized_to_layout};

/// The `Snapshot` struct lets a host inspect one evaluated frame's layout and hit-test pointer positions against it.
///
/// It borrows the engine's retained layout, so it lives only until the engine is used again. Get one from
/// [`crate::ui::Engine::evaluate`], then drop it and call [`crate::ui::Engine::render`].
pub struct Snapshot<'a> {
	/// The evaluated layout, which tests inspect directly.
	#[cfg(test)]
	pub(super) elements: &'a [super::LayoutElement],
	pub(super) acceleration: &'a MouseClickAcceleration,
	pub(super) size: Size,
}

impl Snapshot<'_> {
	/// Retains this render's clipped hit geometry without borrowing frame storage.
	/// Next, use [`crate::ui::intersection::HitTest::query`] to arbitrate input
	/// before the following layout evaluation. Reuse `target` across frames.
	pub fn retain_hit_test(&self, target: &mut crate::ui::intersection::HitTest) {
		self.acceleration.retain(target, self.size);
	}

	/// Returns the frontmost surface under normalized window coordinates, skipping `excluded` such as a held drag
	/// source.
	///
	/// The engine routes clicks, scrolls, and hover changes through this same query, so a host that hit-tests here
	/// sees the target the components hear about.
	pub fn hit(&self, position: UiPoint, excluded: Option<Id>) -> Option<Id> {
		let point = normalized_to_layout(position, self.size);
		self.acceleration
			.query_excluding(Location::new(point.x, point.y), excluded.map(Id::get))
			.and_then(Id::new)
	}

	pub fn size(&self) -> Size {
		self.size
	}
}
