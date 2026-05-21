#[link(name = "CoreGraphics", kind = "framework")]
extern "C" {}

use std::sync::atomic::AtomicU64;

use ::utils::hash::HashMap;
use ::utils::Extent;
use objc2::rc::Retained;
use objc2::runtime::ProtocolObject;
use objc2_app_kit::NSView;
use objc2_foundation::NSSize;
use objc2_metal as mtl;
use objc2_quartz_core::{CAMetalDrawable, CAMetalLayer};

use crate::binding::DescriptorSetBindingHandle;
use crate::buffer::BufferHandle;
use crate::graphics_hardware_interface;
use crate::image::ImageHandle;
use crate::sampler::SamplerHandle;
use crate::PrivateHandles;

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub(super) enum Descriptor {
	Image {
		image: ImageHandle,
		layout: crate::Layouts,
	},
	CombinedImageSampler {
		image: ImageHandle,
		sampler: SamplerHandle,
		layout: crate::Layouts,
	},
	Buffer {
		buffer: BufferHandle,
		size: graphics_hardware_interface::Ranges,
	},
	Sampler {
		sampler: SamplerHandle,
	},
	Swapchain {
		handle: crate::swapchain::SwapchainHandle,
	},
}

impl Descriptor {
	pub(super) fn tracked_resource(self) -> Option<PrivateHandles> {
		match self {
			Descriptor::Buffer { buffer, .. } => Some(PrivateHandles::Buffer(buffer)),
			Descriptor::Image { image, .. } => Some(PrivateHandles::Image(image)),
			Descriptor::CombinedImageSampler { image, .. } => Some(PrivateHandles::Image(image)),
			Descriptor::Sampler { .. } => None,
			Descriptor::Swapchain { handle } => Some(PrivateHandles::Swapchain(handle)),
		}
	}
}

#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub(super) struct TopLevelAccelerationStructureHandle(pub(super) u64);

#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub(super) struct BottomLevelAccelerationStructureHandle(pub(super) u64);

#[derive(Clone, PartialEq)]
pub(super) struct Consumption {
	pub(super) handle: PrivateHandles,
	pub(super) stages: crate::Stages,
	pub(super) access: crate::AccessPolicies,
	pub(super) layout: crate::Layouts,
}

#[derive(Clone, PartialEq)]
pub(super) struct MetalConsumption {
	pub(super) handle: PrivateHandles,
	pub(super) stages: crate::Stages,
	pub(super) access: crate::AccessPolicies,
	pub(super) layout: crate::Layouts,
}

const MAX_FRAMES_IN_FLIGHT: usize = 3;
const MAX_SWAPCHAIN_IMAGES: usize = 8;

/// Returns the current/old drawable size, the new drawable size, and the scale factor.
fn get_layer_sizes(layer: &CAMetalLayer, view: &NSView) -> (NSSize, NSSize, f64) {
	let logical_size = view.frame().size;
	let drawable_size = view.convertSizeToBacking(logical_size);
	let scale_factor = if logical_size.width > 0.0 {
		(drawable_size.width / logical_size.width).max(1.0)
	} else if logical_size.height > 0.0 {
		(drawable_size.height / logical_size.height).max(1.0)
	} else {
		1.0
	};

	let current_size = layer.drawableSize();
	let new_size = NSSize::new(drawable_size.width, drawable_size.height);

	(current_size, new_size, scale_factor)
}

fn get_layer_extent(layer: &CAMetalLayer, view: &NSView) -> Extent {
	let (_, new_size, _) = get_layer_sizes(layer, view);

	Extent::rectangle(
		new_size.width.round().max(0.0) as u32,
		new_size.height.round().max(0.0) as u32,
	)
}

/// Updates the CAMetalLayer's drawable size to match the view's backing size, but only when
/// the size has actually changed. Calling `setDrawableSize` unconditionally invalidates the
/// layer's drawable pool, forcing Metal to allocate new drawables every frame.
fn update_layer_extent(layer: &CAMetalLayer, view: &NSView) -> Extent {
	let (current_size, new_size, scale_factor) = get_layer_sizes(layer, view);

	if (current_size.width - new_size.width).abs() > 0.5 || (current_size.height - new_size.height).abs() > 0.5 {
		layer.setContentsScale(scale_factor);
		layer.setDrawableSize(new_size);
	}

	Extent::rectangle(
		new_size.width.round().max(0.0) as u32,
		new_size.height.round().max(0.0) as u32,
	)
}

#[derive(Clone)]
pub(crate) struct DescriptorSetLayout {
	bindings: Vec<DescriptorSetLayoutBinding>,
	argument_encoder: Retained<ProtocolObject<dyn mtl::MTLArgumentEncoder>>,
	encoded_length: usize,
}

#[derive(Clone)]
pub(crate) struct DescriptorSetLayoutBinding {
	binding: u32,
	descriptor_type: crate::descriptors::DescriptorType,
	descriptor_count: u32,
	stages: crate::Stages,
	immutable_samplers: Option<Vec<graphics_hardware_interface::SamplerHandle>>,
	argument_slots: ArgumentBindingSlots,
}

#[derive(Clone)]
pub(crate) enum ArgumentBindingSlots {
	Buffer(Vec<u32>),
	Texture(Vec<u32>),
	Sampler(Vec<u32>),
	CombinedImageSampler { textures: Vec<u32>, samplers: Vec<u32> },
}

