use math::Matrix;
use utils::Extent;

use crate::{rendering::view::View, time::MediaTime};

/// The shortest gap between the focus distance and the focal length, in meters. A lens can't focus nearer than its
/// focal length, where the circle of confusion grows without bound.
const MIN_FOCUS_BEYOND_FOCAL_LENGTH: f32 = 0.001;

/// The `Sink` struct provides a per-frame render destination for a resolved camera view.
///
/// It keeps the view transform, renderable extent, camera exposure and lens, frame interval, and renderer sink index
/// together while scene managers and render passes prepare an output surface.
#[derive(Debug, Clone, Copy)]
pub struct Sink {
	/// The view the frame renders from: the camera's, moved to a point of the aperture while the lens is sampled.
	view: View,
	/// The camera's own view; see [`Self::camera_view`].
	camera_view: View,
	extent: Extent,
	exposure_scale: f32,
	/// The lens's f-number, focus distance in meters, and sensor height in meters; see [`Self::with_lens`].
	lens: [f32; 3],
	frame_interval: MediaTime,
	index: usize,
}

impl Sink {
	/// Creates a sink with neutral exposure and a pinhole lens for render passes that target a specific view and
	/// extent.
	///
	/// Next, call [`Self::with_exposure_scale`] and [`Self::with_lens`] when the view comes from a
	/// [`crate::rendering::Camera`].
	pub fn new(view: View, extent: Extent, index: usize) -> Self {
		Self {
			view,
			camera_view: view,
			extent,
			exposure_scale: 1.0,
			lens: [f32::INFINITY, 10.0, 0.024],
			frame_interval: MediaTime::ZERO,
			index,
		}
	}

	/// Sets the camera's lens, as [`crate::rendering::Camera::f_number`], [`crate::rendering::Camera::focus_distance`],
	/// and [`crate::rendering::Camera::sensor_height`] return it. The depth-of-field pass blurs by it.
	pub fn with_lens(mut self, f_number: f32, focus_distance: f32, sensor_height: f32) -> Self {
		self.lens = [f_number, focus_distance, sensor_height];
		self
	}

	/// Returns the lens's f-number, focus distance in meters, and sensor height in meters. See [`Self::with_lens`].
	pub fn lens(&self) -> [f32; 3] {
		self.lens
	}

	/// Returns the thin lens the camera's view and [lens](Self::with_lens) describe: its focal length, aperture
	/// diameter, and focus distance, all in meters. The focus distance is held beyond the focal length, nearer than
	/// which a lens can't focus.
	pub(crate) fn thin_lens(&self) -> [f32; 3] {
		let [f_number, focus_distance, sensor_height] = self.lens;
		let focal_length = sensor_height / (2.0 * (self.camera_view.y_fov().to_radians() * 0.5).tan());
		[
			focal_length,
			focal_length / f_number,
			focus_distance.max(focal_length + MIN_FOCUS_BEYOND_FOCAL_LENGTH),
		]
	}

	/// Moves the view to the point `sample` of the lens's aperture, a point within the unit disc, keeping the focus
	/// distance in focus. Call it after [`Self::with_lens`]. A pass that sees a frame from many such points, such as
	/// the depth-of-field reference, averages them into the lens's blur.
	///
	/// The aperture is scaled by `S / (S − F)`, for focus distance `S` and focal length `F`, so that each depth blurs
	/// by the thin-lens circle of confusion a lens of that focal length on the camera's sensor gives it.
	pub fn with_lens_sample(mut self, sample: [f32; 2]) -> Self {
		let [focal_length, aperture, focus_distance] = self.thin_lens();
		let radius = aperture * 0.5 * focus_distance / (focus_distance - focal_length);
		self.view = self
			.camera_view
			.with_lens_offset([sample[0] * radius, sample[1] * radius], focus_distance);
		self
	}

	/// Returns the camera's view before [`Self::with_lens_sample`] moved it, which stays the same while the camera
	/// stands still.
	pub fn camera_view(&self) -> View {
		self.camera_view
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
