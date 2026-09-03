//! Retained wireframe geometry for renderer and gameplay diagnostics.
//!
//! Register the pass with
//! [`crate::application::graphics::setup_debug_mesh_render_pass`], then create
//! [`DebugMesh`] values through the returned factory. Derive a replacement under
//! the same handle to update a mesh, or delete its handle through the world.

mod render_pass;
mod scene_manager;

pub use render_pass::DebugMeshRenderPass;
pub use scene_manager::DebugSceneManager;

/// The `DebugShape` enum describes world-space geometry expanded by the debug scene manager.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum DebugShape {
	/// A sphere centered at `center` with a world-space `radius`.
	Sphere { center: Point, radius: f32 },
	/// An oriented box whose size extends by `half_extents` from `center`.
	Box {
		center: Point,
		half_extents: Vector,
		orientation: Orientation,
	},
	/// A capsule whose center line runs from `start` to `end`.
	Capsule { start: Point, end: Point, radius: f32 },
	/// A thin line segment between two world-space endpoints.
	Segment { start: Point, end: Point },
}

/// The `DebugDepthMode` enum selects whether a debug shape is hidden by nearer scene geometry.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum DebugDepthMode {
	/// Tests against the depth image produced by the active scene pipeline without changing it.
	#[default]
	Scene,
	/// Draws after depth-aware debug geometry without a depth attachment.
	Ignore,
}

/// The `DebugMesh` struct describes retained diagnostic geometry shared by every render sink.
///
/// Create a message with [`Self::new`], optionally select [`Self::depth_mode`],
/// then publish it through the factory returned by
/// [`crate::application::graphics::setup_debug_mesh_render_pass`].
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct DebugMesh {
	shape: DebugShape,
	color: RGBA,
	depth_mode: DebugDepthMode,
}

impl DebugMesh {
	/// Creates a retained, scene-depth-aware debug mesh.
	pub const fn new(shape: DebugShape, color: RGBA) -> Self {
		Self {
			shape,
			color,
			depth_mode: DebugDepthMode::Scene,
		}
	}

	/// Selects whether this shape tests against the scene depth image.
	pub const fn depth_mode(mut self, depth_mode: DebugDepthMode) -> Self {
		self.depth_mode = depth_mode;
		self
	}

	/// Returns the world-space shape carried by this message.
	pub const fn shape(self) -> DebugShape {
		self.shape
	}

	/// Returns the straight-alpha color written by the debug fragment shader.
	pub const fn color(self) -> RGBA {
		self.color
	}

	/// Returns the scene-depth behavior selected for this shape.
	pub const fn selected_depth_mode(self) -> DebugDepthMode {
		self.depth_mode
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn debug_mesh_defaults_to_scene_depth_and_can_ignore_it() {
		let shape = DebugShape::Sphere {
			center: Point::origin(),
			radius: 1.0,
		};
		let message = DebugMesh::new(shape, RGBA::white());
		assert_eq!(message.selected_depth_mode(), DebugDepthMode::Scene);

		let message = message.depth_mode(DebugDepthMode::Ignore);
		assert_eq!(message.selected_depth_mode(), DebugDepthMode::Ignore);
	}
}

use math::{Orientation, Point, Vector};
use utils::RGBA;