impl DescriptorSetLayout {
	pub(crate) fn binding(&self, binding: u32) -> Option<&DescriptorSetLayoutBinding> {
		self.bindings.iter().find(|layout_binding| layout_binding.binding == binding)
	}
}

impl DescriptorSetLayoutBinding {
	pub(crate) fn slot_for_array_element(&self, array_element: u32) -> DescriptorBindingSlot {
		let index = array_element as usize;

		match &self.argument_slots {
			ArgumentBindingSlots::Buffer(indices) => DescriptorBindingSlot::Buffer(indices[index]),
			ArgumentBindingSlots::Texture(indices) => DescriptorBindingSlot::Texture(indices[index]),
			ArgumentBindingSlots::Sampler(indices) => DescriptorBindingSlot::Sampler(indices[index]),
			ArgumentBindingSlots::CombinedImageSampler { textures, samplers } => DescriptorBindingSlot::CombinedImageSampler {
				texture: textures[index],
				sampler: samplers[index],
			},
		}
	}
}

#[derive(Clone, Copy)]
pub(crate) enum DescriptorBindingSlot {
	Buffer(u32),
	Texture(u32),
	Sampler(u32),
	CombinedImageSampler { texture: u32, sampler: u32 },
}

#[derive(Clone, PartialEq, Eq)]
pub(crate) struct PipelineLayout {
	descriptor_set_template_indices: HashMap<graphics_hardware_interface::DescriptorSetTemplateHandle, u32>,
	push_constant_ranges: Vec<crate::pipelines::PushConstantRange>,
	push_constant_size: usize,
}

#[derive(Clone, PartialEq, Eq, Hash)]
struct PipelineLayoutKey {
	descriptor_set_templates: Vec<graphics_hardware_interface::DescriptorSetTemplateHandle>,
	push_constant_ranges: Vec<crate::pipelines::PushConstantRange>,
}

#[derive(Clone)]
pub(crate) struct VertexLayout {
	elements: Vec<VertexElementDescriptor>,
	strides: Vec<u32>,
	vertex_descriptor: Retained<mtl::MTLVertexDescriptor>,
}

#[derive(Clone, PartialEq, Eq, Hash)]
struct VertexLayoutKey {
	elements: Vec<VertexElementDescriptor>,
}

#[derive(Clone, PartialEq, Eq, Hash)]
pub(crate) struct VertexElementDescriptor {
	name: String,
	format: crate::DataTypes,
	binding: u32,
}

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub(crate) struct VertexLayoutHandle(pub(crate) u64);

#[derive(Clone)]
pub(crate) struct Shader {
	stage: crate::Stages,
	shader_binding_descriptors: Vec<crate::shader::BindingDescriptor>,
	metal_library: Option<Retained<ProtocolObject<dyn mtl::MTLLibrary>>>,
	metal_entry_point: Option<String>,
	spirv: Option<Vec<u8>>,
	threadgroup_size: Option<Extent>,
}

#[derive(Clone)]
pub(crate) struct Pipeline {
	pipeline: PipelineState,
	depth_stencil_state: Option<Retained<ProtocolObject<dyn mtl::MTLDepthStencilState>>>,
	layout: graphics_hardware_interface::PipelineLayoutHandle,
	vertex_layout: Option<VertexLayoutHandle>,
	shader_handles: HashMap<graphics_hardware_interface::ShaderHandle, [u8; 32]>,
	resource_access: Vec<((u32, u32), (crate::Stages, crate::AccessPolicies))>,
	compute_threadgroup_size: Option<Extent>,
	object_threadgroup_size: Option<Extent>,
	mesh_threadgroup_size: Option<Extent>,
	face_winding: crate::pipelines::raster::FaceWinding,
	cull_mode: crate::pipelines::raster::CullMode,
}

#[derive(Clone)]
pub(crate) enum PipelineState {
	Raster(Option<Retained<ProtocolObject<dyn mtl::MTLRenderPipelineState>>>),
	Compute(Option<Retained<ProtocolObject<dyn mtl::MTLComputePipelineState>>>),
	RayTracing,
}

#[derive(Clone)]
pub(super) struct CommandBufferInternal {
	queue: Retained<ProtocolObject<dyn mtl::MTLCommandQueue>>,
	command_buffer: Retained<ProtocolObject<dyn mtl::MTLCommandBuffer>>,
}

#[derive(Clone)]
pub(crate) struct StoredCommandBuffer {
	queue_handle: graphics_hardware_interface::QueueHandle,
	name: Option<String>,
}

pub struct CommandBuffer<'a> {
	pub(crate) device: &'a mut context::Context,
	pub(crate) command_buffer_handle: graphics_hardware_interface::CommandBufferHandle,
}

#[derive(Clone)]
pub(crate) struct Allocation {
	buffer: Retained<ProtocolObject<dyn mtl::MTLBuffer>>,
	pointer: *mut u8,
	size: usize,
}

pub(crate) struct DebugCallbackData {
	error_count: AtomicU64,
	error_log_function: fn(&str),
}

#[derive(PartialEq, Eq, Clone, Copy)]
pub(crate) struct TransitionState {
	layout: crate::Layouts,
}

pub(crate) struct Mesh {
	vertex_buffers: Vec<Option<Retained<ProtocolObject<dyn mtl::MTLBuffer>>>>,
	index_buffer: Retained<ProtocolObject<dyn mtl::MTLBuffer>>,
	vertex_count: u32,
	index_count: u32,
	vertex_size: usize,
}

