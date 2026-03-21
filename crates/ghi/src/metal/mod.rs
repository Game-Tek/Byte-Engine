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
use smallvec::SmallVec;

use self::binding::DescriptorSetBindingHandle;
use self::buffer::BufferHandle;
use self::image::ImageHandle;
use self::sampler::SamplerHandle;
use self::synchronizer::SynchronizerHandle;
use crate::graphics_hardware_interface;

pub(super) enum Descriptor {
	Image {
		image: ImageHandle,
		layout: graphics_hardware_interface::Layouts,
	},
	CombinedImageSampler {
		image: ImageHandle,
		sampler: SamplerHandle,
		layout: graphics_hardware_interface::Layouts,
	},
	Buffer {
		buffer: BufferHandle,
		size: graphics_hardware_interface::Ranges,
	},
	Sampler {
		sampler: SamplerHandle,
	},
}

#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub(super) struct TopLevelAccelerationStructureHandle(pub(super) u64);

#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub(super) struct BottomLevelAccelerationStructureHandle(pub(super) u64);

#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub(super) enum Handle {
	Image(ImageHandle),
	Buffer(BufferHandle),
	TopLevelAccelerationStructure(TopLevelAccelerationStructureHandle),
	BottomLevelAccelerationStructure(BottomLevelAccelerationStructureHandle),
	Synchronizer(SynchronizerHandle),
}

#[derive(Clone, PartialEq)]
pub(super) struct Consumption {
	pub(super) handle: Handle,
	pub(super) stages: graphics_hardware_interface::Stages,
	pub(super) access: graphics_hardware_interface::AccessPolicies,
	pub(super) layout: graphics_hardware_interface::Layouts,
}

#[derive(Clone, PartialEq)]
pub(super) struct MetalConsumption {
	pub(super) handle: Handle,
	pub(super) stages: graphics_hardware_interface::Stages,
	pub(super) access: graphics_hardware_interface::AccessPolicies,
	pub(super) layout: graphics_hardware_interface::Layouts,
}

const MAX_FRAMES_IN_FLIGHT: usize = 3;
const MAX_SWAPCHAIN_IMAGES: usize = 8;

fn update_layer_extent(layer: &CAMetalLayer, view: &NSView) -> Extent {
	let logical_size = view.frame().size;
	let drawable_size = view.convertSizeToBacking(logical_size);
	let scale_factor = if logical_size.width > 0.0 {
		(drawable_size.width / logical_size.width).max(1.0)
	} else if logical_size.height > 0.0 {
		(drawable_size.height / logical_size.height).max(1.0)
	} else {
		1.0
	};

	layer.setContentsScale(scale_factor);
	layer.setDrawableSize(NSSize::new(drawable_size.width, drawable_size.height));

	Extent::rectangle(
		drawable_size.width.round().max(0.0) as u32,
		drawable_size.height.round().max(0.0) as u32,
	)
}

#[derive(Clone)]
pub(crate) struct DescriptorSetLayout {
	bindings: Vec<(graphics_hardware_interface::DescriptorType, u32)>,
}

#[derive(Clone, PartialEq, Eq)]
pub(crate) struct PipelineLayout {
	descriptor_set_template_indices: HashMap<graphics_hardware_interface::DescriptorSetTemplateHandle, u32>,
}

#[derive(Clone, PartialEq, Eq, Hash)]
struct PipelineLayoutKey {
	descriptor_set_templates: Vec<graphics_hardware_interface::DescriptorSetTemplateHandle>,
	push_constant_ranges: Vec<graphics_hardware_interface::PushConstantRange>,
}

#[derive(Clone)]
pub(crate) struct Shader {
	stage: graphics_hardware_interface::Stages,
	shader_binding_descriptors: Vec<graphics_hardware_interface::ShaderBindingDescriptor>,
	metal_function: Option<Retained<ProtocolObject<dyn mtl::MTLFunction>>>,
	spirv: Option<Vec<u8>>,
}

#[derive(Clone)]
pub(crate) struct Pipeline {
	pipeline: PipelineState,
	layout: graphics_hardware_interface::PipelineLayoutHandle,
	shader_handles: HashMap<graphics_hardware_interface::ShaderHandle, [u8; 32]>,
	resource_access: Vec<(
		(u32, u32),
		(
			graphics_hardware_interface::Stages,
			graphics_hardware_interface::AccessPolicies,
		),
	)>,
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
pub(crate) struct CommandBuffer {
	queue_handle: graphics_hardware_interface::QueueHandle,
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
	layout: graphics_hardware_interface::Layouts,
}

pub(crate) struct Mesh {
	vertex_buffer: Retained<ProtocolObject<dyn mtl::MTLBuffer>>,
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
		descriptor_write: graphics_hardware_interface::DescriptorWrite,
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

	pub(crate) fn update_resource_descriptor(
		descriptor_write: graphics_hardware_interface::DescriptorWrite,
		frame: Option<u8>,
	) -> Self {
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
		layout: graphics_hardware_interface::Layouts,
	},
	CombinedImageSampler {
		image_handle: ImageHandle,
		layout: graphics_hardware_interface::Layouts,
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

mod utils {
	use objc2_metal as mtl;

	use crate::{DeviceAccesses, FilteringModes, Formats, SamplerAddressingModes, Uses};

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
		}
	}

	pub(crate) fn resource_options_from_access(_access: DeviceAccesses) -> mtl::MTLResourceOptions {
		// TODO: Map DeviceAccesses to staging buffers + private storage for optimal performance.
		mtl::MTLResourceOptions::StorageModeShared
	}

	pub(crate) fn texture_usage_from_uses(uses: Uses) -> mtl::MTLTextureUsage {
		let mut usage = mtl::MTLTextureUsage::empty();

		if uses.intersects(
			Uses::Image | Uses::Storage | Uses::TransferSource | Uses::TransferDestination | Uses::ShaderBindingTable,
		) {
			usage |= mtl::MTLTextureUsage::ShaderRead;
		}

		if uses.intersects(Uses::Storage | Uses::TransferDestination) {
			usage |= mtl::MTLTextureUsage::ShaderWrite;
		}

		if uses.contains(Uses::RenderTarget) {
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

	pub(crate) fn data_type_size(format: crate::graphics_hardware_interface::DataTypes) -> usize {
		match format {
			crate::graphics_hardware_interface::DataTypes::Float => std::mem::size_of::<f32>(),
			crate::graphics_hardware_interface::DataTypes::Float2 => std::mem::size_of::<f32>() * 2,
			crate::graphics_hardware_interface::DataTypes::Float3 => std::mem::size_of::<f32>() * 3,
			crate::graphics_hardware_interface::DataTypes::Float4 => std::mem::size_of::<f32>() * 4,
			crate::graphics_hardware_interface::DataTypes::U8 => std::mem::size_of::<u8>(),
			crate::graphics_hardware_interface::DataTypes::U16 => std::mem::size_of::<u16>(),
			crate::graphics_hardware_interface::DataTypes::U32 => std::mem::size_of::<u32>(),
			crate::graphics_hardware_interface::DataTypes::Int => std::mem::size_of::<i32>(),
			crate::graphics_hardware_interface::DataTypes::Int2 => std::mem::size_of::<i32>() * 2,
			crate::graphics_hardware_interface::DataTypes::Int3 => std::mem::size_of::<i32>() * 3,
			crate::graphics_hardware_interface::DataTypes::Int4 => std::mem::size_of::<i32>() * 4,
			crate::graphics_hardware_interface::DataTypes::UInt => std::mem::size_of::<u32>(),
			crate::graphics_hardware_interface::DataTypes::UInt2 => std::mem::size_of::<u32>() * 2,
			crate::graphics_hardware_interface::DataTypes::UInt3 => std::mem::size_of::<u32>() * 3,
			crate::graphics_hardware_interface::DataTypes::UInt4 => std::mem::size_of::<u32>() * 4,
		}
	}

	pub(crate) fn vertex_layout_size(layout: &[crate::graphics_hardware_interface::VertexElement<'_>]) -> usize {
		layout.iter().map(|element| data_type_size(element.format)).sum()
	}
}

pub mod queue {
	use super::*;

	pub(crate) struct Queue {
		pub(crate) queue: Retained<ProtocolObject<dyn mtl::MTLCommandQueue>>,
	}
}

pub mod buffer {
	use super::*;
	use crate::{DeviceAccesses, Uses};

	#[derive(Clone)]
	pub(crate) struct Buffer {
		pub(crate) next: Option<BufferHandle>,
		pub(crate) staging: Option<BufferHandle>,
		pub(crate) buffer: Retained<ProtocolObject<dyn mtl::MTLBuffer>>,
		pub(crate) size: usize,
		pub(crate) gpu_address: u64,
		pub(crate) pointer: *mut u8,
		pub(crate) uses: Uses,
		pub(crate) access: DeviceAccesses,
	}

	#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
	pub(crate) struct BufferHandle(pub(crate) u64);

	impl Into<Handle> for BufferHandle {
		fn into(self) -> Handle {
			Handle::Buffer(self)
		}
	}

	impl HandleLike for BufferHandle {
		type Item = Buffer;

		fn build(value: u64) -> Self {
			BufferHandle(value)
		}

		fn access<'a>(&self, collection: &'a [Self::Item]) -> &'a Buffer {
			&collection[self.0 as usize]
		}
	}

	impl Next for Buffer {
		type Handle = BufferHandle;

		fn next(&self) -> Option<Self::Handle> {
			self.next
		}
	}
}

pub mod image {
	use super::*;
	use crate::{DeviceAccesses, Formats, Uses};

