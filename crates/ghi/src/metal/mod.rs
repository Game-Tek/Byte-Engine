#[link(name = "CoreGraphics", kind = "framework")]
extern "C" {}

use std::cell::RefCell;
use std::ffi::c_void;
use std::ptr::NonNull;
use std::rc::Rc;
use std::sync::atomic::AtomicU64;

use ::utils::hash::HashMap;
use ::utils::Extent;
use objc2::rc::Retained;
use objc2::runtime::ProtocolObject;
use objc2_app_kit::NSView;
use objc2_foundation::{NSArray, NSRange, NSSize};
use objc2_metal as mtl;
use objc2_metal::MTLArgumentEncoder as _;
use objc2_metal::MTLDevice as _;
use objc2_quartz_core::{CAMetalDrawable, CAMetalLayer};
use smallvec::SmallVec;

use crate::buffer::BufferHandle;
use crate::graphics_hardware_interface;
use crate::image::ImageHandle;
use crate::PrivateHandles;

mod pipeline;
mod resources;
mod state;
mod synchronization;
mod types;
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

	pub(crate) fn storage_mode_from_access(access: DeviceAccesses) -> mtl::MTLStorageMode {
		if access == DeviceAccesses::DeviceOnly {
			mtl::MTLStorageMode::Private
		} else {
			// Metal 4 has no managed-resource synchronization commands. Shared storage is CPU-coherent after queue completion.
			mtl::MTLStorageMode::Shared
		}
	}

	pub(crate) fn resource_options_from_access(access: DeviceAccesses) -> mtl::MTLResourceOptions {
		if access == DeviceAccesses::DeviceOnly {
			mtl::MTLResourceOptions::StorageModePrivate
		} else {
			mtl::MTLResourceOptions::StorageModeShared
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

	pub(crate) fn texture_upload_layout(format: Formats, extent: Extent) -> Option<(usize, usize, usize)> {
		Some(format.compact_copy_layout(extent.width().max(1), extent.height().max(1)))
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

	#[cfg(not(debug_assertions))]
	pub(crate) fn debug_compressed_upload(
		_enabled: bool,
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
		fn upload_layout_preserves_bc_block_rows_and_minimum_extent() {
			let extent = Extent::rectangle(5, 7);

			let (bytes_per_row, row_count, bytes_per_image) = texture_upload_layout(Formats::BC7, extent).unwrap();

			assert_eq!(bytes_per_row, 2 * 16);
			assert_eq!(row_count, 2);
			assert_eq!(bytes_per_image, 2 * 2 * 16);
			assert_eq!(
				texture_upload_layout(Formats::RGBA8UNORM, Extent::rectangle(0, 0)),
				Some((4, 1, 4))
			);
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
			assert_eq!(to_pixel_format(Formats::BC5SNORM), mtl::MTLPixelFormat::BC5_RGSnorm);
			assert_eq!(to_pixel_format(Formats::BC7), mtl::MTLPixelFormat::BC7_RGBAUnorm);
			assert_eq!(to_pixel_format(Formats::BC7SRGB), mtl::MTLPixelFormat::BC7_RGBAUnorm_sRGB);
		}

		#[test]
		fn depth16_format_mapping_uses_depth16_unorm() {
			assert_eq!(to_pixel_format(Formats::Depth16), mtl::MTLPixelFormat::Depth16Unorm);
		}

		#[test]
		fn sampler_reduction_modes_preserve_the_ghi_contract() {
			assert_eq!(
				sampler_reduction_mode(SamplingReductionModes::WeightedAverage),
				mtl::MTLSamplerReductionMode::WeightedAverage
			);
			assert_eq!(
				sampler_reduction_mode(SamplingReductionModes::Min),
				mtl::MTLSamplerReductionMode::Minimum
			);
			assert_eq!(
				sampler_reduction_mode(SamplingReductionModes::Max),
				mtl::MTLSamplerReductionMode::Maximum
			);
		}

		#[test]
		fn specialization_map_entry_supports_i32_constants() {
			let constant_values = mtl::MTLFunctionConstantValues::new();
			let entry = crate::pipelines::SpecializationMapEntry::new(0, "i32".to_string(), -1i32);

			super::super::apply_specialization_map_entry(&constant_values, &entry);
		}
	}
}

pub(crate) use pipeline::*;
pub(crate) use resources::*;
pub use state::{buffer, descriptor_set, image, queue, sampler, swapchain, synchronizer};
pub(crate) use types::*;

#[cfg(test)]
mod flat_binding_tests {
	use super::*;

	#[test]
	fn sampler_descriptor_applies_maximum_reduction_with_linear_filtering() {
		let descriptor = build_sampler_descriptor(
			&crate::sampler::Builder::new()
				.filtering_mode(crate::FilteringModes::Linear)
				.mip_map_mode(crate::FilteringModes::Linear)
				.reduction_mode(crate::SamplingReductionModes::Max),
		);

		assert_eq!(descriptor.minFilter(), mtl::MTLSamplerMinMagFilter::Linear);
		assert_eq!(descriptor.magFilter(), mtl::MTLSamplerMinMagFilter::Linear);
		assert_eq!(descriptor.mipFilter(), mtl::MTLSamplerMipFilter::Linear);
		assert_eq!(descriptor.reductionMode(), mtl::MTLSamplerReductionMode::Maximum);
	}

	#[test]
	fn sampler_reduction_falls_back_before_apple10() {
		assert_eq!(
			sampler_reduction_mode_for_device(mtl::MTLSamplerReductionMode::Minimum, false),
			mtl::MTLSamplerReductionMode::WeightedAverage
		);
		assert_eq!(
			sampler_reduction_mode_for_device(mtl::MTLSamplerReductionMode::Maximum, false),
			mtl::MTLSamplerReductionMode::WeightedAverage
		);
		assert_eq!(
			sampler_reduction_mode_for_device(mtl::MTLSamplerReductionMode::Maximum, true),
			mtl::MTLSamplerReductionMode::Maximum
		);
	}

	fn resource(
		slot: u32,
		kind: crate::shader::ResourceKind,
		count: u32,
		access: crate::AccessPolicies,
	) -> crate::shader::ShaderResourceDescriptor {
		crate::shader::ShaderResourceDescriptor::new(crate::shader::ResourceSlot::new(slot), kind, count, access)
	}

	#[test]
	fn flat_resource_ranges_treat_arrays_as_reserved_slot_intervals() {
		let array = resource(
			9,
			crate::shader::ResourceKind::SampledImage,
			1024,
			crate::AccessPolicies::READ,
		);
		let inside = resource(10, crate::shader::ResourceKind::Sampler, 1, crate::AccessPolicies::READ);
		let after = resource(1033, crate::shader::ResourceKind::Sampler, 1, crate::AccessPolicies::READ);

		assert!(resource_ranges_overlap(array, inside));
		assert!(!resource_ranges_overlap(array, after));
	}

	#[test]
	fn active_array_interiors_are_not_independent_retained_slot_keys() {
		let array = resource(9, crate::shader::ResourceKind::SampledImage, 4, crate::AccessPolicies::READ);

		assert!(resource_accepts_retained_slot_key(array, crate::shader::ResourceSlot::new(9)));
		assert!(!resource_accepts_retained_slot_key(
			array,
			crate::shader::ResourceSlot::new(10)
		));
		assert!(!resource_accepts_retained_slot_key(
			array,
			crate::shader::ResourceSlot::new(12)
		));
		assert!(resource_accepts_retained_slot_key(
			array,
			crate::shader::ResourceSlot::new(13)
		));
	}

	#[test]
	#[should_panic(expected = "Overlapping Metal shader resources")]
	fn canonical_stage_interface_rejects_overlapping_ranges() {
		canonicalize_stage_resources(&[
			resource(4, crate::shader::ResourceKind::StorageBuffer, 4, crate::AccessPolicies::READ),
			resource(7, crate::shader::ResourceKind::Sampler, 1, crate::AccessPolicies::READ),
		]);
	}

	#[test]
	fn combined_image_sampler_arrays_use_stable_slot_derived_ids() {
		let combined = allocate_argument_binding_slots(resource(
			9,
			crate::shader::ResourceKind::CombinedImageSampler,
			2,
			crate::AccessPolicies::READ,
		));
		let buffer = allocate_argument_binding_slots(resource(
			11,
			crate::shader::ResourceKind::UniformBuffer,
			1,
			crate::AccessPolicies::READ,
		));

		assert_eq!(
			combined,
			ArgumentBindingSlots::CombinedImageSampler {
				textures: ArgumentSlotRange { base: 18, count: 2 },
				samplers: ArgumentSlotRange { base: 20, count: 2 },
			}
		);
		assert_eq!(buffer, ArgumentBindingSlots::Buffer(ArgumentSlotRange { base: 22, count: 1 }));
	}

	#[test]
	fn canonical_stage_interfaces_share_only_when_representation_and_access_match() {
		let split_declarations = canonicalize_stage_resources(&[
			resource(8, crate::shader::ResourceKind::Sampler, 1, crate::AccessPolicies::READ),
			resource(2, crate::shader::ResourceKind::StorageBuffer, 1, crate::AccessPolicies::READ),
			resource(2, crate::shader::ResourceKind::StorageBuffer, 1, crate::AccessPolicies::WRITE),
		]);
		let merged_declaration = canonicalize_stage_resources(&[
			resource(
				2,
				crate::shader::ResourceKind::StorageBuffer,
				1,
				crate::AccessPolicies::READ_WRITE,
			),
			resource(8, crate::shader::ResourceKind::Sampler, 1, crate::AccessPolicies::READ),
		]);
		let read_only = canonicalize_stage_resources(&[
			resource(2, crate::shader::ResourceKind::StorageBuffer, 1, crate::AccessPolicies::READ),
			resource(8, crate::shader::ResourceKind::Sampler, 1, crate::AccessPolicies::READ),
		]);

		assert_eq!(split_declarations, merged_declaration);
		assert_ne!(split_declarations, read_only);
	}

	/// Exercises the production material ordering where scalar resources follow the bindless texture table.
	#[test]
	fn retained_material_resources_after_bindless_array_reach_metal() {
		use objc2_metal::MTLComputePipelineState as _;

		use crate::{
			command_buffer::{BoundComputePipelineMode as _, BoundPipelineLayoutMode as _, CommonCommandBufferMode as _},
			device::Device as _,
			queue::{FrameRequest, Queue as _, QueueExecution as _},
		};

		const TEXTURES_SLOT: crate::shader::ResourceSlot = crate::shader::ResourceSlot::new(9);
		const MATERIAL_SLOT: crate::shader::ResourceSlot = crate::shader::ResourceSlot::new(1046);
		const AO_SLOT: crate::shader::ResourceSlot = crate::shader::ResourceSlot::new(1051);
		const OUTPUT_SLOT: crate::shader::ResourceSlot = crate::shader::ResourceSlot::new(1054);
		const TEXTURE_INDEX: u32 = 7;

		let source = r#"
			#include <metal_stdlib>
			using namespace metal;

			struct _resources {
				texture2d<float> textures [[id(18)]][1024];
				sampler textures_sampler [[id(1042)]][1024];
				constant uint* material_texture_index [[id(2092)]];
				texture2d<float> ao [[id(2102)]];
				sampler ao_sampler [[id(2103)]];
				device uint* output [[id(2108)]];
			};

			kernel void besl_main(
				uint2 gid [[thread_position_in_grid]],
				constant _resources& resources [[buffer(16)]]) {
				if (gid.x != 0 || gid.y != 0) { return; }
				uint texture_index = resources.material_texture_index[0];
				float material = resources.textures[texture_index].sample(
					resources.textures_sampler[texture_index], float2(0.5)).r;
				float ao = resources.ao.sample(resources.ao_sampler, float2(0.5)).r;
				resources.output[0] = uint(round(material * 255.0)) | (uint(round(ao * 255.0)) << 8);
			}
		"#;

		let features = crate::device::Features::new().debug_labels(true);
		let mut instance = super::Instance::new(features)
			.expect("Failed to create a Metal instance. The most likely cause is unavailable Metal device support.");
		let mut queue_handle = None;
		let mut context = instance
			.create_device(
				features,
				&mut [(crate::QueueSelection::new(crate::WorkloadTypes::COMPUTE), &mut queue_handle)],
			)
			.expect("Failed to create a Metal device. The most likely cause is unavailable compute queue support.")
			.create_context()
			.expect("Failed to create a Metal context. The most likely cause is unavailable Metal command support.");
		let queue_handle = queue_handle.expect(
			"Missing Metal compute queue. The most likely cause is that device selection did not return the requested queue.",
		);

		let texture_resource = resource(
			TEXTURES_SLOT.index(),
			crate::shader::ResourceKind::CombinedImageSampler,
			1024,
			crate::AccessPolicies::READ,
		);
		let material_resource = resource(
			MATERIAL_SLOT.index(),
			crate::shader::ResourceKind::StorageBuffer,
			1,
			crate::AccessPolicies::READ,
		);
		let ao_resource = resource(
			AO_SLOT.index(),
			crate::shader::ResourceKind::CombinedImageSampler,
			1,
			crate::AccessPolicies::READ,
		);
		let output_resource = resource(
			OUTPUT_SLOT.index(),
			crate::shader::ResourceKind::StorageBuffer,
			1,
			crate::AccessPolicies::WRITE,
		);
		let shader = context
			.create_shader(
				Some("Retained Material Binding Probe"),
				crate::shader::Sources::MTL {
					source,
					entry_point: "besl_main",
				},
				crate::ShaderTypes::Compute,
				[output_resource, ao_resource, texture_resource, material_resource],
			)
			.expect("Failed to create the material binding probe. The most likely cause is invalid Metal test source.");
		let pipeline = context.create_compute_pipeline(
			crate::pipelines::compute::Builder::new(
				&[],
				crate::pipelines::ShaderParameter::new(&shader, crate::ShaderTypes::Compute),
			)
			.name("Retained Material Binding Probe Pipeline"),
		);
		let PipelineState::Compute(pipeline_state) = &context
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

		let scene_set = context.create_descriptor_set(Some("Material Binding Probe Scene Set"));
		let material_set = context.create_descriptor_set(Some("Material Binding Probe Material Set"));
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
						.bind_descriptor_sets(&[scene_set, material_set])
						.dispatch(crate::DispatchExtent::new(Extent::square(1), Extent::square(1)));
				});
				[]
			});
		context.wait();

		assert_eq!(
			*context.get_buffer_slice(output),
			64 | (192 << 8),
			"Material resources after the bindless table reached the wrong Metal argument IDs. The most likely cause is that retained materialization and the fixed MSL slot mapping disagree.",
		);
	}

	/// Verifies Metal 4 function descriptors preserve specialization constants through pipeline compilation.
	#[test]
	fn metal4_specialized_function_descriptor_reaches_pipeline_compiler() {
		use crate::{
			command_buffer::{BoundComputePipelineMode as _, BoundPipelineLayoutMode as _, CommonCommandBufferMode as _},
			device::Device as _,
			queue::{FrameRequest, Queue as _, QueueExecution as _},
		};

		const OUTPUT_SLOT: crate::shader::ResourceSlot = crate::shader::ResourceSlot::new(0);
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
		let features = crate::device::Features::new();
		let mut instance = super::Instance::new(features)
			.expect("Metal 4 specialization test setup failed. The most likely cause is unavailable Metal device support.");
		let mut queue_handle = None;
		let mut context = instance
			.create_device(
				features,
				&mut [(crate::QueueSelection::new(crate::WorkloadTypes::COMPUTE), &mut queue_handle)],
			)
			.expect(
				"Metal 4 specialization device creation failed. The most likely cause is unavailable compute queue support.",
			)
			.create_context()
			.expect(
				"Metal 4 specialization context creation failed. The most likely cause is unavailable Metal command support.",
			);
		let queue_handle = queue_handle.expect(
			"Metal 4 specialization queue is missing. The most likely cause is that device selection did not return the requested queue.",
		);
		let output_resource = resource(
			OUTPUT_SLOT.index(),
			crate::shader::ResourceKind::StorageBuffer,
			1,
			crate::AccessPolicies::WRITE,
		);
		let shader = context
			.create_shader(
				Some("Metal 4 Specialization Probe"),
				crate::shader::Sources::MTL {
					source,
					entry_point: "specialized_main",
				},
				crate::ShaderTypes::Compute,
				[output_resource],
			)
			.expect("Metal 4 specialization shader creation failed. The most likely cause is invalid Metal test source.");
		let specialization_map = [crate::pipelines::SpecializationMapEntry::new(0, "u32".to_string(), 73u32)];
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
		let descriptor_set = context.create_descriptor_set(Some("Metal 4 Specialization Probe"));
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

	/// Verifies frame-local storage mip views remain valid after argument-buffer materialization.
	#[test]
	fn dynamic_storage_mips_survive_alternating_frame_sequences() {
		use crate::{
			command_buffer::{BoundComputePipelineMode as _, BoundPipelineLayoutMode as _, CommonCommandBufferMode as _},
			device::Device as _,
			queue::{FrameRequest, Queue as _, QueueExecution as _},
		};

		let writer_source = r#"
			#include <metal_stdlib>
			using namespace metal;

			struct _resources {
				texture2d<float, access::write> mip_one [[id(0)]];
				texture2d<float, access::write> mip_two [[id(2)]];
				texture2d<float, access::write> mip_three [[id(4)]];
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

		let features = crate::device::Features::new().debug_labels(true);
		let mut instance = super::Instance::new(features)
			.expect("Failed to create a Metal instance. The most likely cause is unavailable Metal device support.");
		let mut queue_handle = None;
		let mut context = instance
			.create_device(
				features,
				&mut [(crate::QueueSelection::new(crate::WorkloadTypes::COMPUTE), &mut queue_handle)],
			)
			.expect("Failed to create a Metal device. The most likely cause is unavailable compute queue support.")
			.create_context()
			.expect("Failed to create a Metal context. The most likely cause is unavailable Metal command support.");
		let queue_handle = queue_handle.expect(
			"Missing Metal compute queue. The most likely cause is that device selection did not return the requested queue.",
		);
		context.set_frames_in_flight(2);

		let mip_one = crate::shader::ShaderResourceDescriptor::single(
			crate::shader::ResourceSlot::new(0),
			crate::shader::ResourceKind::StorageImage,
			crate::AccessPolicies::WRITE,
		);
		let mip_two = crate::shader::ShaderResourceDescriptor::single(
			crate::shader::ResourceSlot::new(1),
			crate::shader::ResourceKind::StorageImage,
			crate::AccessPolicies::WRITE,
		);
		let mip_three = crate::shader::ShaderResourceDescriptor::single(
			crate::shader::ResourceSlot::new(2),
			crate::shader::ResourceKind::StorageImage,
			crate::AccessPolicies::WRITE,
		);
		let pyramid = crate::shader::ShaderResourceDescriptor::single(
			crate::shader::ResourceSlot::new(0),
			crate::shader::ResourceKind::CombinedImageSampler,
			crate::AccessPolicies::READ,
		);
		let output = crate::shader::ShaderResourceDescriptor::single(
			crate::shader::ResourceSlot::new(1),
			crate::shader::ResourceKind::StorageBuffer,
			crate::AccessPolicies::WRITE,
		);
		let writer_shader = context
			.create_shader(
				Some("Dynamic Storage Mip Writer"),
				crate::shader::Sources::MTL {
					source: writer_source,
					entry_point: "write_mips",
				},
				crate::ShaderTypes::Compute,
				[mip_one, mip_two, mip_three],
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
				[pyramid, output],
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
		let writer_set = context.create_descriptor_set(Some("Dynamic Storage Mip Writer Set"));
		let reader_set = context.create_descriptor_set(Some("Dynamic Storage Mip Reader Set"));
		context.write(&[
			crate::DescriptorWrite::image_mip(
				writer_set,
				crate::shader::ResourceSlot::new(0),
				depth_pyramid,
				crate::Layouts::General,
				1,
			),
			crate::DescriptorWrite::image_mip(
				writer_set,
				crate::shader::ResourceSlot::new(1),
				depth_pyramid,
				crate::Layouts::General,
				2,
			),
			crate::DescriptorWrite::image_mip(
				writer_set,
				crate::shader::ResourceSlot::new(2),
				depth_pyramid,
				crate::Layouts::General,
				3,
			),
			crate::DescriptorWrite::combined_image_sampler(
				reader_set,
				crate::shader::ResourceSlot::new(0),
				depth_pyramid,
				sampler,
				crate::Layouts::Read,
			),
			crate::DescriptorWrite::buffer(reader_set, crate::shader::ResourceSlot::new(1), output_buffer.into()),
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

pub mod command_buffer;
pub mod context;
pub mod device;
pub mod factory;
pub mod frame;
pub mod instance;

pub use self::command_buffer::*;
pub use self::context::*;
pub(crate) use self::descriptor_set::*;
pub use self::device::Device;
pub use self::factory::{ComputePipeline, Factory};
pub use self::frame::*;
pub use self::instance::*;
pub(crate) use self::synchronizer::*;