pub(crate) struct AccelerationStructure {
	structure: Option<Retained<ProtocolObject<dyn mtl::MTLAccelerationStructure>>>,
	buffer: Option<Retained<ProtocolObject<dyn mtl::MTLBuffer>>>,
}

#[derive(Clone, Copy)]
/// The `MemoryBackedResourceCreationResult` struct stores the information of a memory backed resource.
pub struct MemoryBackedResourceCreationResult<T> {
	/// The resource.
	resource: T,
	/// The final size of the resource.
	size: usize,
	/// The memory flags that need used to create the resource.
	memory_flags: u32,
}

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
struct BuildImage {
	previous: ImageHandle,
	master: graphics_hardware_interface::ImageHandle,
}

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
struct BuildBuffer {
	previous: BufferHandle,
	master: graphics_hardware_interface::BaseBufferHandle,
}

#[derive(Debug, Clone, Copy, Hash, PartialEq, Eq)]
pub(crate) enum Tasks {
	/// Delete a Metal texture. Will be associated to a frame index in `Task`.
	DeleteMetalTexture {
		handle: ImageHandle,
	},
	/// Delete a Metal buffer. Will be associated to a frame index in `Task`.
	DeleteMetalBuffer {
		handle: BufferHandle,
	},
	/// Patch all descriptors that reference the buffer.
	/// Usually, this is done when the buffer is resized because the Metal buffer will be swapped.
	UpdateBufferDescriptors {
		handle: BufferHandle,
	},
	/// Patch all descriptors that reference the image.
	/// Usually, this is done when the image is resized because the Metal texture will be swapped.
	UpdateImageDescriptors {
		handle: ImageHandle,
	},
	/// Resize an image.
	ResizeImage {
		handle: ImageHandle,
		extent: Extent,
	},
	/// Update the frame-specific (specified in `Task`) descriptor with the given write information for the master resource and descriptor.
	UpdateDescriptor {
		descriptor_write: crate::descriptors::Write,
	},
	/// Update a particular descriptor.
	/// This task will most likely be enqueued for performance reasons. Since it is cheaper to update multiple descriptors at once instead of sporadically.
	WriteDescriptor {
		binding_handle: DescriptorSetBindingHandle,
		descriptor: Descriptors,
	},
	BuildImage(BuildImage),
	BuildBuffer(BuildBuffer),
}

#[derive(Debug, Clone, PartialEq)]
/// The `Task` struct represents a deferred task that needs to be executed at a later time.
/// This is because some tasks need to be executed at a particular time or frame.
pub(crate) struct Task {
	pub(crate) task: Tasks,
	pub(crate) frame: Option<u8>,
}

impl Task {
	pub(crate) fn new(task: Tasks, frame: Option<u8>) -> Self {
		Self { task, frame }
	}

	pub(crate) fn delete_metal_texture(handle: ImageHandle, frame: u8) -> Self {
		Self {
			task: Tasks::DeleteMetalTexture { handle },
			frame: Some(frame),
		}
	}

	pub(crate) fn delete_metal_buffer(handle: BufferHandle, frame: Option<u8>) -> Self {
		Self {
			task: Tasks::DeleteMetalBuffer { handle },
			frame,
		}
	}

	pub(crate) fn update_buffer_descriptor(handle: BufferHandle, frame: Option<u8>) -> Self {
		Self {
			task: Tasks::UpdateBufferDescriptors { handle },
			frame,
		}
	}

	pub(crate) fn update_image_descriptor(handle: ImageHandle, frame: Option<u8>) -> Self {
		Self {
			task: Tasks::UpdateImageDescriptors { handle },
			frame,
		}
	}

	pub(crate) fn update_resource_descriptor(descriptor_write: crate::descriptors::Write, frame: Option<u8>) -> Self {
		Self {
			task: Tasks::UpdateDescriptor { descriptor_write },
			frame,
		}
	}

	pub(crate) fn frame(&self) -> Option<u8> {
		self.frame
	}

	pub(crate) fn task(&self) -> &Tasks {
		&self.task
	}

	pub(crate) fn into_task(self) -> Tasks {
		self.task
	}

	pub(crate) fn write_descriptor(
		binding_handle: DescriptorSetBindingHandle,
		descriptor: Descriptors,
		frame: Option<u8>,
	) -> Task {
		Self {
			task: Tasks::WriteDescriptor {
				binding_handle,
				descriptor,
			},
			frame,
		}
	}
}

#[derive(Clone, Copy, PartialEq, Eq, Debug, Hash)]
enum Descriptors {
	Buffer {
		handle: BufferHandle,
		size: graphics_hardware_interface::Ranges,
	},
	Image {
		handle: ImageHandle,
		layout: crate::Layouts,
	},
	CombinedImageSampler {
		image_handle: ImageHandle,
		layout: crate::Layouts,
		sampler_handle: SamplerHandle,
		layer: Option<u32>,
	},
	Sampler {
		handle: SamplerHandle,
	},
	CombinedImageSamplerArray,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) struct DescriptorWrite {
	pub(crate) write: Descriptors,
	pub(crate) binding: DescriptorSetBindingHandle,
	pub(crate) array_element: u32,
}

impl DescriptorWrite {
	pub(crate) fn new(write: Descriptors, binding: DescriptorSetBindingHandle) -> Self {
		Self {
			write,
			binding,
			array_element: 0,
		}
	}

	pub(crate) fn index(mut self, index: u32) -> Self {
		self.array_element = index;
		self
	}
}

mod utils {
	use objc2_metal as mtl;
	use utils::Extent;

