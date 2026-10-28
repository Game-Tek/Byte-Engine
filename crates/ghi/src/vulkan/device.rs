use std::num::NonZeroU32;

use ash::vk::{self, TaggedStructure as _};
use utils::Extent;

use super::{
	DebugCallbackData, MemoryBackedResourceCreationResult, StoredQueue,
	pipeline_binaries::PipelineBinaries,
	utils::{extent_into_vk_extent, image_type_from_extent, into_vk_image_usage_flags, to_format},
};
use crate::{
	graphics_hardware_interface,
	vulkan::{Context, Instance, MAX_SWAPCHAIN_IMAGES},
	window,
};

/// The `Device` struct keeps the selected Vulkan device alive for rendering contexts and resource factories.
///
/// Create a context with [`crate::device::Device::create_context`]. Keep this device
/// alive until its contexts, detached factories, and their resources have been dropped.
pub struct Device {
	pub inner: Option<InnerDevice>,
	device: ash::Device,
	descriptor_heap_properties: vk::PhysicalDeviceDescriptorHeapPropertiesEXT<'static>,
	/// The application-owned pipeline cache, shared with every context and factory of the device.
	pipeline_binaries: Option<std::sync::Arc<PipelineBinaries>>,
	/// `None` unless the device traces rays. See [`RayTracing`].
	ray_tracing: Option<std::sync::Arc<RayTracing>>,
	shaders: Vec<crate::vulkan::Shader>,
	/// The closest hits and ray generation shaders this factory compiled for its ray tracing pipelines, so a pipeline
	/// built again with more hit groups compiles only the new ones.
	ray_tracing_libraries: crate::vulkan::ray_tracing::Libraries,
}

/// The `RayTracing` struct exists so a device that traces rays can build acceleration structures and ray tracing
/// pipelines: it holds the functions of the extensions that do it and the limits they impose.
///
/// A device has one when it was created with [`crate::device::Features::ray_tracing`] and the GPU supports
/// `VK_KHR_acceleration_structure`, `VK_KHR_ray_query`, `VK_KHR_ray_tracing_pipeline`, and `VK_KHR_pipeline_library`.
/// Contexts build structures with it, and contexts and factories link pipelines with it.
pub(crate) struct RayTracing {
	pub(crate) acceleration_structure: ash::khr::acceleration_structure::Device,
	pub(crate) pipeline: ash::khr::ray_tracing_pipeline::Device,
	/// The bytes of one shader group handle, which is one shader binding table record.
	pub(crate) shader_group_handle_size: u32,
	/// The alignment of each record inside a shader binding table region.
	pub(crate) shader_group_handle_alignment: u32,
	/// The alignment of the start of each shader binding table region.
	pub(crate) shader_group_base_alignment: u32,
	/// The most invocations one trace may launch.
	pub(crate) max_ray_dispatch_invocation_count: u32,
	/// The alignment of each build's scratch memory.
	pub(crate) scratch_alignment: u64,
}

/// The `DebugDataRef` struct lets a device read the validation error counter that its [`Instance`] owns.
///
/// The Vulkan debug messenger writes through a pointer to the instance's boxed [`DebugCallbackData`], so the data has a
/// stable address for the instance's whole life. A Vulkan device must never outlive the instance it was created from,
/// so the pointer stays valid for as long as any device or context that holds it.
#[derive(Clone, Copy)]
pub(crate) struct DebugDataRef(std::ptr::NonNull<DebugCallbackData>);

// SAFETY: `DebugCallbackData` only holds an atomic counter and a function pointer, so shared access from any thread is sound.
unsafe impl Send for DebugDataRef {}
// SAFETY: See the `Send` implementation above.
unsafe impl Sync for DebugDataRef {}

impl DebugDataRef {
	pub(crate) fn new(debug_data: &DebugCallbackData) -> Self {
		Self(std::ptr::NonNull::from(debug_data))
	}

