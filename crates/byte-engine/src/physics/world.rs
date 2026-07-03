use crate::application::Time;

/// The `World` trait defines the interface for a physics simulation environment.
pub trait World {
	fn update(&mut self, time: Time);
}