	use crate::{DeviceAccesses, FilteringModes, Formats, SamplerAddressingModes, Uses};

	pub(crate) fn parse_threadgroup_size_metadata(source: &str) -> Option<Extent> {
		let metadata_prefix = "// besl-threadgroup-size:";
		let metadata = source.lines().find_map(|line| line.trim().strip_prefix(metadata_prefix))?;
		let mut extents = metadata.split(',').map(|value| value.trim().parse::<u32>().ok());

		Some(Extent::new(extents.next()??, extents.next()??, extents.next()??))
	}

	pub(crate) fn to_pixel_format(format: Formats) -> mtl::MTLPixelFormat {
		match format {
			Formats::R8UNORM => mtl::MTLPixelFormat::R8Unorm,
			Formats::R8SNORM => mtl::MTLPixelFormat::R8Snorm,
			Formats::R8F => mtl::MTLPixelFormat::R8Unorm,
			Formats::R8sRGB => mtl::MTLPixelFormat::R8Unorm,

			Formats::R16F => mtl::MTLPixelFormat::R16Float,
			Formats::R16UNORM => mtl::MTLPixelFormat::R16Unorm,
			Formats::R16SNORM => mtl::MTLPixelFormat::R16Snorm,
			Formats::R16sRGB => mtl::MTLPixelFormat::R16Unorm,

			Formats::R32F => mtl::MTLPixelFormat::R32Float,
			Formats::R32UNORM => mtl::MTLPixelFormat::R32Uint,
			Formats::R32SNORM => mtl::MTLPixelFormat::R32Sint,
			Formats::R32sRGB => mtl::MTLPixelFormat::R32Uint,

			Formats::RG8UNORM => mtl::MTLPixelFormat::RG8Unorm,
			Formats::RG8SNORM => mtl::MTLPixelFormat::RG8Snorm,
			Formats::RG8F => mtl::MTLPixelFormat::RG8Unorm,
			Formats::RG8sRGB => mtl::MTLPixelFormat::RG8Unorm,

			Formats::RG16F => mtl::MTLPixelFormat::RG16Float,
			Formats::RG16UNORM => mtl::MTLPixelFormat::RG16Unorm,
			Formats::RG16SNORM => mtl::MTLPixelFormat::RG16Snorm,
			Formats::RG16sRGB => mtl::MTLPixelFormat::RG16Unorm,

			Formats::RGB8UNORM => mtl::MTLPixelFormat::RGBA8Unorm,
			Formats::RGB8SNORM => mtl::MTLPixelFormat::RGBA8Snorm,
			Formats::RGB8F => mtl::MTLPixelFormat::RGBA8Unorm,
			Formats::RGB8sRGB => mtl::MTLPixelFormat::RGBA8Unorm_sRGB,

			Formats::RGB16F => mtl::MTLPixelFormat::RGBA16Float,
			Formats::RGB16UNORM => mtl::MTLPixelFormat::RGBA16Unorm,
			Formats::RGB16SNORM => mtl::MTLPixelFormat::RGBA16Snorm,
			Formats::RGB16sRGB => mtl::MTLPixelFormat::RGBA16Unorm,

			Formats::RGBA8UNORM => mtl::MTLPixelFormat::RGBA8Unorm,
			Formats::RGBA8SNORM => mtl::MTLPixelFormat::RGBA8Snorm,
			Formats::RGBA8F => mtl::MTLPixelFormat::RGBA8Unorm,
			Formats::RGBA8sRGB => mtl::MTLPixelFormat::RGBA8Unorm_sRGB,

			Formats::RGBA16F => mtl::MTLPixelFormat::RGBA16Float,
			Formats::RGBA16UNORM => mtl::MTLPixelFormat::RGBA16Unorm,
			Formats::RGBA16SNORM => mtl::MTLPixelFormat::RGBA16Snorm,
			Formats::RGBA16sRGB => mtl::MTLPixelFormat::RGBA16Unorm,

			Formats::RGBu11u11u10 => mtl::MTLPixelFormat::RG11B10Float,
			Formats::BGRAu8 => mtl::MTLPixelFormat::BGRA8Unorm,
			Formats::BGRAsRGB => mtl::MTLPixelFormat::BGRA8Unorm_sRGB,
			Formats::Depth32 => mtl::MTLPixelFormat::Depth32Float,
			Formats::U32 => mtl::MTLPixelFormat::R32Uint,

			Formats::BC5 => mtl::MTLPixelFormat::BC5_RGUnorm,
			Formats::BC7 => mtl::MTLPixelFormat::BC7_RGBAUnorm,
			Formats::BC7SRGB => mtl::MTLPixelFormat::BC7_RGBAUnorm_sRGB,
		}
	}

	pub(crate) fn storage_mode_from_access(access: DeviceAccesses) -> mtl::MTLStorageMode {
		if access == DeviceAccesses::DeviceOnly {
			mtl::MTLStorageMode::Private
		} else if access.contains(DeviceAccesses::CpuRead) {
			mtl::MTLStorageMode::Managed
		} else {
			mtl::MTLStorageMode::Shared
		}
	}

	pub(crate) fn resource_options_from_access(access: DeviceAccesses) -> mtl::MTLResourceOptions {
		match storage_mode_from_access(access) {
			mtl::MTLStorageMode::Private => mtl::MTLResourceOptions::StorageModePrivate,
			mtl::MTLStorageMode::Managed => mtl::MTLResourceOptions::StorageModeManaged,
			_ => mtl::MTLResourceOptions::StorageModeShared,
		}
	}

