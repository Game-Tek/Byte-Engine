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
	clippy::needless_range_loop,
	clippy::new_without_default,
	clippy::result_unit_err,
	clippy::tabs_in_doc_comments,
	clippy::too_many_arguments,
	clippy::type_complexity,
	clippy::unnecessary_literal_unwrap
)]
#![feature(generic_const_exprs)]
#![cfg_attr(target_os = "linux", feature(pointer_is_aligned_to, extend_one, str_as_str))]

pub mod window;

pub mod frame_resources;
mod graphics_hardware_interface;
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
	AllocationHandle, AttachmentInformation, BaseBufferHandle, BaseImageHandle, BottomLevelAccelerationStructure,
	BottomLevelAccelerationStructureDescriptions, BottomLevelAccelerationStructureHandle, BufferHandle, ClearValue,
	CommandBufferHandle, DescriptorSetHandle, DispatchExtent, DynamicBufferHandle, DynamicImageHandle, FrameKey, ImageHandle,
	ImageOrSwapchain, MeshHandle, PipelineHandle, PresentKey, PresentationModes, QueueHandle, QueueSelection, RGBAu8,
	SamplerHandle, ShaderHandle, SwapchainHandle, SynchronizerHandle, TextureCopyHandle, TextureViewTypes,
	TopLevelAccelerationStructureHandle,
};
// Legacy backend-only handles remain crate-private while the non-Metal backends migrate on their target machines.
#[cfg(any(target_os = "linux", target_os = "windows"))]
pub(crate) use crate::graphics_hardware_interface::{
	BindingConstructor, DescriptorSetBindingHandle, DescriptorSetBindingTemplate, DescriptorSetTemplateHandle,
	PipelineLayoutHandle,
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

	#[cfg(test)]
	mod tests {
		use super::*;
		use crate::{graphics_hardware_interface, QueueHandle};

		fn create_default_device_setup() -> (Instance, Context, QueueHandle) {
			let features = crate::device::Features::new().validation(true);
			create_default_device_setup_with_features(features)
		}

		fn create_default_device_setup_with_features(features: crate::device::Features) -> (Instance, Context, QueueHandle) {
			let mut instance = Instance::new(features).expect(
				"Failed to create the GHI test instance. The most likely cause is that the active backend has no available device.",
			);
			let mut queue_handle = None;
			let device = instance
				.create_device(
					features,
					&mut [(
						crate::QueueSelection::new(crate::types::WorkloadTypes::RASTER),
						&mut queue_handle,
					)],
				)
				.expect("Failed to create the GHI test device. The most likely cause is unavailable raster queue support.");
			let context = crate::device::Device::create_context(&device)
				.expect("Failed to create the GHI test context. The most likely cause is unavailable backend command support.");
			(instance, context, queue_handle.unwrap())
		}

		#[test]
		fn render_triangle() {
			let (_instance, mut device, queue_handle) = create_default_device_setup();
			graphics_hardware_interface::tests::render_triangle(&mut device, queue_handle);
		}

		#[cfg(target_os = "macos")]
		#[test]
		fn raster_pipeline_can_disable_depth_writes() {
			let (_instance, mut device, queue_handle) = create_default_device_setup();
			graphics_hardware_interface::tests::render_without_depth_writes(&mut device, queue_handle);
		}

		#[test]
		#[ignore = "test is broken because of WSI"]
		fn render_present() {
			let (_instance, mut device, queue_handle) = create_default_device_setup();
			graphics_hardware_interface::tests::present(&mut device, queue_handle);
		}

		#[test]
		#[ignore = "test is broken because of WSI"]
		fn render_multiframe_present() {
			let (_instance, mut device, queue_handle) = create_default_device_setup();
			graphics_hardware_interface::tests::multiframe_present(&mut device, queue_handle);
		}

		#[test]
		fn render_multiframe() {
			let (_instance, mut device, queue_handle) = create_default_device_setup();
			graphics_hardware_interface::tests::multiframe_rendering(&mut device, queue_handle);
		}

		#[test]
		fn render_change_frames() {
			let (_instance, mut device, queue_handle) = create_default_device_setup();
			graphics_hardware_interface::tests::change_frames(&mut device, queue_handle);
		}

		#[test]
		fn render_resize() {
			let (_instance, mut device, queue_handle) = create_default_device_setup();
			graphics_hardware_interface::tests::resize(&mut device, queue_handle);
		}

		#[test]
		fn render_dynamic_data() {
			let (_instance, mut device, queue_handle) = create_default_device_setup();
			graphics_hardware_interface::tests::dynamic_data(&mut device, queue_handle);
		}

		#[test]
		fn render_dynamic_textures() {
			let (_instance, mut device, queue_handle) = create_default_device_setup();
			graphics_hardware_interface::tests::dynamic_textures(&mut device, queue_handle);
		}

		#[test]
		fn render_with_descriptor_sets() {
			let (_instance, mut device, queue_handle) = create_default_device_setup();
			graphics_hardware_interface::tests::descriptor_sets(&mut device, queue_handle);
		}

		#[test]
		fn render_with_multiframe_resources() {
			let (_instance, mut device, queue_handle) = create_default_device_setup();
			graphics_hardware_interface::tests::multiframe_resources(&mut device, queue_handle);
		}

		#[test]
		#[ignore = "not working on supporting rt right now"]
		fn render_with_ray_tracing() {
			let (_instance, mut device, queue_handle) =
				create_default_device_setup_with_features(crate::device::Features::new().validation(true).ray_tracing(true));
			graphics_hardware_interface::tests::ray_tracing(&mut device, queue_handle);
		}
	}
}

#[cfg(any(target_os = "linux", target_os = "windows"))]
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
pub use descriptors::DescriptorWrite;
pub use device::Device;
pub use frame::Frame;
pub use pipelines::ShaderParameter;
pub use queue::Queue;
pub use shader::{ResourceKind, ResourceSlot, ShaderResourceDescriptor};
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

#[cfg(any(target_os = "linux", target_os = "windows"))]
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