	#[derive(Clone)]
	pub(crate) struct Image {
		pub(crate) next: Option<ImageHandle>,
		pub(crate) texture: Retained<ProtocolObject<dyn mtl::MTLTexture>>,
		pub(crate) extent: Extent,
		pub(crate) format: Formats,
		pub(crate) uses: Uses,
		pub(crate) access: DeviceAccesses,
		pub(crate) array_layers: u32,
		pub(crate) staging: Option<Vec<u8>>,
	}

	#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
	pub(crate) struct ImageHandle(pub(crate) u64);

	impl Into<Handle> for ImageHandle {
		fn into(self) -> Handle {
			Handle::Image(self)
		}
	}

	impl HandleLike for ImageHandle {
		type Item = Image;

		fn build(value: u64) -> Self {
			ImageHandle(value)
		}

		fn access<'a>(&self, collection: &'a [Self::Item]) -> &'a Image {
			&collection[self.0 as usize]
		}
	}

	impl Next for Image {
		type Handle = ImageHandle;

		fn next(&self) -> Option<Self::Handle> {
			self.next
		}
	}
}

pub mod sampler {
	use super::*;

	#[derive(Clone)]
	pub(crate) struct Sampler {
		pub(crate) sampler: Retained<ProtocolObject<dyn mtl::MTLSamplerState>>,
	}

	#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
	pub(crate) struct SamplerHandle(pub(crate) u64);

	impl HandleLike for SamplerHandle {
		type Item = Sampler;

		fn build(value: u64) -> Self {
			SamplerHandle(value)
		}

		fn access<'a>(&self, collection: &'a [Self::Item]) -> &'a Sampler {
			&collection[self.0 as usize]
		}
	}

	impl Next for Sampler {
		type Handle = SamplerHandle;

		fn next(&self) -> Option<Self::Handle> {
			None
		}
	}
}

pub mod descriptor_set {
	use super::*;

	#[derive(Clone)]
	pub(crate) struct DescriptorSet {
		pub next: Option<DescriptorSetHandle>,
		pub descriptor_set_layout: graphics_hardware_interface::DescriptorSetTemplateHandle,
	}

	impl Next for DescriptorSet {
		type Handle = DescriptorSetHandle;

		fn next(&self) -> Option<DescriptorSetHandle> {
			self.next
		}
	}

	#[derive(Clone, Copy, PartialEq, Eq, Hash)]
	pub(crate) struct DescriptorSetHandle(pub u64);

	impl HandleLike for DescriptorSetHandle {
		type Item = DescriptorSet;

		fn build(value: u64) -> Self {
			DescriptorSetHandle(value)
		}

		fn access<'a>(&self, collection: &'a [Self::Item]) -> &'a DescriptorSet {
			&collection[self.0 as usize]
		}
	}
}

pub mod binding {
	use super::*;

	#[derive(Clone)]
	pub(crate) struct Binding {
		pub next: Option<DescriptorSetBindingHandle>,
		pub descriptor_set_handle: DescriptorSetHandle,
		pub descriptor_type: graphics_hardware_interface::DescriptorType,
		pub index: u32,
		pub count: u32,
	}

	#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
	pub struct DescriptorSetBindingHandle(pub u64);

	impl HandleLike for DescriptorSetBindingHandle {
		type Item = Binding;

		fn build(value: u64) -> Self {
			DescriptorSetBindingHandle(value)
		}

		fn access<'a>(&self, collection: &'a [Self::Item]) -> &'a Binding {
			&collection[self.0 as usize]
		}
	}

	impl Next for Binding {
		type Handle = DescriptorSetBindingHandle;

		fn next(&self) -> Option<Self::Handle> {
			self.next
		}
	}
}

pub mod synchronizer {
	use super::*;

	#[derive(Clone)]
	pub(crate) struct Synchronizer {
		pub next: Option<SynchronizerHandle>,
		pub signaled: bool,
	}

	#[derive(Clone, Copy, PartialEq, Eq, Hash)]
	pub(crate) struct SynchronizerHandle(pub(crate) u64);

	impl Into<Handle> for SynchronizerHandle {
		fn into(self) -> Handle {
			Handle::Synchronizer(self)
		}
	}

	impl HandleLike for SynchronizerHandle {
		type Item = Synchronizer;

		fn build(value: u64) -> Self {
			SynchronizerHandle(value)
		}

		fn access<'a>(&self, collection: &'a [Self::Item]) -> &'a Synchronizer {
			&collection[self.0 as usize]
		}
	}

	impl Next for Synchronizer {
		type Handle = SynchronizerHandle;

		fn next(&self) -> Option<SynchronizerHandle> {
			self.next
		}
	}
}

pub mod swapchain {
	use super::*;

	#[derive(Clone)]
	pub(crate) struct Swapchain {
		pub layer: Retained<CAMetalLayer>,
		pub view: Retained<NSView>,
		pub drawables: [Option<Retained<ProtocolObject<dyn CAMetalDrawable>>>; MAX_SWAPCHAIN_IMAGES],
		pub extent: Extent,
		pub pixel_format: mtl::MTLPixelFormat,
	}

	impl Swapchain {
		pub(crate) fn new(
			layer: Retained<CAMetalLayer>,
			view: Retained<NSView>,
			extent: Extent,
			pixel_format: mtl::MTLPixelFormat,
		) -> Self {
			Self {
				layer,
				view,
				drawables: std::array::from_fn(|_| None),
				extent,
				pixel_format,
			}
		}

		pub(crate) fn store_drawable(&mut self, drawable: Retained<ProtocolObject<dyn CAMetalDrawable>>) -> u8 {
			let slot = self.drawables.iter().position(|d| d.is_none()).unwrap_or(0);
			self.drawables[slot] = Some(drawable);
			slot as u8
		}

		pub(crate) fn take_drawable(&mut self, index: u8) -> Option<Retained<ProtocolObject<dyn CAMetalDrawable>>> {
			self.drawables.get_mut(index as usize).and_then(|drawable| drawable.take())
		}
	}
}

pub mod instance {
	use objc2_metal::MTLDevice;

	use super::*;

	pub struct Instance {
		devices: Vec<Retained<ProtocolObject<dyn mtl::MTLDevice>>>,
		settings: graphics_hardware_interface::Features,
	}

	unsafe impl Send for Instance {}

	impl Instance {
		pub fn new(settings: graphics_hardware_interface::Features) -> Result<Instance, &'static str> {
			let devices = mtl::MTLCopyAllDevices().to_vec();

			if devices.is_empty() {
				return Err("No Metal devices available. The most likely cause is that the system does not support Metal.");
			}

			Ok(Instance { devices, settings })
		}

		pub fn create_device(
			&mut self,
			settings: graphics_hardware_interface::Features,
			queues: &mut [(
				graphics_hardware_interface::QueueSelection,
				&mut Option<graphics_hardware_interface::QueueHandle>,
			)],
		) -> Result<super::Device, &'static str> {
			let device = if let Some(preferred_name) = settings.gpu {
				let selected = self.devices.iter().find(|device| device.name().to_string() == preferred_name);

				match selected {
					Some(device) => device.clone(),
					None => {
						return Err(
							"Requested Metal device not found. The most likely cause is that the device name does not match any available GPU.",
						);
					}
				}
			} else {
				mtl::MTLCreateSystemDefaultDevice()
					.or_else(|| self.devices.first().cloned())
					.ok_or(
						"Metal device creation failed. The most likely cause is that no compatible Metal device is available.",
					)?
			};