	pub(crate) fn texture_usage_from_uses(uses: Uses) -> mtl::MTLTextureUsage {
		let mut usage = mtl::MTLTextureUsage::empty();

		if uses.intersects(Uses::Image | Uses::Storage | Uses::ShaderBindingTable) {
			usage |= mtl::MTLTextureUsage::ShaderRead;
		}

		if uses.contains(Uses::Storage) {
			usage |= mtl::MTLTextureUsage::ShaderWrite;
		}

		if uses.intersects(Uses::RenderTarget | Uses::DepthStencil) {
			usage |= mtl::MTLTextureUsage::RenderTarget;
		}

		usage
	}

	pub(crate) fn sampler_min_mag_filter(filter: FilteringModes) -> mtl::MTLSamplerMinMagFilter {
		match filter {
			FilteringModes::Closest => mtl::MTLSamplerMinMagFilter::Nearest,
			FilteringModes::Linear => mtl::MTLSamplerMinMagFilter::Linear,
		}
	}

	pub(crate) fn sampler_mip_filter(filter: FilteringModes) -> mtl::MTLSamplerMipFilter {
		match filter {
			FilteringModes::Closest => mtl::MTLSamplerMipFilter::Nearest,
			FilteringModes::Linear => mtl::MTLSamplerMipFilter::Linear,
		}
	}

	pub(crate) fn sampler_address_mode(mode: SamplerAddressingModes) -> mtl::MTLSamplerAddressMode {
		match mode {
			SamplerAddressingModes::Repeat => mtl::MTLSamplerAddressMode::Repeat,
			SamplerAddressingModes::Mirror => mtl::MTLSamplerAddressMode::MirrorRepeat,
			SamplerAddressingModes::Clamp => mtl::MTLSamplerAddressMode::ClampToEdge,
			SamplerAddressingModes::Border { .. } => mtl::MTLSamplerAddressMode::ClampToBorderColor,
		}
	}

	pub(crate) fn bytes_per_pixel(format: Formats) -> Option<usize> {
		let channel_bytes = match format.channel_bit_size() {
			crate::ChannelBitSize::Bits8 => 1,
			crate::ChannelBitSize::Bits16 => 2,
			crate::ChannelBitSize::Bits32 => 4,
			crate::ChannelBitSize::Bits11_11_10 => 4,
			crate::ChannelBitSize::Compressed => return None,
		};

		let channels = match format.channel_layout() {
			crate::ChannelLayout::R => 1,
			crate::ChannelLayout::RG => 2,
			crate::ChannelLayout::RGB => 3,
			crate::ChannelLayout::RGBA => 4,
			crate::ChannelLayout::BGRA => 4,
			crate::ChannelLayout::Depth => 1,
			crate::ChannelLayout::Packed => 1,
			crate::ChannelLayout::BC => return None,
		};

		Some(channel_bytes * channels)
	}

	pub(crate) fn texture_upload_layout(format: Formats, extent: Extent) -> Option<(usize, usize, usize)> {
		let width = extent.width().max(1) as usize;
		let height = extent.height().max(1) as usize;

		if let Some(layout) = format.bc_layout(width as u32, height as u32) {
			Some((
				layout.bytes_per_row as usize,
				layout.blocks_h as usize,
				layout.bytes_per_image as usize,
			))
		} else {
			let bytes_per_pixel = bytes_per_pixel(format)?;
			let bytes_per_row = width * bytes_per_pixel;
			Some((bytes_per_row, height, bytes_per_row * height))
		}
	}

	pub(crate) fn texture_copy_size(_format: Formats, extent: Extent) -> mtl::MTLSize {
		mtl::MTLSize {
			width: extent.width().max(1) as _,
			height: extent.height().max(1) as _,
			depth: extent.depth().max(1) as _,
		}
	}

	pub(crate) fn is_block_compressed(format: Formats) -> bool {
		format.bc_bytes_per_block().is_some()
	}

	#[cfg(debug_assertions)]
	pub(crate) fn debug_compressed_upload(
		format: Formats,
		mip_index: usize,
		slice_index: usize,
		extent: Extent,
		bytes_per_row: usize,
		bytes_per_image: usize,
		source_offset: usize,
	) {
		let Some(layout) = format.bc_layout(extent.width(), extent.height()) else {
			return;
		};
		let expected_next_offset = source_offset + bytes_per_image;

		eprintln!(
			"Metal compressed texture upload: format={format:?}, mip={mip_index}, slice={slice_index}, width={}, height={}, blocks_w={}, blocks_h={}, bytes_per_block={}, bytes_per_row={bytes_per_row}, bytes_per_image={bytes_per_image}, compact_bytes_per_row={}, compact_bytes_per_image={}, source_offset={source_offset}, expected_next_offset={expected_next_offset}",
			extent.width().max(1),
			extent.height().max(1),
			layout.blocks_w,
			layout.blocks_h,
			layout.bytes_per_block,
			layout.bytes_per_row,
			layout.bytes_per_image,
		);
	}

	#[cfg(not(debug_assertions))]
	pub(crate) fn debug_compressed_upload(
		_format: Formats,
		_mip_index: usize,
		_slice_index: usize,
		_extent: Extent,
		_bytes_per_row: usize,
		_bytes_per_image: usize,
		_source_offset: usize,
	) {
	}

	#[cfg(test)]
	mod tests {
		use super::*;