	pub(crate) fn get(&self) -> &DebugCallbackData {
		// SAFETY: The instance owns the boxed data and outlives every device created from it. See the type docs.
		unsafe { self.0.as_ref() }
	}
}

#[derive(Clone)]
pub struct InnerDevice {
	pub(super) debug_utils: Option<ash::ext::debug_utils::Device>,

	pub(super) debug_data: DebugDataRef,

	/// Distinct Vulkan queues, indexed by [`StoredQueue::vk_queue_index`]. Vulkan requires external synchronization
	/// for submissions, so each queue has one lock that every context of the device shares.
	pub(super) vk_queues: std::sync::Arc<[std::sync::Mutex<vk::Queue>]>,

	pub(crate) physical_device: vk::PhysicalDevice,
	pub(super) device: ash::Device,
	pub(super) swapchain: ash::khr::swapchain::Device,
	pub(super) surface: ash::khr::surface::Instance,
	pub(super) mesh_shading: ash::ext::mesh_shader::Device,
	pub(super) descriptor_heap: ash::ext::descriptor_heap::Device,
	pub(super) descriptor_heap_properties: vk::PhysicalDeviceDescriptorHeapPropertiesEXT<'static>,
	/// `None` unless the device was created with [`crate::device::Features::pipeline_cache`] and the driver supports
	/// `VK_KHR_pipeline_binary`.
	pub(super) pipeline_binaries: Option<std::sync::Arc<PipelineBinaries>>,
	/// `None` unless the device traces rays. See [`RayTracing`]. It sits behind a pointer, so the device state every
	/// command reads stays as compact as it is without ray tracing.
	pub(crate) ray_tracing: Option<std::sync::Arc<RayTracing>>,
	pub(super) surface_capabilities: ash::khr::get_surface_capabilities2::Instance,
	pub(super) wayland_surface: ash::khr::wayland_surface::Instance,
	pub(super) memory_properties: vk::PhysicalDeviceMemoryProperties,
	/// Nanoseconds per GPU timestamp tick, which converts counter slots into durations.
	pub(super) timestamp_period: f32,
	/// The timestamp bits every used queue family writes; zero means the device cannot time counters.
	pub(super) timestamp_valid_bits: u32,
	pub(super) queues: Vec<StoredQueue>,
	pub(super) settings: crate::device::Features,
	pub(super) swapchain_native_supports_formatless_storage_write: bool,
	pub(super) swapchain_proxy_supports_formatless_storage_write: bool,
}

impl Drop for Device {
	/// Waits for pending device work before destroying the Vulkan device.
	fn drop(&mut self) {
		// Detached factories borrow the native device; only the primary device owns it.
		let Some(inner) = &self.inner else {
			return;
		};
		// Callers must drop all contexts and factories before the primary device.
		inner.wait();
		// Ray tracing libraries are pipelines of the device, so they go before it.
		self.ray_tracing_libraries.clear();
		unsafe { self.device.destroy_device(None) };
	}
}

impl std::ops::Deref for InnerDevice {
	type Target = ash::Device;

	fn deref(&self) -> &Self::Target {
		&self.device
	}
}

/// The `ComputePipeline` struct carries a Vulkan compute pipeline before it has a public GHI handle.
pub struct ComputePipeline {
	pub(crate) pipeline: vk::Pipeline,
	pub(crate) layout: crate::vulkan::PipelineLayout,
}

/// The `RasterPipeline` struct carries a Vulkan raster pipeline that a factory thread compiled, until a frame gives
/// it a public GHI handle.
///
/// Compiling on the factory thread keeps `vkCreateGraphicsPipelines` off the render thread.
pub struct RasterPipeline {
	pub(crate) pipeline: vk::Pipeline,
	pub(crate) layout: crate::vulkan::PipelineLayout,
}

/// Returns `flag` when `condition` holds and no flags otherwise.
fn flag_if<F: Default>(condition: bool, flag: F) -> F {
	if condition { flag } else { F::default() }
}

mod detached_resources;
mod setup;
mod swapchain;