			let merged_settings = graphics_hardware_interface::Features {
				validation: settings.validation || self.settings.validation,
				gpu_validation: settings.gpu_validation || self.settings.gpu_validation,
				api_dump: settings.api_dump || self.settings.api_dump,
				ray_tracing: settings.ray_tracing || self.settings.ray_tracing,
				debug_log_function: settings.debug_log_function.or(self.settings.debug_log_function),
				gpu: settings.gpu.or(self.settings.gpu),
				sparse: settings.sparse || self.settings.sparse,
				geometry_shader: settings.geometry_shader || self.settings.geometry_shader,
				mesh_shading: settings.mesh_shading || self.settings.mesh_shading,
			};

			super::Device::new(merged_settings, device, queues)
		}
	}
}

pub mod device {
	use std::collections::VecDeque;
	use std::num::NonZeroU32;
	use std::ptr::NonNull;

	use ::utils::hash::HashSet;
	use objc2::ClassType;
	use objc2_foundation::NSString;
	use objc2_metal::{MTLBuffer, MTLCommandQueue, MTLDevice, MTLResource, MTLTexture};

	use super::*;
	use crate::{buffer as buffer_builder, image as image_builder, raster_pipeline, sampler as sampler_builder, window};

	pub struct Device {
		pub(crate) device: Retained<ProtocolObject<dyn mtl::MTLDevice>>,
		pub(crate) frames: u8,
		pub(crate) queues: Vec<queue::Queue>,
		pub(crate) buffers: Vec<buffer::Buffer>,
		pub(crate) images: Vec<image::Image>,
		pub(crate) samplers: Vec<sampler::Sampler>,
		pub(crate) allocations: Vec<Allocation>,
		pub(crate) descriptor_sets_layouts: Vec<DescriptorSetLayout>,
		pub(crate) pipeline_layouts: Vec<PipelineLayout>,
		pipeline_layout_indices: HashMap<PipelineLayoutKey, graphics_hardware_interface::PipelineLayoutHandle>,
		pub(crate) bindings: Vec<binding::Binding>,
		pub(crate) descriptor_sets: Vec<descriptor_set::DescriptorSet>,
		pub(crate) meshes: Vec<Mesh>,
		pub(crate) acceleration_structures: Vec<AccelerationStructure>,
		pub(crate) shaders: Vec<Shader>,
		pub(crate) pipelines: Vec<Pipeline>,
		pub(crate) command_buffers: Vec<CommandBuffer>,
		pub(crate) synchronizers: Vec<synchronizer::Synchronizer>,
		pub(crate) swapchains: Vec<swapchain::Swapchain>,

		pub(crate) descriptors: HashMap<descriptor_set::DescriptorSetHandle, HashMap<u32, HashMap<u32, Descriptor>>>,
		pub(crate) resource_to_descriptor: HashMap<Handle, HashSet<(binding::DescriptorSetBindingHandle, u32)>>,
		pub(crate) descriptor_set_to_resource: HashMap<(descriptor_set::DescriptorSetHandle, u32), HashSet<Handle>>,

		pub settings: graphics_hardware_interface::Features,
		pub(crate) states: HashMap<Handle, TransitionState>,
		pub(crate) pending_buffer_syncs: VecDeque<buffer::BufferHandle>,
		pub(crate) pending_image_syncs: VecDeque<image::ImageHandle>,
		pub(crate) tasks: Vec<Task>,
		pub(crate) texture_copies: Vec<Vec<u8>>,

		#[cfg(debug_assertions)]
		pub names: HashMap<graphics_hardware_interface::Handle, String>,
	}

	impl Device {
		pub fn new(
			settings: graphics_hardware_interface::Features,
			device: Retained<ProtocolObject<dyn mtl::MTLDevice>>,
			queues: &mut [(
				graphics_hardware_interface::QueueSelection,
				&mut Option<graphics_hardware_interface::QueueHandle>,
			)],
		) -> Result<Device, &'static str> {
			let mut created_queues = Vec::with_capacity(queues.len());

			for (_selection, output_handle) in queues.iter_mut() {
				let queue = device.newCommandQueue().ok_or(
					"Metal command queue creation failed. The most likely cause is that the device ran out of command queue resources.",
				)?;
				let handle = graphics_hardware_interface::QueueHandle(created_queues.len() as u64);

				created_queues.push(queue::Queue { queue });

				**output_handle = Some(handle);
			}

			Ok(Device {
				device,
				frames: MAX_FRAMES_IN_FLIGHT as u8,
				queues: created_queues,
				buffers: Vec::new(),
				images: Vec::new(),
				samplers: Vec::new(),
				allocations: Vec::new(),
				descriptor_sets_layouts: Vec::new(),
				pipeline_layouts: Vec::new(),
				pipeline_layout_indices: HashMap::default(),
				bindings: Vec::new(),
				descriptor_sets: Vec::new(),
				meshes: Vec::new(),
				acceleration_structures: Vec::new(),
				shaders: Vec::new(),
				pipelines: Vec::new(),
				command_buffers: Vec::new(),
				synchronizers: Vec::new(),
				swapchains: Vec::new(),
				descriptors: HashMap::default(),
				resource_to_descriptor: HashMap::default(),
				descriptor_set_to_resource: HashMap::default(),
				settings,
				states: HashMap::default(),
				pending_buffer_syncs: VecDeque::new(),
				pending_image_syncs: VecDeque::new(),
				tasks: Vec::new(),
				texture_copies: Vec::new(),

				#[cfg(debug_assertions)]
				names: HashMap::default(),
			})
		}

		fn create_buffer_internal(
			&mut self,
			next: Option<buffer::BufferHandle>,
			name: Option<&str>,
			size: usize,
			resource_uses: graphics_hardware_interface::Uses,
			device_accesses: graphics_hardware_interface::DeviceAccesses,
		) -> buffer::BufferHandle {
			let options = utils::resource_options_from_access(device_accesses);
			let buffer = self
				.device
				.newBufferWithLength_options(size as _, options)
				.expect("Metal buffer creation failed. The most likely cause is that the device is out of memory.");

			if let Some(name) = name {
				buffer.setLabel(Some(&NSString::from_str(name)));
			}

			let pointer = buffer.contents().as_ptr() as *mut u8;
			let gpu_address = buffer.gpuAddress() as u64;

			let handle = buffer::BufferHandle(self.buffers.len() as u64);
			self.buffers.push(buffer::Buffer {
				next,
				staging: Some(handle),
				buffer,
				size,
				gpu_address,
				pointer,
				uses: resource_uses,
				access: device_accesses,
			});

			handle
		}

		pub(super) fn create_image_internal(
			&mut self,
			next: Option<image::ImageHandle>,
			name: Option<&str>,
			extent: Extent,
			format: graphics_hardware_interface::Formats,
			resource_uses: graphics_hardware_interface::Uses,
			device_accesses: graphics_hardware_interface::DeviceAccesses,
			array_layers: u32,
		) -> image::ImageHandle {
			let pixel_format = utils::to_pixel_format(format);

			let width = extent.width().max(1);
			let height = extent.height().max(1);
			let mipmapped = false;

			let descriptor = unsafe {
				mtl::MTLTextureDescriptor::texture2DDescriptorWithPixelFormat_width_height_mipmapped(
					pixel_format,
					width as _,
					height as _,
					mipmapped,
				)
			};
			if extent.depth() > 1 {
				descriptor.setTextureType(mtl::MTLTextureType::Type3D);
			} else if array_layers > 1 {
				descriptor.setTextureType(mtl::MTLTextureType::Type2DArray);
			}
			descriptor.setUsage(utils::texture_usage_from_uses(resource_uses));
			descriptor.setStorageMode(mtl::MTLStorageMode::Shared);
			unsafe {
				descriptor.setArrayLength(array_layers as _);
			}

			let texture = self
				.device
				.newTextureWithDescriptor(&descriptor)
				.expect("Metal texture creation failed. The most likely cause is that the device is out of memory.");

			if let Some(name) = name {
				texture.setLabel(Some(&NSString::from_str(name)));
			}

			let staging = utils::bytes_per_pixel(format).map(|bytes_per_pixel| {
				let depth = extent.depth().max(1) as usize;
				let size = width as usize * height as usize * depth * bytes_per_pixel * array_layers as usize;
				vec![0u8; size]
			});

			let handle = image::ImageHandle(self.images.len() as u64);
			self.images.push(image::Image {
				next,
				texture,
				extent,
				format,
				uses: resource_uses,
				access: device_accesses,
				array_layers,
				staging,
			});

			handle
		}

