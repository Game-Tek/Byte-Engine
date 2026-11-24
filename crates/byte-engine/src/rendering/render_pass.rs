//! Composable sink-local rendering stages.
//!
//! Implement a [`RenderFeature`] and its [`RenderPass`] for post-processing or overlays that run after scene
//! pipelines: the feature holds what every sink shares, and each sink gets its own pass. See
//! [render features and passes](/docs/develop/rendering/render-features). Construct resources through
//! [`RenderPassBuilder`] so the renderer
//! can track access policies and named render targets. Existing implementations
//! live in [`crate::rendering::render_passes`].
//! Replace the color flowing through the graph with
//! [`RenderPassBuilder::create_main_render_target`]. The builder supplies an
//! intermediate image or the sink's final target, a window's swapchain or a render image, without exposing graph
//! position to the pass.

pub mod pixel_steps;
pub mod simple_compute;
pub mod simple_tile;

use std::sync::{Arc, atomic::AtomicU32};

use ghi::context::ContextCreate as _;
use smallvec::SmallVec;

use crate::rendering::{
	Sink,
	debug_views::{DebugViewSettings, DebugViews, MAX_VIEWS_PER_PASS},
	render_passes::blit::ImageBypassPass,
	renderer::{Placement, RenderTargets},
};

pub trait RenderPassFunction = Fn(&mut ghi::implementation::CommandBufferRecording);

/// Returns the name and format a render target registers under, which its image builder carries.
fn target_identity<'a>(builder: &ghi::image::Builder<'a>) -> (&'a str, ghi::Formats) {
	let name = builder.get_name().expect(
		"Render target name is missing. The most likely cause is that the image builder was not given a name before creating the target.",
	);
	(name, builder.get_format())
}

/// A frame-allocated command that records one render pass.
pub type RenderPassReturn<'a> = &'a (dyn RenderPassFunction + Send + Sync + 'a);

/// Allocates a prepared render command in the application frame allocator.
pub fn allocate_render_command<'a>(
	frame_allocator: &'a bumpalo::Bump,
	command: impl RenderPassFunction + Send + Sync + 'a,
) -> RenderPassReturn<'a> {
	frame_allocator.alloc(command)
}

/// The `Adoption` struct holds what a simple pass builds from its published pipeline, such as its descriptor sets,
/// and builds it again each time the pipeline is rebuilt.
///
/// A pass whose pipeline or adoption failed logs the failure once and stays idle until the pipeline is rebuilt.
#[derive(Clone, Copy)]
struct Adoption<T: Copy> {
	/// The pipeline revision that `ready`, or the failure, belongs to.
	revision: u64,
	ready: Option<T>,
	failed: bool,
}

impl<T: Copy> Default for Adoption<T> {
	fn default() -> Self {
		Self {
			revision: 0,
			ready: None,
			failed: false,
		}
	}
}

impl<T: Copy> Adoption<T> {
	/// Returns what `adopt` built from `pipeline`, calling it once the pipeline is published and again after each
	/// rebuild. Returns `None` while the pipeline is pending, or when it or `adopt` failed. `label` names the pass in
	/// the logged errors.
	fn get(
		&mut self,
		pipeline_manager: &crate::rendering::PipelineManagerClient,
		pipeline: crate::rendering::PipelineRef,
		label: &str,
		adopt: impl FnOnce() -> Result<T, String>,
	) -> Option<T> {
		let revision = pipeline_manager.revision(pipeline);
		if self.revision != revision {
			*self = Self {
				revision,
				..Self::default()
			};
		}
		if self.ready.is_some() || self.failed {
			return self.ready;
		}
		match pipeline_manager.get(pipeline) {
			crate::rendering::PipelineState::Pending => return None,
			crate::rendering::PipelineState::Failed => {
				log::error!(
					"The pipeline of pass `{label}` is unavailable. The most likely cause is that its pipeline asset or shader dependency failed to bake or compile."
				);
				self.failed = true;
				return None;
			}
			crate::rendering::PipelineState::Ready(_) => {}
		}
		match adopt() {
			Ok(ready) => self.ready = Some(ready),
			Err(error) => {
				log::error!("Pass `{label}` could not adopt its pipeline: {error}");
				self.failed = true;
			}
		}
		self.ready
	}
}

/// The `RenderFeature` trait holds what every sink's [`RenderPass`] of one registration shares, such as settings that
/// arrive on a configuration port, a listener for submitted UI, or state the application drives, like the editor
/// gizmo's.
///
/// Register a feature with [`crate::rendering::Renderer::add_render_pass`], or with
/// [`crate::rendering::Renderer::add_pixel_step`] when its pass is a [`pixel_steps::PixelStep`]. The renderer owns the
/// feature, builds one pass per sink the registration reaches with [`Self::create_pass`], and lends the feature to that
/// pass on every call, so state that exists once lives in one place. Reach it from application code with
/// [`crate::rendering::Renderer::render_feature_mut`]. A pass with nothing to share uses a feature with no fields.
/// See [render features and passes](/docs/develop/rendering/render-features).
pub trait RenderFeature: Sized + 'static {
	/// The pass every sink the registration reaches gets, a [`RenderPass`] or a [`pixel_steps::PixelStep`].
	type Pass;

	/// Returns the stable name that `render.pass.<name>` enables or bypasses and the inspector reports under.
	fn name(&self) -> &'static str;

	/// Builds the pass of the sink whose graph `builder` builds, when the sink joins the render graph.
	fn create_pass(&mut self, builder: &mut RenderPassBuilder) -> Self::Pass;

	/// Brings the shared state up to date for the frame, before any sink's pass prepares, such as by draining a
	/// configuration port, so every sink records with the same settings.
	fn update(&mut self) {}

	/// Reports whether the shared state changed in a way the last presented frame does not show; see
	/// [`RenderPass::needs_frame`]. Adopt pending inputs here: the renderer asks once per registration and tick, before
	/// it asks the passes.
	fn needs_frame(&mut self) -> bool {
		false
	}
}

