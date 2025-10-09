use crate::core::event::Event;

pub mod collider;
pub mod body;
pub mod intersection;
pub mod collision;

pub mod world;
pub mod dynabit;

pub use world::World;
pub use body::BodyTypes;
pub use collider::Collider;
pub use body::Body;

pub struct CollisionEvent {}

impl Event for CollisionEvent {}
