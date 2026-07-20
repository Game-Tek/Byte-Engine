//! Composable sink-local rendering stages.
//!
//! Implement [`RenderPass`] for post-processing or overlays that run after scene
//! pipelines. Construct resources through [`RenderPassBuilder`] so the renderer
//! can track access policies and named render targets. Existing implementations
//! live in [`crate::rendering::render_passes`].

pub mod simple_compute;

use crate::rendering::{renderer::RenderTargets, shader_store::ShaderSourceDescriptor, Sink};

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
pub trait RenderPass {
	/// Prepares the render pass when its rendering condition is active.
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
	shader_resources: Option<&'a resource_management::resource::resource_manager::ResourceManager>,
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
			shader_resources: None,
		}
	}

	pub fn with_shader_storage(mut self, shader_storage: &'a dyn resource_management::resource::StorageBackend) -> Self {
		self.shader_storage = Some(shader_storage);
		self
	}

	pub fn with_shader_resources(
		mut self,
		shader_resources: &'a resource_management::resource::resource_manager::ResourceManager,
	) -> Self {
		self.shader_resources = Some(shader_resources);
		self
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

		let (image, format) = *self.images.get(name, self.sink_id).expect("Image not found");

		RenderToResult { image, format }
	}

	/// Creates a render-target image and returns it for writing by this render pass.
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

	/// Loads a previously baked shader resource and creates its GHI shader handle.
	pub fn load_shader(&mut self, id: &str, name: &str) -> Result<crate::rendering::shader_store::LoadedShader, String> {
		let resource_manager = self.shader_resources.ok_or_else(|| {
			format!("Failed to load render-pass shader '{id}'. The renderer has no resource manager configured.")
		})?;
		crate::rendering::shader_store::load_shader_resource(self.context, resource_manager, id, name)
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
