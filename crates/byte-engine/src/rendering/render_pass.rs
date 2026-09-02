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
}

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
		let (image, format) = self.images.image(image_index).expect("Image not found");

		RenderToResult { image, format }
	}

	/// Creates a transferable render-target image and returns it for writing by this render pass.
	pub fn create_render_target(&mut self, builder: ghi::image::Builder<'a>) -> RenderToResult {
		let name = builder.get_name().expect(
			"Render target name is missing. The most likely cause is that the image builder was not given a name before creating the target.",
		);
		let format = builder.get_format();
		self.consumed_resources.push((name, ghi::AccessPolicies::WRITE));

		let image = self.context.build_image(builder.additional_uses(ghi::Uses::TransferSource));

		let image_index = self.images.insert(name.to_string(), self.sink_id, image.into(), format);
		self.written_image_indices.push(image_index);

		RenderToResult {
			image: image.into(),
			format,
		}
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
