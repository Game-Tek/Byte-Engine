use crate::application::Time;

/// The `World` trait defines the update boundary for a physics simulation.
pub trait World {
	fn update(&mut self, time: Time);
}
