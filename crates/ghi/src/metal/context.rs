use std::collections::VecDeque;
use std::ptr::NonNull;

use ::utils::hash::{HashMap, HashSet};
use dispatch2::DispatchData;
use objc2::runtime::ProtocolObject;
use objc2::ClassType;
use objc2_foundation::{NSAutoreleasePool, NSString};
use objc2_metal::{MTL4CommandEncoder, MTL4ComputeCommandEncoder, MTLBuffer, MTLDevice, MTLResource, MTLTexture};
use smallvec::SmallVec;

use super::*;
use crate::{
	buffer::{self as buffer_builder, BufferHandle},
	descriptors::DescriptorSetHandle,
	image::{self as image_builder, ImageHandle},
	metal::swapchain::Swapchain,
	metal::utils::parse_threadgroup_size_metadata,
	pipelines::raster as raster_pipeline,
	sampler::{self as sampler_builder, SamplerHandle},
	window, DeviceAccesses, HandleLike as _, MasterHandle as _, ResourceCollection, Uses,
};

/// The `Context` struct owns resources created for rendering on a Metal GPU device.
pub struct Context {
	pub(crate) device: Retained<ProtocolObject<dyn mtl::MTLDevice>>,
	pub(crate) compiler: Retained<ProtocolObject<dyn mtl::MTL4Compiler>>,
	pub(crate) frames: u8,
	pub(crate) queues: Vec<queue::StoredQueue>,
	pub(crate) buffers: ResourceCollection<buffer::Buffer, graphics_hardware_interface::BaseBufferHandle, BufferHandle>,
	pub(crate) images: ResourceCollection<image::Image, graphics_hardware_interface::BaseImageHandle, ImageHandle>,
	pub(crate) samplers: Vec<sampler::Sampler>,
	pub(crate) allocations: Vec<Allocation>,
	pub(crate) pipeline_layouts: Vec<PipelineLayout>,
	pub(crate) vertex_layouts: Vec<VertexLayout>,
	vertex_layout_indices: HashMap<VertexLayoutKey, VertexLayoutHandle>,
	pub(crate) descriptor_sets: Vec<descriptor_set::DescriptorSet>,
	pub(crate) meshes: Vec<Mesh>,
	pub(crate) acceleration_structures: Vec<AccelerationStructure>,
	pub(crate) shaders: Vec<Shader>,
	pub(crate) pipelines: Vec<Pipeline>,
	pub(crate) command_buffers: Vec<StoredCommandBuffer>,
	pub(crate) synchronizers: ResourceCollection<
		synchronizer::Synchronizer,
		graphics_hardware_interface::SynchronizerHandle,
		crate::synchronizer::SynchronizerHandle,
	>,
	internal_upload_synchronizer: Option<graphics_hardware_interface::SynchronizerHandle>,
	internal_upload_queues: Vec<Option<graphics_hardware_interface::QueueHandle>>,
	pub(crate) swapchains: Vec<swapchain::Swapchain>,

	pub(crate) resource_to_descriptor:
		HashMap<PrivateHandles, HashSet<(DescriptorSetHandle, crate::shader::ResourceSlot, u32, u8)>>,
	pub(crate) descriptor_set_to_resource:
		HashMap<(DescriptorSetHandle, crate::shader::ResourceSlot, u32, u8), HashSet<PrivateHandles>>,
	descriptor_sources:
		HashMap<(DescriptorSetHandle, crate::shader::ResourceSlot, u32, u8), (crate::descriptors::WriteData, i32)>,

	pub settings: crate::device::Features,
	pub(crate) pending_buffer_syncs: VecDeque<BufferHandle>,
	pub(crate) pending_image_syncs: VecDeque<ImageHandle>,
	pub(crate) tasks: Vec<Task>,

	#[cfg(debug_assertions)]
	pub names: HashMap<graphics_hardware_interface::Handles, String>,
}

impl Drop for Context {
	fn drop(&mut self) {
		// Metal 4 command buffers do not retain resources, so all queue work must finish before context-owned resources drop.
		for synchronizer in self.synchronizers.iter() {
			synchronizer.wait();
		}
	}
}

/// Reports whether a CAMetalLayer drawable can satisfy the requested texture uses directly.
fn drawable_supports_uses(uses: crate::Uses) -> bool {
	let drawable_uses = Uses::RenderTarget
		| Uses::Storage
		| Uses::Image
		| Uses::InputAttachment
		| Uses::TransferSource
		| Uses::TransferDestination
		| Uses::Clear;
	!uses.is_empty() && drawable_uses.contains(uses)
}

mod recording;
mod resources;
mod traits;

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn drawable_uses_accept_render_and_shader_output() {

		assert!(drawable_supports_uses(Uses::RenderTarget | Uses::Storage));
	}

	#[test]
	fn drawable_uses_reject_non_texture_roles() {

		assert!(!drawable_supports_uses(Uses::RenderTarget | Uses::Vertex));
	}
}
