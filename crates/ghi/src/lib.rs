//! Use the graphics hardware interface (GHI) to issue rendering work across supported GPU backends.
//!
//! Start with the platform [`implementation::Instance`], select a device and
//! queues, then create context-owned resources through [`ContextCreate`]. Record
//! work with [`command_buffer::CommandBuffer`] and submit it through [`Queue`].

#![allow(dead_code)]
#![allow(incomplete_features)]
#![allow(private_interfaces)]
// GHI mirrors backend API shapes closely; these lint classes are deferred until the graphics interfaces are redesigned intentionally.
#![allow(
	clippy::module_inception,
	clippy::collapsible_if,
	clippy::cognitive_complexity,
	clippy::excessive_nesting,
	clippy::needless_range_loop,
	clippy::new_without_default,
	clippy::multiple_unsafe_ops_per_block,
	clippy::result_unit_err,
	clippy::tabs_in_doc_comments,
	clippy::too_many_arguments,
	clippy::too_many_lines,
	clippy::type_complexity,
	clippy::undocumented_unsafe_blocks,
	clippy::unnecessary_literal_unwrap
)]
#![feature(allocator_api)]
#![cfg_attr(target_os = "linux", feature(pointer_is_aligned_to, str_as_str))]

pub mod window;

pub mod frame_resources;
mod graphics_hardware_interface;
pub mod io;
pub mod render_debugger;

pub mod debug;
pub mod factory;

#[cfg(target_os = "windows")]
pub mod dx12;
#[cfg(target_os = "macos")]
pub mod metal;
#[cfg(target_os = "linux")]
pub mod vulkan;

pub use bytemuck::{Pod, Zeroable};

#[cfg(not(target_os = "windows"))]
pub(crate) use crate::frame_resources::*;
#[cfg(target_os = "windows")]
pub(crate) use crate::graphics_hardware_interface::PipelineLayoutHandle;
pub use crate::graphics_hardware_interface::{
	AllocationHandle, AttachmentInformation, BaseBufferHandle, BaseImageHandle, BottomLevelAccelerationStructure,
	BottomLevelAccelerationStructureDescriptions, BottomLevelAccelerationStructureHandle, BufferHandle, ClearValue,
	CommandBufferHandle, DescriptorSetHandle, DispatchExtent, DynamicBufferHandle, DynamicImageHandle, FrameKey, ImageHandle,
	ImageOrSwapchain, MeshHandle, PipelineHandle, PresentKey, PresentationModes, QueueHandle, QueueSelection, RGBAu8,
	SamplerHandle, ShaderHandle, SwapchainHandle, SynchronizerHandle, TextureCopyHandle, TextureViewTypes,
	TopLevelAccelerationStructureHandle,
};
pub(crate) use crate::graphics_hardware_interface::{MasterHandle, PrivateHandle, Ranges};
pub use crate::window::Window;

pub mod implementation {
	pub const USES_DX12: bool = cfg!(target_os = "windows");
	pub const USES_METAL: bool = cfg!(target_os = "macos");
	pub const USES_VULKAN: bool = cfg!(target_os = "linux");

	#[cfg(target_os = "windows")]
	pub use crate::dx12::*;
	#[cfg(target_os = "macos")]
	pub use crate::metal::*;
	#[cfg(target_os = "linux")]
	pub use crate::vulkan::*;
}

#[cfg(target_os = "windows")]
pub mod binding;
pub mod buffer;
pub mod command_buffer;
pub mod context;
pub mod descriptors;
pub mod device;
pub mod frame;
pub mod image;
pub mod pipelines;
pub mod queue;
pub mod rt;
pub mod sampler;
pub mod shader;
pub mod swapchain;
pub mod synchronizer;

pub mod types;

pub use context::{Context, ContextCreate, TextureReadback, TextureTransferError};
pub use descriptors::DescriptorWrite;
pub use device::Device;
pub use frame::Frame;
pub use pipelines::ShaderParameter;
pub use queue::Queue;
pub use shader::{ResourceKind, ResourceSlot, ShaderResourceDescriptor};
use smallvec::SmallVec;
pub use types::{
	AccessPolicies, BufferCopyDescriptor, BufferDescriptor, BufferImageCopyDescriptor, BufferStridedRange, ChannelBitSize,
	ChannelLayout, DataTypes, DeviceAccesses, Encodings, FilteringModes, Formats, Layouts, SamplerAddressingModes,
	SamplingReductionModes, ShaderTypes, Size, Stages, UseCases, Uses, WorkloadTypes,
};

pub(crate) const MAX_FRAMES_IN_FLIGHT: usize = 3;

#[cfg(debug_assertions)]
#[inline]
pub(crate) fn debug_name(name: Option<&str>) -> Option<String> {
	name.map(str::to_owned)
}

#[cfg(not(debug_assertions))]
#[inline]
pub(crate) fn debug_name(_name: Option<&str>) -> Option<String> {
	None
}

#[cfg(target_os = "windows")]
pub(crate) use implementation::Binding;
pub(crate) use implementation::DescriptorSet;
pub(crate) use implementation::Synchronizer;

#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum PrivateHandles {
	Image(image::ImageHandle),
	Buffer(buffer::BufferHandle),
	Synchronizer(synchronizer::SynchronizerHandle),
	Swapchain(swapchain::SwapchainHandle),
	#[cfg(target_os = "linux")]
	VkBuffer(ash::vk::Buffer),
	#[cfg(any(target_os = "linux", target_os = "windows"))]
	TopLevelAccelerationStructure(TopLevelAccelerationStructureHandle),
	#[cfg(any(target_os = "linux", target_os = "windows"))]
	BottomLevelAccelerationStructure(BottomLevelAccelerationStructureHandle),
}

pub(crate) trait HandleLike
where
	Self: Sized,
	Self: PartialEq<Self>,
	Self: Clone,
	Self: Copy,
{
	type Item: Next<Handle = Self>;

	fn build(value: u64) -> Self;

	fn access<'a>(&self, collection: &'a [Self::Item]) -> &'a Self::Item;

	fn root(&self, collection: &[Self::Item]) -> Self {
		let handle_option = Some(*self);

		if let Some(e) = collection
			.iter()
			.enumerate()
			.find(|(_, e)| e.next() == handle_option)
			.map(|(i, _)| Self::build(i as u64))
		{
			e.root(collection)
		} else {
			handle_option.unwrap()
		}
	}

	fn get_all(&self, collection: &[Self::Item]) -> SmallVec<[Self; MAX_FRAMES_IN_FLIGHT]> {
		let mut handles = SmallVec::new();
		let mut handle_option = Some(*self);

		while let Some(handle) = handle_option {
			let binding = handle.access(collection);
			handles.push(handle);
			handle_option = binding.next();
		}

		handles
	}
}

pub(crate) trait Next
where
	Self: Sized,
{
	type Handle: HandleLike<Item = Self>;

	fn next(&self) -> Option<Self::Handle>;
}
