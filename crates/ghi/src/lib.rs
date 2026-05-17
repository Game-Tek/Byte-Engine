//! The G.H.I. module (graphics hardware interface) is responsible for abstracting the access to the graphics hardware.

#![feature(generic_const_exprs)]
#![feature(str_as_str)]
#![feature(pointer_is_aligned_to)]
#![feature(extend_one)]

pub mod window;

pub mod frame_resources;
pub mod graphics_hardware_interface;
pub mod render_debugger;

pub mod debug;

#[cfg(target_os = "windows")]
pub mod dx12;
#[cfg(target_os = "macos")]
pub mod metal;
#[cfg(target_os = "linux")]
pub mod vulkan;

pub use crate::frame_resources::*;
pub use crate::graphics_hardware_interface::*;
pub use crate::window::*;

pub mod implementation {
	pub const USES_DX12: bool = cfg!(target_os = "windows");
	pub const USES_METAL: bool = cfg!(target_os = "macos");

	#[cfg(target_os = "windows")]
	pub use dx12::factory::{ComputePipeline, Factory, FactoryImage, FactorySampler, RasterPipeline};
	#[cfg(target_os = "windows")]
	pub use dx12::CommandBufferRecording;
	#[cfg(target_os = "windows")]
	pub use dx12::Device as Context;
	#[cfg(target_os = "windows")]
	pub use dx12::Device;
	#[cfg(target_os = "windows")]
	pub use dx12::Frame;
	#[cfg(target_os = "windows")]
	pub use dx12::Instance;
	#[cfg(target_os = "windows")]
	pub(crate) struct Binding {
		pub(crate) next: Option<crate::binding::DescriptorSetBindingHandle>,
	}
	#[cfg(target_os = "windows")]
	pub(crate) struct Buffer;
	#[cfg(target_os = "windows")]
	pub(crate) struct DescriptorSet {
		pub(crate) next: Option<crate::descriptors::DescriptorSetHandle>,
	}
	#[cfg(target_os = "windows")]
	pub(crate) struct Image;
	#[cfg(target_os = "windows")]
	pub(crate) struct Synchronizer {
		pub(crate) next: Option<crate::synchronizer::SynchronizerHandle>,
	}
	#[cfg(target_os = "macos")]
	pub(crate) use metal::buffer::Buffer;
	#[cfg(target_os = "macos")]
	pub(crate) use metal::image::Image;
	#[cfg(target_os = "macos")]
	pub use metal::pipelines::factory::{
		ComputePipeline, Factory, Image as FactoryImage, Pipeline as RasterPipeline, Sampler as FactorySampler,
	};
	#[cfg(target_os = "macos")]
	pub use metal::queue::Queue;
	#[cfg(target_os = "macos")]
	pub(crate) use metal::Binding;
	#[cfg(target_os = "macos")]
	pub use metal::CommandBufferRecording;
	#[cfg(target_os = "macos")]
	pub use metal::Context;
	#[cfg(target_os = "macos")]
	pub(crate) use metal::DescriptorSet;
	#[cfg(target_os = "macos")]
	pub use metal::Device;
	#[cfg(target_os = "macos")]
	pub use metal::Frame;
	#[cfg(target_os = "macos")]
	pub use metal::Instance;
	#[cfg(target_os = "macos")]
	pub(crate) use metal::Synchronizer;
	#[cfg(target_os = "linux")]
	pub(crate) use vulkan::binding::Binding;
	#[cfg(target_os = "linux")]
	pub(crate) use vulkan::buffer::Buffer;
	#[cfg(target_os = "linux")]
	pub(crate) use vulkan::descriptor_set::DescriptorSet;
	#[cfg(target_os = "linux")]
	pub use vulkan::factory::{ComputePipeline, Factory, FactoryImage, FactorySampler, RasterPipeline};
	#[cfg(target_os = "linux")]
	pub(crate) use vulkan::image::Image;
	#[cfg(target_os = "linux")]
	pub use vulkan::queue::Queue;
	#[cfg(target_os = "linux")]
	pub use vulkan::CommandBufferRecording;
	#[cfg(target_os = "linux")]
	pub use vulkan::Context;
	#[cfg(target_os = "linux")]
	pub use vulkan::Device;
	#[cfg(target_os = "linux")]
	pub use vulkan::Frame;
	#[cfg(target_os = "linux")]
	pub use vulkan::Instance;
	#[cfg(target_os = "linux")]
	pub(crate) use vulkan::Synchronizer;

	#[cfg(target_os = "windows")]
	use crate::dx12;
	#[cfg(target_os = "macos")]
	use crate::metal;
	#[cfg(target_os = "linux")]
	use crate::vulkan;
}

pub mod binding;
pub mod buffer;
pub mod command_buffer;
pub mod context;
pub mod descriptors;
pub mod device;
pub mod factory;
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
pub use types::*;

pub(crate) const MAX_FRAMES_IN_FLIGHT: usize = 3;

pub(crate) use implementation::Binding;
pub(crate) use implementation::Buffer;
pub(crate) use implementation::DescriptorSet;
pub(crate) use implementation::Image;
pub(crate) use implementation::Synchronizer;

#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum PrivateHandles {
	Image(image::ImageHandle),
	Buffer(buffer::BufferHandle),
	Synchronizer(synchronizer::SynchronizerHandle),
	Swapchain(swapchain::SwapchainHandle),
	#[cfg(any(target_os = "linux", target_os = "windows"))]
	VkBuffer(ash::vk::Buffer),
	#[cfg(any(target_os = "linux", target_os = "windows"))]
	TopLevelAccelerationStructure(vulkan::TopLevelAccelerationStructureHandle),
	#[cfg(any(target_os = "linux", target_os = "windows"))]
	BottomLevelAccelerationStructure(vulkan::BottomLevelAccelerationStructureHandle),
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

		return if let Some(e) = collection
			.iter()
			.enumerate()
			.find(|(_, e)| e.next() == handle_option)
			.map(|(i, _)| Self::build(i as u64))
		{
			e.root(collection)
		} else {
			handle_option.unwrap()
		};
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
