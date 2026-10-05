//! Composable sink-local rendering stages.
//!
//! Implement [`RenderPass`] for post-processing or overlays that run after scene
//! pipelines. Construct resources through [`RenderPassBuilder`] so the renderer
//! can track access policies and named render targets. Existing implementations
//! live in [`crate::rendering::render_passes`].
//! Replace the color flowing through the graph with
//! [`RenderPassBuilder::create_main_render_target`]. The builder supplies an
//! intermediate image or the swapchain without exposing graph position to the pass.

pub mod simple_compute;

use std::{
	cell::{Cell, RefCell},
	rc::Rc,
};

use ghi::context::ContextCreate as _;
use smallvec::SmallVec;
use utils::{Box, hash::HashMap};

use crate::rendering::{Sink, render_passes::blit::ImageBypassPass, renderer::RenderTargets};

pub trait RenderPassFunction = Fn(&mut ghi::implementation::CommandBufferRecording);

/// A frame-allocated command that records one render pass.
pub type RenderPassReturn<'a> = &'a (dyn RenderPassFunction + Send + Sync + 'a);

/// Allocates a prepared render command in the application frame allocator.
pub fn allocate_render_command<'a>(
	frame_allocator: &'a bumpalo::Bump,
	command: impl RenderPassFunction + Send + Sync + 'a,
) -> RenderPassReturn<'a> {
	frame_allocator.alloc(command)
}

/// The `RenderPass` trait defines a composable rendering step for a prepared sink.
///
/// Build persistent images and shader state through [`RenderPassBuilder`], then
/// return frame-local recording work from [`Self::prepare`]. Register the
/// implementation with [`crate::rendering::renderer::Renderer`].
pub trait RenderPass {
	/// Returns the stable name used to control every sink-local instance of this pass.
	fn name(&self) -> &'static str;

	/// Prepares the render pass when its rendering condition is active.
	fn prepare<'a>(
		&mut self,
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
	fn needs_frame(&mut self) -> bool {
		false
	}
}

/// The `RenderPassState` enum identifies which preparation path a render pass uses: [`RenderPass::prepare`] or
/// [`RenderPass::bypass`].
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

/// The state every pass instance with one [`RenderPass::name`] shares, keyed by that name.
///
/// Each harness holds a clone of its name's cell, so changing the cell changes every sink's instance at once.
pub(crate) type RenderPassStates = HashMap<String, Rc<Cell<RenderPassState>>>;

/// The `RenderPassHarness` struct owns one render pass and keeps its execution state outside the implementation.
///
/// The renderer creates one per sink-local pass. Its state is shared by every instance with the same name, so
/// [`crate::rendering::renderer::Renderer::set_render_pass_state`] controls them together. Call [`Self::prepare`]
/// once per eligible sink and frame so the harness can select the active or bypass preparation path.
pub(crate) struct RenderPassHarness {
	render_pass: Box<dyn RenderPass>,
	state: Rc<Cell<RenderPassState>>,
}

impl RenderPassHarness {
	/// Creates a harness that follows the state shared by every pass with the same name, adding it when it is new.
	pub(crate) fn new(render_pass: Box<dyn RenderPass>, states: &mut RenderPassStates) -> Self {
		let state = states.entry(render_pass.name().to_string()).or_default().clone();
		Self { render_pass, state }
	}

	/// Reports whether the wrapped pass asks for a new frame; see [`RenderPass::needs_frame`].
	pub(crate) fn needs_frame(&mut self) -> bool {
		self.render_pass.needs_frame()
	}

	/// Returns the pass state used for the next frame preparation.
	pub(crate) fn state(&self) -> RenderPassState {
		self.state.get()
	}

	/// Returns the stable name supplied by the render pass implementation.
	pub(crate) fn name(&self) -> &'static str {
		self.render_pass.name()
	}

	/// Prepares the active or bypass path selected by [`Self::state`].
	pub(crate) fn prepare<'a>(
		&mut self,
		frame: &mut ghi::implementation::Frame,
		sink: &Sink,
		frame_allocator: &'a bumpalo::Bump,
	) -> Option<RenderPassReturn<'a>> {
		match self.state.get() {
			RenderPassState::Enabled => self.render_pass.prepare(frame, sink, frame_allocator),
			RenderPassState::Bypassed => self.render_pass.bypass(frame, sink, frame_allocator),
		}
	}
}

/// Builds the pass that fills a scene's background for one sink; see [`RenderPassBuilder::create_scene_background`].
pub type SceneBackgroundFactory = dyn for<'builder, 'resources> Fn(
	&'builder mut RenderPassBuilder<'resources>,
	SceneBackgroundTargets,
) -> Box<dyn RenderPass>;

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