		fn update_descriptor_for_binding(
			&mut self,
			binding_handle: binding::DescriptorSetBindingHandle,
			descriptor: Descriptor,
			array_element: u32,
		) {
			let binding = &self.bindings[binding_handle.0 as usize];
			let set_handle = binding.descriptor_set_handle;

			let bindings = self.descriptors.entry(set_handle).or_default();
			let arrays = bindings.entry(binding.index).or_default();
			arrays.insert(array_element, descriptor);
		}

		pub(super) fn copy_texture_to_cpu(
			&mut self,
			image_handle: image::ImageHandle,
		) -> graphics_hardware_interface::TextureCopyHandle {
			let image = &self.images[image_handle.0 as usize];
			let Some(bytes_per_pixel) = utils::bytes_per_pixel(image.format) else {
				self.texture_copies.push(Vec::new());
				return graphics_hardware_interface::TextureCopyHandle((self.texture_copies.len() - 1) as u64);
			};

			let extent = image.extent;
			let width = extent.width() as usize;
			let height = extent.height() as usize;
			let bytes_per_row = width * bytes_per_pixel;
			let size = bytes_per_row * height;

			let mut data = vec![0u8; size];
			let data_ptr = NonNull::new(data.as_mut_ptr() as *mut std::ffi::c_void)
				.expect("Texture readback buffer was null. The most likely cause is an empty allocation.");
			let region = mtl::MTLRegion {
				origin: mtl::MTLOrigin { x: 0, y: 0, z: 0 },
				size: mtl::MTLSize {
					width: width as _,
					height: height as _,
					depth: 1,
				},
			};

			unsafe {
				image
					.texture
					.getBytes_bytesPerRow_fromRegion_mipmapLevel(data_ptr, bytes_per_row as _, region, 0);
			}

			self.texture_copies.push(data);
			graphics_hardware_interface::TextureCopyHandle((self.texture_copies.len() - 1) as u64)
		}
	}

	impl Device {
		#[cfg(debug_assertions)]
		pub fn has_errors(&self) -> bool {
			false
		}

		pub fn set_frames_in_flight(&mut self, frames: u8) {
			self.frames = frames.max(1);
			// TODO: Rebuild dynamic resources for new frame count.
		}

		pub fn create_allocation(
			&mut self,
			size: usize,
			_resource_uses: graphics_hardware_interface::Uses,
			device_accesses: graphics_hardware_interface::DeviceAccesses,
		) -> graphics_hardware_interface::AllocationHandle {
			let options = utils::resource_options_from_access(device_accesses);
			let buffer = self
				.device
				.newBufferWithLength_options(size as _, options)
				.expect("Metal allocation failed. The most likely cause is that the device is out of memory.");
			let pointer = buffer.contents().as_ptr() as *mut u8;

			self.allocations.push(Allocation { buffer, pointer, size });
			graphics_hardware_interface::AllocationHandle((self.allocations.len() - 1) as u64)
		}

		pub fn add_mesh_from_vertices_and_indices(
			&mut self,
			vertex_count: u32,
			index_count: u32,
			vertices: &[u8],
			indices: &[u8],
			vertex_layout: &[graphics_hardware_interface::VertexElement],
		) -> graphics_hardware_interface::MeshHandle {
			let options = mtl::MTLResourceOptions::StorageModeShared;
			let vertex_ptr = NonNull::new(vertices.as_ptr() as *mut std::ffi::c_void)
				.expect("Vertex data pointer was null. The most likely cause is an empty vertex slice.");
			let index_ptr = NonNull::new(indices.as_ptr() as *mut std::ffi::c_void)
				.expect("Index data pointer was null. The most likely cause is an empty index slice.");
			let vertex_buffer = unsafe {
				self.device
					.newBufferWithBytes_length_options(vertex_ptr, vertices.len() as _, options)
			}
			.expect("Metal vertex buffer creation failed. The most likely cause is that the device is out of memory.");
			let index_buffer = unsafe {
				self.device
					.newBufferWithBytes_length_options(index_ptr, indices.len() as _, options)
			}
			.expect("Metal index buffer creation failed. The most likely cause is that the device is out of memory.");
			let vertex_size = utils::vertex_layout_size(vertex_layout);

			self.meshes.push(Mesh {
				vertex_buffer,
				index_buffer,
				vertex_count,
				index_count,
				vertex_size,
			});

			graphics_hardware_interface::MeshHandle((self.meshes.len() - 1) as u64)
		}

		pub fn create_shader(
			&mut self,
			_name: Option<&str>,
			shader_source_type: graphics_hardware_interface::ShaderSource,
			stage: graphics_hardware_interface::ShaderTypes,
			shader_binding_descriptors: impl IntoIterator<Item = graphics_hardware_interface::ShaderBindingDescriptor>,
		) -> Result<graphics_hardware_interface::ShaderHandle, ()> {
			let spirv = match shader_source_type {
				graphics_hardware_interface::ShaderSource::SPIRV(data) => Some(data.to_vec()),
			};

			let stages = match stage {
				graphics_hardware_interface::ShaderTypes::Vertex => graphics_hardware_interface::Stages::VERTEX,
				graphics_hardware_interface::ShaderTypes::Fragment => graphics_hardware_interface::Stages::FRAGMENT,
				graphics_hardware_interface::ShaderTypes::Compute => graphics_hardware_interface::Stages::COMPUTE,
				graphics_hardware_interface::ShaderTypes::RayGen => graphics_hardware_interface::Stages::RAYGEN,
				graphics_hardware_interface::ShaderTypes::Intersection => graphics_hardware_interface::Stages::INTERSECTION,
				graphics_hardware_interface::ShaderTypes::AnyHit => graphics_hardware_interface::Stages::ANY_HIT,
				graphics_hardware_interface::ShaderTypes::ClosestHit => graphics_hardware_interface::Stages::CLOSEST_HIT,
				graphics_hardware_interface::ShaderTypes::Miss => graphics_hardware_interface::Stages::MISS,
				graphics_hardware_interface::ShaderTypes::Callable => graphics_hardware_interface::Stages::CALLABLE,
				graphics_hardware_interface::ShaderTypes::Task => graphics_hardware_interface::Stages::TASK,
				graphics_hardware_interface::ShaderTypes::Mesh => graphics_hardware_interface::Stages::MESH,
			};

			self.shaders.push(Shader {
				stage: stages,
				shader_binding_descriptors: shader_binding_descriptors.into_iter().collect(),
				metal_function: None,
				spirv,
			});

			// TODO: Compile SPIR-V to MSL and create MTLLibrary/MTLFunction.
			Ok(graphics_hardware_interface::ShaderHandle((self.shaders.len() - 1) as u64))
		}

		pub fn create_descriptor_set_template(
			&mut self,
			_name: Option<&str>,
			binding_templates: &[graphics_hardware_interface::DescriptorSetBindingTemplate],
		) -> graphics_hardware_interface::DescriptorSetTemplateHandle {
			let bindings = binding_templates
				.iter()
				.map(|template| (template.descriptor_type, template.descriptor_count))
				.collect();
			self.descriptor_sets_layouts.push(DescriptorSetLayout { bindings });
			graphics_hardware_interface::DescriptorSetTemplateHandle((self.descriptor_sets_layouts.len() - 1) as u64)
		}

		pub fn create_descriptor_set(
			&mut self,
			_name: Option<&str>,
			descriptor_set_template_handle: &graphics_hardware_interface::DescriptorSetTemplateHandle,
		) -> graphics_hardware_interface::DescriptorSetHandle {
			self.descriptor_sets.push(descriptor_set::DescriptorSet {
				next: None,
				descriptor_set_layout: *descriptor_set_template_handle,
			});
			graphics_hardware_interface::DescriptorSetHandle((self.descriptor_sets.len() - 1) as u64)
		}

		pub fn create_descriptor_binding(
			&mut self,
			descriptor_set: graphics_hardware_interface::DescriptorSetHandle,
			binding_constructor: graphics_hardware_interface::BindingConstructor,
		) -> graphics_hardware_interface::DescriptorSetBindingHandle {
			let descriptor_type = binding_constructor.descriptor_set_binding_template.descriptor_type;
			let binding_index = binding_constructor.descriptor_set_binding_template.binding;
			let count = binding_constructor.descriptor_set_binding_template.descriptor_count;

			self.bindings.push(binding::Binding {
				next: None,
				descriptor_set_handle: descriptor_set::DescriptorSetHandle(descriptor_set.0),
				descriptor_type,
				index: binding_index,
				count,
			});

			let binding_handle = binding::DescriptorSetBindingHandle((self.bindings.len() - 1) as u64);

			match binding_constructor.descriptor {
				graphics_hardware_interface::Descriptor::Buffer { handle, size } => {
					self.update_descriptor_for_binding(
						binding_handle,
						Descriptor::Buffer {
							buffer: buffer::BufferHandle(handle.0),
							size,
						},
						binding_constructor.array_element,
					);
				}
				graphics_hardware_interface::Descriptor::Image { handle, layout } => {
					self.update_descriptor_for_binding(
						binding_handle,
						Descriptor::Image {
							image: image::ImageHandle(handle.0),
							layout,
						},
						binding_constructor.array_element,
					);
				}
				graphics_hardware_interface::Descriptor::CombinedImageSampler {
					image_handle,
					sampler_handle,
					layout,
					..
				} => {
					self.update_descriptor_for_binding(
						binding_handle,
						Descriptor::CombinedImageSampler {
							image: image::ImageHandle(image_handle.0),
							sampler: sampler::SamplerHandle(sampler_handle.0),
							layout,
						},
						binding_constructor.array_element,
					);
				}
				graphics_hardware_interface::Descriptor::Sampler(handle) => {
					self.update_descriptor_for_binding(
						binding_handle,
						Descriptor::Sampler {
							sampler: sampler::SamplerHandle(handle.0),
						},
						binding_constructor.array_element,
					);
				}
				_ => {
					// TODO: Map acceleration structures, swapchains, and static samplers to Metal argument buffers.
				}
			}

			graphics_hardware_interface::DescriptorSetBindingHandle(binding_handle.0)
		}

		fn get_or_create_pipeline_layout(
			&mut self,
			descriptor_set_template_handles: &[graphics_hardware_interface::DescriptorSetTemplateHandle],
			push_constant_ranges: &[graphics_hardware_interface::PushConstantRange],
		) -> graphics_hardware_interface::PipelineLayoutHandle {
			let key = PipelineLayoutKey {
				descriptor_set_templates: descriptor_set_template_handles.to_vec(),
				push_constant_ranges: push_constant_ranges.to_vec(),
			};

			if let Some(handle) = self.pipeline_layout_indices.get(&key) {
				return *handle;
			}

			let descriptor_set_template_indices = descriptor_set_template_handles
				.iter()
				.enumerate()
				.map(|(i, handle)| (*handle, i as u32))
				.collect();
			self.pipeline_layouts.push(PipelineLayout {
				descriptor_set_template_indices,
			});
			let handle = graphics_hardware_interface::PipelineLayoutHandle((self.pipeline_layouts.len() - 1) as u64);
			self.pipeline_layout_indices.insert(key, handle);
			handle
		}

		pub fn create_raster_pipeline(
			&mut self,
			builder: raster_pipeline::Builder,
		) -> graphics_hardware_interface::PipelineHandle {
			let layout = self.get_or_create_pipeline_layout(
				builder.descriptor_set_templates.as_ref(),
				builder.push_constant_ranges.as_ref(),
			);
			self.pipelines.push(Pipeline {
				pipeline: PipelineState::Raster(None),
				layout,
				shader_handles: HashMap::default(),
				resource_access: Vec::new(),
			});
			// TODO: Create MTLRenderPipelineState from shader functions + vertex descriptor.
			graphics_hardware_interface::PipelineHandle((self.pipelines.len() - 1) as u64)
		}

		pub fn create_compute_pipeline(
			&mut self,
			builder: graphics_hardware_interface::pipelines::compute::Builder,
		) -> graphics_hardware_interface::PipelineHandle {
			let layout = self.get_or_create_pipeline_layout(builder.descriptor_set_templates, builder.push_constant_ranges);
			self.pipelines.push(Pipeline {
				pipeline: PipelineState::Compute(None),
				layout,
				shader_handles: HashMap::default(),
				resource_access: Vec::new(),
			});
			// TODO: Create MTLComputePipelineState from shader function.
			graphics_hardware_interface::PipelineHandle((self.pipelines.len() - 1) as u64)
		}

		pub fn create_ray_tracing_pipeline(
			&mut self,
			builder: graphics_hardware_interface::pipelines::ray_tracing::Builder,
		) -> graphics_hardware_interface::PipelineHandle {
			let layout = self.get_or_create_pipeline_layout(
				builder.descriptor_set_templates.as_ref(),
				builder.push_constant_ranges.as_ref(),
			);
			self.pipelines.push(Pipeline {
				pipeline: PipelineState::RayTracing,
				layout,
				shader_handles: HashMap::default(),
				resource_access: Vec::new(),
			});
			// TODO: Metal ray tracing pipeline mapping.
			graphics_hardware_interface::PipelineHandle((self.pipelines.len() - 1) as u64)
		}

		pub fn create_command_buffer(
			&mut self,
			_name: Option<&str>,
			queue_handle: graphics_hardware_interface::QueueHandle,
		) -> graphics_hardware_interface::CommandBufferHandle {
			self.command_buffers.push(CommandBuffer { queue_handle });
			graphics_hardware_interface::CommandBufferHandle((self.command_buffers.len() - 1) as u64)
		}

		pub fn create_command_buffer_recording<'a>(
			&'a mut self,
			command_buffer_handle: graphics_hardware_interface::CommandBufferHandle,
		) -> super::CommandBufferRecording<'a> {
			let command_buffer = &self.command_buffers[command_buffer_handle.0 as usize];
			let queue = &self.queues[command_buffer.queue_handle.0 as usize];
			let mtl_command_buffer = queue.queue.commandBuffer().expect(
				"Metal command buffer creation failed. The most likely cause is that the command queue did not provide a command buffer.",
			);

			super::CommandBufferRecording::new(self, command_buffer_handle, mtl_command_buffer, None)
		}

		pub fn build_buffer<T: Copy>(
			&mut self,
			builder: buffer_builder::Builder,
		) -> graphics_hardware_interface::BufferHandle<T> {
			let size = std::mem::size_of::<T>();
			let buffer_handle =
				self.create_buffer_internal(None, builder.name, size, builder.resource_uses, builder.device_accesses);
			graphics_hardware_interface::BufferHandle::<T>(buffer_handle.0, std::marker::PhantomData)
		}

		pub fn build_dynamic_buffer<T: Copy>(
			&mut self,
			builder: buffer_builder::Builder,
		) -> graphics_hardware_interface::DynamicBufferHandle<T> {
			let size = std::mem::size_of::<T>();
			let mut first_handle: Option<buffer::BufferHandle> = None;
			let mut previous_handle: Option<buffer::BufferHandle> = None;

			for _ in 0..self.frames {
				let handle =
					self.create_buffer_internal(None, builder.name, size, builder.resource_uses, builder.device_accesses);
				if let Some(previous) = previous_handle {
					self.buffers[previous.0 as usize].next = Some(handle);
				} else {
					first_handle = Some(handle);
				}
				previous_handle = Some(handle);
			}

			let master =
				first_handle.expect("Dynamic buffer creation failed. The most likely cause is that no buffers were allocated.");
			graphics_hardware_interface::DynamicBufferHandle::<T>(master.0, std::marker::PhantomData)
		}

		pub fn build_dynamic_image(
			&mut self,
			builder: image_builder::Builder,
		) -> graphics_hardware_interface::DynamicImageHandle {
			let handle = self.build_image(builder.use_case(crate::UseCases::DYNAMIC));
			graphics_hardware_interface::DynamicImageHandle(handle.0)
		}

		pub fn get_buffer_address(&self, buffer_handle: graphics_hardware_interface::BaseBufferHandle) -> u64 {
			self.buffers[buffer_handle.0 as usize].gpu_address
		}

		pub fn get_buffer_slice<T: Copy>(&mut self, buffer_handle: graphics_hardware_interface::BufferHandle<T>) -> &T {
			let buffer = &self.buffers[buffer_handle.0 as usize];
			unsafe { &*(buffer.pointer as *const T) }
		}

		pub fn get_mut_buffer_slice<'a, T: Copy>(
			&'a self,
			buffer_handle: graphics_hardware_interface::BufferHandle<T>,
		) -> &'a mut T {
			let buffer = &self.buffers[buffer_handle.0 as usize];
			unsafe { &mut *(buffer.pointer as *mut T) }
		}

		pub fn get_texture_slice_mut(&mut self, texture_handle: graphics_hardware_interface::ImageHandle) -> &'static mut [u8] {
			let image = &mut self.images[texture_handle.0 as usize];
			let Some(staging) = image.staging.as_mut() else {
				return &mut [];
			};

			unsafe { std::mem::transmute::<&mut [u8], &'static mut [u8]>(staging.as_mut_slice()) }
		}

		pub fn write_texture(&mut self, texture_handle: graphics_hardware_interface::ImageHandle, f: impl FnOnce(&mut [u8])) {
			let image = &mut self.images[texture_handle.0 as usize];
			let Some(staging) = image.staging.as_mut() else {
				return;
			};

			f(staging);

			let Some(bytes_per_pixel) = utils::bytes_per_pixel(image.format) else {
				return;
			};

			let extent = image.extent;
			let width = extent.width() as usize;
			let height = extent.height() as usize;
			let bytes_per_row = width * bytes_per_pixel;

			let region = mtl::MTLRegion {
				origin: mtl::MTLOrigin { x: 0, y: 0, z: 0 },
				size: mtl::MTLSize {
					width: width as _,
					height: height as _,
					depth: 1,
				},
			};

			let staging_ptr = NonNull::new(staging.as_ptr() as *mut std::ffi::c_void)
				.expect("Texture staging pointer was null. The most likely cause is a zero-sized texture.");
			unsafe {
				image
					.texture
					.replaceRegion_mipmapLevel_withBytes_bytesPerRow(region, 0, staging_ptr, bytes_per_row as _);
			}
		}

		pub fn build_image(&mut self, builder: image_builder::Builder) -> graphics_hardware_interface::ImageHandle {
			let layers = builder.array_layers.map(|l| l.get()).unwrap_or(1);
			let image_handle = self.create_image_internal(
				None,
				builder.get_name(),
				builder.extent,
				builder.format,
				builder.resource_uses,
				builder.device_accesses,
				layers,
			);
			graphics_hardware_interface::ImageHandle(image_handle.0)
		}

		pub fn build_sampler(&mut self, builder: sampler_builder::Builder) -> graphics_hardware_interface::SamplerHandle {
			let descriptor = mtl::MTLSamplerDescriptor::new();
			descriptor.setMinFilter(utils::sampler_min_mag_filter(builder.filtering_mode));
			descriptor.setMagFilter(utils::sampler_min_mag_filter(builder.filtering_mode));
			descriptor.setMipFilter(utils::sampler_mip_filter(builder.mip_map_mode));
			descriptor.setSAddressMode(utils::sampler_address_mode(builder.addressing_mode));
			descriptor.setTAddressMode(utils::sampler_address_mode(builder.addressing_mode));
			descriptor.setRAddressMode(utils::sampler_address_mode(builder.addressing_mode));
			descriptor.setLodMinClamp(builder.min_lod);
			descriptor.setLodMaxClamp(builder.max_lod);

			if let Some(anisotropy) = builder.anisotropy {
				descriptor.setMaxAnisotropy(anisotropy as _);
			}

			let sampler_state = self
				.device
				.newSamplerStateWithDescriptor(&descriptor)
				.expect("Metal sampler creation failed. The most likely cause is that the device is out of sampler resources.");
			self.samplers.push(super::sampler::Sampler { sampler: sampler_state });
			graphics_hardware_interface::SamplerHandle((self.samplers.len() - 1) as u64)
		}

		pub fn create_acceleration_structure_instance_buffer(
			&mut self,
			name: Option<&str>,
			max_instance_count: u32,
		) -> graphics_hardware_interface::BaseBufferHandle {
			let size = max_instance_count as usize * std::mem::size_of::<mtl::MTLAccelerationStructureInstanceDescriptor>();
			let handle = self.create_buffer_internal(
				None,
				name,
				size,
				graphics_hardware_interface::Uses::AccelerationStructure,
				graphics_hardware_interface::DeviceAccesses::DeviceOnly,
			);
			graphics_hardware_interface::BaseBufferHandle(handle.0)
		}

		pub fn create_top_level_acceleration_structure(
			&mut self,
			_name: Option<&str>,
			_max_instance_count: u32,
		) -> graphics_hardware_interface::TopLevelAccelerationStructureHandle {
			self.acceleration_structures.push(AccelerationStructure {
				structure: None,
				buffer: None,
			});
			// TODO: Build MTLAccelerationStructure and backing buffer.
			graphics_hardware_interface::TopLevelAccelerationStructureHandle((self.acceleration_structures.len() - 1) as u64)
		}

		pub fn create_bottom_level_acceleration_structure(
			&mut self,
			_description: &graphics_hardware_interface::BottomLevelAccelerationStructure,
		) -> graphics_hardware_interface::BottomLevelAccelerationStructureHandle {
			self.acceleration_structures.push(AccelerationStructure {
				structure: None,
				buffer: None,
			});
			// TODO: Build MTLAccelerationStructure for mesh or AABB.
			graphics_hardware_interface::BottomLevelAccelerationStructureHandle((self.acceleration_structures.len() - 1) as u64)
		}

		pub fn write(&mut self, descriptor_set_writes: &[graphics_hardware_interface::DescriptorWrite]) {
			for write in descriptor_set_writes {
				let binding_handle = binding::DescriptorSetBindingHandle(write.binding_handle.0);
				let array_element = write.array_element;

				match write.descriptor {
					graphics_hardware_interface::Descriptor::Buffer { handle, size } => {
						self.update_descriptor_for_binding(
							binding_handle,
							Descriptor::Buffer {
								buffer: buffer::BufferHandle(handle.0),
								size,
							},
							array_element,
						);
					}
					graphics_hardware_interface::Descriptor::Image { handle, layout } => {
						self.update_descriptor_for_binding(
							binding_handle,
							Descriptor::Image {
								image: image::ImageHandle(handle.0),
								layout,
							},
							array_element,
						);
					}
					graphics_hardware_interface::Descriptor::CombinedImageSampler {
						image_handle,
						sampler_handle,
						layout,
						..
					} => {
						self.update_descriptor_for_binding(
							binding_handle,
							Descriptor::CombinedImageSampler {
								image: image::ImageHandle(image_handle.0),
								sampler: sampler::SamplerHandle(sampler_handle.0),
								layout,
							},
							array_element,
						);
					}
					graphics_hardware_interface::Descriptor::Sampler(handle) => {
						self.update_descriptor_for_binding(
							binding_handle,
							Descriptor::Sampler {
								sampler: sampler::SamplerHandle(handle.0),
							},
							array_element,
						);
					}
					_ => {
						// TODO: Implement descriptor writes for Metal acceleration structures and swapchains.
					}
				}
			}
		}

		pub fn write_instance(
			&mut self,
			_instances_buffer_handle: graphics_hardware_interface::BaseBufferHandle,
			_instance_index: usize,
			_transform: [[f32; 4]; 3],
			_custom_index: u16,
			_mask: u8,
			_sbt_record_offset: usize,
			_acceleration_structure: graphics_hardware_interface::BottomLevelAccelerationStructureHandle,
		) {
			// TODO: Populate MTLAccelerationStructureInstanceDescriptor buffer.
		}

		pub fn write_sbt_entry(
			&mut self,
			_sbt_buffer_handle: graphics_hardware_interface::BaseBufferHandle,
			_sbt_record_offset: usize,
			_pipeline_handle: graphics_hardware_interface::PipelineHandle,
			_shader_handle: graphics_hardware_interface::ShaderHandle,
		) {
			// TODO: Metal ray tracing shader binding table mapping.
		}

		pub fn bind_to_window(
			&mut self,
			window_os_handles: &window::Handles,
			_presentation_mode: graphics_hardware_interface::PresentationModes,
			fallback_extent: Extent,
		) -> graphics_hardware_interface::SwapchainHandle {
			let layer = CAMetalLayer::new();
			layer.setDevice(Some(&self.device));
			layer.setPixelFormat(mtl::MTLPixelFormat::BGRA8Unorm);
			layer.setFramebufferOnly(false);

			window_os_handles.view.setWantsLayer(true);
			window_os_handles.view.setLayer(Some(layer.as_super()));
			let extent = update_layer_extent(&layer, &window_os_handles.view);

			self.swapchains.push(swapchain::Swapchain::new(
				layer,
				window_os_handles.view.clone(),
				if extent.width() == 0 || extent.height() == 0 {
					fallback_extent
				} else {
					extent
				},
				mtl::MTLPixelFormat::BGRA8Unorm,
			));
			graphics_hardware_interface::SwapchainHandle((self.swapchains.len() - 1) as u64)
		}

		pub fn get_image_data<'a>(&'a self, texture_copy_handle: graphics_hardware_interface::TextureCopyHandle) -> &'a [u8] {
			self.texture_copies
				.get(texture_copy_handle.0 as usize)
				.map(|data| data.as_slice())
				.unwrap_or(&[])
		}

		pub fn create_synchronizer(
			&mut self,
			_name: Option<&str>,
			signaled: bool,
		) -> graphics_hardware_interface::SynchronizerHandle {
			self.synchronizers.push(synchronizer::Synchronizer { next: None, signaled });
			graphics_hardware_interface::SynchronizerHandle((self.synchronizers.len() - 1) as u64)
		}

		pub fn start_frame<'a>(
			&'a mut self,
			index: u32,
			_synchronizer_handle: graphics_hardware_interface::SynchronizerHandle,
		) -> super::Frame<'a> {
			let frame_key = graphics_hardware_interface::FrameKey {
				frame_index: index,
				sequence_index: (index % self.frames as u32) as u8,
			};
			super::Frame::new(self, frame_key)
		}

		pub fn resize_buffer(&mut self, buffer_handle: graphics_hardware_interface::BaseBufferHandle, size: usize) {
			let handle = buffer::BufferHandle(buffer_handle.0);
			let buffer = &self.buffers[handle.0 as usize];

			if buffer.size >= size {
				return;
			}

			let name = buffer.buffer.label().map(|l| l.to_string());
			let new_handle = self.create_buffer_internal(None, name.as_deref(), size, buffer.uses, buffer.access);
			self.buffers[handle.0 as usize] = self.buffers[new_handle.0 as usize].clone();
		}

		pub fn start_frame_capture(&self) {
			// TODO: Hook into MTLCaptureManager when needed.
		}

		pub fn end_frame_capture(&self) {
			// TODO: Hook into MTLCaptureManager when needed.
		}

		pub fn wait(&self) {
			// TODO: Track pending command buffers and wait for completion.
		}
	}
}

