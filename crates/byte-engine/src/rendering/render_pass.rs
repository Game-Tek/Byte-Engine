//! Composable sink-local rendering stages.
//!
//! Implement [`RenderPass`] for post-processing or overlays that run after scene
//! pipelines. Construct resources through [`RenderPassBuilder`] so the renderer
//! can track access policies and named render targets. Existing implementations
//! live in [`crate::rendering::render_passes`].

pub mod simple_compute;

use crate::rendering::{renderer::RenderTargets, shader_store::ShaderSourceDescriptor, Sink};

pub trait RenderPassFunction = Fn(&mut ghi::implementation::CommandBufferRecording, &[ghi::AttachmentInformation]);

/// The type of a frame-allocated function object that writes a render pass to a command buffer.
pub type RenderPassReturn<'a> = &'a (dyn RenderPassFunction + Send + Sync + 'a);

/// Allocates a prepared render command in the application frame allocator.
pub fn allocate_render_command<'a>(
	frame_allocator: &'a bumpalo::Bump,
	command: impl RenderPassFunction + Send + Sync + 'a,
) -> RenderPassReturn<'a> {
	frame_allocator.alloc(command)
}

/// The `RenderPass` trait defines a composable rendering step for a prepared sink.
pub trait RenderPass {
	/// Evaluates rendering condition and potentially prepares the render pass.
	fn prepare<'a>(
		&mut self,
		frame: &mut ghi::implementation::Frame,
		sink: &Sink,
		frame_allocator: &'a bumpalo::Bump,
	) -> Option<RenderPassReturn<'a>>;
}

/// The [`RenderPassBuilder`] struct provides sink resources and records the
/// dependencies of a render pass.
pub struct RenderPassBuilder<'a> {
	context: &'a mut ghi::implementation::Context,
	sink_id: usize,
	swapchain: ghi::SwapchainHandle,
	pub(crate) consumed_resources: Vec<(&'a str, ghi::AccessPolicies)>,
	pub(crate) images: &'a mut RenderTargets,
	shader_storage: Option<&'a dyn resource_management::resource::StorageBackend>,
}

impl<'a> RenderPassBuilder<'a> {
	pub fn new(
		context: &'a mut ghi::implementation::Context,
		images: &'a mut RenderTargets,
		sink_id: usize,
		swapchain: ghi::SwapchainHandle,
	) -> Self {
		RenderPassBuilder {
			context,
			sink_id,
			swapchain,
			consumed_resources: Vec::new(),
			images,
			shader_storage: None,
		}
	}

	pub fn with_shader_storage(mut self, shader_storage: &'a dyn resource_management::resource::StorageBackend) -> Self {
		self.shader_storage = Some(shader_storage);
		self
	}

	pub fn alias(&mut self, orig: &'a str, alias: &'a str) {
		self.images.alias(self.sink_id, orig, alias);
	}

	pub fn format_of(&self, name: &str) -> ghi::Formats {
		self.images.get(name, self.sink_id).expect("Image not found").1
	}

	/// Use `render_to` to get a reference to an image you expect to exist.
	pub fn render_to(&mut self, name: &'a str) -> RenderToResult {
		self.consumed_resources.push((name, ghi::AccessPolicies::WRITE));
		self.images.write_to(name, self.sink_id);

		let (image, format) = *self.images.get(name, self.sink_id).expect("Image not found");

		RenderToResult { image, format }
	}

	/// Use `create_render_target` to create a new image and get a reference to it.
	pub fn create_render_target(&mut self, builder: ghi::image::Builder<'a>) -> RenderToResult {
		self.consumed_resources
			.push((builder.get_name().unwrap(), ghi::AccessPolicies::WRITE));

		let name = builder.get_name().unwrap().to_string();
		let format = builder.get_format();

		let image = self.context.build_image(builder);

		self.images.insert(name, self.sink_id, image.into(), format);

		RenderToResult {
			image: image.into(),
			format,
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

	pub fn create_shader(&mut self, descriptor: &ShaderSourceDescriptor<'_>) -> Result<ghi::ShaderHandle, String> {
		crate::rendering::shader_store::create_shader(self.context, self.shader_storage, descriptor)
	}

	pub(crate) fn shader_storage(&self) -> Option<&'a dyn resource_management::resource::StorageBackend> {
		self.shader_storage
	}

	pub(crate) fn render_to_swapchain(&self) -> ghi::SwapchainHandle {
		self.swapchain
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

impl From<RenderToResult> for ghi::pipelines::raster::AttachmentDescriptor {
	fn from(val: RenderToResult) -> Self {
		ghi::pipelines::raster::AttachmentDescriptor::new(val.format)
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