/// The `RenderPass` trait is one sink's step of a post-scene effect, which owns that sink's targets, descriptor sets,
/// and history.
///
/// Build persistent images and shader state through [`RenderPassBuilder`] in the constructor
/// [`RenderFeature::create_pass`] calls, then return frame-local recording work from [`Self::prepare`]. `F` is the
/// [`RenderFeature`] that builds the pass, which every call borrows.
pub trait RenderPass<F> {
	/// Prepares the pass's commands for one frame of its sink while the pass is enabled.
	fn prepare<'a>(
		&mut self,
		feature: &mut F,
		frame: &mut ghi::implementation::Frame,
		sink: &Sink,
		frame_allocator: &'a bumpalo::Bump,
	) -> Option<RenderPassReturn<'a>>;

	/// Prepares the maintenance work this pass still needs while it is bypassed.
	///
	/// The renderer keeps `main` flowing on its own: a bypassed pass that called
	/// [`RenderPassBuilder::create_main_render_target`] gets the incoming `main` copied into its replacement after this
	/// command. Override this only for work that must not stop, such as draining or adopting pending messages, or to
	/// forward `main` yourself after taking that copy from the builder, as passes that also forward on frames with
	/// nothing to draw do.
	fn bypass<'a>(
		&mut self,
		_feature: &mut F,
		_frame: &mut ghi::implementation::Frame,
		_sink: &Sink,
		_frame_allocator: &'a bumpalo::Bump,
	) -> Option<RenderPassReturn<'a>> {
		None
	}

	/// Reports whether this pass has output that the last presented frame does not show yet.
	///
	/// On-demand rendering only draws a frame when something asks for one. Return `true` while the pass's inputs
	/// changed since it last recorded, or while it animates. Passes that only transform their input keep the default.
	fn needs_frame(&mut self, _feature: &mut F) -> bool {
		false
	}

	/// Reports whether this pass needs its sink's view moved to a new point of the camera's aperture every frame; see
	/// [`Sink::with_lens_sample`].
	///
	/// While an enabled pass of a sink answers `true`, every pass and scene pipeline of that sink sees the moved view,
	/// and [`Sink::camera_view`] keeps the camera's own. Passes that average frames into a lens's blur answer `true`;
	/// every other pass keeps the default.
	fn samples_lens(&self, _feature: &F) -> bool {
		false
	}

	/// Reports whether this pass needs its sink's view moved by a different sub-pixel offset every frame; see
	/// [`Sink::with_jitter`].
	///
	/// While an enabled pass of a sink answers `true`, scene pipelines rasterize that sink through the moved view, and
	/// [`Sink::view`] keeps the view without the offset. Passes that blend frames into a resolve finer than a pixel,
	/// such as temporal antialiasing, answer `true`; every other pass keeps the default.
	fn jitters_view(&self, _feature: &F) -> bool {
		false
	}

	/// Returns the fraction of its sink's output extent this pass wants the scene rendered at, on each axis; see the
	/// [render scale](/docs/reference/rendering#render-scale) specification.
	///
	/// While an enabled pass of a sink answers less than `1`, scene pipelines and the passes before this one render at
	/// [`Sink::render_extent`], and this pass and the ones after it at [`Sink::output_extent`]. Only a pass that
	/// writes `main` through [`RenderPassBuilder::create_output_main_render_target`] may answer below `1`, since it
	/// is the one that upscales. Every other pass keeps the default.
	fn render_scale(&self, _feature: &F) -> f32 {
		1.0
	}
}

/// The `RenderPassState` enum identifies which preparation path a registration's passes use: [`RenderPass::prepare`]
/// or [`RenderPass::bypass`].
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum RenderPassState {
	#[default]
	Enabled,
	Bypassed,
}

impl RenderPassState {
	/// Returns the startup-parameter value representing this render-pass state.
	pub(crate) fn as_parameter_value(self) -> &'static str {
		match self {
			Self::Enabled => "enabled",
			Self::Bypassed => "bypassed",
		}
	}
}

/// The `SinkScope` enum says which sinks a post-scene registration builds a pass for, so an overlay that belongs to a
/// window, such as a game's HUD, stays out of the pictures render images draw.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SinkScope {
	/// Every window with a camera and every render image.
	All,
	/// Every window with a camera, and no render image.
	Windows,
	/// The one window created with this handle, which must have no camera, such as a tool's own window that shows only
	/// its interface.
	Window(crate::core::factory::Handle),
}

/// The `RenderFeatureId` struct names one registration of the feature `F`, so application code can reach that
/// feature's state with [`crate::rendering::Renderer::render_feature_mut`].
pub struct RenderFeatureId<F> {
	pub(crate) index: usize,
	feature: std::marker::PhantomData<fn() -> F>,
}

impl<F> RenderFeatureId<F> {
	/// Names the registration at `index` of the renderer's registrations.
	pub(crate) fn new(index: usize) -> Self {
		Self {
			index,
			feature: std::marker::PhantomData,
		}
	}
}

impl<F> Clone for RenderFeatureId<F> {
	fn clone(&self) -> Self {
		*self
	}
}

impl<F> Copy for RenderFeatureId<F> {}

impl<F> std::fmt::Debug for RenderFeatureId<F> {
	fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		formatter.debug_tuple("RenderFeatureId").field(&self.index).finish()
	}
}

/// The `InstalledPasses` trait lets the renderer keep features of different types in one list, each with the passes it
/// built, and reach each sink's pass by the sink's index.
pub(crate) trait InstalledPasses {
	/// Builds the pass of `sink` through `builder`.
	fn create_pass(&mut self, sink: usize, builder: &mut RenderPassBuilder);
	/// Drops the pass of `sink`, which no frame renders again.
	fn remove_sink(&mut self, sink: usize);
	/// Brings the feature up to date for the frame; see [`RenderFeature::update`].
	fn update(&mut self);
	/// Asks the feature and then every pass whether they need a frame, so each adopts its inputs.
	fn needs_frame(&mut self) -> bool;
	/// Prepares the pass of `sink` on the path `state` selects.
	fn prepare<'a>(
		&mut self,
		state: RenderPassState,
		frame: &mut ghi::implementation::Frame,
		sink: &Sink,
		frame_allocator: &'a bumpalo::Bump,
	) -> Option<RenderPassReturn<'a>>;
	/// Reports whether the pass of `sink` asks for lens samples; see [`RenderPass::samples_lens`].
	fn samples_lens(&self, sink: usize) -> bool;
	/// Reports whether the pass of `sink` asks for jitter; see [`RenderPass::jitters_view`].
	fn jitters_view(&self, sink: usize) -> bool;
	/// Returns the render scale the pass of `sink` asks for; see [`RenderPass::render_scale`].
	fn render_scale(&self, sink: usize) -> f32;
	/// Returns the feature for [`crate::rendering::Renderer::render_feature_mut`] to downcast.
	fn feature_mut(&mut self) -> &mut dyn std::any::Any;
}

