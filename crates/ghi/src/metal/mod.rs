#[link(name = "CoreGraphics", kind = "framework")]
unsafe extern "C" {}

use std::ptr::NonNull;
use std::sync::atomic::AtomicU64;

use ::utils::Extent;
use ::utils::hash::HashMap;
use objc2::rc::Retained;
use objc2::runtime::ProtocolObject;
use objc2_app_kit::NSView;
use objc2_foundation::NSRange;
use objc2_metal as mtl;
use objc2_metal::MTL4CommandBuffer as _;
use objc2_metal::MTL4CounterHeap as _;
use objc2_metal::MTLAccelerationStructure as _;
use objc2_metal::MTLDevice as _;
use objc2_metal::MTLResource as _;
use objc2_quartz_core::{CAMetalDrawable, CAMetalLayer};
use smallvec::SmallVec;

use crate::buffer::BufferHandle;
use crate::context::Context as _;
use crate::graphics_hardware_interface;
use crate::image::ImageHandle;
use crate::{MAX_FRAMES_IN_FLIGHT, PrivateHandles};

pub(crate) mod acceleration_structures;
mod allocator;
mod display_link;
mod io;
mod pipeline;
pub mod queue;
mod resources;
mod state;
mod synchronization;
mod upscaler;
pub(crate) use io::write_compressed_file;
pub use io::{ResourceIoQueue, ResourceIoTicket};
pub(crate) mod utils {
	use objc2_metal as mtl;
	use utils::Extent;

	use crate::{FilteringModes, Formats, SamplerAddressingModes, SamplingReductionModes, Uses};

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
			Formats::Depth16 => mtl::MTLPixelFormat::Depth16Unorm,
			Formats::Depth32 => mtl::MTLPixelFormat::Depth32Float,
			Formats::U32 => mtl::MTLPixelFormat::R32Uint,

