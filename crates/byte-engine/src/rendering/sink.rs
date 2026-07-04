use math::Matrix4;
use utils::Extent;

use crate::rendering::view::View;

/// The `Sink` struct represents a per-frame render destination for a resolved camera view.
///
/// A sink exists to keep the view transform, renderable extent, and renderer sink index together
/// while scene managers and render passes prepare work for a specific output surface.
#[derive(Debug, Clone, Copy)]
pub struct Sink {
	view: View,
	extent: Extent,
	index: usize,
}

impl Sink {
	/// Creates a sink for render passes that target a specific view and extent.
	pub fn new(view: View, extent: Extent, index: usize) -> Self {
		Self { view, extent, index }
	}

	/// Returns the camera or light view used by this render target.
	pub fn view(&self) -> View {
		self.view
	}

	/// Returns the pixel extent available to render passes for this sink.
	pub fn extent(&self) -> Extent {
		self.extent
	}

	/// Returns the combined projection and view matrix for shader setup.
	pub fn view_projection(&self) -> Matrix4 {
		self.view.view_projection()
	}

	/// Returns the renderer-local sink index.
	pub fn index(&self) -> usize {
		self.index
	}
}
