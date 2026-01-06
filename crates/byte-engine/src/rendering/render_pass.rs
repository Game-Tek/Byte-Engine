
use std::{borrow::Borrow, rc::Rc, sync::Arc};

use crate::{core::{EntityHandle, entity::EntityBuilder}, rendering::Viewport};

use ghi::{command_buffer::{BoundComputePipelineMode as _, BoundPipelineLayoutMode as _, CommandBufferRecordable as _, CommonCommandBufferMode as _}, device::Device as _, Device as _};
use resource_management::glsl;
use utils::{hash::{HashMap, HashMapExt}, sync::RwLock, Box, Extent};

/// The type of a boxed function object that writes a render pass to a command buffer
pub type RenderPassCommand = Box<dyn Fn(&mut ghi::CommandBufferRecording, &[ghi::AttachmentInformation]) + Send + Sync>;

/// A `RenderPass` represents the definition of a rendering step.
/// It might own resources that are used during the rendering process.
pub trait RenderPass {
	/// Evaluates rendering condition and potentially prepares the render pass.
	fn prepare(&mut self, frame: &mut ghi::Frame, viewport: &Viewport) -> Option<RenderPassCommand>;
}

pub struct RenderPassBuilder<'a> {
	device: &'a mut ghi::Device,
	pub(crate) consumed_resources: Vec<(&'a str, ghi::AccessPolicies)>,
	pub(crate) images: &'a mut HashMap<String, (ghi::ImageHandle, ghi::Formats, i8)>,
}

impl <'a> RenderPassBuilder<'a> {
	pub fn new(device: &'a mut ghi::Device, images: &'a mut HashMap<String, (ghi::ImageHandle, ghi::Formats, i8)>) -> Self {
		RenderPassBuilder {
			device,
			consumed_resources: Vec::new(),
			images,
		}
	}

	/// Use `render_to` to get a reference to an image you expect to exist.
	pub fn render_to(&mut self, name: &'a str) -> RenderToResult {
		self.consumed_resources.push((name, ghi::AccessPolicies::WRITE));

		let (image, format, _) = self.images.get(name).expect("Image not found").clone();

		RenderToResult { image, format }
	}

	/// Use `create_render_target` to create a new image and get a reference to it.
	pub fn create_render_target(&mut self, builder: ghi::image::Builder<'a>) -> RenderToResult {
		self.consumed_resources.push((builder.get_name().unwrap(), ghi::AccessPolicies::WRITE));

		let name = builder.get_name().unwrap().to_string();
		let format = builder.get_format();

		let image = self.device.build_image(builder);

		let (image, format, _) = self.images.insert(name, (image, format, 0)).unwrap();

		RenderToResult { image, format }
	}

	pub fn read_from(&mut self, name: &'a str) -> ReadFromResult {
		self.consumed_resources.push((name, ghi::AccessPolicies::READ));

		let (image, _, _) = self.images.get(name).expect("Image not found").clone();

		ReadFromResult { image, }
	}

	pub fn device(&mut self) -> &'_ mut ghi::Device {
		self.device
	}
}

pub struct ReadFromResult {
	image: ghi::ImageHandle,
}

impl Into<ghi::ImageHandle> for ReadFromResult {
	fn into(self) -> ghi::ImageHandle {
		self.image
	}
}

impl Into<ghi::ImageHandle> for &ReadFromResult {
	fn into(self) -> ghi::ImageHandle {
		self.image
	}
}

pub struct RenderToResult {
	image: ghi::ImageHandle,
	format: ghi::Formats,
}

impl Into<ghi::ImageHandle> for RenderToResult {
	fn into(self) -> ghi::ImageHandle {
		self.image
	}
}

impl Into<ghi::PipelineAttachmentInformation> for RenderToResult {
	fn into(self) -> ghi::PipelineAttachmentInformation {
		ghi::PipelineAttachmentInformation::new(self.format)
	}
}

#[derive(Hash)]
pub struct FramePrepare {

}

impl FramePrepare {
	pub fn new() -> Self {
		FramePrepare {

		}
	}

	pub fn viewports(&self) -> &[Viewport] {
		&[]
	}
}


pub struct RenderPassViewBuilder<'a> {
	device: &'a mut ghi::Device,
	pub(crate) consumed_resources: Vec<(&'a str, ghi::AccessPolicies)>,
	pub(crate) images: &'a mut HashMap<String, (ghi::ImageHandle, ghi::Formats, i8)>,
}

impl <'a> RenderPassViewBuilder<'a> {
	pub fn new(device: &'a mut ghi::Device, images: &'a mut HashMap<String, (ghi::ImageHandle, ghi::Formats, i8)>) -> Self {
		Self {
			device,
			consumed_resources: Vec::new(),
			images,
		}
	}

	/// Use `render_to` to get a reference to an image you expect to exist.
	pub fn render_to(&mut self, name: &'a str) -> RenderToResult {
		self.consumed_resources.push((name, ghi::AccessPolicies::WRITE));

		let (image, format, _) = self.images.get(name).expect("Image not found").clone();

		RenderToResult { image, format }
	}

	/// Use `create_render_target` to create a new image and get a reference to it.
	pub fn create_render_target(&mut self, builder: ghi::image::Builder<'a>) -> RenderToResult {
		self.consumed_resources.push((builder.get_name().unwrap(), ghi::AccessPolicies::WRITE));

		let name = builder.get_name().unwrap().to_string();
		let format = builder.get_format();

		let image = self.device.build_image(builder);

		let (image, format, _) = self.images.insert(name, (image, format, 0)).unwrap();

		RenderToResult { image, format }
	}

	pub fn read_from(&mut self, name: &'a str) -> ReadFromResult {
		self.consumed_resources.push((name, ghi::AccessPolicies::READ));

		let (image, _, _) = self.images.get(name).expect("Image not found").clone();

		ReadFromResult { image, }
	}

	pub fn device(&mut self) -> &'_ mut ghi::Device {
		self.device
	}
}
