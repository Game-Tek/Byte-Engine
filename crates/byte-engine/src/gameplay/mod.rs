//! World-level gameplay objects and transform coordination.
//!
//! [`world::DefaultWorld`] is the standard owner used by
//! [`crate::application::graphics::GraphicsApplication`]. Create renderables,
//! lights, cameras, and physics bodies through its factories so the renderer and
//! physics system receive the same lifecycle messages. [`transform::Transform`]
//! and [`Anchor`] provide the shared spatial model for gameplay objects.

pub mod anchor;
pub mod collider;
pub mod killer;
pub mod object;
pub mod timer;
pub mod transform;
pub mod world;

pub use anchor::Anchor;
pub use object::Object;