			Formats::BC5 => mtl::MTLPixelFormat::BC5_RGUnorm,
			Formats::BC5SNORM => mtl::MTLPixelFormat::BC5_RGSnorm,
			Formats::BC7 => mtl::MTLPixelFormat::BC7_RGBAUnorm,
			Formats::BC7SRGB => mtl::MTLPixelFormat::BC7_RGBAUnorm_sRGB,
		}
	}

	/// Selects the storage of a buffer: private when only the GPU accesses it, CPU-coherent shared storage otherwise.
	pub(crate) fn buffer_options(host_access: crate::HostAccess) -> mtl::MTLResourceOptions {
		match host_access {
			crate::HostAccess::None => mtl::MTLResourceOptions::StorageModePrivate,
			crate::HostAccess::Write | crate::HostAccess::Read => mtl::MTLResourceOptions::StorageModeShared,
		}
	}

	pub(crate) fn texture_usage_from_uses(uses: Uses) -> mtl::MTLTextureUsage {
		let mut usage = mtl::MTLTextureUsage::empty();

		if uses.intersects(Uses::Image | Uses::Storage) {
			usage |= mtl::MTLTextureUsage::ShaderRead;
		}

		if uses.contains(Uses::Storage) {
			usage |= mtl::MTLTextureUsage::ShaderWrite;
		}

		// Metal rejects texture atomics on a texture created without this usage.
		if uses.contains(Uses::Atomic) {
			usage |= mtl::MTLTextureUsage::ShaderAtomic;
		}

		// Metal clears images by loading them as render-pass attachments.
		if uses.intersects(Uses::RenderTarget | Uses::DepthStencil | Uses::Clear) {
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

	pub(crate) fn sampler_reduction_mode(mode: SamplingReductionModes) -> mtl::MTLSamplerReductionMode {
		match mode {
			SamplingReductionModes::WeightedAverage => mtl::MTLSamplerReductionMode::WeightedAverage,
			SamplingReductionModes::Min => mtl::MTLSamplerReductionMode::Minimum,
			SamplingReductionModes::Max => mtl::MTLSamplerReductionMode::Maximum,
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

	/// Copies `rows` rows of `row_bytes` bytes between two row pitches, such as compact CPU data and a padded copy buffer.
	///
	/// # Safety
	///
	/// For every row `r`, `source + r * source_pitch` must address `row_bytes` readable bytes and
	/// `destination + r * destination_pitch` must address `row_bytes` writable bytes that do not overlap them.
	pub(crate) unsafe fn copy_rows(
		source: *const u8,
		source_pitch: usize,
		destination: *mut u8,
		destination_pitch: usize,
		row_bytes: usize,
		rows: usize,
	) {
		// Rows packed on both sides form one contiguous range, which one copy moves faster than a copy per row.
		if source_pitch == row_bytes && destination_pitch == row_bytes {
			// SAFETY: With both pitches equal to the row size, the caller's rows join into `rows * row_bytes` contiguous
			// bytes on each side.
			unsafe { std::ptr::copy_nonoverlapping(source, destination, rows * row_bytes) };
			return;
		}
		for row in 0..rows {
			// SAFETY: The caller guarantees both rows lie inside distinct live allocations.
			unsafe {
				std::ptr::copy_nonoverlapping(
					source.add(row * source_pitch),
					destination.add(row * destination_pitch),
					row_bytes,
				);
			}
		}
	}

	/// Converts an extent to a Metal size, raising each empty dimension to one.
	pub(crate) fn mtl_size(extent: Extent) -> mtl::MTLSize {
		mtl::MTLSize {
			width: extent.width().max(1) as _,
			height: extent.height().max(1) as _,
			depth: extent.depth().max(1) as _,
		}
	}

	/// Maps a GHI index data type to the Metal index type that indexed draws and triangle acceleration structures read.
	pub(crate) fn to_index_type(data_type: crate::DataTypes) -> mtl::MTLIndexType {
		match data_type {
			crate::DataTypes::U16 => mtl::MTLIndexType::UInt16,
			crate::DataTypes::U32 => mtl::MTLIndexType::UInt32,
			_ => panic!(
				"Unsupported Metal index type. The most likely cause is that an index buffer or triangle geometry used a DataTypes value other than U16 or U32.",
			),
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

	pub(crate) fn load_action(load: crate::LoadOp) -> mtl::MTLLoadAction {
		match load {
			crate::LoadOp::Load => mtl::MTLLoadAction::Load,
			crate::LoadOp::Clear(_) => mtl::MTLLoadAction::Clear,
			crate::LoadOp::Discard => mtl::MTLLoadAction::DontCare,
		}
	}

	pub(crate) fn store_action(store: crate::StoreOp) -> mtl::MTLStoreAction {
		match store {
			crate::StoreOp::Store => mtl::MTLStoreAction::Store,
			crate::StoreOp::Discard => mtl::MTLStoreAction::DontCare,
		}
	}

	pub(crate) fn clear_color(clear: crate::ClearValue) -> mtl::MTLClearColor {
		let (red, green, blue, alpha) = match clear {
			crate::ClearValue::None => (0.0, 0.0, 0.0, 0.0),
			crate::ClearValue::Color(color) => (color.r as f64, color.g as f64, color.b as f64, color.a as f64),
			crate::ClearValue::Integer(r, g, b, a) => (r as f64, g as f64, b as f64, a as f64),
			crate::ClearValue::Depth(depth) => (depth as f64, 0.0, 0.0, 0.0),
		};
		mtl::MTLClearColor { red, green, blue, alpha }
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

	/// Maps portable triangle fill behavior to Metal's encoder state.
	pub(crate) fn fill_mode(fill_mode: crate::pipelines::raster::FillMode) -> mtl::MTLTriangleFillMode {
		match fill_mode {
			crate::pipelines::raster::FillMode::Solid => mtl::MTLTriangleFillMode::Fill,
			crate::pipelines::raster::FillMode::Wireframe => mtl::MTLTriangleFillMode::Lines,
		}
	}
}

pub(crate) use pipeline::*;
pub(crate) use resources::*;
pub use state::Image;
pub(crate) use state::{
	ArgumentBufferSnapshot, ArgumentResources, Buffer, DescriptorSet, GroupSlot, ImageDescription, Swapchain, Synchronizer,
};

pub mod command_buffer;
pub mod context;
pub mod device;
pub mod factory;
pub mod frame;
pub mod instance;

pub use self::command_buffer::*;
pub use self::context::*;
pub use self::device::Device;
pub use self::factory::{ComputePipeline, DetachedImage, Factory, RayTracingPipeline, SharedBuffer};
pub use self::frame::*;
pub use self::instance::*;
