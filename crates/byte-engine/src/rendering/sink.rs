use math::Matrix;
use utils::Extent;

use crate::{
	core::factory::Handle,
	rendering::{ExponentialHeightFog, SceneId, Sun, TargetId, TargetSet, view::View},
	time::MediaTime,
};

/// The shortest gap between the focus distance and the focal length, in meters. A lens can't focus nearer than its
/// focal length, where the circle of confusion grows without bound.
const MIN_FOCUS_BEYOND_FOCAL_LENGTH: f32 = 0.001;

/// The `Sink` struct provides a per-frame render destination for a resolved camera view.
///
/// It keeps the view transform, renderable extent, camera exposure and lens, frame interval, and renderer sink index
/// together while scene managers and render passes prepare an output surface.
#[derive(Debug, Clone, Copy)]
pub struct Sink {
	/// The view the frame renders from, without jitter: the camera's, moved to a point of the aperture while the lens
	/// is sampled.
	view: View,
	/// The sub-pixel offset the frame rasterizes through, in pixels; see [`Self::with_jitter`].
	jitter: [f32; 2],
	/// The camera's own view; see [`Self::camera_view`].
	camera_view: View,
	/// The extent scene pipelines and the passes before an upscaling pass render at; see [`Self::with_render_scale`].
	render_extent: Extent,
	/// The sink's own pixel extent: its window's or render image's.
	output_extent: Extent,
	/// Whether [`Self::extent`] is the output extent, for the passes from the upscaling pass on; see
	/// [`Self::at_output`].
	at_output: bool,
	exposure_scale: f32,
	/// The lens's f-number, focus distance in meters, and sensor height in meters; see [`Self::with_lens`].
	lens: [f32; 3],
	frame_interval: MediaTime,
	/// The player's motion preference; see [`Self::with_motion`].
	motion: crate::accessibility::MotionPreference,
	/// The shown debug views of the pass or scene pipeline preparing with this sink; see [`Self::with_debug_views`].
	debug_views: u32,
	/// The scene's sun; see [`Self::with_sun`].
	sun: Option<Sun>,
	/// The scene's fog; see [`Self::with_fog`].
	fog: Option<ExponentialHeightFog>,
	/// The camera the sink shows; see [`Self::with_camera`].
	camera: Option<Handle>,
	/// The scene the sink shows; see [`Self::with_scene`].
	scene: SceneId,
	/// The detail level the camera draws meshes at; see [`Self::with_detail_level`].
	detail_level: Option<u32>,
	/// The targets the sink's enabled passes read this frame; see [`Self::with_read_targets`].
	read_targets: TargetSet,
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
			jitter: [0.0; 2],
			render_extent: extent,
			output_extent: extent,
			at_output: false,
			exposure_scale: 1.0,
			lens: [f32::INFINITY, 10.0, 0.024],
			frame_interval: MediaTime::ZERO,
			motion: crate::accessibility::MotionPreference::Full,
			debug_views: 0,
			sun: None,
			fog: None,
			camera: None,
			scene: SceneId::WORLD,
			detail_level: None,
			read_targets: TargetSet::default(),
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

	/// Moves every point the frame rasterizes by `offset` pixels, with `x` growing right and `y` growing down. A pass
	/// that blends frames rasterized through different offsets, such as temporal antialiasing, resolves detail finer
	/// than a pixel.
	///
	/// [`Self::view`] keeps the view without the offset, which reprojection between frames and velocity use. Scene
	/// pipelines rasterize through [`Self::jittered_view`].
	pub fn with_jitter(mut self, offset: [f32; 2]) -> Self {
		self.jitter = offset;
		self
	}

	/// Sets the targets the sink's enabled passes read this frame. The renderer sets it before scene pipelines record, so
	/// a pipeline skips the work that writes a target nothing reads, such as the velocity stage while neither temporal
	/// antialiasing nor motion blur runs.
	pub fn with_read_targets(mut self, targets: TargetSet) -> Self {
		self.read_targets = targets;
		self
	}