/// The `Installed` struct is one registration as the renderer keeps it: the feature and each sink's pass, by sink.
pub(crate) struct Installed<F: RenderFeature> {
	pub(crate) feature: F,
	pub(crate) passes: SmallVec<[(usize, F::Pass); 4]>,
}

impl<F: RenderFeature> Installed<F> {
	/// Wraps `feature`, which has built no pass yet.
	pub(crate) fn new(feature: F) -> Self {
		Self {
			feature,
			passes: SmallVec::new(),
		}
	}

	/// Returns the pass of `sink`, or `None` when the registration does not reach the sink or the sink was removed.
	pub(crate) fn pass(&self, sink: usize) -> Option<&F::Pass> {
		self.passes.iter().find_map(|(of, pass)| (*of == sink).then_some(pass))
	}

	/// Returns the feature and the pass of `sink` together, so the pass can borrow the feature.
	pub(crate) fn pass_mut(&mut self, sink: usize) -> Option<(&mut F, &mut F::Pass)> {
		let pass = self.passes.iter_mut().find_map(|(of, pass)| (*of == sink).then_some(pass))?;
		Some((&mut self.feature, pass))
	}
}

impl<F: RenderFeature> InstalledPasses for Installed<F>
where
	F::Pass: RenderPass<F>,
{
	fn create_pass(&mut self, sink: usize, builder: &mut RenderPassBuilder) {
		let pass = self.feature.create_pass(builder);
		self.passes.push((sink, pass));
	}

	fn remove_sink(&mut self, sink: usize) {
		self.passes.retain(|(of, _)| *of != sink);
	}

	fn update(&mut self) {
		self.feature.update();
	}

	fn needs_frame(&mut self) -> bool {
		// Every pass answers, even after one already did, so each adopts its inputs this tick.
		let feature_needs = self.feature.needs_frame();
		self.passes
			.iter_mut()
			.fold(feature_needs, |needs, (_, pass)| pass.needs_frame(&mut self.feature) | needs)
	}

	fn prepare<'a>(
		&mut self,
		state: RenderPassState,
		frame: &mut ghi::implementation::Frame,
		sink: &Sink,
		frame_allocator: &'a bumpalo::Bump,
	) -> Option<RenderPassReturn<'a>> {
		let (feature, pass) = self.pass_mut(sink.index())?;
		match state {
			RenderPassState::Enabled => pass.prepare(feature, frame, sink, frame_allocator),
			RenderPassState::Bypassed => pass.bypass(feature, frame, sink, frame_allocator),
		}
	}

	fn samples_lens(&self, sink: usize) -> bool {
		self.pass(sink).is_some_and(|pass| pass.samples_lens(&self.feature))
	}

	fn jitters_view(&self, sink: usize) -> bool {
		self.pass(sink).is_some_and(|pass| pass.jitters_view(&self.feature))
	}

	fn render_scale(&self, sink: usize) -> f32 {
		self.pass(sink).map_or(1.0, |pass| pass.render_scale(&self.feature))
	}

	fn feature_mut(&mut self) -> &mut dyn std::any::Any {
		&mut self.feature
	}
}

/// The `SceneBackground` trait is the feature behind a scene's background, such as the atmosphere sky, which a scene
/// pipeline records after opaque surfaces and before transparent ones, and which may also generate the environment the
/// scene lights with.
///
/// Register it with [`crate::rendering::Renderer::set_scene_background`]. The renderer builds one [`Self::Pass`] per
/// sink and scene pipeline that calls [`RenderPassBuilder::create_scene_background`], and one [`Self::Environment`] per
/// scene with [`Self::create_environment`]. Both borrow the one feature, so settings arrive once and reach both, and
/// one `render.pass.<name>` bypasses both. See [render features and passes](/docs/develop/rendering/render-features).
pub trait SceneBackground: Sized + 'static {
	/// The pass that fills one sink's scene color where no surface was drawn.
	type Pass: RenderPass<Self>;
	/// The pass that fills one scene's [`EnvironmentMaps`]; [`NoEnvironment`] for a background that lights nothing.
	type Environment: RenderPass<Self>;

	/// Returns the stable name that `render.pass.<name>` enables or bypasses and the inspector reports under.
	fn name(&self) -> &'static str;

	/// Brings the shared state up to date for the frame; see [`RenderFeature::update`].
	fn update(&mut self) {}

	/// Reports whether the shared state changed in a way the last presented frame does not show; see
	/// [`RenderFeature::needs_frame`].
	fn needs_frame(&mut self) -> bool {
		false
	}

	/// Builds one sink's background over the scene pipeline's `targets`, with the scene pipeline's `builder`.
	fn create_pass(&mut self, builder: &mut RenderPassBuilder, targets: SceneBackgroundTargets) -> Self::Pass;

	/// Builds one scene's generated environment and the maps it fills, or `None` when the background lights nothing.
	///
	/// The renderer prepares it once per frame with the scene's first sink, and hands its maps to the scene pipelines
	/// once a frame has filled them. The maps hold unexposed radiance in the baked maps' layout: a cube of irradiance
	/// over π, and a cube of radiance prefiltered for roughness `level / 7` over eight levels.
	fn create_environment(&mut self, _builder: &mut RenderPassBuilder) -> Option<(Self::Environment, EnvironmentMaps)> {
		None
	}
}

/// The `NoEnvironment` enum is the [`SceneBackground::Environment`] of a background that generates no environment; it
/// has no values.
pub enum NoEnvironment {}

impl<F> RenderPass<F> for NoEnvironment {
	fn prepare<'a>(
		&mut self,
		_feature: &mut F,
		_frame: &mut ghi::implementation::Frame,
		_sink: &Sink,
		_frame_allocator: &'a bumpalo::Bump,
	) -> Option<RenderPassReturn<'a>> {
		match *self {}
	}
}

