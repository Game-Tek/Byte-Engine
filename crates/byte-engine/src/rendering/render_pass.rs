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

use ghi::context::ContextCreate as _;
use utils::Box;

use crate::rendering::{Sink, renderer::RenderTargets};

pub trait RenderPassFunction = Fn(&mut ghi::implementation::CommandBufferRecording, &[ghi::AttachmentInformation]);

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

	/// Preserves downstream frame flow and required maintenance work without applying the pass's effect.
	///
	/// Return a forwarding command when later passes depend on this pass's output. A pass that writes in place may
	/// return `None`, while a pass fed by channels should still drain or adopt pending messages before returning.
	fn bypass<'a>(
		&mut self,
		frame: &mut ghi::implementation::Frame,
		sink: &Sink,
		frame_allocator: &'a bumpalo::Bump,
	) -> Option<RenderPassReturn<'a>>;

	/// Reports whether this pass has output that the last presented frame does not show yet.
	///
	/// On-demand rendering only draws a frame when something asks for one. Return `true` while the pass's inputs
	/// changed since it last recorded, or while it animates. Passes that only transform their input keep the default.
	fn needs_frame(&mut self) -> bool {
		false
	}
}

/// Implements a [`RenderPass`] method that only hands the frame to an inner pass.
///
/// Composite passes keep their real work in a field: an effect pass forwards `prepare` to the compute
/// pass it wraps, and bypasses by forwarding to an
/// [`ImageBypassPass`](crate::rendering::render_passes::blit::ImageBypassPass) so later passes still see
/// an image. Name the field once instead of repeating the signature:
///
/// ```ignore
/// impl RenderPass for AgxToneMapPass {
///     fn name(&self) -> &'static str { "agx" }
///     crate::rendering::render_pass::forward_to_inner_pass!(prepare = render_pass);
///     crate::rendering::render_pass::forward_to_inner_pass!(bypass = bypass_pass);
/// }
/// ```
///
/// A pass that must also do maintenance work on one of these paths writes that method out itself.
macro_rules! forward_to_inner_pass {
	(prepare = $field:ident) => {
		fn prepare<'a>(
			&mut self,
			frame: &mut ::ghi::implementation::Frame,
			sink: &$crate::rendering::Sink,
			frame_allocator: &'a bumpalo::Bump,
		) -> Option<$crate::rendering::render_pass::RenderPassReturn<'a>> {
			self.$field.prepare(frame, sink, frame_allocator)
		}
	};
	(bypass = $field:ident) => {
		fn bypass<'a>(
			&mut self,
			frame: &mut ::ghi::implementation::Frame,
			sink: &$crate::rendering::Sink,
			frame_allocator: &'a bumpalo::Bump,
		) -> Option<$crate::rendering::render_pass::RenderPassReturn<'a>> {
			self.$field.prepare(frame, sink, frame_allocator)
		}
	};
}

pub(crate) use forward_to_inner_pass;

/// The `RenderPassState` enum identifies which preparation path a [`RenderPassHarness`] uses.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RenderPassState {
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

/// The `RenderPassHarness` struct owns one render pass and keeps its execution state outside the implementation.
///
/// Construct a harness with [`Self::new`], then change its state with [`Self::set_state`]. Call [`Self::prepare`]
/// once per eligible sink and frame so the harness can select the active or bypass preparation path.
pub struct RenderPassHarness {
	render_pass: Box<dyn RenderPass>,
	state: RenderPassState,
}

impl RenderPassHarness {
	/// Creates an enabled harness for a render pass.
	pub fn new(render_pass: Box<dyn RenderPass>) -> Self {
		Self {
			render_pass,
			state: RenderPassState::Enabled,
		}
	}

	/// Reports whether the wrapped pass asks for a new frame; see [`RenderPass::needs_frame`].
	pub fn needs_frame(&mut self) -> bool {
		self.render_pass.needs_frame()
	}

	/// Returns the pass state used for the next frame preparation.
	pub fn state(&self) -> RenderPassState {
		self.state
	}

	/// Returns the stable name supplied by the render pass implementation.
	pub fn name(&self) -> &'static str {
		self.render_pass.name()
	}

	/// Selects whether future frame preparation applies or bypasses the pass.
	pub fn set_state(&mut self, state: RenderPassState) {
		self.state = state;
	}

	/// Prepares the active or bypass path selected by [`Self::state`].
	pub fn prepare<'a>(
		&mut self,
		frame: &mut ghi::implementation::Frame,
		sink: &Sink,
		frame_allocator: &'a bumpalo::Bump,
	) -> Option<RenderPassReturn<'a>> {
		match execution_path(self.state) {
			RenderPassExecutionPath::Prepare => self.render_pass.prepare(frame, sink, frame_allocator),
			RenderPassExecutionPath::Bypass => self.render_pass.bypass(frame, sink, frame_allocator),
		}
	}
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum RenderPassExecutionPath {
	Prepare,
	Bypass,
}

