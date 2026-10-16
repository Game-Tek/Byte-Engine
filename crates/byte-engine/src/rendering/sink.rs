use math::Matrix;
use utils::Extent;

use crate::{rendering::view::View, time::MediaTime};

/// The `Sink` struct provides a per-frame render destination for a resolved camera view.
///
/// It keeps the view transform, renderable extent, camera exposure, frame interval, and renderer sink index
/// together while scene managers and render passes prepare an output surface.
#[derive(Debug, Clone, Copy)]
pub struct Sink {
	view: View,
	extent: Extent,
	exposure_scale: f32,
	frame_interval: MediaTime,
	index: usize,
}

impl Sink {
	/// Creates a sink with neutral exposure for render passes that target a specific view and extent.
	///
	/// Next, call [`Self::with_exposure_scale`] when the view comes from a [`crate::rendering::Camera`].
	pub fn new(view: View, extent: Extent, index: usize) -> Self {
		Self {
			view,
			extent,
			exposure_scale: 1.0,
			frame_interval: MediaTime::ZERO,
			index,
		}
	}

	/// Sets the camera exposure as a linear factor, as returned by [`crate::rendering::Camera::exposure_scale`].
	pub fn with_exposure_scale(mut self, exposure_scale: f32) -> Self {
		self.exposure_scale = exposure_scale;
		self
	}

	/// Sets the time since the previous frame. Passes that simulate exposure over time, such as motion blur, scale
	/// what changed since the previous frame by it.
	pub fn with_frame_interval(mut self, frame_interval: MediaTime) -> Self {
		self.frame_interval = frame_interval;
		self
	}

	/// Returns the camera or light view used by this render target.
	pub fn view(&self) -> View {
		self.view
	}

	/// Returns the pixel extent available to render passes for this sink.
	pub fn extent(&self) -> Extent {
		self.extent
	}

	/// Returns the camera exposure as a linear factor. Passes that write scene light multiply it in, so later passes
	/// and the tone mapper receive exposed light.
	pub fn exposure_scale(&self) -> f32 {
		self.exposure_scale
	}

	/// Returns the time since the previous frame, or zero when the sink was created without one, such as the
	/// first frame or a sink built for a test.
	pub fn frame_interval(&self) -> MediaTime {
		self.frame_interval
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
