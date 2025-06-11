use math::Vector3;

use crate::{audio::source::Source, core::{Entity, EntityHandle}, gameplay::Positionable};

/// The `Emitter` struct represents an audio source in a three-dimensional space.
pub struct Emitter {
    position: Vector3,
    source: EntityHandle<dyn Source>,
}

impl Emitter {
    pub fn new(position: Vector3, source: EntityHandle<dyn Source>) -> Self {
        Emitter {
            position,
            source,
        }
    }

    pub fn source(&self) -> &EntityHandle<dyn Source> {
        &self.source
    }
}

impl Entity for Emitter {}

impl Positionable for Emitter {
	fn get_position(&self) -> Vector3 {
		self.position
	}

	fn set_position(&mut self, position: Vector3) {
		self.position = position;
	}
}