		#[test]
		fn bc_upload_layout_uses_block_rows_for_non_multiple_of_four_extent() {
			let extent = Extent::rectangle(5, 7);

			let (bytes_per_row, row_count, bytes_per_image) = texture_upload_layout(Formats::BC7, extent).unwrap();

			assert_eq!(bytes_per_row, 2 * 16);
			assert_eq!(row_count, 2);
			assert_eq!(bytes_per_image, 2 * 2 * 16);
		}

		#[test]
		fn bc_copy_size_uses_texel_extent_not_padded_block_extent() {
			let size = texture_copy_size(Formats::BC7, Extent::rectangle(5, 7));

			assert_eq!(size.width, 5);
			assert_eq!(size.height, 7);
			assert_eq!(size.depth, 1);
		}

		#[test]
		fn bc_format_mapping_preserves_linear_and_srgb_variants() {
			assert_eq!(to_pixel_format(Formats::BC5), mtl::MTLPixelFormat::BC5_RGUnorm);
			assert_eq!(to_pixel_format(Formats::BC7), mtl::MTLPixelFormat::BC7_RGBAUnorm);
			assert_eq!(to_pixel_format(Formats::BC7SRGB), mtl::MTLPixelFormat::BC7_RGBAUnorm_sRGB);
		}
	}

	pub(crate) fn data_type_size(format: crate::DataTypes) -> usize {
		match format {
			crate::DataTypes::Float => std::mem::size_of::<f32>(),
			crate::DataTypes::Float2 => std::mem::size_of::<f32>() * 2,
			crate::DataTypes::Float3 => std::mem::size_of::<f32>() * 3,
			crate::DataTypes::Float4 => std::mem::size_of::<f32>() * 4,
			crate::DataTypes::U8 => std::mem::size_of::<u8>(),
			crate::DataTypes::U16 => std::mem::size_of::<u16>(),
			crate::DataTypes::U32 => std::mem::size_of::<u32>(),
			crate::DataTypes::Int => std::mem::size_of::<i32>(),
			crate::DataTypes::Int2 => std::mem::size_of::<i32>() * 2,
			crate::DataTypes::Int3 => std::mem::size_of::<i32>() * 3,
			crate::DataTypes::Int4 => std::mem::size_of::<i32>() * 4,
			crate::DataTypes::UInt => std::mem::size_of::<u32>(),
			crate::DataTypes::UInt2 => std::mem::size_of::<u32>() * 2,
			crate::DataTypes::UInt3 => std::mem::size_of::<u32>() * 3,
			crate::DataTypes::UInt4 => std::mem::size_of::<u32>() * 4,
		}
	}

	pub(crate) fn vertex_format(format: crate::DataTypes) -> mtl::MTLVertexFormat {
		match format {
			crate::DataTypes::Float => mtl::MTLVertexFormat::Float,
			crate::DataTypes::Float2 => mtl::MTLVertexFormat::Float2,
			crate::DataTypes::Float3 => mtl::MTLVertexFormat::Float3,
			crate::DataTypes::Float4 => mtl::MTLVertexFormat::Float4,
			crate::DataTypes::U8 => mtl::MTLVertexFormat::UChar,
			crate::DataTypes::U16 => mtl::MTLVertexFormat::UShort,
			crate::DataTypes::U32 | crate::DataTypes::UInt => mtl::MTLVertexFormat::UInt,
			crate::DataTypes::Int => mtl::MTLVertexFormat::Int,
			crate::DataTypes::Int2 => mtl::MTLVertexFormat::Int2,
			crate::DataTypes::Int3 => mtl::MTLVertexFormat::Int3,
			crate::DataTypes::Int4 => mtl::MTLVertexFormat::Int4,
			crate::DataTypes::UInt2 => mtl::MTLVertexFormat::UInt2,
			crate::DataTypes::UInt3 => mtl::MTLVertexFormat::UInt3,
			crate::DataTypes::UInt4 => mtl::MTLVertexFormat::UInt4,
		}
	}

	pub(crate) fn vertex_layout_size(layout: &[crate::pipelines::VertexElement<'_>]) -> usize {
		layout.iter().map(|element| data_type_size(element.format)).sum()
	}

	pub(crate) fn load_action(load: bool) -> mtl::MTLLoadAction {
		if load {
			mtl::MTLLoadAction::Load
		} else {
			mtl::MTLLoadAction::Clear
		}
	}

	pub(crate) fn store_action(store: bool) -> mtl::MTLStoreAction {
		if store {
			mtl::MTLStoreAction::Store
		} else {
			mtl::MTLStoreAction::DontCare
		}
	}

	pub(crate) fn clear_color(clear: crate::ClearValue) -> mtl::MTLClearColor {
		match clear {
			crate::ClearValue::None => mtl::MTLClearColor {
				red: 0.0,
				green: 0.0,
				blue: 0.0,
				alpha: 0.0,
			},
			crate::ClearValue::Color(color) => mtl::MTLClearColor {
				red: color.r as f64,
				green: color.g as f64,
				blue: color.b as f64,
				alpha: color.a as f64,
			},
			crate::ClearValue::Integer(r, g, b, a) => mtl::MTLClearColor {
				red: r as f64,
				green: g as f64,
				blue: b as f64,
				alpha: a as f64,
			},
			crate::ClearValue::Depth(depth) => mtl::MTLClearColor {
				red: depth as f64,
				green: 0.0,
				blue: 0.0,
				alpha: 0.0,
			},
		}
	}

	pub(crate) fn clear_depth(clear: crate::ClearValue) -> std::os::raw::c_double {
		match clear {
			crate::ClearValue::Depth(depth) => depth as _,
			_ => 0.0,
		}
	}

	pub(crate) fn winding(winding: crate::pipelines::raster::FaceWinding) -> mtl::MTLWinding {
		match winding {
			crate::pipelines::raster::FaceWinding::Clockwise => mtl::MTLWinding::Clockwise,
			crate::pipelines::raster::FaceWinding::CounterClockwise => mtl::MTLWinding::CounterClockwise,
		}
	}

	pub(crate) fn cull_mode(cull_mode: crate::pipelines::raster::CullMode) -> mtl::MTLCullMode {
		match cull_mode {
			crate::pipelines::raster::CullMode::None => mtl::MTLCullMode::None,
			crate::pipelines::raster::CullMode::Front => mtl::MTLCullMode::Front,
			crate::pipelines::raster::CullMode::Back => mtl::MTLCullMode::Back,
		}
	}
}

pub mod queue {
	use super::*;