/// The `SceneBackground` struct shares one sink's background pass between the scene pipeline that records it and
/// the renderer that asks it for frames.
///
/// A scene pipeline records it after opaque surfaces and before transparent ones, so transparent surfaces composite
/// over the background inside the scene color and the scene color needs no coverage channel. Its
/// [`RenderPassState`] is shared by name like any other pass.
#[derive(Clone)]
pub struct SceneBackground(Rc<RefCell<RenderPassHarness>>);

impl SceneBackground {
	/// Prepares the active or bypass path selected by the background's state.
	pub fn prepare<'a>(
		&self,
		frame: &mut ghi::implementation::Frame,
		sink: &Sink,
		frame_allocator: &'a bumpalo::Bump,
	) -> Option<RenderPassReturn<'a>> {
		self.0.borrow_mut().prepare(frame, sink, frame_allocator)
	}

	/// Reports whether the background asks for a new frame; see [`RenderPass::needs_frame`].
	pub(crate) fn needs_frame(&self) -> bool {
		self.0.borrow_mut().needs_frame()
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
	sink_id: usize,
	swapchain: ghi::SwapchainHandle,
	/// The step of the sink's frame this builder's pass records as.
	node: crate::rendering::renderer::RenderNode,
	final_output: bool,
	final_output_written: bool,
	/// Every render-target image this pass reads or writes, by index, so the renderer knows when each one is used.
	accesses: Vec<(usize, ghi::AccessPolicies)>,
	external_writable_targets: Vec<(String, ghi::ImageOrSwapchain)>,
	images: &'a mut RenderTargets,
	render_pass_states: &'a mut RenderPassStates,
	pipeline_manager: crate::rendering::PipelineManagerClient,
	scene_background_factory: Option<&'a SceneBackgroundFactory>,
	created_scene_backgrounds: Vec<SceneBackground>,
	/// The copy that forwards the incoming `main` into this pass's replacement while the pass is bypassed.
	main_copy: Option<ImageBypassPass>,
	/// Every stage counter this node created, with the name the renderer reports it under.
	gpu_counters: Vec<(String, ghi::CounterHandle)>,
}

impl<'a> RenderPassBuilder<'a> {
	/// Creates the builder for one node of a sink's graph.
	///
	/// `node` is the step of the sink's frame the pass records as. `final_output` marks the terminal post-scene pass,
	/// whose replacement `main` is the sink swapchain. `scene_background_factory` is what
	/// [`Self::create_scene_background`] builds from; pass it only for scene nodes.
	#[allow(clippy::too_many_arguments)]
	pub(crate) fn new(
		context: &'a mut ghi::implementation::Context,
		images: &'a mut RenderTargets,
		render_pass_states: &'a mut RenderPassStates,
		sink_id: usize,
		swapchain: ghi::SwapchainHandle,
		pipeline_manager: crate::rendering::PipelineManagerClient,
		scene_background_factory: Option<&'a SceneBackgroundFactory>,
		node: crate::rendering::renderer::RenderNode,
		final_output: bool,
	) -> Self {
		RenderPassBuilder {
			context,
			sink_id,
			swapchain,
			node,
			final_output,
			final_output_written: false,
			accesses: Vec::new(),
			external_writable_targets: Vec::new(),
			images,
			render_pass_states,
			pipeline_manager,
			scene_background_factory,
			created_scene_backgrounds: Vec::new(),
			main_copy: None,
			gpu_counters: Vec::new(),
		}
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

	/// Creates this sink's scene background, or returns `None` when the application registered none.
	///
	/// Scene pipelines call this with their color and depth `targets`. Record the result after opaque surfaces and
	/// before transparent ones.
	pub fn create_scene_background(&mut self, targets: SceneBackgroundTargets) -> Option<SceneBackground> {
		let factory = self.scene_background_factory?;
		let render_pass = factory(self, targets);
		let background = SceneBackground(Rc::new(RefCell::new(RenderPassHarness::new(
			render_pass,
			self.render_pass_states,
		))));
		self.created_scene_backgrounds.push(background.clone());
		Some(background)
	}

	/// Hands the renderer every background created through this builder so it can ask them for frames.
	pub(crate) fn take_scene_backgrounds(&mut self) -> Vec<SceneBackground> {
		std::mem::take(&mut self.created_scene_backgrounds)
	}

	/// Hands over the copy that forwards `main` while this builder's pass is bypassed.
	///
	/// The renderer takes it after the pass is built. A pass that also forwards `main` on frames with nothing to
	/// draw takes it first, so it keeps one copy, and then forwards `main` from [`RenderPass::bypass`] too.
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
		self.insert_render_target(
			builder.group(group).additional_uses(ghi::Uses::Clear),
			resolution_divisor,
			true,
		)
	}

