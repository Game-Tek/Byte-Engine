//! Physics contracts and the built-in Dynabit simulation.
//!
//! Implement [`Body`] and [`Collider`] on world entities that participate in
//! simulation. Most headed applications use [`dynabit::World`] indirectly
//! through the default world, which forwards body creation, transform updates,
//! and deletion messages.

#[doc(hidden)]
pub mod body;
#[doc(hidden)]
pub mod bounds;
#[doc(hidden)]
pub mod collider;
#[doc(hidden)]
pub mod intersection;

#[doc(hidden)]
pub mod dynabit;
#[doc(hidden)]
pub mod world;

pub use body::Body;
pub use body::BodyTypes;
pub use bounds::Bounds;
pub use collider::Collider;
pub use collider::Shapes;
pub use dynabit::body::PhysicsBody;
pub use dynabit::contact::{Contact, Pair, Side};
pub use dynabit::World as DynabitWorld;
pub use intersection::{Intersection, PseudoBody};
pub use world::World;
