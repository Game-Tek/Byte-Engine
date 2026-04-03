use std::{borrow::Borrow, rc::Rc, sync::Arc};

use crate::{
	core::EntityHandle,
	rendering::{renderer::RenderTargets, Viewport},
};

use ghi::{
	command_buffer::{
		BoundComputePipelineMode as _, BoundPipelineLayoutMode as _, CommandBufferRecording as _, CommonCommandBufferMode as _,
	},
	device::{Device as _, DeviceCreate as _},
};
use resource_management::glsl;
use utils::{
	hash::{HashMap, HashMapExt},
	sync::RwLock,
	Box, Extent,
};

pub trait RenderPassFunction = Fn(&mut ghi::implementation::CommandBufferRecording, &[ghi::AttachmentInformation]);

/// The type of a boxed function object that writes a render pass to a command buffer
pub type RenderPassReturn = Box<dyn RenderPassFunction + Send + Sync>;

/// A `RenderPass` represents the definition of a rendering step.
/// It might own resources that are used during the rendering process.
pub trait RenderPass {
	/// Evaluates rendering condition and potentially prepares the render pass.
	fn prepare(&mut self, frame: &mut ghi::implementation::Frame, viewport: &Viewport) -> Option<RenderPassReturn>;
}

pub struct RenderPassBuilder<'a> {
	device: &'a mut ghi::implementation::Device,
	view_id: usize,
	pub(crate) consumed_resources: Vec<(&'a str, ghi::AccessPolicies)>,
	pub(crate) images: &'a mut RenderTargets,
}

impl<'a> RenderPassBuilder<'a> {
	pub fn new(device: &'a mut ghi::implementation::Device, images: &'a mut RenderTargets, view_id: usize) -> Self {
		RenderPassBuilder {
			device,
			view_id,
			consumed_resources: Vec::new(),
			images,
		}
	}

	pub fn alias(&mut self, orig: &'a str, alias: &'a str) {
		self.images.alias(orig, alias);
	}

	pub fn format_of(&self, name: &str) -> ghi::Formats {
		self.images.get(name).expect("Image not found").1
	}

	/// Use `render_to` to get a reference to an image you expect to exist.
	pub fn render_to(&mut self, name: &'a str) -> RenderToResult {
		self.consumed_resources.push((name, ghi::AccessPolicies::WRITE));
		self.images.write_to(name, self.view_id);

		let (image, format) = self.images.get(name).expect("Image not found").clone();

		RenderToResult {
			image: image.into(),
			format,
		}
	}

	/// Use `create_render_target` to create a new image and get a reference to it.
	pub fn create_render_target(&mut self, builder: ghi::image::Builder<'a>) -> RenderToResult {
		self.consumed_resources
			.push((builder.get_name().unwrap(), ghi::AccessPolicies::WRITE));

		let name = builder.get_name().unwrap().to_string();
		let format = builder.get_format();

		let image = self.device.build_image(builder);

		self.images.insert(name, self.view_id, image.into(), format);

		RenderToResult {
			image: image.into(),
			format,
		}
	}

	pub fn read_from(&mut self, name: &'a str) -> ReadFromResult {
		self.consumed_resources.push((name, ghi::AccessPolicies::READ));
		self.images.read_from(name, self.view_id);

		let (image, _) = self.images.get(name).expect("Image not found").clone();

		ReadFromResult { image: image.into() }
	}

	pub fn device(&mut self) -> &'_ mut ghi::implementation::Device {
		self.device
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

impl Into<ghi::pipelines::raster::AttachmentDescriptor> for RenderToResult {
	fn into(self) -> ghi::pipelines::raster::AttachmentDescriptor {
		ghi::pipelines::raster::AttachmentDescriptor::new(self.format)
	}
}

#[derive(Hash)]
pub struct FramePrepare {}

impl FramePrepare {
	pub fn new() -> Self {
		FramePrepare {}
	}

	pub fn viewports(&self) -> &[Viewport] {
		&[]
	}
}
