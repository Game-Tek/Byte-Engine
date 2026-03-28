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

#[cfg(false)]
pub mod dx12;
#[cfg(target_os = "macos")]
pub mod metal;
#[cfg(any(target_os = "linux", target_os = "windows"))]
pub mod vulkan;

pub use crate::frame_resources::*;
pub use crate::graphics_hardware_interface::*;
pub use crate::window::*;

pub mod implementation {
	pub const USES_METAL: bool = cfg!(all(target_os = "macos", feature = "metal"));

	#[cfg(all(target_os = "macos", feature = "metal"))]
	pub use metal::Instance;

	#[cfg(all(target_os = "macos", feature = "metal"))]
	pub use metal::Device;

	#[cfg(all(target_os = "macos", feature = "metal"))]
	pub use metal::Frame;

	#[cfg(all(target_os = "macos", feature = "metal"))]
	pub use metal::CommandBufferRecording;

	#[cfg(all(target_os = "macos", feature = "metal"))]
	pub use metal::queue::Queue;

	#[cfg(all(target_os = "macos", feature = "metal"))]
	pub(crate) use metal::buffer::Buffer;

	#[cfg(all(target_os = "macos", feature = "metal"))]
	pub(crate) use metal::image::Image;

	#[cfg(all(target_os = "macos", feature = "metal"))]
	pub(crate) use metal::DescriptorSet;

	#[cfg(all(target_os = "macos", feature = "metal"))]
	pub(crate) use metal::Binding;

	#[cfg(all(target_os = "macos", feature = "metal"))]
	pub(crate) use metal::Synchronizer;

	#[cfg(all(target_os = "macos", feature = "metal"))]
	use crate::metal;

	#[cfg(any(
		target_os = "linux",
		target_os = "windows",
		all(target_os = "macos", not(feature = "metal"))
	))]
	pub use vulkan::Instance;

	#[cfg(any(
		target_os = "linux",
		target_os = "windows",
		all(target_os = "macos", not(feature = "metal"))
	))]
	pub use vulkan::Device;

	#[cfg(any(
		target_os = "linux",
		target_os = "windows",
		all(target_os = "macos", not(feature = "metal"))
	))]
	pub use vulkan::Frame;

	#[cfg(any(
		target_os = "linux",
		target_os = "windows",
		all(target_os = "macos", not(feature = "metal"))
	))]
	pub use vulkan::CommandBufferRecording;

	#[cfg(any(
		target_os = "linux",
		target_os = "windows",
		all(target_os = "macos", not(feature = "metal"))
	))]
	pub use vulkan::queue::Queue;

	#[cfg(any(
		target_os = "linux",
		target_os = "windows",
		all(target_os = "macos", not(feature = "metal"))
	))]
	pub(crate) use vulkan::buffer::Buffer;

	#[cfg(any(
		target_os = "linux",
		target_os = "windows",
		all(target_os = "macos", not(feature = "metal"))
	))]
	pub(crate) use vulkan::image::Image;

	#[cfg(any(
		target_os = "linux",
		target_os = "windows",
		all(target_os = "macos", not(feature = "metal"))
	))]
	pub(crate) use vulkan::descriptor_set::DescriptorSet;

	#[cfg(any(
		target_os = "linux",
		target_os = "windows",
		all(target_os = "macos", not(feature = "metal"))
	))]
	pub(crate) use vulkan::binding::Binding;

	#[cfg(any(
		target_os = "linux",
		target_os = "windows",
		all(target_os = "macos", not(feature = "metal"))
	))]
	use crate::vulkan;
}

pub mod binding;
pub mod buffer;
pub mod command_buffer;
pub mod descriptors;
pub mod device;
pub mod frame;
pub mod image;
pub mod pipelines;
pub mod queue;
pub mod rt;
pub mod sampler;
pub mod shader;
pub mod synchronizer;

pub mod types;
mod utils;

pub use device::Device;
pub use frame::Frame;
pub use queue::Queue;

pub use pipelines::ShaderParameter;

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