	/// Returns whether an enabled pass of the sink reads `target` this frame; see [`Self::with_read_targets`]. Scene
	/// pipelines get `target` from [`crate::rendering::RenderPassBuilder::target_id`].
	pub fn reads(&self, target: TargetId) -> bool {
		self.read_targets.contains(target)
	}

	/// Returns the targets the sink's enabled passes read in its frame; see [`Self::with_read_targets`].
	pub(crate) fn read_targets(&self) -> TargetSet {
		self.read_targets
	}

	/// Returns the sub-pixel offset set by [`Self::with_jitter`], or zero while the sink isn't jittered.
	pub fn jitter(&self) -> [f32; 2] {
		self.jitter
	}

	/// Returns whether the frame rasterizes through a sub-pixel offset; see [`Self::with_jitter`]. Material texture
	/// reads select finer levels, and blue noise changes from frame to frame, while it does.
	pub fn is_jittered(&self) -> bool {
		self.jitter != [0.0; 2]
	}

	/// Returns the view scene pipelines rasterize through: [`Self::view`] moved by the [jitter](Self::with_jitter).
	pub fn jittered_view(&self) -> View {
		if self.is_jittered() {
			self.view.with_pixel_offset(self.jitter, self.render_extent)
		} else {
			self.view
		}
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

	/// Sets the player's motion preference. Passes that draw motion, such as motion blur, draw none under
	/// [`crate::accessibility::MotionPreference::Reduced`].
	pub fn with_motion(mut self, motion: crate::accessibility::MotionPreference) -> Self {
		self.motion = motion;
		self
	}

	/// Sets which debug views the pass or scene pipeline that prepares with this sink shows, packed as the debug view
	/// registry packs them. The renderer sets it for each node; passes read it through
	/// [`crate::rendering::DebugViews::shown`].
	pub(crate) fn with_debug_views(mut self, packed: u32) -> Self {
		self.debug_views = packed;
		self
	}

	/// Returns the packed debug views set with [`Self::with_debug_views`], `0` when none is shown.
	pub(crate) fn debug_views(&self) -> u32 {
		self.debug_views
	}

	/// Sets the sun the frame shows. Passes that draw sunlight in the air, such as the sky and the fog, read it with
	/// [`Self::sun`].
	pub fn with_sun(mut self, sun: Sun) -> Self {
		self.sun = Some(sun);
		self
	}

	/// Returns the scene's sun, or `None` while the scene has no [`crate::rendering::DirectionalLight`].
	pub fn sun(&self) -> Option<Sun> {
		self.sun
	}

	/// Sets the fog the frame shows. The height fog pass reads it with [`Self::fog`].
	pub fn with_fog(mut self, fog: ExponentialHeightFog) -> Self {
		self.fog = Some(fog);
		self
	}

	/// Returns the scene's newest [`ExponentialHeightFog`], or `None` while the scene has none.
	pub fn fog(&self) -> Option<ExponentialHeightFog> {
		self.fog
	}

	/// Returns the camera or light view used by this render target, without [jitter](Self::with_jitter). Passes that
	/// reproject between frames, cull, or place things in the world read it; rasterization reads
	/// [`Self::jittered_view`].
	pub fn view(&self) -> View {
		self.view
	}

	/// Returns the pixel extent the current node renders at: the [render extent](Self::render_extent) for scene
	/// pipelines and the passes before an upscaling pass, and the [output extent](Self::output_extent) from that pass
	/// on.
	pub fn extent(&self) -> Extent {
		if self.at_output {
			self.output_extent
		} else {
			self.render_extent
		}
	}

	/// Renders the scene at `scale` of the sink's output extent on each axis, rounded and at least one pixel, for the
	/// frames an enabled pass upscales; see the [render scale](/docs/reference/rendering#render-scale) specification.
	/// The renderer sets it from [`crate::rendering::render_pass::PassHints::render_scale`].
	pub fn with_render_scale(mut self, scale: f32) -> Self {
		self.render_extent = render_extent(self.output_extent, scale);
		self
	}

	/// Returns this sink as the passes from the upscaling pass on see it: [`Self::extent`] is the output extent. The
	/// renderer hands it to those passes.
	pub fn at_output(mut self) -> Self {
		self.at_output = true;
		self
	}

	/// Returns the extent the scene renders at, which an upscaling pass reads its inputs at.
	pub fn render_extent(&self) -> Extent {
		self.render_extent
	}

	/// Returns the sink's own pixel extent, which an upscaling pass writes `main` at.
	pub fn output_extent(&self) -> Extent {
		self.output_extent
	}

	/// Sets the detail level every mesh draws at, as [`crate::rendering::Camera::detail_level`] returns it, or `None`
	/// to select levels by distance.
	pub fn with_detail_level(mut self, level: Option<u32>) -> Self {
		self.detail_level = level;
		self
	}

	/// Returns the detail level every mesh draws at, or `None` while levels are selected by distance. See
	/// [`Self::with_detail_level`].
	pub fn detail_level(&self) -> Option<u32> {
		self.detail_level
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

	/// Returns the player's motion preference, which is full unless the renderer was told otherwise; see
	/// [`crate::rendering::Renderer::set_motion`].
	pub fn motion(&self) -> crate::accessibility::MotionPreference {
		self.motion
	}

	/// Returns the combined projection and view matrix that scene geometry rasterizes through, with the
	/// [jitter](Self::with_jitter). Overlays drawn after temporal antialiasing read [`Self::view`]'s instead.
	pub fn view_projection(&self) -> Matrix {
		self.jittered_view().view_projection()
	}

	/// Returns the renderer-local sink index.
	pub fn index(&self) -> usize {
		self.index
	}

	/// Sets the camera the sink shows, so passes can answer requests that name the camera, such as a
	/// [`crate::rendering::ScreenSceneQuery`].
	pub fn with_camera(mut self, camera: Handle) -> Self {
		self.camera = Some(camera);
		self
	}

	/// Returns the camera the sink shows, or `None` for a sink built without one, such as one made for a test.
	pub fn camera(&self) -> Option<Handle> {
		self.camera
	}

	/// Sets the scene the sink shows, so passes that draw one scene's objects, such as the world's debug meshes, leave
	/// the other scenes' sinks alone. A sink shows the world unless this sets another scene.
	pub fn with_scene(mut self, scene: SceneId) -> Self {
		self.scene = scene;
		self
	}

	/// Returns the scene the sink shows; see [`Self::with_scene`].
	pub fn scene(&self) -> SceneId {
		self.scene
	}
}

/// Returns `output` times `scale` on each axis, rounded to the nearest pixel and at least one, as the
/// [render scale](/docs/reference/rendering#render-scale) specification sizes the render extent.
fn render_extent(output: Extent, scale: f32) -> Extent {
	let axis = |pixels: u32| ((pixels as f32 * scale).round() as u32).max(1);
	Extent::rectangle(axis(output.width()), axis(output.height()))
}

#[cfg(test)]
mod tests {
	use utils::Extent;

	use super::Sink;
	use crate::rendering::view::View;

	#[test]
	fn passes_before_the_upscaler_see_the_scaled_extent_and_later_ones_the_output_extent() {
		let sink = Sink::new(View::identity(), Extent::rectangle(1920, 1080), 0).with_render_scale(2.0 / 3.0);
		assert_eq!(sink.extent(), Extent::rectangle(1280, 720));
		assert_eq!(sink.at_output().extent(), Extent::rectangle(1920, 1080));
		assert_eq!(sink.at_output().render_extent(), Extent::rectangle(1280, 720));
	}

	#[test]
	fn the_render_extent_keeps_at_least_one_pixel() {
		let sink = Sink::new(View::identity(), Extent::rectangle(3, 1), 0).with_render_scale(0.1);
		assert_eq!(sink.extent(), Extent::rectangle(1, 1));
	}
}
