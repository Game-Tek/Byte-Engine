use math::Matrix;
use utils::Extent;

use crate::rendering::view::View;

/// The `Sink` struct provides a per-frame render destination for a resolved camera view.
///
/// It keeps the view transform, renderable extent, and renderer sink index
/// together while scene managers and render passes prepare an output surface.
#[derive(Debug, Clone, Copy)]
pub struct Sink {
	view: View,
	/// The view this sink rendered with in the previous frame, when that frame's images are still valid history.
	previous_view: Option<View>,
	extent: Extent,
	index: usize,
}

impl Sink {
	/// Creates a sink for render passes that target a specific view and extent.
	///
	/// The sink starts without history. Next, call [`Self::with_previous_view`] when the sink also rendered the
	/// previous frame at the same extent.
	pub fn new(view: View, extent: Extent, index: usize) -> Self {
		Self {
			view,
			previous_view: None,
			extent,
			index,
		}
	}

	/// Records the view this sink rendered with in the previous frame.
	///
	/// Set it only when the previous frame rendered this sink at the same extent. Temporal passes read
	/// [`Self::previous_view`] to reproject last frame's images, and treat `None` as "no usable history".
	pub fn with_previous_view(self, previous_view: View) -> Self {
		Self {
			previous_view: Some(previous_view),
			..self
		}
	}

	/// Returns the camera or light view used by this render target.
	pub fn view(&self) -> View {
		self.view
	}

	/// Returns the view of the previous frame, or `None` when this frame has no usable history.
	///
	/// History is unusable on the first frame of a sink and after a resize, because the images from the previous
	/// frame were never written or no longer match this frame's extent.
	pub fn previous_view(&self) -> Option<View> {
		self.previous_view
	}

	/// Returns the pixel extent available to render passes for this sink.
	pub fn extent(&self) -> Extent {
		self.extent
	}

	/// Returns the combined projection and view matrix for shader setup.
	pub fn view_projection(&self) -> Matrix {
		self.view.view_projection()
	}

	/// Returns the renderer-local sink index.
	pub fn index(&self) -> usize {
		self.index
	}
}