/// The `SceneBackgroundTargets` struct hands a scene background the scene pipeline's own targets.
///
/// They belong to the scene pipeline's render node, so the background uses them directly instead of declaring them
/// through [`RenderPassBuilder::read_from`] or [`RenderPassBuilder::render_to`].
#[derive(Clone, Copy)]
pub struct SceneBackgroundTargets {
	/// The scene color, written in place wherever `depth` is at infinity.
	pub color: ghi::BaseImageHandle,
	/// The scene's reverse-Z depth, where `0` means no surface.
	pub depth: ghi::BaseImageHandle,
}

/// The `EnvironmentMaps` struct names the cubes a generated environment fills, which scene pipelines bind as they bind
/// a baked environment's while the scene selects no [`crate::rendering::Environment`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct EnvironmentMaps {
	/// The cube of irradiance over π.
	pub diffuse: ghi::BaseImageHandle,
	/// The cube of radiance prefiltered for roughness `level / 7` over eight levels.
	pub specular: ghi::BaseImageHandle,
}

/// The `SceneBackgroundFrame` struct lends one scene pipeline the scene background for one frame, so the pipeline
/// prepares each sink's background and its scene's environment at the point of its own preparation it chooses.
///
/// [`crate::rendering::PipelineManager::prepare`] receives it. Prepare a sink's background after everything that may
/// make the pipeline give up on the sink's frame, since the background may record one-time work, such as building its
/// lookup tables, that a dropped command would lose; record it after opaque surfaces and before transparent ones.
/// Prepare the environment once per frame, the same way, and record it before any work that reads it, such as global
/// illumination's rays.
pub struct SceneBackgroundFrame<'f> {
	background: Option<&'f mut dyn InstalledBackground>,
	state: RenderPassState,
	/// The scene pipeline the backgrounds belong to.
	manager: usize,
	scene: crate::rendering::SceneId,
}

impl<'f> SceneBackgroundFrame<'f> {
	/// Lends `background`, in `state`, to the scene pipeline `manager` of `scene`.
	pub(crate) fn new(
		background: Option<&'f mut dyn InstalledBackground>,
		state: RenderPassState,
		manager: usize,
		scene: crate::rendering::SceneId,
	) -> Self {
		Self {
			background,
			state,
			manager,
			scene,
		}
	}

	/// Returns a frame with no scene background, for a scene pipeline driven without a renderer, such as in tests.
	pub fn none() -> Self {
		Self::new(None, RenderPassState::Enabled, 0, crate::rendering::SceneId::WORLD)
	}

	/// Prepares the background of `sink` for this frame, or returns `None` when the scene pipeline asked for none for
	/// the sink, or the background records nothing.
	pub fn prepare_background<'a>(
		&mut self,
		frame: &mut ghi::implementation::Frame,
		sink: &Sink,
		frame_allocator: &'a bumpalo::Bump,
	) -> Option<RenderPassReturn<'a>> {
		self.background
			.as_mut()?
			.prepare_background(self.state, self.manager, frame, sink, frame_allocator)
	}

	/// Prepares the command that brings the scene's environment maps up to date this frame, with the scene's first sink
	/// `sink`, or returns `None` when they already are or the background generates none.
	pub fn prepare_environment<'a>(
		&mut self,
		frame: &mut ghi::implementation::Frame,
		sink: &Sink,
		frame_allocator: &'a bumpalo::Bump,
	) -> Option<RenderPassReturn<'a>> {
		self.background
			.as_mut()?
			.prepare_environment(self.state, self.scene, frame, sink, frame_allocator)
	}

	/// Returns the scene's generated environment maps once a frame has recorded their first fill, so a reader binds
	/// them from the next frame on, or `None` before, and when the background generates none.
	pub fn environment_maps(&self) -> Option<EnvironmentMaps> {
		self.background.as_ref()?.environment_maps(self.scene)
	}
}

/// The `InstalledBackground` trait lets the renderer keep the scene background of any type, with the passes and
/// environments it built.
pub(crate) trait InstalledBackground {
	/// Builds the background of `sink` for the scene pipeline `manager`.
	fn create_pass(&mut self, manager: usize, sink: usize, builder: &mut RenderPassBuilder, targets: SceneBackgroundTargets);
	/// Builds the environment of `scene`, unless it has one, and reports whether it built one.
	fn create_environment(&mut self, scene: crate::rendering::SceneId, builder: &mut RenderPassBuilder) -> bool;
	/// Drops every background of `sink`, which no frame renders again.
	fn remove_sink(&mut self, sink: usize);
	/// Brings the feature up to date for the frame; see [`SceneBackground::update`].
	fn update(&mut self);
	/// Asks the feature and then every pass whether they need a frame, so each adopts its inputs.
	fn needs_frame(&mut self) -> bool;
	/// Prepares the background of `sink` for the scene pipeline `manager` on the path `state` selects.
	fn prepare_background<'a>(
		&mut self,
		state: RenderPassState,
		manager: usize,
		frame: &mut ghi::implementation::Frame,
		sink: &Sink,
		frame_allocator: &'a bumpalo::Bump,
	) -> Option<RenderPassReturn<'a>>;
	/// Prepares the environment of `scene` with its first sink `sink` on the path `state` selects.
	fn prepare_environment<'a>(
		&mut self,
		state: RenderPassState,
		scene: crate::rendering::SceneId,
		frame: &mut ghi::implementation::Frame,
		sink: &Sink,
		frame_allocator: &'a bumpalo::Bump,
	) -> Option<RenderPassReturn<'a>>;
	/// Returns the maps of the environment of `scene` once a frame recorded their first fill.
	fn environment_maps(&self, scene: crate::rendering::SceneId) -> Option<EnvironmentMaps>;
	/// Returns the feature for [`crate::rendering::Renderer::render_feature_mut`] to downcast.
	fn feature_mut(&mut self) -> &mut dyn std::any::Any;
}

/// The `SceneEnvironment` struct is one scene's generated environment: its pass, the maps it fills, and whether a
/// frame filled them.
struct SceneEnvironment<P> {
	scene: crate::rendering::SceneId,
	pass: P,
	maps: EnvironmentMaps,
	/// Whether the pass has recorded its first fill, so the maps hold light rather than whatever memory they started
	/// with.
	filled: bool,
}

