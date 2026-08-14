use std::{borrow::Cow, num::NonZeroU32, u64};

use ash::vk::{self, Handle as _, TaggedStructure as _};
use smallvec::SmallVec;
use utils::hash::{HashSet, HashSetExt};
use utils::{
	hash::{HashMap, HashMapExt},
	Extent,
};

use super::{
	utils::{
		into_vk_image_usage_flags, texture_format_and_resource_use_to_image_layout, to_format, to_shader_stage_flags,
		uses_to_vk_usage_flags,
	},
	AccelerationStructure, Allocation, Buffer, BufferHandle, CommandBuffer, CommandBufferInternal, DescriptorHeapArena,
	DescriptorHeaps, DescriptorMaterialization, DescriptorMaterializationHandle, DescriptorSet, Image, MaterializationKey,
	MemoryBackedResourceCreationResult, Mesh, Pipeline, PipelineLayout, PipelineLayoutKey, PipelineResourceDescriptor,
	ResolvedPipelineDescriptor, Sampler, Shader, Swapchain, Synchronizer, TopLevelAccelerationStructureHandle, TransitionState,
	MAX_FRAMES_IN_FLIGHT,
};
use crate::vulkan::{Device, InnerDevice, StoredQueue};
use crate::{
	graphics_hardware_interface, image, sampler,
	synchronizer::SynchronizerHandle,
	vulkan::{
		BufferCopy, BuildBuffer, CommandBufferRecording, Descriptor, Frame, ImageCopy, ImageHandle, Task, Tasks,
		MAX_SWAPCHAIN_IMAGES,
	},
	window, FrameKey, HandleLike, MasterHandle as _, ResourceCollection, Size,
};

/// The `Context` struct owns Vulkan device state while presenting the GHI context API.
pub struct Context {
	pub(super) device: InnerDevice,

	pub(super) frames: u8,

	pub(super) queues: Vec<StoredQueue>,
	pub(super) buffers: ResourceCollection<Buffer, graphics_hardware_interface::BaseBufferHandle, BufferHandle>,
	pub(super) images: Vec<Image>,
	pub(super) samplers: Vec<Sampler>,
	pub(super) allocations: Vec<Allocation>,
	pub(super) pipeline_layouts: Vec<PipelineLayout>,
	pipeline_layout_indices: HashMap<PipelineLayoutKey, graphics_hardware_interface::PipelineLayoutHandle>,
	pub(super) descriptor_sets: Vec<DescriptorSet>,
	pub(super) descriptor_heaps: Option<DescriptorHeaps>,
	pub(super) descriptor_materializations: Vec<Option<DescriptorMaterialization>>,
	materialization_indices: HashMap<MaterializationKey, DescriptorMaterializationHandle>,
	retired_materializations: [Vec<DescriptorMaterializationHandle>; MAX_FRAMES_IN_FLIGHT],
	free_materialization_handles: Vec<DescriptorMaterializationHandle>,
	descriptor_sequence_epochs: [u64; MAX_FRAMES_IN_FLIGHT],
	pub(super) meshes: Vec<Mesh>,
	pub(super) acceleration_structures: Vec<AccelerationStructure>,
	pub(super) shaders: Vec<Shader>,
	pub(super) pipelines: Vec<Pipeline>,
	pub(super) command_buffers: Vec<CommandBuffer>,
	pub(super) synchronizers: Vec<Synchronizer>,
	pub(super) swapchains: Vec<Swapchain>,

	pub settings: crate::device::Features,

	pub(super) states: HashMap<super::Handles, TransitionState>,
	pub(super) buffer_states: HashMap<super::Handles, Vec<super::BufferTransitionState>>,

	/// Tracks pending buffer host to device, or device to host synchronization operations.
	pub(super) pending_buffer_syncs: HashSet<BufferHandle>,
	/// Tracks pending image host to device, or device to host synchronization operations.
	pub(super) pending_image_syncs: HashSet<ImageHandle>,

	/// Tracks all dynamic buffer master handles that use the persistent write mode.
	/// These buffers have their source buffer memcpy'd into the per-frame staging
	/// buffer every frame before GPU copies are issued.
	pub(super) persistent_write_dynamic_buffers: Vec<graphics_hardware_interface::BaseBufferHandle>,

	swapchain_native_supports_formatless_storage_write: bool,
	swapchain_proxy_supports_formatless_storage_write: bool,

	memory_properties: vk::PhysicalDeviceMemoryProperties,

	/// Stores the debug names for resources.
	/// Used when inspecting resources from a rendering debugger such as RenderDoc.
	#[cfg(debug_assertions)]
	pub names: HashMap<graphics_hardware_interface::Handles, String>,

	/// A queue of deferred tasks. Usually object deletions and resource updates.
	pub(crate) tasks: Vec<Task>,
}

/// Accepts deferred descriptor work only while both its payload and post-write version remain current.
fn descriptor_task_is_current(
	set: &DescriptorSet,
	descriptor_write: crate::descriptors::DescriptorWrite,
	expected_set_version: u64,
) -> bool {
	if set.version != expected_set_version {
		return false;
	}

	let frame_offset = descriptor_write.frame_offset.unwrap_or(0);
	set.descriptors
		.get(&descriptor_write.slot)
		.and_then(|elements| elements.get(&descriptor_write.array_element))
		.is_some_and(|current| current.descriptor == descriptor_write.descriptor && current.frame_offset == frame_offset)
}

mod descriptors;
mod drop;
mod pipelines;
mod resources;
mod runtime;
mod traits;

#[cfg(test)]
mod descriptor_task_tests {
	use super::*;

	fn retained_set(write: crate::descriptors::DescriptorWrite, version: u64) -> DescriptorSet {
		let mut descriptors = HashMap::new();
		descriptors.entry(write.slot).or_insert_with(HashMap::new).insert(
			write.array_element,
			crate::vulkan::descriptor_set::RetainedDescriptor {
				descriptor: write.descriptor,
				frame_offset: write.frame_offset.unwrap_or(0),
			},
		);
		DescriptorSet {
			next: None,
			version,
			sequence_versions: [0; MAX_FRAMES_IN_FLIGHT],
			descriptors,
		}
	}

	fn buffer_write(buffer: u64) -> crate::descriptors::DescriptorWrite {
		crate::descriptors::DescriptorWrite::buffer(
			graphics_hardware_interface::DescriptorSetHandle(0),
			crate::shader::ResourceSlot::new(3),
			graphics_hardware_interface::BaseBufferHandle(buffer),
		)
	}

	#[test]
	fn deferred_descriptor_task_ignores_an_overwritten_payload() {
		let old = buffer_write(4);
		let current = buffer_write(5);
		let set = retained_set(current, 2);

		assert!(!descriptor_task_is_current(&set, old, 1));
		assert!(descriptor_task_is_current(&set, current, 2));
	}

	#[test]
	fn deferred_descriptor_task_uses_version_to_reject_aba_writes() {
		let value = buffer_write(7);
		let set = retained_set(value, 3);

		assert!(!descriptor_task_is_current(&set, value, 1));
		assert!(descriptor_task_is_current(&set, value, 3));
	}
}
