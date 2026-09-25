use std::{borrow::Cow, num::NonZeroU32, sync::Arc};

use ash::vk::{self, TaggedStructure as _};
use utils::{Extent, hash::HashMap};

use super::{
	DebugCallbackData, MemoryBackedResourceCreationResult, StoredQueue,
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
	shaders: Vec<crate::vulkan::Shader>,
}

#[derive(Clone)]
pub struct InnerDevice {
	pub(super) debug_utils: Option<ash::ext::debug_utils::Device>,

	debug_data: Arc<DebugCallbackData>,

	pub(crate) physical_device: vk::PhysicalDevice,
	pub(super) device: ash::Device,
	pub(super) swapchain: ash::khr::swapchain::Device,
	pub(super) surface: ash::khr::surface::Instance,
	pub(super) acceleration_structure: ash::khr::acceleration_structure::Device,
	pub(super) ray_tracing_pipeline: ash::khr::ray_tracing_pipeline::Device,
	pub(super) mesh_shading: ash::ext::mesh_shader::Device,
	pub(super) descriptor_heap: ash::ext::descriptor_heap::Device,
	pub(super) descriptor_heap_properties: vk::PhysicalDeviceDescriptorHeapPropertiesEXT<'static>,
	pub(super) surface_capabilities: ash::khr::get_surface_capabilities2::Instance,
	pub(super) wayland_surface: ash::khr::wayland_surface::Instance,
	pub(super) memory_properties: vk::PhysicalDeviceMemoryProperties,
	pub(super) queues: Vec<StoredQueue>,
	pub(super) settings: crate::device::Features,
	pub(super) swapchain_native_supports_formatless_storage_write: bool,
	pub(super) swapchain_proxy_supports_formatless_storage_write: bool,
}

impl Drop for Device {
	/// Waits for pending device work before destroying the Vulkan device.
	fn drop(&mut self) {
		// Detached factories borrow the native device; only the primary device owns it.
		if self.inner.is_none() {
			return;
		}
		// Callers must drop all contexts and factories before the primary device.
		unsafe {
			self.device.device_wait_idle().expect(
				"Failed to wait for the Vulkan device during destruction. The most likely cause is that the device was lost.",
			);
			self.device.destroy_device(None);
		}
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
	pub(crate) shader_handles: HashMap<graphics_hardware_interface::ShaderHandle, [u8; 32]>,
}

/// The `RasterPipeline` struct carries detached Vulkan raster state until a frame interns it.
pub struct RasterPipeline {
	pub(crate) name: Option<String>,
	pub(crate) push_constant_ranges: Vec<crate::pipelines::PushConstantRange>,
	pub(crate) vertex_elements: Vec<FactoryVertexElement>,
	pub(crate) shaders: Vec<FactoryShaderParameter>,
	pub(crate) render_targets: Vec<crate::pipelines::raster::AttachmentDescriptor>,
	pub(crate) face_winding: crate::pipelines::raster::FaceWinding,
	pub(crate) cull_mode: crate::pipelines::raster::CullMode,
	pub(crate) fill_mode: crate::pipelines::raster::FillMode,
	pub(crate) depth_write: bool,
	pub(crate) factory_shaders: Vec<crate::vulkan::Shader>,
}

/// The `FactoryVertexElement` struct owns vertex input metadata used by a detached Vulkan raster pipeline.
pub(crate) struct FactoryVertexElement {
	pub(crate) name: String,
	pub(crate) format: crate::DataTypes,
	pub(crate) binding: u32,
}

/// The `FactoryShaderParameter` struct owns shader selection data used by a detached Vulkan raster pipeline.
pub(crate) struct FactoryShaderParameter {
	pub(crate) handle_index: usize,
	pub(crate) stage: crate::ShaderTypes,
	pub(crate) specialization_map: Vec<crate::pipelines::SpecializationMapEntry>,
}

/// The `FactoryImage` struct carries Vulkan image parameters until a context interns them.
pub struct FactoryImage {
	pub(crate) name: Option<String>,
	pub(crate) extent: Extent,
	pub(crate) format: crate::Formats,
	pub(crate) resource_uses: crate::Uses,
	pub(crate) device_accesses: crate::DeviceAccesses,
	pub(crate) use_case: crate::UseCases,
	pub(crate) array_layers: Option<NonZeroU32>,
	pub(crate) cube_compatible: bool,
	pub(crate) cube_array_compatible: bool,
}

/// The `FactorySampler` struct carries Vulkan sampler parameters until a context interns them.
pub struct FactorySampler {
	pub(crate) filtering_mode: crate::FilteringModes,
	pub(crate) reduction_mode: crate::SamplingReductionModes,
	pub(crate) mip_map_mode: crate::FilteringModes,
	pub(crate) addressing_mode: crate::SamplerAddressingModes,
	pub(crate) anisotropy: Option<f32>,
	pub(crate) min_lod: f32,
	pub(crate) max_lod: f32,
}

/// Returns `flag` when `condition` holds and no flags otherwise.
fn flag_if<F: Default>(condition: bool, flag: F) -> F {
	if condition { flag } else { F::default() }
}

mod detached_resources;
mod setup;
mod swapchain;
