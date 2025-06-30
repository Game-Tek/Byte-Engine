use crate::core::event::Event;

pub mod collider;
pub mod body;

pub mod world;

pub use world::World;
pub use collider::Collider;
pub use body::Body;

pub struct CollisionEvent {}

impl Event for CollisionEvent {}