pub mod frame {
	use super::*;

	pub struct Frame<'a> {
		frame_key: graphics_hardware_interface::FrameKey,
		device: &'a mut device::Device,
	}

	impl<'a> Frame<'a> {
		pub fn new(device: &'a mut device::Device, frame_key: graphics_hardware_interface::FrameKey) -> Self {
			Self { frame_key, device }
		}
	}

	impl Frame<'_> {
		pub fn get_mut_dynamic_buffer_slice<'a, T: Copy>(
			&'a self,
			buffer_handle: graphics_hardware_interface::DynamicBufferHandle<T>,
		) -> &'a mut T {
			let handles = buffer::BufferHandle(buffer_handle.0).get_all(&self.device.buffers);
			let handle = handles[self.frame_key.sequence_index as usize];
			let buffer = &self.device.buffers[handle.0 as usize];

			unsafe { &mut *(buffer.pointer as *mut T) }
		}

		pub fn resize_image(&mut self, image_handle: graphics_hardware_interface::ImageHandle, extent: Extent) {
			let handles = image::ImageHandle(image_handle.0).get_all(&self.device.images);
			let handle = handles[self.frame_key.sequence_index as usize];
			let image = &self.device.images[handle.0 as usize];

			if image.extent == extent {
				return;
			}

			let new_handle = self.device.create_image_internal(
				None,
				None,
				extent,
				image.format,
				image.uses,
				image.access,
				image.array_layers,
			);
			self.device.images[handle.0 as usize] = self.device.images[new_handle.0 as usize].clone();
			// TODO: Update descriptor references for resized image.
		}

		pub fn create_command_buffer_recording<'a>(
			&'a mut self,
			command_buffer_handle: graphics_hardware_interface::CommandBufferHandle,
		) -> super::CommandBufferRecording<'a> {
			self.device.create_command_buffer_recording(command_buffer_handle)
		}

		pub fn acquire_swapchain_image(
			&mut self,
			swapchain_handle: graphics_hardware_interface::SwapchainHandle,
		) -> (graphics_hardware_interface::PresentKey, Extent) {
			let swapchain = &mut self.device.swapchains[swapchain_handle.0 as usize];
			swapchain.extent = update_layer_extent(&swapchain.layer, &swapchain.view);
			let drawable = swapchain.layer.nextDrawable().expect(
				"Failed to acquire Metal drawable. The most likely cause is that the layer has no available drawables.",
			);
			let index = swapchain.store_drawable(drawable);

			let present_key = graphics_hardware_interface::PresentKey {
				image_index: index,
				sequence_index: self.frame_key.sequence_index,
				swapchain: swapchain_handle,
			};
			(present_key, swapchain.extent)
		}

		pub fn device(&mut self) -> &mut device::Device {
			self.device
		}
	}
}

