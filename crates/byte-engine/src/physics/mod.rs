//! Physics contracts and the built-in Dynabit simulation.
//!
//! Implement [`Body`] and [`Collider`] on world entities that participate in
//! simulation. Collider geometry uses [`LocalSpace`], while simulated positions,
//! velocities, and contacts remain in the engine world space.

#[doc(hidden)]
pub mod body;
#[doc(hidden)]
pub mod bounds;
#[doc(hidden)]
pub mod collider;
#[doc(hidden)]
pub mod dynabit;
#[doc(hidden)]
pub mod intersection;
#[doc(hidden)]
pub mod world;

/// The `LocalSpace` struct brands coordinates stored relative to a collider's origin.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct LocalSpace;

pub use body::{Body, BodyTypes};
pub use bounds::Bounds;
pub use collider::{Collider, Shapes};
pub use dynabit::body::PhysicsBody;
pub use dynabit::contact::{Contact, Pair, Side};
pub use dynabit::World as DynabitWorld;
pub use intersection::{Intersection, PseudoBody};
pub use world::World;
