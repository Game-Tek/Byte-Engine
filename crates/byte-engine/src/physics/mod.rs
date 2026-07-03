//! Physics contracts and the built-in Dynabit simulation.
//!
//! Implement [`Body`] and [`Collider`] on world entities that participate in
//! simulation. Most applications use [`dynabit::World`] indirectly through
//! [`crate::gameplay::world::DefaultWorld`], which forwards body creation,
//! transform updates, and deletion messages.

pub mod body;
pub mod bounds;
pub mod collider;
pub mod intersection;

pub mod dynabit;
pub mod world;

pub use body::Body;
pub use body::BodyTypes;
pub use collider::Collider;
pub use world::World;