/// The `InstalledSceneBackground` struct is the scene background as the renderer keeps it: the feature, each scene
/// pipeline's background on each sink, and each scene's environment.
pub(crate) struct InstalledSceneBackground<B: SceneBackground> {
	feature: B,
	passes: SmallVec<[(usize, usize, B::Pass); 4]>,
	environments: SmallVec<[SceneEnvironment<B::Environment>; 2]>,
}

impl<B: SceneBackground> InstalledSceneBackground<B> {
	/// Wraps `feature`, which has built nothing yet.
	pub(crate) fn new(feature: B) -> Self {
		Self {
			feature,
			passes: SmallVec::new(),
			environments: SmallVec::new(),
		}
	}
}

impl<B: SceneBackground> InstalledBackground for InstalledSceneBackground<B> {
	fn create_pass(&mut self, manager: usize, sink: usize, builder: &mut RenderPassBuilder, targets: SceneBackgroundTargets) {
		let pass = self.feature.create_pass(builder, targets);
		self.passes.push((manager, sink, pass));
	}

	fn create_environment(&mut self, scene: crate::rendering::SceneId, builder: &mut RenderPassBuilder) -> bool {
		if self.environments.iter().any(|environment| environment.scene == scene) {
			return false;
		}
		let Some((pass, maps)) = self.feature.create_environment(builder) else {
			return false;
		};
		self.environments.push(SceneEnvironment {
			scene,
			pass,
			maps,
			filled: false,
		});
		true
	}

	fn remove_sink(&mut self, sink: usize) {
		self.passes.retain(|(_, of, _)| *of != sink);
	}

	fn update(&mut self) {
		self.feature.update();
	}

	fn needs_frame(&mut self) -> bool {
		// Every pass answers, even after one already did, so each adopts its inputs this tick.
		let mut needs = self.feature.needs_frame();
		for (_, _, pass) in &mut self.passes {
			needs |= pass.needs_frame(&mut self.feature);
		}
		for environment in &mut self.environments {
			needs |= environment.pass.needs_frame(&mut self.feature);
		}
		needs
	}

	fn prepare_background<'a>(
		&mut self,
		state: RenderPassState,
		manager: usize,
		frame: &mut ghi::implementation::Frame,
		sink: &Sink,
		frame_allocator: &'a bumpalo::Bump,
	) -> Option<RenderPassReturn<'a>> {
		let pass = self
			.passes
			.iter_mut()
			.find_map(|(of_manager, of_sink, pass)| (*of_manager == manager && *of_sink == sink.index()).then_some(pass))?;
		match state {
			RenderPassState::Enabled => pass.prepare(&mut self.feature, frame, sink, frame_allocator),
			RenderPassState::Bypassed => pass.bypass(&mut self.feature, frame, sink, frame_allocator),
		}
	}

	fn prepare_environment<'a>(
		&mut self,
		state: RenderPassState,
		scene: crate::rendering::SceneId,
		frame: &mut ghi::implementation::Frame,
		sink: &Sink,
		frame_allocator: &'a bumpalo::Bump,
	) -> Option<RenderPassReturn<'a>> {
		let environment = self.environments.iter_mut().find(|environment| environment.scene == scene)?;
		let command = match state {
			RenderPassState::Enabled => environment.pass.prepare(&mut self.feature, frame, sink, frame_allocator),
			RenderPassState::Bypassed => environment.pass.bypass(&mut self.feature, frame, sink, frame_allocator),
		};
		environment.filled |= command.is_some();
		command
	}

	fn environment_maps(&self, scene: crate::rendering::SceneId) -> Option<EnvironmentMaps> {
		// Readers bind the maps once a frame filled them, so nothing reads them before they hold light.
		self.environments
			.iter()
			.find(|environment| environment.scene == scene && environment.filled)
			.map(|environment| environment.maps)
	}

	fn feature_mut(&mut self) -> &mut dyn std::any::Any {
		&mut self.feature
	}
}

/// The [`RenderPassBuilder`] struct provides sink resources and records the
/// dependencies of a render pass.
///
/// Declare auxiliary outputs with [`Self::create_render_target`] or
/// [`Self::render_to`] and inputs with [`Self::read_from`]. Use
/// [`Self::create_main_render_target`] for a replacement color result so the
/// renderer can select the final destination. Then construct the [`RenderPass`]
/// that records commands for those resources.
pub struct RenderPassBuilder<'a> {
	context: &'a mut ghi::implementation::Context,
	/// The queue the renderer records frames on, where a pass uploads its lookup textures.
	queue: ghi::QueueHandle,
	sink_id: usize,
	/// Where the sink's graph ends: the window's swapchain or the render image, which the terminal pass writes.
	final_target: MainRenderTarget,
	/// The step of the sink's frame this builder's pass records as.
	node: crate::rendering::renderer::RenderNode,
	final_output: bool,
	final_output_written: bool,
	/// Every render-target image this pass reads or writes, by index, so the renderer knows when each one is used.
	accesses: Vec<(usize, ghi::AccessPolicies)>,
	external_writable_targets: Vec<(String, ghi::ImageOrSwapchain)>,
	images: &'a mut RenderTargets,
	pipeline_manager: crate::rendering::PipelineManagerClient,
	/// The targets a scene pipeline asked the scene background to fill; see [`Self::create_scene_background`].
	scene_background: Option<SceneBackgroundTargets>,
	/// The copy that forwards the incoming `main` into this pass's replacement while the pass is bypassed.
	main_copy: Option<ImageBypassPass>,
	/// Every stage counter this node created, with the name the renderer reports it under.
	gpu_counters: Vec<(String, ghi::CounterHandle)>,
	/// Whether the build has debug views and how large each sink's pool is.
	debug_view_settings: DebugViewSettings,
	/// The views this node declared and the cell it reads its shown views from, for the renderer to register.
	debug_views: Option<(&'static [&'static str], Arc<AtomicU32>)>,
}

