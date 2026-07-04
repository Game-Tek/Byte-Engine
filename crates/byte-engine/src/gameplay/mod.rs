//! World-level gameplay objects and transform coordination.
//!
//! Headed applications use the default world to create renderables, lights,
//! cameras, and physics bodies through shared factories so rendering and physics
//! receive the same lifecycle messages. [`transform::Transform`] and [`Anchor`]
//! provide the shared spatial model for gameplay objects.

#[doc(hidden)]
pub mod anchor;
#[doc(hidden)]
pub mod collider;
#[doc(hidden)]
pub mod killer;
#[cfg(feature = "headed")]
#[doc(hidden)]
pub mod object;
#[doc(hidden)]
pub mod timer;
#[doc(hidden)]
pub mod transform;
#[cfg(feature = "headed")]
#[doc(hidden)]
pub mod world;

pub use anchor::{Anchor, AnchorSystem, Anchorage, Anchoring};
pub use collider::{Cube, Sphere};
pub use killer::KillMessage;
#[cfg(feature = "headed")]
pub use object::Object;
pub use transform::{Applicator, Transform, TransformationUpdate};
#[cfg(feature = "headed")]
pub use world::DefaultWorld;
