//! The G.H.I. module (graphics hardware interface) is responsible for abstracting the access to the graphics hardware.

#![allow(dead_code)]
#![allow(incomplete_features)]
#![allow(private_interfaces)]
#![feature(generic_const_exprs)]

pub mod window;

pub mod frame_resources;
pub mod graphics_hardware_interface;
pub mod render_debugger;

pub mod debug;
pub mod factory;

#[cfg(target_os = "windows")]
pub mod dx12;
#[cfg(target_os = "macos")]
pub mod metal;
#[cfg(target_os = "linux")]
pub mod vulkan;

pub(crate) use crate::frame_resources::*;
pub use crate::graphics_hardware_interface::{
	AllocationHandle, AttachmentInformation, BaseBufferHandle, BaseImageHandle, BindingConstructor,
	BottomLevelAccelerationStructure, BottomLevelAccelerationStructureDescriptions, BottomLevelAccelerationStructureHandle,
	BufferHandle, ClearValue, CommandBufferHandle, DescriptorSetBindingHandle, DescriptorSetBindingTemplate,
	DescriptorSetHandle, DescriptorSetTemplateHandle, DispatchExtent, DynamicBufferHandle, DynamicImageHandle, FrameKey,
	ImageHandle, ImageOrSwapchain, MeshHandle, PipelineHandle, PipelineLayoutHandle, PresentKey, PresentationModes,
	QueueHandle, QueueSelection, RGBAu8, SamplerHandle, ShaderHandle, SwapchainHandle, SynchronizerHandle, TextureCopyHandle,
	TextureViewTypes, TopLevelAccelerationStructureHandle,
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
mod utils;

pub use context::{Context, ContextCreate};
pub use device::Device;
pub use frame::Frame;
pub use pipelines::ShaderParameter;
pub use queue::Queue;
use smallvec::SmallVec;
pub use types::{
	AccessPolicies, BufferCopyDescriptor, BufferDescriptor, BufferImageCopyDescriptor, BufferStridedRange, ChannelBitSize,
	ChannelLayout, DataTypes, DeviceAccesses, Encodings, FilteringModes, Formats, ImageBufferCopyDescriptor, Layouts,
	SamplerAddressingModes, SamplingReductionModes, ShaderTypes, Size, Stages, UseCases, Uses, WorkloadTypes,
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
