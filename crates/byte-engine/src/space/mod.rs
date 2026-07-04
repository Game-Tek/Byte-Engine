//! Small spatial capability traits shared by gameplay, physics, and rendering.
//!
//! Implement [`Positionable`], [`Orientable`], and [`Scalable`] on types that
//! expose individual transform components. Implement [`Transformable`] when a
//! system needs to consume or replace the complete transform. These traits are
//! used by physics bodies, cameras, lights, and renderable meshes.

#[doc(hidden)]
pub mod orientable;
#[doc(hidden)]
pub mod positionable;
#[doc(hidden)]
pub mod scalable;
#[doc(hidden)]
pub mod transformable;

pub use orientable::Orientable;
pub use positionable::Positionable;
pub use scalable::Scalable;
pub use transformable::Transformable;
