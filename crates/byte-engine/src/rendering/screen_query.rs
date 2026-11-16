//! Asking the renderer which entity a camera shows at a pixel.
//!
//! The rules a query and its answer follow are specified under
//! [Screen scene queries](/docs/reference/rendering#screen-scene-queries).

use crate::core::factory::Handle;

/// The `ScreenSceneQuery` struct asks which entity a camera shows at one pixel, so tools and games can pick what the
/// pointer is over, such as an editor selecting the mesh under a click.
///
/// Create it through [`crate::core::message_bus::MessageScope::factory`] on the world scope, and keep the handle the
/// creation returns. Next, listen to the world's [`ScreenSceneHit`] factory and match [`ScreenSceneHit::query`] to it:
/// the visibility pipeline answers every query exactly once, about two frames later.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ScreenSceneQuery {
	camera: Handle,
	point: [u32; 2],
}

impl ScreenSceneQuery {
	/// Asks which entity `camera` shows at `point`, in pixels of its window's render extent from the top-left pixel.
	pub fn new(camera: Handle, point: [u32; 2]) -> Self {
		Self { camera, point }
	}

	/// The camera whose window the point is in.
	pub fn camera(&self) -> Handle {
		self.camera
	}

	/// The pixel, as `[x, y]` from the top-left pixel of the window's render extent.
	pub fn point(&self) -> [u32; 2] {
		self.point
	}
}

/// The `ScreenSceneHit` struct answers one [`ScreenSceneQuery`], so the application learns what was under the pixel.
///
/// The visibility pipeline creates it on the world scope once the frame that read the pixel completes on the GPU.
/// Match [`Self::query`] to the handle the query was created with.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ScreenSceneHit {
	query: Handle,
	entity: Option<Handle>,
}

impl ScreenSceneHit {
	/// Answers `query` with the renderable mesh `entity` at its pixel, or with `None` when the pixel shows no mesh.
	pub fn new(query: Handle, entity: Option<Handle>) -> Self {
		Self { query, entity }
	}

	/// The handle the answered query was created with.
	pub fn query(&self) -> Handle {
		self.query
	}

	/// The renderable mesh whose surface covers the pixel, or `None` when the pixel shows no mesh, such as the sky.
	pub fn entity(&self) -> Option<Handle> {
		self.entity
	}
}