pub mod command_buffer {
	use std::ptr::NonNull;

	use objc2_foundation::NSString;
	use objc2_metal::{MTLCommandBuffer, MTLTexture};

	use super::*;
	use crate::command_buffer::{
		BoundComputePipelineMode, BoundPipelineLayoutMode, BoundRasterizationPipelineMode, BoundRayTracingPipelineMode,
		CommandBufferRecordable, CommonCommandBufferMode, RasterizationRenderPassMode,
	};

	pub struct CommandBufferRecording<'a> {
		device: &'a mut device::Device,
		command_buffer: Retained<ProtocolObject<dyn mtl::MTLCommandBuffer>>,
		present_drawables: Vec<Retained<ProtocolObject<dyn CAMetalDrawable>>>,
		bound_pipeline_layout: Option<graphics_hardware_interface::PipelineLayoutHandle>,
		bound_pipeline: Option<graphics_hardware_interface::PipelineHandle>,
	}

	impl<'a> CommandBufferRecording<'a> {
		pub fn new(
			device: &'a mut device::Device,
			_command_buffer_handle: graphics_hardware_interface::CommandBufferHandle,
			command_buffer: Retained<ProtocolObject<dyn mtl::MTLCommandBuffer>>,
			_frame_key: Option<graphics_hardware_interface::FrameKey>,
		) -> Self {
			Self {
				device,
				command_buffer,
				present_drawables: Vec::new(),
				bound_pipeline_layout: None,
				bound_pipeline: None,
			}
		}

		fn take_drawable(
			&mut self,
			present_key: graphics_hardware_interface::PresentKey,
		) -> Option<Retained<ProtocolObject<dyn CAMetalDrawable>>> {
			let swapchain = &mut self.device.swapchains[present_key.swapchain.0 as usize];
			swapchain.take_drawable(present_key.image_index)
		}
	}

	impl CommandBufferRecordable for CommandBufferRecording<'_> {
		fn sync_buffers(&mut self) {
			// TODO: Track pending buffer uploads and encode blit operations.
		}

		fn sync_textures(&mut self) {
			// TODO: Track pending texture uploads and encode blit operations.
		}

		fn build_top_level_acceleration_structure(
			&mut self,
			_acceleration_structure_build: &graphics_hardware_interface::TopLevelAccelerationStructureBuild,
		) {
			// TODO: Map acceleration structure build to MTLAccelerationStructureCommandEncoder.
		}

		fn build_bottom_level_acceleration_structures(
			&mut self,
			_acceleration_structure_builds: &[graphics_hardware_interface::BottomLevelAccelerationStructureBuild],
		) {
			// TODO: Map acceleration structure build to MTLAccelerationStructureCommandEncoder.
		}

		fn start_render_pass(
			&mut self,
			_extent: Extent,
			_attachments: &[graphics_hardware_interface::AttachmentInformation],
		) -> &mut impl RasterizationRenderPassMode {
			// TODO: Create MTLRenderCommandEncoder when pipeline setup is implemented.
			self
		}

		fn clear_images<I: graphics_hardware_interface::ImageHandleLike>(
			&mut self,
			_textures: &[(I, graphics_hardware_interface::ClearValue)],
		) {
			// TODO: Encode blit clears for textures.
		}

		fn clear_buffers(&mut self, _buffer_handles: &[graphics_hardware_interface::BaseBufferHandle]) {
			// TODO: Encode fillBuffer on MTLBlitCommandEncoder.
		}

		fn transfer_textures(
			&mut self,
			texture_handles: &[impl graphics_hardware_interface::ImageHandleLike],
		) -> Vec<graphics_hardware_interface::TextureCopyHandle> {
			texture_handles
				.iter()
				.map(|handle| {
					self.device
						.copy_texture_to_cpu(image::ImageHandle(handle.into_image_handle().0))
				})
				.collect()
		}

		fn write_image_data(
			&mut self,
			image_handle: impl graphics_hardware_interface::ImageHandleLike,
			data: &[graphics_hardware_interface::RGBAu8],
		) {
			let image_handle = image_handle.into_image_handle();
			let image = &mut self.device.images[image_handle.0 as usize];
			let Some(staging) = image.staging.as_mut() else {
				return;
			};
			let bytes = unsafe {
				std::slice::from_raw_parts(
					data.as_ptr() as *const u8,
					data.len() * std::mem::size_of::<graphics_hardware_interface::RGBAu8>(),
				)
			};
			let length = staging.len().min(bytes.len());
			staging[..length].copy_from_slice(&bytes[..length]);

			let Some(bytes_per_pixel) = utils::bytes_per_pixel(image.format) else {
				return;
			};
			let width = image.extent.width() as usize;
			let height = image.extent.height() as usize;
			let bytes_per_row = width * bytes_per_pixel;
			let region = mtl::MTLRegion {
				origin: mtl::MTLOrigin { x: 0, y: 0, z: 0 },
				size: mtl::MTLSize {
					width: width as _,
					height: height as _,
					depth: 1,
				},
			};
			let staging_ptr = NonNull::new(staging.as_ptr() as *mut std::ffi::c_void)
				.expect("Texture staging pointer was null. The most likely cause is a zero-sized texture.");

			unsafe {
				image
					.texture
					.replaceRegion_mipmapLevel_withBytes_bytesPerRow(region, 0, staging_ptr, bytes_per_row as _);
			}
		}

		fn blit_image(
			&mut self,
			_source_image: impl graphics_hardware_interface::ImageHandleLike,
			_source_layout: graphics_hardware_interface::Layouts,
			_destination_image: impl graphics_hardware_interface::ImageHandleLike,
			_destination_layout: graphics_hardware_interface::Layouts,
		) {
			// TODO: Encode MTLBlitCommandEncoder copyFromTexture.
		}

		fn copy_to_swapchain(
			&mut self,
			_source_texture_handle: impl graphics_hardware_interface::ImageHandleLike,
			_present_key: graphics_hardware_interface::PresentKey,
			_swapchain_handle: graphics_hardware_interface::SwapchainHandle,
		) {
			// TODO: Render/copy source texture into swapchain drawable.
		}

		fn bind_vertex_buffers(&mut self, _buffer_descriptors: &[graphics_hardware_interface::BufferDescriptor]) {
			// TODO: Bind vertex buffers on MTLRenderCommandEncoder.
		}

		fn bind_index_buffer(&mut self, _buffer_descriptor: &graphics_hardware_interface::BufferDescriptor) {
			// TODO: Bind index buffer on MTLRenderCommandEncoder.
		}

		fn present(&mut self, present_key: graphics_hardware_interface::PresentKey) {
			if let Some(drawable) = self.take_drawable(present_key) {
				self.present_drawables.push(drawable);
			}
		}

		fn execute(
			self,
			_wait_for_synchronizer_handles: &[graphics_hardware_interface::SynchronizerHandle],
			_signal_synchronizer_handles: &[graphics_hardware_interface::SynchronizerHandle],
			_presentations: &[graphics_hardware_interface::PresentKey],
			_execution_synchronizer_handle: graphics_hardware_interface::SynchronizerHandle,
		) {
			for drawable in &self.present_drawables {
				let drawable_ref: &ProtocolObject<dyn mtl::MTLDrawable> = drawable.as_ref();
				self.command_buffer.presentDrawable(drawable_ref);
			}

			self.command_buffer.commit();
			self.command_buffer.waitUntilCompleted();
		}
	}

	impl CommonCommandBufferMode for CommandBufferRecording<'_> {
		fn bind_compute_pipeline(
			&mut self,
			pipeline_handle: graphics_hardware_interface::PipelineHandle,
		) -> &mut impl BoundComputePipelineMode {
			self.bound_pipeline = Some(pipeline_handle);
			self.bound_pipeline_layout = Some(self.device.pipelines[pipeline_handle.0 as usize].layout);
			self
		}

		fn bind_ray_tracing_pipeline(
			&mut self,
			pipeline_handle: graphics_hardware_interface::PipelineHandle,
		) -> &mut impl BoundRayTracingPipelineMode {
			self.bound_pipeline = Some(pipeline_handle);
			self.bound_pipeline_layout = Some(self.device.pipelines[pipeline_handle.0 as usize].layout);
			self
		}

		fn start_region(&self, name: &str) {
			self.command_buffer.pushDebugGroup(&NSString::from_str(name));
		}

		fn end_region(&self) {
			self.command_buffer.popDebugGroup();
		}

		fn region(&mut self, name: &str, f: impl FnOnce(&mut Self)) {
			self.start_region(name);
			f(self);
			self.end_region();
		}
	}

	impl RasterizationRenderPassMode for CommandBufferRecording<'_> {
		fn bind_raster_pipeline(
			&mut self,
			pipeline_handle: graphics_hardware_interface::PipelineHandle,
		) -> &mut impl BoundRasterizationPipelineMode {
			self.bound_pipeline = Some(pipeline_handle);
			self.bound_pipeline_layout = Some(self.device.pipelines[pipeline_handle.0 as usize].layout);
			self
		}

		fn end_render_pass(&mut self) {
			// TODO: End current render command encoder.
		}
	}

	impl BoundPipelineLayoutMode for CommandBufferRecording<'_> {
		fn bind_descriptor_sets(&mut self, _sets: &[graphics_hardware_interface::DescriptorSetHandle]) -> &mut Self {
			// TODO: Map descriptor sets to Metal argument buffers and encoder bindings.
			self
		}

		fn write_push_constant<T: Copy + 'static>(&mut self, _offset: u32, _data: T)
		where
			[(); std::mem::size_of::<T>()]: Sized,
		{
			// TODO: Map push constants to MTLBuffer/bytes per stage.
		}
	}

	impl BoundRasterizationPipelineMode for CommandBufferRecording<'_> {
		fn draw_mesh(&mut self, _mesh_handle: &graphics_hardware_interface::MeshHandle) {
			// TODO: Issue draw call using mesh buffers.
		}

		fn draw(&mut self, _vertex_count: u32, _instance_count: u32, _first_vertex: u32, _first_instance: u32) {
			// TODO: Issue non-indexed draw call.
		}

		fn draw_indexed(
			&mut self,
			_index_count: u32,
			_instance_count: u32,
			_first_index: u32,
			_vertex_offset: i32,
			_first_instance: u32,
		) {
			// TODO: Issue indexed draw call.
		}

		fn dispatch_meshes(&mut self, _x: u32, _y: u32, _z: u32) {
			// TODO: Map mesh shading to Metal mesh shaders when supported.
		}
	}

	impl BoundComputePipelineMode for CommandBufferRecording<'_> {
		fn dispatch(&mut self, _dispatch: graphics_hardware_interface::DispatchExtent) {
			// TODO: Encode dispatch on MTLComputeCommandEncoder.
		}

		fn indirect_dispatch<const N: usize>(
			&mut self,
			_buffer: graphics_hardware_interface::BufferHandle<[(u32, u32, u32); N]>,
			_entry_index: usize,
		) {
			// TODO: Encode indirect dispatch.
		}
	}

	impl BoundRayTracingPipelineMode for CommandBufferRecording<'_> {
		fn trace_rays(&mut self, _binding_tables: graphics_hardware_interface::BindingTables, _x: u32, _y: u32, _z: u32) {
			// TODO: Encode Metal ray tracing dispatch.
		}
	}
}

pub use self::command_buffer::*;
pub(crate) use self::descriptor_set::*;
pub use self::device::*;
pub use self::frame::*;
pub use self::instance::*;