	/// Creates a full-resolution render target whose contents stay valid across frames.
	///
	/// Use it for images a pass updates incrementally, such as a layer that only redraws damaged regions. Unlike
	/// [`Self::create_render_target`], the target never shares memory with other targets.
	pub fn create_persistent_render_target(&mut self, builder: ghi::image::Builder<'a>) -> RenderToResult {
		self.insert_render_target(builder, 1, false)
	}

	/// Builds a transferable render target and registers it as written by this pass.
	fn insert_render_target(
		&mut self,
		builder: ghi::image::Builder<'a>,
		resolution_divisor: u32,
		member: bool,
	) -> RenderToResult {
		let name = builder.get_name().expect(
			"Render target name is missing. The most likely cause is that the image builder was not given a name before creating the target.",
		);
		let format = builder.get_format();

		let image = self.context.build_image(builder.additional_uses(ghi::Uses::TransferSource));

		let image_index = self.images.insert(
			name.to_string(),
			self.sink_id,
			image.into(),
			format,
			resolution_divisor,
			member,
		);
		self.accesses.push((image_index, ghi::AccessPolicies::WRITE));

		RenderToResult {
			image: image.into(),
			format,
		}
	}

	/// Creates a sink-sized image that keeps one copy per frame in flight, so later frames can read it as history.
	///
	/// The renderer sizes the image to the sink extent divided by `resolution_divisor`, but never binds it as an
	/// attachment, so no pass clears it.
	/// The creating pass writes this frame's copy. Any pass reads the previous frame's copy by binding the handle
	/// with [`ghi::DescriptorWrite::combined_image_sampler_with_frame`] and an offset of `-1`.
	///
	/// A previous-frame copy holds no usable data on a sink's first frame or right after a resize, so the reading pass
	/// must track whether it recorded the sink at the same extent in the previous frame.
	pub fn create_history_target(
		&mut self,
		builder: ghi::image::Builder<'a>,
		resolution_divisor: u32,
	) -> ghi::DynamicImageHandle {
		let name = builder.get_name().expect(
			"History target name is missing. The most likely cause is that the image builder was not given a name before creating the target.",
		);
		let image = self
			.context
			.build_dynamic_image(builder.additional_uses(ghi::Uses::TransferSource));
		self.images
			.insert_history(name.to_string(), self.sink_id, image, resolution_divisor);
		image
	}

	/// Creates a replacement for the color image named `main`.
	///
	/// The renderer supplies the sink swapchain when this pass is terminal. In
	/// every other position, this creates and aliases the requested image. Bind
	/// the returned target as a writable image without inspecting its variant.
	/// Compute shaders must use a formatless storage-image output because the
	/// swapchain format can differ from the intermediate format. Raster passes
	/// should derive their attachment descriptor from the returned target. Next,
	/// bind a compute output with [`simple_compute::Resource::image`].
	///
	/// While the pass is bypassed, the renderer copies the incoming `main` into the replacement, so later passes read
	/// the image this pass would have transformed.
	pub fn create_main_render_target(&mut self, builder: ghi::image::Builder<'a>) -> MainRenderTarget {
		let name = builder.get_name().expect(
			"Main render target name is missing. The most likely cause is that the image builder was not given a name before replacing `main`.",
		);
		// Resolve the incoming `main` before the replacement takes its name.
		let source = self.read_from("main");
		let main = if self.final_output {
			self.final_output_written = true;
			let target = ghi::ImageOrSwapchain::Swapchain(self.swapchain);
			self.external_writable_targets.push((name.to_string(), target));
			self.external_writable_targets.push(("main".to_string(), target));
			MainRenderTarget {
				target,
				format: ghi::Formats::BGRAsRGB,
			}
		} else {
			let output = self.create_render_target(builder);
			self.alias(name, "main");
			MainRenderTarget {
				target: output.image.into(),
				format: output.format,
			}
		};
		self.main_copy = Some(ImageBypassPass::new(self, source, main));
		main
	}

	pub fn read_from(&mut self, name: &'a str) -> ReadFromResult {
		let index = self.target_index(name);
		self.accesses.push((index, ghi::AccessPolicies::READ));

		ReadFromResult {
			image: self.images.image(index).0,
		}
	}

	pub fn context(&mut self) -> &'_ mut ghi::implementation::Context {
		self.context
	}

	/// Returns a client for requesting pipelines shared by renderer dependants.
	pub fn pipeline_manager(&self) -> &crate::rendering::PipelineManagerClient {
		&self.pipeline_manager
	}

	/// Reports whether this builder gave its pass the terminal swapchain target.
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

/// The `MainRenderTarget` struct hides whether a replacement `main` output is an image or the sink swapchain.
///
/// Pass it directly to [`simple_compute::Resource::image`] or convert it into a
/// raster attachment descriptor.
#[derive(Clone, Copy)]
pub struct MainRenderTarget {
	target: ghi::ImageOrSwapchain,
	format: ghi::Formats,
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
