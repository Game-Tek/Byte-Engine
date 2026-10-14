//! World-level gameplay objects and transform coordination.
//!
//! Headed applications use the default world to create renderables, lights,
//! cameras, and physics bodies through shared factories so rendering and physics
//! receive the same lifecycle messages. [`transform::Transform`] and [`Anchor`]
//! provide the shared spatial model for gameplay objects. [`Scene`] and
//! [`SceneNode`] group entities so a level can be destroyed as a unit.

#[doc(hidden)]
pub mod anchor;
#[doc(hidden)]
pub mod name;

#[doc(hidden)]
pub mod pathfinding;
#[doc(hidden)]
pub mod scene;
#[doc(hidden)]
pub mod transform;
#[doc(hidden)]
pub mod world;

pub use anchor::{Anchor, Anchorage, Anchoring};
pub use name::Name;
pub use scene::{Scene, SceneNode};
pub use transform::{Transform, TransformationUpdate};
pub use world::DefaultWorld;