	#[derive(Clone)]
	pub(crate) struct StoredQueue {
		pub(crate) queue: Retained<ProtocolObject<dyn mtl::MTLCommandQueue>>,
		pub(crate) workloads: crate::WorkloadTypes,
	}

	/// The `Queue` struct owns the queue submission entry point without borrowing the device.
	pub struct Queue {
		pub(crate) device: std::ptr::NonNull<context::Context>,
		pub(crate) queue_handle: graphics_hardware_interface::QueueHandle,
	}

	unsafe impl Send for Queue {}

	/// The `QueueReference` struct preserves the borrowed queue API while queue ownership is being split out.
	pub struct QueueReference<'a> {
		pub(crate) device: &'a mut context::Context,
		pub(crate) queue_handle: graphics_hardware_interface::QueueHandle,
	}

	/// The `Execution` struct gathers Metal command-buffer recordings before queue submission.
	pub struct Execution<'a> {
		frame: Option<super::Frame<'a>>,
		completed_frame: Option<graphics_hardware_interface::FrameKey>,
		command_buffers: Vec<super::FinishedCommandBuffer<'static>>,
	}

	impl<'a> crate::queue::QueueExecution<'a> for Execution<'a> {
		type Frame = super::Frame<'a>;

		fn frame(&mut self) -> Option<&mut Self::Frame> {
			self.frame.as_mut()
		}

		fn completed_frame(&self) -> Option<graphics_hardware_interface::FrameKey> {
			self.completed_frame
		}

		fn record<'record>(
			&'record mut self,
			command_buffer_handle: graphics_hardware_interface::CommandBufferHandle,
			record: impl FnOnce(&mut <Self::Frame as crate::frame::Frame<'a>>::CBR<'record>),
		) where
			Self::Frame: 'record,
		{
			let frame = self.frame.as_mut().expect(
				"Frame is required to record a frame command buffer. The most likely cause is that Queue::execute was called with None and the closure tried to record frame work.",
			);
			let mut command_buffer = frame.create_command_buffer_recording(command_buffer_handle);
			record(&mut command_buffer);
			self.command_buffers.push(command_buffer.into_finished());
		}
	}

	impl Queue {
		/// Returns mutable device access for the queue wrapper until device state is split out.
		fn device_mut(&mut self) -> &mut context::Context {
			// The owned queue is created from a live Device and must not outlive it.
			// Thread-safe ownership will require moving queue-local state out of Device.
			unsafe { self.device.as_mut() }
		}
	}

	impl crate::queue::Queue for Queue {
		type Frame<'a> = super::Frame<'a>;
		type Execution<'a> = Execution<'a>;

		fn create_command_buffer(&mut self, name: Option<&str>) -> graphics_hardware_interface::CommandBufferHandle {
			let queue_handle = self.queue_handle;
			self.device_mut().create_command_buffer(name, queue_handle)
		}

		fn start_frame<'a>(
			&'a mut self,
			index: u32,
			synchronizer_handle: graphics_hardware_interface::SynchronizerHandle,
		) -> crate::queue::StartedFrame<Self::Frame<'a>> {
			self.device_mut().start_frame(index, synchronizer_handle)
		}

		fn execute<'a, P>(
			&'a mut self,
			frame: Option<crate::queue::FrameRequest>,
			wait_for: &[graphics_hardware_interface::SynchronizerHandle],
			synchronizer: graphics_hardware_interface::SynchronizerHandle,
			execute: impl FnOnce(&mut Self::Execution<'a>) -> P,
		) where
			P: AsRef<[graphics_hardware_interface::PresentKey]>,
		{
			let device = self.device_mut();
			for &wait_synchronizer in wait_for {
				device.wait_for_synchronizer(wait_synchronizer);
			}

			let frame = frame.map(|frame| device.start_frame(frame.index, frame.synchronizer));
			let completed_frame = frame.as_ref().and_then(|frame| frame.completed_frame);
			let frame = frame.map(|frame| frame.frame);
			let mut execution = Execution {
				frame,
				completed_frame,
				command_buffers: Vec::new(),
			};
			let present_keys = execute(&mut execution);

			let Some(mut frame) = execution.frame else {
				return;
			};
			let last_index = execution.command_buffers.len().saturating_sub(1);
			for (index, command_buffer) in execution.command_buffers.into_iter().enumerate() {
				let present_keys = if index == last_index { present_keys.as_ref() } else { &[] };
				frame.execute_finished(command_buffer, present_keys, synchronizer);
			}
		}
	}

	impl crate::queue::Queue for QueueReference<'_> {
		type Frame<'a> = super::Frame<'a>;
		type Execution<'a> = Execution<'a>;

		fn create_command_buffer(&mut self, name: Option<&str>) -> graphics_hardware_interface::CommandBufferHandle {
			self.device.create_command_buffer(name, self.queue_handle)
		}

		fn start_frame<'a>(
			&'a mut self,
			index: u32,
			synchronizer_handle: graphics_hardware_interface::SynchronizerHandle,
		) -> crate::queue::StartedFrame<Self::Frame<'a>> {
			self.device.start_frame(index, synchronizer_handle)
		}

		fn execute<'a, P>(
			&'a mut self,
			frame: Option<crate::queue::FrameRequest>,
			wait_for: &[graphics_hardware_interface::SynchronizerHandle],
			synchronizer: graphics_hardware_interface::SynchronizerHandle,
			execute: impl FnOnce(&mut Self::Execution<'a>) -> P,
		) where
			P: AsRef<[graphics_hardware_interface::PresentKey]>,
		{
			for &wait_synchronizer in wait_for {
				self.device.wait_for_synchronizer(wait_synchronizer);
			}

			let frame = frame.map(|frame| self.device.start_frame(frame.index, frame.synchronizer));
			let completed_frame = frame.as_ref().and_then(|frame| frame.completed_frame);
			let frame = frame.map(|frame| frame.frame);
			let mut execution = Execution {
				frame,
				completed_frame,
				command_buffers: Vec::new(),
			};
			let present_keys = execute(&mut execution);

			let Some(mut frame) = execution.frame else {
				return;
			};
			let last_index = execution.command_buffers.len().saturating_sub(1);
			for (index, command_buffer) in execution.command_buffers.into_iter().enumerate() {
				let present_keys = if index == last_index { present_keys.as_ref() } else { &[] };
				frame.execute_finished(command_buffer, present_keys, synchronizer);
			}
		}
	}
}

