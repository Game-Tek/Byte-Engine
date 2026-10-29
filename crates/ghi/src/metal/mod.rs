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
// Tests build resources through the creation trait the context implements.
#[cfg(test)]
use crate::context::ContextCreate as _;
use crate::graphics_hardware_interface;
use crate::image::ImageHandle;
use crate::{MAX_FRAMES_IN_FLIGHT, PrivateHandles};

pub(crate) mod acceleration_structures;
mod allocator;
mod io;
mod pipeline;
pub mod queue;
#[cfg(test)]
mod ray_tracing_spikes;
mod resources;
mod state;
mod synchronization;
pub(crate) use io::write_compressed_file;
pub use io::{ResourceIoQueue, ResourceIoTicket};
pub(crate) mod utils {
	use objc2_metal as mtl;
	use utils::Extent;

	use crate::{DeviceAccesses, FilteringModes, Formats, SamplerAddressingModes, SamplingReductionModes, Uses};

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

	/// Selects the storage of a buffer or texture, with default CPU caching and hazard tracking.
	pub(crate) fn resource_options_from_access(access: DeviceAccesses) -> mtl::MTLResourceOptions {
		if access == DeviceAccesses::DeviceOnly {
			mtl::MTLResourceOptions::StorageModePrivate
		} else {
			// Metal 4 has no managed-resource synchronization commands. Shared storage is CPU-coherent after queue completion.
			mtl::MTLResourceOptions::StorageModeShared
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

	/// Returns the compact row pitch, row count, and image pitch of one texture subresource.
	pub(crate) fn texture_upload_layout(format: Formats, extent: Extent) -> (usize, usize, usize) {
		format.copy_layout(extent).expect(
			"Metal texture layout overflowed. The most likely cause is an image extent too large for the host address space.",
		)
	}

	/// Returns the row and image pitches Metal copies of one texture subresource use, padded for the GPU.
	pub(crate) fn texture_copy_pitches(bytes_per_row: usize, row_count: usize) -> (usize, usize) {
		crate::aligned_copy_pitches(bytes_per_row, row_count).expect(
			"Metal texture copy pitch overflowed. The most likely cause is an image extent too large for the host address space.",
		)
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

	#[cfg(test)]
	mod tests {
		use super::*;

		#[test]
		fn upload_layout_preserves_bc_block_rows_and_minimum_extent() {
			let extent = Extent::rectangle(5, 7);

			let (bytes_per_row, row_count, bytes_per_image) = texture_upload_layout(Formats::BC7, extent);

			assert_eq!(bytes_per_row, 2 * 16);
			assert_eq!(row_count, 2);
			assert_eq!(bytes_per_image, 2 * 2 * 16);
			assert_eq!(texture_upload_layout(Formats::RGBA8UNORM, Extent::rectangle(0, 0)), (4, 1, 4));
		}
	}
}

pub(crate) use pipeline::*;
pub(crate) use resources::*;
pub use state::Image;
pub(crate) use state::{
	ArgumentBufferSnapshot, ArgumentResources, Buffer, DescriptorSet, GroupSlot, ImageDescription, Swapchain, Synchronizer,
};

/// Creates a real Metal context with one queue for the requested workloads, for tests that run GPU work.
#[cfg(test)]
pub(crate) fn test_context(workloads: crate::WorkloadTypes) -> (Context, crate::QueueHandle) {
	use crate::device::Device as _;

	let features = crate::device::Features::new().debug_labels(true).ray_tracing(true);
	let mut instance = Instance::new(features)
		.expect("Failed to create a Metal test instance. The most likely cause is that no Metal device is available.");
	let mut queue_handle = None;
	let context = instance
		.create_device(features, &mut [(crate::QueueSelection::new(workloads), &mut queue_handle)])
		.expect("Failed to create a Metal test device. The most likely cause is that the device lacks the requested queue.")
		.create_context()
		.expect("Failed to create a Metal test context. The most likely cause is unavailable Metal 4 support.");
	let queue_handle = queue_handle
		.expect("Missing Metal test queue. The most likely cause is that device selection did not return the requested queue.");
	(context, queue_handle)
}

#[cfg(test)]
mod flat_binding_tests {
	use super::*;
	use crate::{
		AccessPolicies,
		command_buffer::{BoundComputePipelineMode as _, BoundPipelineLayoutMode as _, CommonCommandBufferMode as _},
		queue::{FrameRequest, Queue as _, QueueExecution as _},
		shader::{ResourceKind, ResourceSlot},
	};

	fn resource(slot: u32, kind: ResourceKind, count: u32, access: AccessPolicies) -> crate::shader::ShaderResourceDescriptor {
		crate::shader::ShaderResourceDescriptor::new(ResourceSlot::new(slot), kind, count, access)
	}

	/// Builds a layout and declares it to a shader that uses every resource of it.
	fn layout(
		name: &str,
		resources: impl IntoIterator<Item = crate::shader::ShaderResourceDescriptor>,
	) -> (crate::DescriptorSetLayout, [crate::ShaderDescriptorSet; 1]) {
		let layout = crate::DescriptorSetLayout::new(name, resources);
		(layout.clone(), [crate::ShaderDescriptorSet::new(layout)])
	}

	/// Exercises the production material ordering where scalar resources follow the bindless texture table.
	#[test]
	fn retained_material_resources_after_bindless_array_reach_metal() {
		use objc2_metal::MTLComputePipelineState as _;

		const TEXTURES_SLOT: ResourceSlot = ResourceSlot::new(9);
		const MATERIAL_SLOT: ResourceSlot = ResourceSlot::new(1046);
		const AO_SLOT: ResourceSlot = ResourceSlot::new(1051);
		const OUTPUT_SLOT: ResourceSlot = ResourceSlot::new(1054);
		const TEXTURE_INDEX: u32 = 7;

		let source = r#"
			#include <metal_stdlib>
			using namespace metal;

			struct _set0 {
				texture2d<float> textures [[id(0)]][1024];
				sampler textures_sampler [[id(1024)]][1024];
				constant uint* material_texture_index [[id(2048)]];
			};
			struct _set1 {
				texture2d<float> ao [[id(0)]];
				sampler ao_sampler [[id(1)]];
				device uint* output [[id(2)]];
			};

			kernel void besl_main(
				uint2 gid [[thread_position_in_grid]],
				constant _set0& scene [[buffer(16)]],
				constant _set1& material [[buffer(17)]]) {
				if (gid.x != 0 || gid.y != 0) { return; }
				uint texture_index = scene.material_texture_index[0];
				float albedo = scene.textures[texture_index].sample(
					scene.textures_sampler[texture_index], float2(0.5)).r;
				float ao = material.ao.sample(material.ao_sampler, float2(0.5)).r;
				material.output[0] = uint(round(albedo * 255.0)) | (uint(round(ao * 255.0)) << 8);
			}
		"#;

		let (mut context, queue_handle) = test_context(crate::WorkloadTypes::COMPUTE);

		let texture_resource = resource(
			TEXTURES_SLOT.index(),
			ResourceKind::CombinedImageSampler,
			1024,
			AccessPolicies::READ,
		);
		let material_resource = resource(MATERIAL_SLOT.index(), ResourceKind::StorageBuffer, 1, AccessPolicies::READ);
		let ao_resource = resource(AO_SLOT.index(), ResourceKind::CombinedImageSampler, 1, AccessPolicies::READ);
		let output_resource = resource(OUTPUT_SLOT.index(), ResourceKind::StorageBuffer, 1, AccessPolicies::WRITE);
		let (scene_layout, [scene]) = layout("Scene", [material_resource, texture_resource]);
		let (material_layout, [material]) = layout("Material", [output_resource, ao_resource]);
		let shader = context
			.create_shader(
				Some("Retained Material Binding Probe"),
				crate::shader::Sources::MTL {
					source,
					entry_point: "besl_main",
				},
				crate::ShaderTypes::Compute,
				[scene, material],
			)
			.expect("Failed to create the material binding probe. The most likely cause is invalid Metal test source.");
		let pipeline = context.create_compute_pipeline(
			crate::pipelines::compute::Builder::new(
				&[],
				crate::pipelines::ShaderParameter::new(&shader, crate::ShaderTypes::Compute),
			)
			.name("Retained Material Binding Probe Pipeline"),
		);
		let PipelineState::Compute {
			state: pipeline_state, ..
		} = &context
			.pipelines
			.last()
			.expect(
				"Missing Metal compute pipeline. The most likely cause is compute pipeline creation did not retain its native state.",
			)
			.pipeline
		else {
			panic!("Missing Metal compute pipeline state. The most likely cause is invalid compute pipeline creation.");
		};

		assert_eq!(
			pipeline_state.label().map(|label| label.to_string()),
			Some("Retained Material Binding Probe Pipeline".to_string())
		);

		let material_index = context.build_buffer::<u32>(
			crate::buffer::Builder::new(crate::Uses::Storage)
				.name("Material Texture Index Probe")
				.device_accesses(crate::DeviceAccesses::HostToDevice),
		);
		*context.get_mut_buffer_slice(material_index) = TEXTURE_INDEX;
		let output = context.build_buffer::<u32>(
			crate::buffer::Builder::new(crate::Uses::Storage)
				.name("Material Binding Probe Output")
				.device_accesses(crate::DeviceAccesses::CpuWrite | crate::DeviceAccesses::GpuWrite),
		);
		*context.get_mut_buffer_slice(output) = 0;

		let material_texture = context.build_image(
			crate::image::Builder::new(crate::Formats::RGBA8UNORM, crate::Uses::Image)
				.name("Material Binding Probe Texture")
				.extent(Extent::square(1))
				.device_accesses(crate::DeviceAccesses::HostToDevice),
		);
		context.write_texture(material_texture, |bytes| bytes.copy_from_slice(&[64, 0, 0, 255]));
		let ao_texture = context.build_image(
			crate::image::Builder::new(crate::Formats::R8UNORM, crate::Uses::Image)
				.name("Material Binding Probe AO")
				.extent(Extent::square(1))
				.device_accesses(crate::DeviceAccesses::HostToDevice),
		);
		context.write_texture(ao_texture, |bytes| bytes.copy_from_slice(&[192]));
		let sampler = context.build_sampler(
			crate::sampler::Builder::new()
				.filtering_mode(crate::FilteringModes::Closest)
				.mip_map_mode(crate::FilteringModes::Closest),
		);

		let scene_set = context.create_descriptor_set(Some("Material Binding Probe Scene Set"), &scene_layout);
		let material_set = context.create_descriptor_set(Some("Material Binding Probe Material Set"), &material_layout);
		context.write(&[
			crate::DescriptorWrite::combined_image_sampler_array(
				scene_set,
				TEXTURES_SLOT,
				material_texture,
				sampler,
				crate::Layouts::Read,
				TEXTURE_INDEX,
			),
			crate::DescriptorWrite::buffer(scene_set, MATERIAL_SLOT, material_index.into()),
			crate::DescriptorWrite::combined_image_sampler(material_set, AO_SLOT, ao_texture, sampler, crate::Layouts::Read),
			crate::DescriptorWrite::buffer(material_set, OUTPUT_SLOT, output.into()),
		]);

		let command_buffer = context
			.queue(queue_handle)
			.create_command_buffer(Some("Material Binding Probe"));
		let signal = context.create_synchronizer(Some("Material Binding Probe Signal"), true);
		context
			.queue(queue_handle)
			.execute(Some(FrameRequest::new(0, signal)), &[], signal, |execution| {
				execution.record(command_buffer, |recording| {
					recording
						.bind_compute_pipeline(pipeline)
						// Sets reach their layouts whatever order they are bound in.
						.bind_descriptor_sets(&[material_set, scene_set])
						.dispatch(crate::DispatchExtent::new(Extent::square(1), Extent::square(1)));
				});
				[]
			});
		context.wait();

		assert_eq!(
			*context.get_buffer_slice(output),
			64 | (192 << 8),
			"Material resources after the bindless table reached the wrong Metal argument IDs. The most likely cause is that a set's argument buffer and the shader's argument-buffer struct lay out its layout differently.",
		);
	}

	/// Verifies Metal 4 function descriptors preserve specialization constants through pipeline compilation.
	#[test]
	fn metal4_specialized_function_descriptor_reaches_pipeline_compiler() {
		const OUTPUT_SLOT: ResourceSlot = ResourceSlot::new(0);
		let source = r#"
			#include <metal_stdlib>
			using namespace metal;

			constant uint specialized_value [[function_constant(0)]];
			struct Resources { device uint* output [[id(0)]]; };
			kernel void specialized_main(
				uint gid [[thread_position_in_grid]],
				constant Resources& resources [[buffer(16)]]) {
				if (gid == 0) { resources.output[0] = specialized_value; }
			}
		"#;
		let (mut context, queue_handle) = test_context(crate::WorkloadTypes::COMPUTE);
		let (output_layout, output_set) = layout(
			"Output",
			[resource(
				OUTPUT_SLOT.index(),
				ResourceKind::StorageBuffer,
				1,
				AccessPolicies::WRITE,
			)],
		);
		let shader = context
			.create_shader(
				Some("Metal 4 Specialization Probe"),
				crate::shader::Sources::MTL {
					source,
					entry_point: "specialized_main",
				},
				crate::ShaderTypes::Compute,
				output_set,
			)
			.expect("Metal 4 specialization shader creation failed. The most likely cause is invalid Metal test source.");
		let specialization_map = [crate::pipelines::SpecializationMapEntry::new(0, 73u32)];
		let pipeline = context.create_compute_pipeline(crate::pipelines::compute::Builder::new(
			&[],
			crate::pipelines::ShaderParameter::new(&shader, crate::ShaderTypes::Compute)
				.with_specialization_map(&specialization_map),
		));
		let output = context.build_buffer::<u32>(
			crate::buffer::Builder::new(crate::Uses::Storage)
				.device_accesses(crate::DeviceAccesses::CpuWrite | crate::DeviceAccesses::GpuWrite),
		);
		*context.get_mut_buffer_slice(output) = 0;
		let descriptor_set = context.create_descriptor_set(Some("Metal 4 Specialization Probe"), &output_layout);
		context.write(&[crate::DescriptorWrite::buffer(descriptor_set, OUTPUT_SLOT, output.into())]);
		let command_buffer = context
			.queue(queue_handle)
			.create_command_buffer(Some("Metal 4 Specialization Probe"));
		let signal = context.create_synchronizer(Some("Metal 4 Specialization Probe"), true);
		context
			.queue(queue_handle)
			.execute(Some(FrameRequest::new(0, signal)), &[], signal, |execution| {
				execution.record(command_buffer, |recording| {
					recording
						.bind_compute_pipeline(pipeline)
						.bind_descriptor_sets(&[descriptor_set])
						.dispatch(crate::DispatchExtent::new(Extent::line(1), Extent::line(1)));
				});
				[]
			});
		context.wait();

		assert_eq!(
			*context.get_buffer_slice(output),
			73,
			"Metal 4 specialization produced the wrong value. The most likely cause is that the specialized function descriptor did not reach the pipeline compiler.",
		);
	}

	/// Verifies argument buffers and upload pages are reused across frames and refreshed by descriptor writes.
	#[test]
	fn argument_buffers_and_upload_pages_are_retained_across_frames_and_refreshed_on_writes() {
		const INPUT_SLOT: ResourceSlot = ResourceSlot::new(0);
		const OUTPUT_SLOT: ResourceSlot = ResourceSlot::new(1);
		let source = r#"
			#include <metal_stdlib>
			using namespace metal;

			struct Resources { constant uint* input [[id(0)]]; device uint* output [[id(1)]]; };
			kernel void retention_main(
				uint gid [[thread_position_in_grid]],
				constant Resources& resources [[buffer(16)]],
				constant uint& push [[buffer(15)]]) {
				if (gid == 0) { resources.output[0] = resources.input[0] + push; }
			}
		"#;
		let (mut context, queue_handle) = test_context(crate::WorkloadTypes::COMPUTE);
		let (probe_layout, probe_set) = layout(
			"Retention Probe",
			[
				resource(INPUT_SLOT.index(), ResourceKind::StorageBuffer, 1, AccessPolicies::READ),
				resource(OUTPUT_SLOT.index(), ResourceKind::StorageBuffer, 1, AccessPolicies::WRITE),
			],
		);
		let shader = context
			.create_shader(
				Some("Metal Retention Probe"),
				crate::shader::Sources::MTL {
					source,
					entry_point: "retention_main",
				},
				crate::ShaderTypes::Compute,
				probe_set,
			)
			.expect("Metal retention shader creation failed. The most likely cause is invalid Metal test source.");
		let pipeline = context.create_compute_pipeline(crate::pipelines::compute::Builder::new(
			&[crate::pipelines::PushConstantRange::new(0, 4)],
			crate::pipelines::ShaderParameter::new(&shader, crate::ShaderTypes::Compute),
		));
		let mut input_buffer = |value: u32| {
			let buffer = context.build_buffer::<u32>(
				crate::buffer::Builder::new(crate::Uses::Storage).device_accesses(crate::DeviceAccesses::HostToDevice),
			);
			*context.get_mut_buffer_slice(buffer) = value;
			buffer
		};
		let first_input = input_buffer(10);
		let second_input = input_buffer(20);
		let output = context.build_buffer::<u32>(
			crate::buffer::Builder::new(crate::Uses::Storage)
				.device_accesses(crate::DeviceAccesses::CpuWrite | crate::DeviceAccesses::GpuWrite),
		);
		let descriptor_set = context.create_descriptor_set(Some("Metal Retention Probe"), &probe_layout);
		context.write(&[
			crate::DescriptorWrite::buffer(descriptor_set, INPUT_SLOT, first_input.into()),
			crate::DescriptorWrite::buffer(descriptor_set, OUTPUT_SLOT, output.into()),
		]);
		let command_buffer = context
			.queue(queue_handle)
			.create_command_buffer(Some("Metal Retention Probe"));
		let signal = context.create_synchronizer(Some("Metal Retention Probe"), true);
		let frames = u64::from(context.frames);
		// The argument buffer each frame-local set holds, which changes only when the set is encoded again.
		let argument_buffers = |context: &super::context::Context| {
			context
				.descriptor_sets
				.iter()
				.map(|set| set.argument_buffer.as_ref().map(|snapshot| snapshot.address))
				.collect::<Vec<_>>()
		};
		let encoded_sets = |context: &super::context::Context| argument_buffers(context).iter().flatten().count();

		let run_frame = |context: &mut super::context::Context, frame_index: u64, push: u32| -> u32 {
			*context.get_mut_buffer_slice(output) = 0;
			context
				.queue(queue_handle)
				.execute(Some(FrameRequest::new(frame_index, signal)), &[], signal, |execution| {
					execution.record(command_buffer, |recording| {
						let recording = recording.bind_compute_pipeline(pipeline);
						recording.bind_descriptor_sets(&[descriptor_set]);
						recording.write_push_constant(0, push);
						recording.dispatch(crate::DispatchExtent::new(Extent::line(1), Extent::line(1)));
					});
					[]
				});
			context.wait();
			*context.get_buffer_slice(output)
		};

		// One argument buffer per frame-local set, then every later frame reuses its sequence's buffer and pages.
		let mut first_encoding = Vec::new();
		for frame_index in 0..frames * 2 {
			assert_eq!(
				run_frame(&mut context, frame_index, frame_index as u32),
				10 + frame_index as u32
			);
			assert_eq!(encoded_sets(&context), (frame_index + 1).min(frames) as usize);
			if frame_index + 1 == frames {
				first_encoding = argument_buffers(&context);
			}
			if frame_index >= frames {
				assert_eq!(
					argument_buffers(&context),
					first_encoding,
					"Argument buffers were re-encoded for an unchanged descriptor set on frame {frame_index}. The most likely cause is that the set version comparison changed.",
				);
			}
		}
		for sequence_index in 0..frames as usize {
			assert_eq!(
				context.upload_arenas[sequence_index].page_count(),
				1,
				"Upload pages were reallocated for sequence {sequence_index}. The most likely cause is that the frame arena was not reset or reused.",
			);
		}

		// A descriptor write advances the set version, so the next frame must encode the new binding.
		context.write(&[crate::DescriptorWrite::buffer(
			descriptor_set,
			INPUT_SLOT,
			second_input.into(),
		)]);
		for frame_index in frames * 2..frames * 3 {
			assert_eq!(
				run_frame(&mut context, frame_index, 1),
				21,
				"A rewritten descriptor did not reach the GPU on frame {frame_index}. The most likely cause is that a stale argument buffer was reused after its set version changed.",
			);
			assert_eq!(encoded_sets(&context), frames as usize);
		}
	}

	/// Verifies a recorded command keeps reading a descriptor set as it bound it, when the set is written before a
	/// later command of the same submission binds it.
	#[test]
	fn recorded_command_keeps_the_descriptor_set_it_bound_after_the_set_is_written() {
		use crate::frame::Frame as _;

		const INPUT_SLOT: ResourceSlot = ResourceSlot::new(0);
		const OUTPUT_SLOT: ResourceSlot = ResourceSlot::new(1);
		let source = r#"
			#include <metal_stdlib>
			using namespace metal;

			struct Resources { constant uint* input [[id(0)]]; device uint* output [[id(1)]]; };
			kernel void copy_main(
				uint gid [[thread_position_in_grid]],
				constant Resources& resources [[buffer(16)]],
				constant uint& element [[buffer(15)]]) {
				if (gid == 0) { resources.output[element] = resources.input[0]; }
			}
		"#;
		let (mut context, queue_handle) = test_context(crate::WorkloadTypes::COMPUTE);
		let (probe_layout, probe_set) = layout(
			"Snapshot Probe",
			[
				resource(INPUT_SLOT.index(), ResourceKind::StorageBuffer, 1, AccessPolicies::READ),
				resource(OUTPUT_SLOT.index(), ResourceKind::StorageBuffer, 1, AccessPolicies::WRITE),
			],
		);
		let shader = context
			.create_shader(
				Some("Metal Snapshot Probe"),
				crate::shader::Sources::MTL {
					source,
					entry_point: "copy_main",
				},
				crate::ShaderTypes::Compute,
				probe_set,
			)
			.expect("Metal snapshot shader creation failed. The most likely cause is invalid Metal test source.");
		let pipeline = context.create_compute_pipeline(crate::pipelines::compute::Builder::new(
			&[crate::pipelines::PushConstantRange::new(0, 4)],
			crate::pipelines::ShaderParameter::new(&shader, crate::ShaderTypes::Compute),
		));
		let mut input_buffer = |value: u32| {
			let buffer = context.build_buffer::<u32>(
				crate::buffer::Builder::new(crate::Uses::Storage).device_accesses(crate::DeviceAccesses::HostToDevice),
			);
			*context.get_mut_buffer_slice(buffer) = value;
			buffer
		};
		let first_input = input_buffer(10);
		let second_input = input_buffer(20);
		let output = context.build_buffer::<[u32; 2]>(
			crate::buffer::Builder::new(crate::Uses::Storage)
				.device_accesses(crate::DeviceAccesses::CpuWrite | crate::DeviceAccesses::GpuWrite),
		);
		let descriptor_set = context.create_descriptor_set(Some("Metal Snapshot Probe"), &probe_layout);
		context.write(&[
			crate::DescriptorWrite::buffer(descriptor_set, INPUT_SLOT, first_input.into()),
			crate::DescriptorWrite::buffer(descriptor_set, OUTPUT_SLOT, output.into()),
		]);
		let command_buffers = [(); 2].map(|_| {
			context
				.queue(queue_handle)
				.create_command_buffer(Some("Metal Snapshot Probe"))
		});
		let signal = context.create_synchronizer(Some("Metal Snapshot Probe"), true);

		// Each frame-local copy of the set is bound, written and bound again while its first reader is not submitted.
		for frame_index in 0..u64::from(context.frames) * 2 {
			*context.get_mut_buffer_slice(output) = [0; 2];
			context.write(&[crate::DescriptorWrite::buffer(descriptor_set, INPUT_SLOT, first_input.into())]);
			context
				.queue(queue_handle)
				.execute(Some(FrameRequest::new(frame_index, signal)), &[], signal, |execution| {
					for (element, command_buffer) in command_buffers.into_iter().enumerate() {
						if element == 1 {
							execution.frame().unwrap().write(&[crate::DescriptorWrite::buffer(
								descriptor_set,
								INPUT_SLOT,
								second_input.into(),
							)]);
						}
						execution.record(command_buffer, |recording| {
							let recording = recording.bind_compute_pipeline(pipeline);
							recording.bind_descriptor_sets(&[descriptor_set]);
							recording.write_push_constant(0, element as u32);
							recording.dispatch(crate::DispatchExtent::new(Extent::line(1), Extent::line(1)));
						});
					}
					[]
				});
			context.wait();

			assert_eq!(
				*context.get_buffer_slice(output),
				[10, 20],
				"A recorded command read a descriptor set write made after it bound the set, on frame {frame_index}. The most likely cause is that the set's argument buffer was written over while a command still read it.",
			);
		}
	}

	/// Verifies frame-local storage mip views remain valid after argument-buffer materialization.
	#[test]
	fn dynamic_storage_mips_survive_alternating_frame_sequences() {
		let writer_source = r#"
			#include <metal_stdlib>
			using namespace metal;

			struct _resources {
				texture2d<float, access::write> mip_one [[id(0)]];
				texture2d<float, access::write> mip_two [[id(1)]];
				texture2d<float, access::write> mip_three [[id(2)]];
			};

			kernel void write_mips(
				uint2 gid [[thread_position_in_grid]],
				constant _resources& resources [[buffer(16)]]) {
				if (gid.x != 0 || gid.y != 0) {
					return;
				}
				for (uint y = 0; y < 4; ++y) {
					for (uint x = 0; x < 4; ++x) {
						resources.mip_one.write(float4(0.25), uint2(x, y));
					}
				}
				for (uint y = 0; y < 2; ++y) {
					for (uint x = 0; x < 2; ++x) {
						resources.mip_two.write(float4(0.5), uint2(x, y));
					}
				}
				resources.mip_three.write(float4(0.75), uint2(0));
			}
		"#;
		let reader_source = r#"
			#include <metal_stdlib>
			using namespace metal;

			struct _resources {
				texture2d<float> pyramid [[id(0)]];
				sampler pyramid_sampler [[id(1)]];
				device uint* output [[id(2)]];
			};

			kernel void read_mips(
				uint2 gid [[thread_position_in_grid]],
				constant _resources& resources [[buffer(16)]]) {
				if (gid.x != 0 || gid.y != 0) {
					return;
				}
				const float2 uv = float2(0.5);
				resources.output[0] = uint(resources.pyramid.sample(resources.pyramid_sampler, uv, level(1.0)).x * 1000.0 + 0.5);
				resources.output[1] = uint(resources.pyramid.sample(resources.pyramid_sampler, uv, level(2.0)).x * 1000.0 + 0.5);
				resources.output[2] = uint(resources.pyramid.sample(resources.pyramid_sampler, uv, level(3.0)).x * 1000.0 + 0.5);
			}
		"#;

		let (mut context, queue_handle) = test_context(crate::WorkloadTypes::COMPUTE);
		context.set_frames_in_flight(2);

		let mip_one = resource(0, ResourceKind::StorageImage, 1, AccessPolicies::WRITE);
		let mip_two = resource(1, ResourceKind::StorageImage, 1, AccessPolicies::WRITE);
		let mip_three = resource(2, ResourceKind::StorageImage, 1, AccessPolicies::WRITE);
		let pyramid = resource(0, ResourceKind::CombinedImageSampler, 1, AccessPolicies::READ);
		let output = resource(1, ResourceKind::StorageBuffer, 1, AccessPolicies::WRITE);
		let (writer_layout, writer_shader_set) = layout("Mip Writer", [mip_one, mip_two, mip_three]);
		let (reader_layout, reader_shader_set) = layout("Mip Reader", [pyramid, output]);
		let writer_shader = context
			.create_shader(
				Some("Dynamic Storage Mip Writer"),
				crate::shader::Sources::MTL {
					source: writer_source,
					entry_point: "write_mips",
				},
				crate::ShaderTypes::Compute,
				writer_shader_set,
			)
			.expect("Failed to create the Metal mip writer. The most likely cause is invalid test source.");
		let reader_shader = context
			.create_shader(
				Some("Dynamic Storage Mip Reader"),
				crate::shader::Sources::MTL {
					source: reader_source,
					entry_point: "read_mips",
				},
				crate::ShaderTypes::Compute,
				reader_shader_set,
			)
			.expect("Failed to create the Metal mip reader. The most likely cause is invalid test source.");
		let writer_pipeline = context.create_compute_pipeline(
			crate::pipelines::compute::Builder::new(
				&[],
				crate::pipelines::ShaderParameter::new(&writer_shader, crate::ShaderTypes::Compute),
			)
			.name("Dynamic Storage Mip Writer Pipeline"),
		);
		let reader_pipeline = context.create_compute_pipeline(
			crate::pipelines::compute::Builder::new(
				&[],
				crate::pipelines::ShaderParameter::new(&reader_shader, crate::ShaderTypes::Compute),
			)
			.name("Dynamic Storage Mip Reader Pipeline"),
		);

		let depth_pyramid = context.build_dynamic_image(
			crate::image::Builder::new(crate::Formats::R32F, crate::Uses::Storage | crate::Uses::Image)
				.name("Dynamic Storage Mip Test Pyramid")
				.extent(Extent::square(8))
				.device_accesses(crate::DeviceAccesses::DeviceOnly)
				.mip_levels(4),
		);
		let output_buffer = context.build_buffer::<[u32; 3]>(
			crate::buffer::Builder::new(crate::Uses::Storage)
				.name("Dynamic Storage Mip Test Output")
				.device_accesses(crate::DeviceAccesses::CpuWrite | crate::DeviceAccesses::GpuWrite),
		);
		let sampler = context.build_sampler(
			crate::sampler::Builder::new()
				.filtering_mode(crate::FilteringModes::Closest)
				.mip_map_mode(crate::FilteringModes::Closest)
				.min_lod(0.0)
				.max_lod(3.0),
		);
		let writer_set = context.create_descriptor_set(Some("Dynamic Storage Mip Writer Set"), &writer_layout);
		let reader_set = context.create_descriptor_set(Some("Dynamic Storage Mip Reader Set"), &reader_layout);
		context.write(&[
			crate::DescriptorWrite::image_mip(writer_set, ResourceSlot::new(0), depth_pyramid, crate::Layouts::General, 1),
			crate::DescriptorWrite::image_mip(writer_set, ResourceSlot::new(1), depth_pyramid, crate::Layouts::General, 2),
			crate::DescriptorWrite::image_mip(writer_set, ResourceSlot::new(2), depth_pyramid, crate::Layouts::General, 3),
			crate::DescriptorWrite::combined_image_sampler(
				reader_set,
				ResourceSlot::new(0),
				depth_pyramid,
				sampler,
				crate::Layouts::Read,
			),
			crate::DescriptorWrite::buffer(reader_set, ResourceSlot::new(1), output_buffer.into()),
		]);

		let command_buffer = context
			.queue(queue_handle)
			.create_command_buffer(Some("Dynamic Storage Mip Test"));
		let completion = context.create_synchronizer(Some("Dynamic Storage Mip Test Completion"), true);

		for frame_index in 0..4 {
			*context.get_mut_buffer_slice(output_buffer) = [0; 3];
			context.queue(queue_handle).execute(
				Some(FrameRequest::new(frame_index, completion)),
				&[],
				completion,
				|execution| {
					execution.record(command_buffer, |recording| {
						recording
							.bind_compute_pipeline(writer_pipeline)
							.bind_descriptor_sets(&[writer_set])
							.dispatch(crate::DispatchExtent::new(Extent::square(4), Extent::square(1)));
						recording
							.bind_compute_pipeline(reader_pipeline)
							.bind_descriptor_sets(&[reader_set])
							.dispatch(crate::DispatchExtent::new(Extent::square(1), Extent::square(1)));
					});
					[]
				},
			);
			context.wait();

			assert_eq!(
				*context.get_buffer_slice(output_buffer),
				[250, 500, 750],
				"Dynamic mip storage views produced stale or incorrect values on frame {frame_index}. The most likely cause is that a frame-local mip descriptor was not retained or rebound."
			);
		}
	}
}

#[cfg(test)]
mod tile_tests {
	use super::*;
	use crate::{
		AccessPolicies,
		command_buffer::{BoundPipelineLayoutMode as _, BoundTilePipelineMode as _, CommandBufferRecording as _},
		shader::{ResourceKind, ResourceSlot},
	};

	/// A tile function shaped like the ones BESL generates: 8 x 8 threads per 16 x 16 tile, each writing a 2 x 2
	/// block, that shares a value through a threadgroup argument and reads a push constant and a descriptor set.
	const SOURCE: &str = r#"
		// besl-threadgroup-size: 8, 8, 1
		#include <metal_stdlib>
		using namespace metal;

		struct Outputs { float4 color [[color(0)]]; };
		struct PushConstant { float4 tint; };
		struct Inputs { device const float* value [[id(0)]]; };

		kernel void besl_main(
			imageblock<Outputs, imageblock_layout_implicit> block,
			ushort2 lane [[thread_position_in_threadgroup]],
			uint2 tile [[threadgroup_position_in_grid]],
			constant uint2& extent [[buffer(13)]],
			constant PushConstant& push_constant [[buffer(15)]],
			constant Inputs& inputs [[buffer(16)]],
			threadgroup float4* shared [[threadgroup(0)]]) {
			uint index = uint(lane.y) * 8u + uint(lane.x);
			shared[index] = float4(inputs.value[0], push_constant.tint.b, 0.0, 0.0);
			threadgroup_barrier(mem_flags::mem_threadgroup);
			float4 other = shared[63u - index];
			for (uint corner = 0; corner < 4; ++corner) {
				uint2 offset = uint2(lane) * 2u + uint2(corner & 1u, corner >> 1u);
				uint2 pixel = tile * 16u + offset;
				if (pixel.x >= extent.x || pixel.y >= extent.y) { continue; }
				Outputs outputs = block.read(ushort2(offset));
				outputs.color = float4(float(pixel.x) / 255.0, float(pixel.y) / 255.0, other.y, other.x);
				block.write(outputs, ushort2(offset));
			}
		}
	"#;

	/// Verifies a tile pass runs every tile of its extent, the partial ones at the edges too, with its push constant,
	/// descriptor set, threadgroup memory, and extent, and that its output needs no shader-write usage.
	#[test]
	fn tile_pass_writes_every_pixel_of_a_render_target_only_image() {
		use objc2_metal::MTLTexture as _;

		const VALUE_SLOT: ResourceSlot = ResourceSlot::new(0);
		// Three by two tiles, the last column and row partial.
		let extent = Extent::rectangle(40, 24);
		let (mut context, queue_handle) = test_context(crate::WorkloadTypes::RASTER);
		assert!(context.supports_tile_functions());

		let layout = crate::DescriptorSetLayout::new(
			"Tile Inputs",
			[crate::shader::ShaderResourceDescriptor::new(
				VALUE_SLOT,
				ResourceKind::StorageBuffer,
				1,
				AccessPolicies::READ,
			)],
		);
		let shader = context
			.create_shader(
				Some("Tile Probe"),
				crate::shader::Sources::MTL {
					source: SOURCE,
					entry_point: "besl_main",
				},
				crate::ShaderTypes::Tile,
				[crate::ShaderDescriptorSet::new(layout.clone())],
			)
			.expect("Failed to create the tile probe. The most likely cause is invalid Metal test source.");
		let push_constant_ranges = [crate::pipelines::PushConstantRange::new(0, 16)];
		let pipeline = context.create_tile_pipeline(
			crate::pipelines::tile::Builder::new(
				&push_constant_ranges,
				crate::pipelines::ShaderParameter::new(&shader, crate::ShaderTypes::Tile),
				&[crate::Formats::RGBA8UNORM],
				16,
			)
			.threadgroup_memory(&[64 * 16])
			.name("Tile Probe Pipeline"),
		);

		let value = context.build_buffer::<f32>(
			crate::buffer::Builder::new(crate::Uses::Storage)
				.name("Tile Probe Value")
				.device_accesses(crate::DeviceAccesses::HostToDevice),
		);
		*context.get_mut_buffer_slice(value) = 128.0 / 255.0;
		let set = context.create_descriptor_set(Some("Tile Probe Inputs"), &layout);
		context.write(&[crate::DescriptorWrite::buffer(set, VALUE_SLOT, value.into())]);
		let image = context.build_image(
			crate::image::Builder::new(
				crate::Formats::RGBA8UNORM,
				crate::Uses::RenderTarget | crate::Uses::TransferSource,
			)
			.name("Tile Probe Output")
			.extent(extent),
		);
		let usage = context
			.images
			.resource(context.images.nth_handle(image.into(), 0).unwrap())
			.texture
			.usage();
		assert!(
			!usage.contains(mtl::MTLTextureUsage::ShaderWrite),
			"The tile probe's output has shader-write usage. The most likely cause is that render-target uses map to shader write."
		);

		let synchronizer = context.create_synchronizer(None, false);
		let command_buffer = context.create_command_buffer(None, queue_handle);
		let mut recording = context.create_command_buffer_recording(command_buffer);
		let outputs = [crate::AttachmentInformation::new(
			image,
			crate::Layouts::RenderTarget,
			crate::LoadOp::Clear(crate::ClearValue::Color(::utils::RGBA::new(0.0, 0.0, 0.0, 0.0))),
			crate::StoreOp::Store,
		)];
		let pass = recording.start_tile_pass(pipeline, extent, &outputs);
		pass.bind_descriptor_sets(&[set]);
		pass.write_push_constant(0, [0.0f32, 0.0, 64.0 / 255.0, 0.0]);
		pass.dispatch_tiles();
		let readback = recording
			.transfer_texture(image.into())
			.expect("Tile probe readback failed. The most likely cause is that the test image lacks transfer-source support.");
		recording.execute(synchronizer);
		let mapped = context
			.get_image_data(readback)
			.expect("Tile probe mapping failed. The most likely cause is that the tile pass did not complete successfully.");

		for y in 0..extent.height() as usize {
			for x in 0..extent.width() as usize {
				let offset = y * mapped.bytes_per_row as usize + x * 4;
				assert_eq!(
					&mapped.bytes[offset..offset + 4],
					&[x as u8, y as u8, 64, 128],
					"Tile probe pixel ({x}, {y}) is wrong. The most likely cause is a tile that didn't run, a partial tile cut short, or a push constant, descriptor set, or threadgroup argument the tile stage didn't receive.",
				);
			}
		}
	}

	/// The same arithmetic per pixel for the tile and compute forms of the timing probe: long enough that a pass over
	/// 2048 x 2048 pixels takes milliseconds.
	const HEAVY_TILE_SOURCE: &str = r#"
		// besl-threadgroup-size: 16, 16, 1
		#include <metal_stdlib>
		using namespace metal;

		struct Outputs { float4 color [[color(0)]]; };

		kernel void besl_main(
			imageblock<Outputs, imageblock_layout_implicit> block,
			ushort2 lane [[thread_position_in_threadgroup]],
			uint2 tile [[threadgroup_position_in_grid]]) {
			uint2 pixel = tile * 16u + uint2(lane);
			float value = float(pixel.x ^ pixel.y);
			for (uint step = 0; step < 512u; ++step) { value = fract(sin(value) * 43758.5453); }
			Outputs outputs;
			outputs.color = float4(value);
			block.write(outputs, lane);
		}
	"#;
	const HEAVY_COMPUTE_SOURCE: &str = r#"
		// besl-threadgroup-size: 16, 16, 1
		#include <metal_stdlib>
		using namespace metal;

		struct Resources { texture2d<float, access::write> image [[id(0)]]; };

		kernel void besl_main(uint2 pixel [[thread_position_in_grid]], constant Resources& resources [[buffer(16)]]) {
			float value = float(pixel.x ^ pixel.y);
			for (uint step = 0; step < 512u; ++step) { value = fract(sin(value) * 43758.5453); }
			resources.image.write(float4(value), pixel);
		}
	"#;

	/// The pipelines the counter probes record: a heavy tile function and a compute kernel doing the same arithmetic,
	/// and the one image both write.
	struct CounterProbe {
		tile_pipeline: crate::PipelineHandle,
		compute_pipeline: crate::PipelineHandle,
		set: crate::DescriptorSetHandle,
		image: crate::ImageHandle,
	}

	/// Creates the counter probe's pipelines and an image of `extent` that both write.
	fn counter_probe(context: &mut Context, extent: Extent) -> CounterProbe {
		const IMAGE_SLOT: ResourceSlot = ResourceSlot::new(0);

		let tile_shader = context
			.create_shader(
				Some("Heavy Tile Probe"),
				crate::shader::Sources::MTL {
					source: HEAVY_TILE_SOURCE,
					entry_point: "besl_main",
				},
				crate::ShaderTypes::Tile,
				[],
			)
			.expect("Failed to create the heavy tile probe. The most likely cause is invalid Metal test source.");
		let tile_pipeline = context.create_tile_pipeline(crate::pipelines::tile::Builder::new(
			&[],
			crate::pipelines::ShaderParameter::new(&tile_shader, crate::ShaderTypes::Tile),
			&[crate::Formats::RGBA8UNORM],
			16,
		));
		let layout = crate::DescriptorSetLayout::new(
			"Heavy Compute Probe",
			[crate::shader::ShaderResourceDescriptor::new(
				IMAGE_SLOT,
				ResourceKind::StorageImage,
				1,
				AccessPolicies::WRITE,
			)],
		);
		let compute_shader = context
			.create_shader(
				Some("Heavy Compute Probe"),
				crate::shader::Sources::MTL {
					source: HEAVY_COMPUTE_SOURCE,
					entry_point: "besl_main",
				},
				crate::ShaderTypes::Compute,
				[crate::ShaderDescriptorSet::new(layout.clone())],
			)
			.expect("Failed to create the heavy compute probe. The most likely cause is invalid Metal test source.");
		let compute_pipeline = context.create_compute_pipeline(crate::pipelines::compute::Builder::new(
			&[],
			crate::pipelines::ShaderParameter::new(&compute_shader, crate::ShaderTypes::Compute),
		));
		// Both passes write one image, so they run in order, as passes that feed each other do in a frame. Passes
		// without a dependency may overlap, and then neither counter measures one pass alone.
		let image = context.build_image(
			crate::image::Builder::new(crate::Formats::RGBA8UNORM, crate::Uses::RenderTarget | crate::Uses::Storage)
				.name("Heavy Probe Output")
				.extent(extent),
		);
		let set = context.create_descriptor_set(Some("Heavy Compute Probe"), &layout);
		context.write(&[crate::DescriptorWrite::image(set, IMAGE_SLOT, image, crate::Layouts::General)]);

		CounterProbe {
			tile_pipeline,
			compute_pipeline,
			set,
			image,
		}
	}

	/// Verifies a counter around a tile pass measures the pass's work: a tile pass takes about as long as a compute
	/// dispatch doing the same arithmetic, so their counters must agree within a factor of two.
	#[test]
	fn tile_pass_counter_covers_its_work() {
		use crate::command_buffer::{BoundComputePipelineMode as _, CommonCommandBufferMode as _};
		use crate::queue::{FrameRequest, Queue as _, QueueExecution as _};

		let extent = Extent::square(2048);
		let (mut context, queue_handle) = test_context(crate::WorkloadTypes::RASTER | crate::WorkloadTypes::COMPUTE);
		let CounterProbe {
			tile_pipeline,
			compute_pipeline,
			set,
			image,
		} = counter_probe(&mut context, extent);

		let command_buffer = context.queue(queue_handle).create_command_buffer(Some("Counter Probe"));
		let synchronizer = context.create_synchronizer(Some("Counter Probe"), true);
		let tile_counters = [context.create_counter(Some("Tile A")), context.create_counter(Some("Tile B"))];
		let compute_counter = context.create_counter(Some("Compute"));
		let outputs = [crate::AttachmentInformation::new(
			image,
			crate::Layouts::RenderTarget,
			crate::LoadOp::Discard,
			crate::StoreOp::Store,
		)];
		let frames = MAX_FRAMES_IN_FLIGHT as u64 + 1;
		for frame_index in 0..frames {
			context.queue(queue_handle).execute(
				Some(FrameRequest::new(frame_index, synchronizer)),
				&[],
				synchronizer,
				|execution| {
					execution.record(command_buffer, |recording| {
						// The tile pass sits between compute work that writes the same image, as passes do in a frame.
						recording.counter(compute_counter, |recording| {
							recording
								.bind_compute_pipeline(compute_pipeline)
								.bind_descriptor_sets(&[set])
								.dispatch(crate::DispatchExtent::new(extent, Extent::square(16)));
						});
						// Two tile passes in a row, as a chain of screen passes records them, so one pass's end and the
						// next one's start fall between two render encoders.
						for tile_counter in tile_counters {
							recording.counter(tile_counter, |recording| {
								recording.start_tile_pass(tile_pipeline, extent, &outputs).dispatch_tiles();
							});
						}
						recording
							.bind_compute_pipeline(compute_pipeline)
							.bind_descriptor_sets(&[set])
							.dispatch(crate::DispatchExtent::new(Extent::square(16), Extent::square(16)));
					});
					[]
				},
			);
		}
		context.wait();
		let compute = context
			.counter_duration(compute_counter)
			.expect("The compute counter must resolve.");
		for tile_counter in tile_counters {
			let tile = context
				.counter_duration(tile_counter)
				.expect("The tile pass counter must resolve.");
			eprintln!("tile pass {tile:?}, compute dispatch {compute:?}");
			assert!(
				tile * 2 > compute && tile < compute * 2,
				"The tile pass counter reports {tile:?} for work a compute dispatch takes {compute:?} for. The most likely cause is a timestamp that doesn't wait for the render encoder's tile stage."
			);
		}
	}

	/// Verifies counters that end back to back at the tail of a compute encoder all resolve when a tile pass follows.
	///
	/// A scene's stage counter, the scene's own counter, and the next pass's start write three timestamps with no
	/// command between them. Metal drops a precise timestamp that no command follows, so ending the encoder for the
	/// tile pass must write every one of them, not only the last.
	#[test]
	fn trailing_counters_before_a_tile_pass_resolve() {
		use crate::command_buffer::{BoundComputePipelineMode as _, CommonCommandBufferMode as _};
		use crate::queue::{FrameRequest, Queue as _, QueueExecution as _};

		let extent = Extent::square(256);
		let (mut context, queue_handle) = test_context(crate::WorkloadTypes::RASTER | crate::WorkloadTypes::COMPUTE);
		let CounterProbe {
			tile_pipeline,
			compute_pipeline,
			set,
			image,
		} = counter_probe(&mut context, extent);
		let command_buffer = context.queue(queue_handle).create_command_buffer(Some("Trailing Counters"));
		let synchronizer = context.create_synchronizer(Some("Trailing Counters"), true);
		let scene_counter = context.create_counter(Some("Scene"));
		let stage_counter = context.create_counter(Some("Stage"));
		let tile_counter = context.create_counter(Some("Tile"));
		let outputs = [crate::AttachmentInformation::new(
			image,
			crate::Layouts::RenderTarget,
			crate::LoadOp::Discard,
			crate::StoreOp::Store,
		)];
		let frames = MAX_FRAMES_IN_FLIGHT as u64 + 1;
		for frame_index in 0..frames {
			context.queue(queue_handle).execute(
				Some(FrameRequest::new(frame_index, synchronizer)),
				&[],
				synchronizer,
				|execution| {
					execution.record(command_buffer, |recording| {
						recording.counter(scene_counter, |recording| {
							recording.counter(stage_counter, |recording| {
								recording
									.bind_compute_pipeline(compute_pipeline)
									.bind_descriptor_sets(&[set])
									.dispatch(crate::DispatchExtent::new(extent, Extent::square(16)));
							});
						});
						recording.counter(tile_counter, |recording| {
							recording.start_tile_pass(tile_pipeline, extent, &outputs).dispatch_tiles();
						});
					});
					[]
				},
			);
		}
		context.wait();
		for (name, counter) in [("stage", stage_counter), ("scene", scene_counter), ("tile", tile_counter)] {
			assert!(
				context.counter_duration(counter).is_some(),
				"The {name} counter did not resolve. The most likely cause is that ending an encoder rewrote only the last of several trailing timestamps, so Metal dropped the others."
			);
		}
	}
}

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
