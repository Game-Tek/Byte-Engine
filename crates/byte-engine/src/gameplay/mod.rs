//! World-level gameplay objects and transform coordination.
//!
//! Applications create renderables, lights, cameras, and physics bodies through
//! the factories of their world scope, so rendering and physics receive the same
//! lifecycle messages; [`World`] names the routes those systems share.
//! [`transform::Transform`] and [`Anchor`] provide the shared spatial model for
//! gameplay objects. [`Scene`] and [`SceneNode`] group entities so a level can be
//! destroyed as a unit, which [`SceneGraph`] carries out.

#[doc(hidden)]
pub mod anchor;
#[doc(hidden)]
pub mod name;

#[doc(hidden)]
pub mod pathfinding;
#[doc(hidden)]
pub mod scene;
pub mod tasks;
#[doc(hidden)]
pub mod transform;
#[doc(hidden)]
pub mod world;

pub use anchor::{Anchor, Anchorage, Anchoring};
pub use name::Name;
pub use scene::{Scene, SceneGraph, SceneNode};
pub use tasks::{TaskRuntime, Tasks};
pub use transform::{Transform, TransformationUpdate};
pub use world::World;