pub mod buffer {
	use super::*;
	use crate::{DeviceAccesses, Uses};

	#[derive(Clone)]
	pub(crate) struct Buffer {
		pub(crate) name: Option<String>,
		pub(crate) staging: Option<BufferHandle>,
		pub(crate) buffer: Retained<ProtocolObject<dyn mtl::MTLBuffer>>,
		pub(crate) size: usize,
		pub(crate) gpu_address: u64,
		pub(crate) pointer: *mut u8,
		pub(crate) uses: Uses,
		pub(crate) access: DeviceAccesses,
	}
}

pub mod image {
	use super::*;
	use crate::{DeviceAccesses, Formats, Uses};

	#[derive(Clone)]
	pub(crate) struct Image {
		pub(crate) name: Option<String>,
		pub(crate) texture: Retained<ProtocolObject<dyn mtl::MTLTexture>>,
		pub(crate) extent: Extent,
		pub(crate) format: Formats,
		pub(crate) uses: Uses,
		pub(crate) access: DeviceAccesses,
		pub(crate) array_layers: u32,
		pub(crate) staging: Option<Vec<u8>>,
	}
}

pub mod sampler {
	use super::*;

	#[derive(Clone)]
	pub(crate) struct Sampler {
		pub(crate) sampler: Retained<ProtocolObject<dyn mtl::MTLSamplerState>>,
	}
}

pub mod descriptor_set {
	use super::*;
	use crate::descriptors::DescriptorSetHandle;

	/// The `DescriptorSet` struct stores the Metal descriptor state for one frame.
	#[derive(Clone)]
	pub(crate) struct DescriptorSet {
		pub next: Option<DescriptorSetHandle>,
		pub descriptor_set_layout: graphics_hardware_interface::DescriptorSetTemplateHandle,
		pub argument_buffer: Retained<ProtocolObject<dyn mtl::MTLBuffer>>,
		pub descriptors: HashMap<u32, HashMap<u32, Descriptor>>,
	}
}

pub mod binding {
	use super::*;
	use crate::descriptors::DescriptorSetHandle;

	#[derive(Clone)]
	pub(crate) struct Binding {
		pub next: Option<DescriptorSetBindingHandle>,
		pub descriptor_set_handle: DescriptorSetHandle,
		pub descriptor_type: crate::descriptors::DescriptorType,
		pub index: u32,
		pub count: u32,
	}
}

pub mod synchronizer {
	use crate::synchronizer::SynchronizerHandle;

	#[derive(Clone)]
	pub(crate) struct Synchronizer {
		pub next: Option<SynchronizerHandle>,
		pub signaled: bool,
	}
}

pub mod swapchain {
	use super::*;
	use crate::image::ImageHandle;

	#[derive(Clone)]
	pub(crate) struct Swapchain {
		pub layer: Retained<CAMetalLayer>,
		pub view: Retained<NSView>,
		pub images: [Option<ImageHandle>; MAX_SWAPCHAIN_IMAGES],
		pub extent: Extent,
	}
}

pub mod command_buffer;
pub mod context;
pub mod device;
pub mod factory;
pub mod frame;
pub mod instance;

pub(crate) use self::binding::*;
pub use self::command_buffer::*;
pub use self::context::*;
pub(crate) use self::descriptor_set::*;
pub use self::device::Device;
pub use self::factory::{ComputePipeline, Factory};
pub use self::frame::*;
pub use self::instance::*;
pub(crate) use self::synchronizer::*;
