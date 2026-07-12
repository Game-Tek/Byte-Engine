use math::Vector3;

use crate::{
	audio::source::Source,
	core::{Entity, EntityHandle},
	space::Positionable,
};

/// The `Emitter` struct represents an audio source in a three-dimensional space.
pub struct Emitter {
	position: Vector3,
	source: EntityHandle<dyn Source>,
}

impl Emitter {
	pub fn new(position: Vector3, source: EntityHandle<dyn Source>) -> Self {
		Emitter { position, source }
	}

	pub fn source(&self) -> &EntityHandle<dyn Source> {
		&self.source
	}
}

impl Entity for Emitter {}

impl Positionable for Emitter {
	fn position(&self) -> Vector3 {
		self.position
	}

	fn set_position(&mut self, position: Vector3) {
		self.position = position;
	}
}

#[cfg(test)]
mod tests {
	use math::Vector3;

	use super::Emitter;
	use crate::{
		audio::{RoundRobin, Source},
		core::EntityHandle,
		space::Positionable,
	};

	#[test]
	fn emitter_preserves_source_identity_while_moving() {
		let concrete = EntityHandle::from(RoundRobin::new(vec!["step.wav".into()]));
		let source: EntityHandle<dyn Source> = concrete.clone();
		let mut emitter = Emitter::new(Vector3::new(1.0, 2.0, 3.0), source);

		assert_eq!(emitter.position(), Vector3::new(1.0, 2.0, 3.0));
		let emitter_source = emitter.source().get_lock();
		let concrete_source = concrete.get_lock();
		assert_eq!(
			std::sync::Arc::as_ptr(&emitter_source) as *const (),
			std::sync::Arc::as_ptr(&concrete_source) as *const ()
		);
		emitter.set_position(Vector3::new(-4.0, 5.0, -6.0));
		assert_eq!(emitter.position(), Vector3::new(-4.0, 5.0, -6.0));
	}
}