/// Converts public pass state into the one method the harness must invoke.
fn execution_path(state: RenderPassState) -> RenderPassExecutionPath {
	match state {
		RenderPassState::Enabled => RenderPassExecutionPath::Prepare,
		RenderPassState::Bypassed => RenderPassExecutionPath::Bypass,
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
	final_output: bool,
	final_output_written: bool,
	pub(crate) consumed_resources: Vec<(&'a str, ghi::AccessPolicies)>,
	/// Every render-target image this pass reads or writes, by index, so the renderer knows when each one is used.
	pub(crate) accessed_image_indices: Vec<usize>,
	written_image_indices: Vec<usize>,
	external_writable_targets: Vec<(String, ghi::ImageOrSwapchain)>,
	pub(crate) images: &'a mut RenderTargets,
	pipeline_manager: crate::rendering::PipelineManagerClient,
}

impl<'a> RenderPassBuilder<'a> {
	pub fn new(
		context: &'a mut ghi::implementation::Context,
		images: &'a mut RenderTargets,
		sink_id: usize,
		swapchain: ghi::SwapchainHandle,
		pipeline_manager: crate::rendering::PipelineManagerClient,
	) -> Self {
		RenderPassBuilder {
			context,
			sink_id,
			swapchain,
			final_output: false,
			final_output_written: false,
			consumed_resources: Vec::new(),
			accessed_image_indices: Vec::new(),
			written_image_indices: Vec::new(),
			external_writable_targets: Vec::new(),
			images,
			pipeline_manager,
		}
	}

	/// Creates the builder used for the terminal pass in one sink-local graph.
	pub(crate) fn new_for_final_pass(
		context: &'a mut ghi::implementation::Context,
		images: &'a mut RenderTargets,
		sink_id: usize,
		swapchain: ghi::SwapchainHandle,
		pipeline_manager: crate::rendering::PipelineManagerClient,
	) -> Self {
		let mut builder = Self::new(context, images, sink_id, swapchain, pipeline_manager);
		builder.final_output = true;
		builder
	}

	pub fn alias(&mut self, orig: &'a str, alias: &'a str) {
		self.images.alias(self.sink_id, orig, alias);
	}

	pub fn format_of(&self, name: &str) -> ghi::Formats {
		self.images.get(name, self.sink_id).expect("Image not found").1
	}

	/// Returns an existing image for writing by this render pass.
	pub fn render_to(&mut self, name: &'a str) -> RenderToResult {
		self.consumed_resources.push((name, ghi::AccessPolicies::WRITE));
		self.images.write_to(name, self.sink_id);

		let image_index = self.images.get_image_index(name, self.sink_id).expect("Image not found");
		self.written_image_indices.push(image_index);
		self.accessed_image_indices.push(image_index);
		let (image, format) = self.images.image(image_index).expect("Image not found");

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
		self.consumed_resources.push((name, ghi::AccessPolicies::WRITE));

		let image = self.context.build_image(builder.additional_uses(ghi::Uses::TransferSource));

		let image_index = self.images.insert(
			name.to_string(),
			self.sink_id,
			image.into(),
			format,
			resolution_divisor,
			member,
		);
		self.written_image_indices.push(image_index);
		self.accessed_image_indices.push(image_index);

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
	pub fn create_main_render_target(&mut self, builder: ghi::image::Builder<'a>) -> MainRenderTarget {
		let name = builder.get_name().expect(
			"Main render target name is missing. The most likely cause is that the image builder was not given a name before replacing `main`.",
		);
		if self.final_output {
			self.final_output_written = true;
			let target = ghi::ImageOrSwapchain::Swapchain(self.swapchain);
			self.consumed_resources.push(("main", ghi::AccessPolicies::WRITE));
			self.external_writable_targets.push((name.to_string(), target));
			self.external_writable_targets.push(("main".to_string(), target));
			return MainRenderTarget {
				target,
				format: ghi::Formats::BGRAsRGB,
			};
		}

		let output = self.create_render_target(builder);
		self.alias(name, "main");
		MainRenderTarget {
			target: output.image.into(),
			format: output.format,
		}
	}

	pub fn read_from(&mut self, name: &'a str) -> ReadFromResult {
		self.consumed_resources.push((name, ghi::AccessPolicies::READ));
		self.images.read_from(name, self.sink_id);
		if let Some(index) = self.images.get_image_index(name, self.sink_id) {
			self.accessed_image_indices.push(index);
		}

		let (image, _) = *self.images.get(name, self.sink_id).expect("Image not found");

		ReadFromResult { image }
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

	/// Snapshots every current name and alias that resolves to a target written by this pass.
	pub(crate) fn writable_targets(&self) -> Vec<(String, ghi::ImageOrSwapchain)> {
		self.images
			.names_for_images(self.sink_id, &self.written_image_indices)
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

#[derive(Hash)]
pub struct FramePrepare {}

impl Default for FramePrepare {
	fn default() -> Self {
		Self::new()
	}
}

impl FramePrepare {
	pub fn new() -> Self {
		FramePrepare {}
	}

	pub fn sinks(&self) -> &[Sink] {
		&[]
	}
}

#[cfg(test)]
mod tests {
	use super::{RenderPassExecutionPath, RenderPassState, execution_path};

	#[test]
	fn render_pass_state_selects_the_expected_execution_path() {
		assert_eq!(execution_path(RenderPassState::Enabled), RenderPassExecutionPath::Prepare);
		assert_eq!(execution_path(RenderPassState::Bypassed), RenderPassExecutionPath::Bypass);
	}
}
