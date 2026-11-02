//! World-level gameplay objects and transform coordination.
//!
//! Applications create renderables, lights, cameras, and physics bodies through
//! the factories of their world scope, so rendering and physics receive the same
//! lifecycle messages on the routes each message type names.
//! [`transform::Transform`] and [`Anchor`] provide the shared spatial model for
//! gameplay objects. [`Scene`] and [`SceneNode`] group entities so a level can be
//! destroyed as a unit, which [`SceneGraph`] carries out. A [`Spawner`] spawns a
//! baked level's entities under one scene.

pub mod anchor;
pub mod level;
pub mod name;
pub mod pathfinding;
pub mod scene;
pub mod tasks;
pub mod transform;

pub use anchor::{Anchor, Anchorage, Anchoring};
pub use level::{EntityRef, Level, Spawner};
pub use name::Name;
pub use scene::{Scene, SceneGraph, SceneNode};
pub use tasks::{Interval, Runtime, Tasks};
pub use transform::{Transform, TransformationUpdate};
