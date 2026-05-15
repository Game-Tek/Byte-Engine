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

#[cfg(all(target_os = "windows", feature = "dx12"))]
pub mod dx12;
#[cfg(target_os = "macos")]
pub mod metal;
#[cfg(any(target_os = "linux", all(target_os = "windows", not(feature = "dx12"))))]
pub mod vulkan;

pub use crate::frame_resources::*;
pub use crate::graphics_hardware_interface::*;
pub use crate::window::*;

pub mod implementation {
	pub const USES_DX12: bool = cfg!(all(target_os = "windows", feature = "dx12"));
	pub const USES_METAL: bool = cfg!(target_os = "macos");

	#[cfg(all(target_os = "windows", feature = "dx12"))]
	pub use dx12::factory::{ComputePipeline, Factory, FactoryImage, FactorySampler, RasterPipeline};
	#[cfg(all(target_os = "windows", feature = "dx12"))]
	pub use dx12::CommandBufferRecording;
	#[cfg(all(target_os = "windows", feature = "dx12"))]
	pub use dx12::Device as Context;
	#[cfg(all(target_os = "windows", feature = "dx12"))]
	pub use dx12::Device;
	#[cfg(all(target_os = "windows", feature = "dx12"))]
	pub use dx12::Frame;
	#[cfg(all(target_os = "windows", feature = "dx12"))]
	pub use dx12::Instance;
	#[cfg(all(target_os = "windows", feature = "dx12"))]
	pub(crate) struct Binding {
		pub(crate) next: Option<crate::binding::DescriptorSetBindingHandle>,
	}
	#[cfg(all(target_os = "windows", feature = "dx12"))]
	pub(crate) struct Buffer;
	#[cfg(all(target_os = "windows", feature = "dx12"))]
	pub(crate) struct DescriptorSet {
		pub(crate) next: Option<crate::descriptors::DescriptorSetHandle>,
	}
	#[cfg(all(target_os = "windows", feature = "dx12"))]
	pub(crate) struct Image;
	#[cfg(all(target_os = "windows", feature = "dx12"))]
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
	#[cfg(any(target_os = "linux", all(target_os = "windows", not(feature = "dx12"))))]
	pub(crate) use vulkan::binding::Binding;
	#[cfg(any(target_os = "linux", all(target_os = "windows", not(feature = "dx12"))))]
	pub(crate) use vulkan::buffer::Buffer;
	#[cfg(any(target_os = "linux", all(target_os = "windows", not(feature = "dx12"))))]
	pub(crate) use vulkan::descriptor_set::DescriptorSet;
	#[cfg(any(target_os = "linux", all(target_os = "windows", not(feature = "dx12"))))]
	pub(crate) use vulkan::image::Image;
	#[cfg(any(target_os = "linux", all(target_os = "windows", not(feature = "dx12"))))]
	pub use vulkan::queue::Queue;
	#[cfg(any(target_os = "linux", all(target_os = "windows", not(feature = "dx12"))))]
	pub use vulkan::CommandBufferRecording;
	#[cfg(any(target_os = "linux", all(target_os = "windows", not(feature = "dx12"))))]
	pub use vulkan::Context;
	#[cfg(any(target_os = "linux", all(target_os = "windows", not(feature = "dx12"))))]
	pub use vulkan::Device;
	#[cfg(any(target_os = "linux", all(target_os = "windows", not(feature = "dx12"))))]
	pub use vulkan::Frame;
	#[cfg(any(target_os = "linux", all(target_os = "windows", not(feature = "dx12"))))]
	pub use vulkan::Instance;
	#[cfg(any(target_os = "linux", all(target_os = "windows", not(feature = "dx12"))))]
	pub(crate) use vulkan::Synchronizer;

	#[cfg(all(target_os = "windows", feature = "dx12"))]
	use crate::dx12;
	#[cfg(target_os = "macos")]
	use crate::metal;
	#[cfg(any(target_os = "linux", all(target_os = "windows", not(feature = "dx12"))))]
	use crate::vulkan;

	#[cfg(any(target_os = "linux", all(target_os = "windows", not(feature = "dx12"))))]
	/// The `Factory` struct marks the unsupported detached-resource factory for the Vulkan implementation.
	pub struct Factory;

	#[cfg(any(target_os = "linux", all(target_os = "windows", not(feature = "dx12"))))]
	/// The `ComputePipeline` struct marks the unsupported detached compute pipeline for the Vulkan implementation.
	pub struct ComputePipeline;

	#[cfg(any(target_os = "linux", all(target_os = "windows", not(feature = "dx12"))))]
	/// The `RasterPipeline` struct marks the unsupported detached raster pipeline for the Vulkan implementation.
	pub struct RasterPipeline;

	#[cfg(any(target_os = "linux", all(target_os = "windows", not(feature = "dx12"))))]
	/// The `FactoryImage` struct marks the unsupported detached image for the Vulkan implementation.
	pub struct FactoryImage;

	#[cfg(any(target_os = "linux", all(target_os = "windows", not(feature = "dx12"))))]
	/// The `FactorySampler` struct marks the unsupported detached sampler for the Vulkan implementation.
	pub struct FactorySampler;

	#[cfg(any(target_os = "linux", all(target_os = "windows", not(feature = "dx12"))))]
	impl crate::factory::Factory for Factory {
		type RasterPipeline = RasterPipeline;
		type ComputePipeline = ComputePipeline;
		type Image = FactoryImage;
		type Sampler = FactorySampler;

		fn create_shader(
			&mut self,
			_name: Option<&str>,
			_shader_source_type: crate::shader::Sources,
			_stage: crate::ShaderTypes,
			_shader_binding_descriptors: impl IntoIterator<Item = crate::shader::BindingDescriptor>,
		) -> Result<crate::ShaderHandle, ()> {
			Err(())
		}

		fn create_raster_pipeline(&mut self, _builder: crate::pipelines::raster::Builder) -> Self::RasterPipeline {
			RasterPipeline
		}

		fn create_compute_pipeline(&mut self, _builder: crate::pipelines::compute::Builder) -> Self::ComputePipeline {
			ComputePipeline
		}

		fn build_image(&mut self, _builder: crate::image::Builder) -> Self::Image {
			FactoryImage
		}

		fn build_sampler(&mut self, _builder: crate::sampler::Builder) -> Self::Sampler {
			FactorySampler
		}
	}
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
