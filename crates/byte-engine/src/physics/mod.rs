//! Physics contracts and the built-in Dynabit simulation.
//!
//! Create [`Body`] payloads for entities that participate in simulation. Collider
//! geometry uses [`LocalSpace`], while simulated positions, velocities, and contacts
//! remain in the engine world space. Publish a transform under the same entity handle
//! to place the body in the world.

#[doc(hidden)]
pub mod body;
#[doc(hidden)]
pub mod collider;
mod contact_patch;
#[doc(hidden)]
pub mod convex_hull;
#[doc(hidden)]
pub mod dynabit;
mod epa;
#[doc(hidden)]
pub mod gjk;
#[doc(hidden)]
pub mod intersection;

/// The `LocalSpace` struct brands coordinates stored relative to a collider's origin.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct LocalSpace;

pub use body::{Body, BodyTypes};
pub use collider::Shapes;
pub use convex_hull::{ConvexHull, build_convex_hull};
pub use dynabit::World as DynabitWorld;
pub use dynabit::body::PhysicsBody;
pub use dynabit::contact::{Contact, Side};
pub use dynabit::step_lanes::{StepLanes, StepLanesError};
pub use epa::ContactScratch;
pub use gjk::{
	MinkowskiPoint, minkowski_support, rounded_shapes_contact, shapes_closest_points, shapes_contact, shapes_overlap,
};
pub use intersection::SweepBounds;