impl<'a> RenderPassBuilder<'a> {
	/// Creates the builder for one node of a sink's graph.
	///
	/// `node` is the step of the sink's frame the pass records as. `final_output` marks the terminal post-scene pass,
	/// whose replacement `main` is `final_target`, the sink's swapchain or render image.
	#[allow(clippy::too_many_arguments)]
	pub(crate) fn new(
		context: &'a mut ghi::implementation::Context,
		queue: ghi::QueueHandle,
		images: &'a mut RenderTargets,
		sink_id: usize,
		final_target: MainRenderTarget,
		pipeline_manager: crate::rendering::PipelineManagerClient,
		node: crate::rendering::renderer::RenderNode,
		final_output: bool,
		debug_view_settings: DebugViewSettings,
	) -> Self {
		RenderPassBuilder {
			context,
			queue,
			sink_id,
			final_target,
			node,
			final_output,
			final_output_written: false,
			accesses: Vec::new(),
			external_writable_targets: Vec::new(),
			images,
			pipeline_manager,
			scene_background: None,
			main_copy: None,
			gpu_counters: Vec::new(),
			debug_view_settings,
			debug_views: None,
		}
	}

	/// Declares the debug views this pass can show, at most [`MAX_VIEWS_PER_PASS`], in the order the shader numbers
	/// them from `0`, and returns the pool they write into.
	///
	/// Each view is selectable as `render.debug.view.<pass name>.<view name>`, where the pass name is
	/// [`RenderPass::name`] or the scene pipeline manager's name. The renderer records this pass as a writer of every
	/// pool slot. When the project leaves debug views out, the result reports no view shown and holds no targets.
	/// Call it once per pass.
	///
	/// # Panics
	///
	/// Panics on a second call, on more than [`MAX_VIEWS_PER_PASS`] views, or on a view name with a dot.
	pub fn debug_views(&mut self, views: &'static [&'static str]) -> DebugViews {
		assert!(
			self.debug_views.is_none(),
			"Debug views were declared twice. The most likely cause is that a pass called `RenderPassBuilder::debug_views` more than once."
		);
		assert!(
			views.len() <= MAX_VIEWS_PER_PASS,
			"Too many debug views ({}). The most likely cause is a pass that declares more than {MAX_VIEWS_PER_PASS} views, which is all a `u32` packs.",
			views.len()
		);
		assert!(
			views.iter().all(|view| !view.is_empty() && !view.contains('.')),
			"Debug view name is invalid ({views:?}). The most likely cause is an empty name or one with a dot, which separates the pass name in `render.debug.view.<pass>.<view>`."
		);
		let shown = Arc::new(AtomicU32::new(0));
		self.debug_views = Some((views, shown.clone()));
		if !self.debug_view_settings.enabled {
			return DebugViews::disabled();
		}
		DebugViews::new(&self.debug_view_pool(ghi::AccessPolicies::WRITE), shown)
	}

	/// Returns the sink's debug target pool in slot order, creating it on first use, and records this node's
	/// `access` to every slot. The renderer's pass that tiles the shown views reads it.
	pub(crate) fn debug_view_pool(&mut self, access: ghi::AccessPolicies) -> SmallVec<[ghi::BaseImageHandle; 15]> {
		let context = &mut *self.context;
		let pool = self
			.images
			.debug_pool(self.sink_id, self.debug_view_settings.pool_size, |builder| {
				context.build_image(builder).into()
			});
		pool.into_iter()
			.map(|(index, image)| {
				self.accesses.push((index, access));
				image
			})
			.collect()
	}

	/// Hands the renderer the views this builder's pass declared, so it can register them under the pass's name.
	pub(crate) fn take_debug_views(&mut self) -> Option<(&'static [&'static str], Arc<AtomicU32>)> {
		self.debug_views.take()
	}

	/// Creates a GPU timing counter for one stage of this node's commands, which the inspector reports as
	/// `stage.<name>` next to the node's own `scene.<manager>` or `pass.<name>` time.
	///
	/// Wrap the stage's commands with [`ghi::command_buffer::CommonCommandBufferMode::counter`] inside the node's
	/// recording, once per frame, and only on frames the stage records: a counter around no commands reports a
	/// meaningless near-zero time. Stage counters nest inside the node's counter, so a node's stages add up to its
	/// time plus whatever it records outside them.
	pub fn create_gpu_counter(&mut self, name: &str) -> ghi::CounterHandle {
		let counter = self.context.create_counter(Some(name));
		self.gpu_counters.push((name.to_string(), counter));
		counter
	}

	/// Hands the renderer every stage counter created through this builder so it can publish their times.
	pub(crate) fn take_gpu_counters(&mut self) -> Vec<(String, ghi::CounterHandle)> {
		std::mem::take(&mut self.gpu_counters)
	}

	/// Asks for this sink's scene background over `targets`, the scene pipeline's color and depth.
	///
	/// Scene pipelines call this once per sink. When the application set a scene background, the renderer builds its
	/// pass for this sink and scene pipeline, and hands its command to [`crate::rendering::PipelineManager::prepare`]
	/// through [`SceneBackgroundFrame::background`]. Record it after opaque surfaces and before transparent ones.
	pub fn create_scene_background(&mut self, targets: SceneBackgroundTargets) {
		self.scene_background = Some(targets);
	}

	/// Hands the renderer the targets a scene pipeline asked the scene background to fill.
	pub(crate) fn take_scene_background(&mut self) -> Option<SceneBackgroundTargets> {
		self.scene_background.take()
	}

	/// Hands over the copy that forwards `main` while this builder's pass is bypassed.
	///
	/// The renderer takes it after the pass is built. A pass that also forwards `main` on frames with nothing to
	/// draw takes it first, so it keeps one copy, and then forwards `main` from [`RenderPass::bypass`] too. It is `None`
	/// when the pass started the `main` chain on a sink without a scene, since there is nothing to forward.
	pub(crate) fn take_main_copy(&mut self) -> Option<ImageBypassPass> {
		self.main_copy.take()
	}

	pub fn alias(&mut self, orig: &'a str, alias: &'a str) {
		self.images.alias(self.sink_id, orig, alias);
	}

	pub fn format_of(&self, name: &str) -> ghi::Formats {
		self.images.image(self.target_index(name)).1
	}

	/// Returns the index of the sink's render target named `name`.
	fn target_index(&self, name: &str) -> usize {
		self.images.get_image_index(name, self.sink_id).unwrap_or_else(|| {
			panic!(
				"Render target image '{name}' does not exist for sink {}. The most likely cause is that a render pass was added before the pipeline that creates this target.",
				self.sink_id
			)
		})
	}

	/// Returns an existing image for writing by this render pass.
	pub fn render_to(&mut self, name: &'a str) -> RenderToResult {
		let image_index = self.target_index(name);
		self.accesses.push((image_index, ghi::AccessPolicies::WRITE));
		let (image, format) = self.images.image(image_index);

		RenderToResult { image, format }
	}

	/// Creates a transferable render-target image and returns it for writing by this render pass.
	pub fn create_render_target(&mut self, builder: ghi::image::Builder<'a>) -> RenderToResult {
		self.create_scaled_render_target(builder, 1)
	}

	/// Creates a transferable render-target image at a fraction of the sink resolution.
	///
	/// The renderer sizes the image to the sink extent divided by `resolution_divisor`, so `2` gives a
	/// half-resolution target. Like every render target, it can be captured by name for debugging.
	///
	/// The target holds valid contents only between the first and the last pass of a frame that uses it, since
	/// targets whose uses do not overlap share memory. Its contents are undefined when its first pass starts, so that
	/// pass must write every pixel it later reads. Use [`Self::create_persistent_render_target`] for contents that
	/// must survive into the next frame.
	pub fn create_scaled_render_target(&mut self, builder: ghi::image::Builder<'a>, resolution_divisor: u32) -> RenderToResult {
		let group = self.images.group(self.sink_id, self.context);
		// The renderer clears a target whose first pass records nothing, so it never reads another target's memory.
		let (name, format) = target_identity(&builder);
		let image = self.context.build_image(
			builder
				.group(group)
				.additional_uses(ghi::Uses::Clear | ghi::Uses::TransferSource),
		);
		self.register_render_target(name, format, image.into(), resolution_divisor, Placement::Member)
	}

	/// Creates a full-resolution render target whose contents stay valid across frames.
	///
	/// Use it for images a pass updates incrementally, such as a layer that only redraws damaged regions. Unlike
	/// [`Self::create_render_target`], the target never shares memory with other targets.
	pub fn create_persistent_render_target(&mut self, builder: ghi::image::Builder<'_>) -> RenderToResult {
		let (name, format) = target_identity(&builder);
		let image = self.context.build_image(builder.additional_uses(ghi::Uses::TransferSource));
		self.register_render_target(name, format, image.into(), 1, Placement::Persistent)
	}

	/// Registers `image` as a render target named `name` that this pass writes.
	fn register_render_target(
		&mut self,
		name: &str,
		format: ghi::Formats,
		image: ghi::BaseImageHandle,
		resolution_divisor: u32,
		placement: Placement,
	) -> RenderToResult {
		let image_index = self
			.images
			.insert(name.to_string(), self.sink_id, image, format, resolution_divisor, placement);
		self.accesses.push((image_index, ghi::AccessPolicies::WRITE));

		RenderToResult { image, format }
	}

	/// Creates a render target that keeps one copy per frame in flight, so the next frame can read what this frame
	/// wrote.
	///
	/// The renderer sizes it to the sink extent divided by `resolution_divisor`. Later passes use it like a target from
	/// [`Self::create_render_target`], and the renderer clears this frame's copy when its first pass records nothing.
	/// Unlike such a target, it never shares memory with other targets. Read the previous frame's copy by binding the
	/// returned handle with [`ghi::DescriptorWrite::combined_image_sampler_with_frame`] and an offset of `-1`.
	///
	/// The previous copy holds no usable data on a sink's first frame or right after a resize, so the reading pass must
	/// track whether it recorded the sink at the same extent in the previous frame.
	pub fn create_history_target(&mut self, builder: ghi::image::Builder<'a>, resolution_divisor: u32) -> ghi::ImageHandle {
		let (name, format) = target_identity(&builder);
		let image = self.context.build_image(
			builder
				.additional_uses(ghi::Uses::Clear | ghi::Uses::TransferSource)
				.copies(ghi::Copies::PerFrame),
		);
		self.register_render_target(name, format, image.into(), resolution_divisor, Placement::PerFrame(image));
		image
	}

	/// Creates a replacement for the color image named `main`.
	///
	/// The renderer supplies the sink's final target, its window's swapchain or its render image, when this pass is
	/// terminal. In every other position, this creates and
	/// aliases the requested image, adding [`simple_tile::output_uses`] and [`ghi::Uses::Image`], which later passes
	/// need to sample `main`, to the uses `builder` asks for. Bind the
	/// returned target without inspecting its variant, and take its format from [`MainRenderTarget::format`], because
	/// the final target's format can differ from the intermediate format: request a tile pipeline for that format, write a
	/// compute output as a formatless storage image, and derive a raster attachment from the target. Next, bind it with
	/// [`simple_tile::Output::new`] or [`simple_compute::Resource::image`].
	///
	/// While the pass is bypassed, the renderer copies the incoming `main` into the replacement, so later passes read
	/// the image this pass would have transformed. A sink without a scene pipeline has no incoming `main`; there the
	/// pass starts the chain, nothing is copied, and a frame the pass records nothing for leaves the replacement
	/// cleared.
	pub fn create_main_render_target(&mut self, builder: ghi::image::Builder<'a>) -> MainRenderTarget {
		self.replace_main(builder)
	}

	/// Creates a replacement for `main` at the sink's output extent, for the pass that upscales the scene; see
	/// [`RenderPass::render_scale`].
	///
	/// It works like [`Self::create_main_render_target`], except that this target and every target later passes
	/// create follow [`Sink::output_extent`], while the targets created before it follow [`Sink::render_extent`].
	/// A sink has at most one such pass.
	///
	/// # Panics
	///
	/// Panics when an earlier pass of the sink already called it.
	pub fn create_output_main_render_target(&mut self, builder: ghi::image::Builder<'a>) -> MainRenderTarget {
		self.images.begin_output(self.sink_id);
		self.replace_main(builder)
	}

	/// Replaces `main` with the image `builder` describes, or with the sink's final target when this pass is
	/// terminal; see [`Self::create_main_render_target`].
	fn replace_main(&mut self, builder: ghi::image::Builder<'a>) -> MainRenderTarget {
		let name = builder.get_name().expect(
			"Main render target name is missing. The most likely cause is that the image builder was not given a name before replacing `main`.",
		);
		// Resolve the incoming `main` before the replacement takes its name.
		let source = self.try_read_from("main");
		let main = if self.final_output {
			self.final_output_written = true;
			let target = self.final_target;
			self.external_writable_targets.push((name.to_string(), target.target));
			self.external_writable_targets.push(("main".to_string(), target.target));
			target
		} else {
			// The copy that forwards `main` while the pass is bypassed writes the replacement as a tile output, and the
			// passes after this one sample it as `main`. A later pass may also draw onto it in place through
			// `render_to`, as the UI does, which needs render-target usage where a tile pass writes storage.
			let uses = simple_tile::output_uses(&*self.context) | ghi::Uses::Image | ghi::Uses::RenderTarget;
			let output = self.create_render_target(builder.additional_uses(uses));
			self.alias(name, "main");
			MainRenderTarget {
				target: output.image.into(),
				format: output.format,
			}
		};
		self.main_copy = source.map(|source| ImageBypassPass::forwarding(self, source, main));
		main
	}

	pub fn read_from(&mut self, name: &'a str) -> ReadFromResult {
		let index = self.target_index(name);
		self.accesses.push((index, ghi::AccessPolicies::READ));

		ReadFromResult {
			image: self.images.image(index).0,
		}
	}

	/// Returns the identity of the target named `name`, which a scene pipeline keeps to ask each frame's
	/// [`Sink::reads`] whether an enabled pass reads it, and skip writing it when none does.
	///
	/// Panics when no target is named `name` for this sink.
	pub fn target_id(&self, name: &str) -> crate::rendering::TargetId {
		self.images.id(self.target_index(name))
	}

	/// Returns the targets this builder's pass reads, which the renderer hands to scene pipelines while the pass is
	/// enabled; see [`Sink::with_read_targets`].
	pub(crate) fn read_targets(&self) -> crate::rendering::TargetSet {
		let mut targets = crate::rendering::TargetSet::default();
		for (index, _) in self
			.accesses
			.iter()
			.filter(|(_, access)| access.intersects(ghi::AccessPolicies::READ))
		{
			targets.insert(self.images.id(*index));
		}
		targets
	}

	/// Returns the image named `name` for reading by this render pass, or `None` when no earlier pass created it, such
	/// as a target only one scene pipeline writes.
	pub fn try_read_from(&mut self, name: &'a str) -> Option<ReadFromResult> {
		self.images.get_image_index(name, self.sink_id)?;
		Some(self.read_from(name))
	}

	/// Returns the queue the renderer records frames on, where a pass records one-time uploads, such as its lookup
	/// textures, while it is built.
	pub fn queue(&self) -> ghi::QueueHandle {
		self.queue
	}

	pub fn context(&mut self) -> &'_ mut ghi::implementation::Context {
		self.context
	}

	/// Returns a client for requesting pipelines shared by renderer dependants.
	pub fn pipeline_manager(&self) -> &crate::rendering::PipelineManagerClient {
		&self.pipeline_manager
	}

	/// Reports whether this builder's pass is the sink's terminal pass, which must replace `main` with
	/// [`Self::create_main_render_target`] to receive the sink's final target. A pass that can draw onto `main` in
	/// place with [`Self::render_to`] asks this first, since the terminal pass can't.
	pub(crate) fn is_terminal(&self) -> bool {
		self.final_output
	}

	/// Reports whether this builder gave its pass the sink's final target.
	pub(crate) fn writes_final_output(&self) -> bool {
		self.final_output_written
	}

	/// Records the images this builder's pass uses as one node of the sink's frame.
	pub(crate) fn record_node(&mut self) {
		self.images
			.record_node(self.sink_id, self.node, self.accesses.iter().map(|(index, _)| *index));
	}

	/// Snapshots every current name and alias that resolves to a target written by this pass.
	pub(crate) fn writable_targets(&self) -> Vec<(String, ghi::ImageOrSwapchain)> {
		let written = self
			.accesses
			.iter()
			.filter(|(_, access)| access.intersects(ghi::AccessPolicies::WRITE))
			.map(|(index, _)| *index)
			.collect::<SmallVec<[usize; 8]>>();
		self.images
			.names_for_images(self.sink_id, &written)
			.into_iter()
			.map(|(name, image)| (name, image.into()))
			.chain(self.external_writable_targets.iter().cloned())
			.collect()
	}
}

