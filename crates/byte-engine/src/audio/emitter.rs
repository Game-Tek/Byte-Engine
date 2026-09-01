use math::Point;

use crate::{
	audio::source::Source,
	core::{Entity, EntityHandle},
	space::Positionable,
};

/// The `Emitter` struct connects an audio [`Source`] to a world-space position.
pub struct Emitter {
	position: Point,
	source: EntityHandle<dyn Source>,
}

impl Emitter {
	/// Creates an emitter at `position` for `source`.
	pub fn new(position: Point, source: EntityHandle<dyn Source>) -> Self {
		Self { position, source }
	}

	/// Returns the source that this emitter plays.
	pub fn source(&self) -> &EntityHandle<dyn Source> {
		&self.source
	}
}

impl Entity for Emitter {}

impl Positionable for Emitter {
	fn position(&self) -> Point {
		self.position
	}

	fn set_position(&mut self, position: Point) {
		self.position = position;
	}
}