#[derive(Clone, Copy)]
pub struct ReadFromResult {
	image: ghi::BaseImageHandle,
}

impl From<ReadFromResult> for ghi::BaseImageHandle {
	fn from(value: ReadFromResult) -> Self {
		value.image
	}
}

impl From<ReadFromResult> for ghi::ImageOrSwapchain {
	fn from(value: ReadFromResult) -> Self {
		Self::Image(value.image)
	}
}

#[derive(Clone, Copy)]
pub struct RenderToResult {
	image: ghi::BaseImageHandle,
	format: ghi::Formats,
}

impl From<RenderToResult> for ghi::BaseImageHandle {
	fn from(value: RenderToResult) -> Self {
		value.image
	}
}

impl From<RenderToResult> for ghi::ImageOrSwapchain {
	fn from(value: RenderToResult) -> Self {
		Self::Image(value.image)
	}
}

impl From<RenderToResult> for ghi::pipelines::raster::AttachmentDescriptor {
	fn from(val: RenderToResult) -> Self {
		ghi::pipelines::raster::AttachmentDescriptor::new(val.format)
	}
}

/// The `MainRenderTarget` struct hides whether a replacement `main` output is an intermediate image or the sink's final
/// target, a window's swapchain or a render image.
///
/// Pass it directly to [`simple_compute::Resource::image`] or convert it into a
/// raster attachment descriptor.
#[derive(Clone, Copy)]
pub struct MainRenderTarget {
	target: ghi::ImageOrSwapchain,
	format: ghi::Formats,
}

impl MainRenderTarget {
	/// Names `target`, which holds images of `format`, as a sink's final target.
	pub(crate) fn new(target: ghi::ImageOrSwapchain, format: ghi::Formats) -> Self {
		Self { target, format }
	}

	/// Returns the format of the image or swapchain behind the target.
	pub fn format(&self) -> ghi::Formats {
		self.format
	}
}

impl From<MainRenderTarget> for ghi::ImageOrSwapchain {
	fn from(value: MainRenderTarget) -> Self {
		value.target
	}
}

impl From<MainRenderTarget> for ghi::pipelines::raster::AttachmentDescriptor {
	fn from(value: MainRenderTarget) -> Self {
		ghi::pipelines::raster::AttachmentDescriptor::new(value.format)
	}
}
