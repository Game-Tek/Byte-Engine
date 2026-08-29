//! Visibility's resource protocol, worker preparation, dependency routing, and GPU storage policy.
//!
//! This module proves that the same shared lifecycle used by Simple can support
//! a renderer with unrelated resource shapes. Visibility loads meshes,
//! materials, images, and environments. Its store writes meshlets and parallel
//! vertex-property streams, assigns stable material and bindless texture slots,
//! and publishes renderer-specific completion values.
//!
//! # Request and preparation direction
//!
//! Scene methods call
//! `VisibilityPipelineResourceManagerClient::request_mesh`,
//! `VisibilityPipelineResourceManagerClient::request_image`, or
//! `VisibilityPipelineResourceManagerClient::request_environment`. The client
//! coalesces a `VisibilityResourceKey` and submits a
//! `VisibilityResourceRequest` to one of four independent
//! `VisibilityResourcePreparer` lanes. Preparers perform resource I/O,
//! validate formats, fill staging, and create detached factory objects. They
//! preserve logical IDs but never assign Visibility table slots or vertex
//! offsets.
//!
//! # Adoption and publication direction
//!
//! `VisibilityPipelineResourceManagerClient::begin_frame` drains prepared
//! values. Meshes and CPU-backed images continue through the shared frame upload
//! queue. Materials and detached factory objects return to the Visibility
//! pipeline manager for render-thread interning and table updates. Native GPU-I/O
//! images use the same loader token transitions, but publish only after their
//! native completion. The client returns all outcomes as
//! `VisibilityResourceCompletion` so the pipeline manager remains the only
//! owner of scene-visible state.
//!
//! # Why dependencies stay here
//!
//! A loaded mesh can discover materials, and a material can discover textures.
//! `VisibilityResourceDependencies` records that graph so a repeated resident
//! mesh request can retry only failed descendants. The shared loader cannot own
//! this rule because Simple has no material or texture dependency graph. A future
//! renderer should keep its equivalent graph beside its client and store.
//!
//! Start at `VisibilityResourcePreparer::spawn` for construction, then follow
//! the client frame callbacks into `VisibilityResourceStore` to see exactly
//! where generic lifecycle ends and Visibility placement begins.

use std::sync::Arc;

use ghi::Device as _;
use ghi::context::{Context as _, ContextCreate as _};
use ghi::{
	Size as _,
	command_buffer::{
		BoundComputePipelineMode as _, BoundPipelineLayoutMode as _, CommandBufferRecording as _, CommonCommandBufferMode as _,
	},
};
use resource_management::Reference;
use resource_management::resource::resource_manager::ResourceManager;
use resource_management::resources::image::Image as ResourceImage;
use resource_management::resources::material::{Value, Variant as ResourceVariant};
use resource_management::resources::mesh::Mesh as ResourceMesh;
use resource_management::types::AlphaMode;
use smallvec::SmallVec;
use utils::Extent;
use utils::hash::{HashMap, HashMapExt};

use crate::core::EntityHandle;
use crate::rendering::pipelines::visibility::gpu_vertex_data_manager::{
	GPUVertexDataManager, MeshData as GpuMeshData, PreparedGpuMesh,
};
use crate::rendering::pipelines::visibility::{MAX_BINDLESS_TEXTURES, MAX_MATERIALS};
use crate::rendering::renderable::mesh::MeshSource;
use crate::rendering::resource_loading as upload_staging;
use crate::rendering::resource_loading::{
	NativeTextureUpload, PreparedTextureSource, PreparedTextureTransfer, StagedTextureUpload as TextureUpload, TextureMetadata,
};
use crate::resource_management::{self};

pub(crate) const IBL_SPECULAR_LEVEL_COUNT: usize =
	resource_management::resources::image::IBL_PREFILTERED_SPECULAR_MIP_COUNT as usize;
pub(crate) const ASYNC_UPLOAD_BUFFER_BYTE_COUNT: usize = 1024 * 1024 * 32;
type CompletionList = SmallVec<[VisibilityResourceCompletion; 16]>;

mod layouts;
mod preparation;
mod state;
mod worker;

pub(crate) use layouts::*;
pub(crate) use preparation::*;
pub(crate) use state::*;
pub(crate) use worker::*;

#[cfg(test)]
mod tests {
	use super::*;

	fn staged_texture_bytes(
		format: ghi::Formats,
		extent: Extent,
		layer_count: usize,
		source: &[u8],
	) -> (Vec<u8>, TextureUploadLayout) {
		let layout = texture_upload_layout(format, extent, layer_count).expect("texture layout");

		assert_eq!(source.len(), layout.compact_size);
		let mut bytes = vec![0u8; layout.padded_size];
		bytes[..source.len()].copy_from_slice(source);
		pack_texture_rows_in_place(&mut bytes, &layout);
		(bytes, layout)
	}

	#[test]
	fn resource_mesh_metadata_is_rejected_before_transfer_recording() {
		let bytes = Box::leak(vec![0u8; 1024 * 1024].into_boxed_slice());
		let executor = resource_management::r#async::Executor::new().expect("mesh metadata test executor");
		let mesh = executor
			.block_on(async {
				let (staging, worker) = crate::rendering::resource_loading::UploadStagingArena::new_for_test(bytes);
				resource_management::r#async::spawn(worker.run()).detach();
				PreparedGpuMesh::prepare_generated_mesh(&crate::rendering::mesh::generator::BoxMeshGenerator::new(), staging)
					.await
			})
			.expect("generated mesh preparation");
		let mut material_indices = Vec::new();
		let mut primitive_skins = Vec::new();

		assert!(!VisibilityPipelineResourceManagerClient::resource_mesh_metadata_is_valid(
			&mesh,
			&material_indices,
			&primitive_skins,
			0,
		));

		material_indices.push(0);
		primitive_skins.push(None);

		assert!(VisibilityPipelineResourceManagerClient::resource_mesh_metadata_is_valid(
			&mesh,
			&material_indices,
			&primitive_skins,
			0,
		));
	}

	#[test]
	fn texture_upload_preserves_minimum_extent_and_bc_row_contents() {
		let extent = Extent::rectangle(5, 7);
		let compact_row = 2 * 16;
		let source = (0..(compact_row * 2)).map(|value| value as u8).collect::<Vec<_>>();

		let (data, upload) = staged_texture_bytes(ghi::Formats::BC7, extent, 1, &source);

		assert_eq!(upload.source_bytes_per_row, 256);
		assert_eq!(upload.source_bytes_per_image, 256 * 2);
		assert_eq!(&data[0..compact_row], &source[0..compact_row]);
		assert_eq!(&data[256..256 + compact_row], &source[compact_row..compact_row * 2]);

		let (zero_data, zero_extent) =
			staged_texture_bytes(ghi::Formats::RGBA8UNORM, Extent::rectangle(0, 0), 1, &[1, 2, 3, 4]);

		assert_eq!(zero_extent.source_bytes_per_row, 256);
		assert_eq!(zero_extent.source_bytes_per_image, 256);
		assert_eq!(&zero_data[..4], &[1, 2, 3, 4]);
	}

	/// Ensures an IES intensity map retains its half-float samples during texture upload.
	#[test]
	fn texture_upload_preserves_r16f_intensity_map_rows() {
		let extent = Extent::rectangle(2, 2);
		let compact_row = 2 * 2;
		let source = (0..compact_row * 2).map(|value| value as u8).collect::<Vec<_>>();

		let (data, upload) = staged_texture_bytes(ghi::Formats::R16F, extent, 1, &source);

		assert_eq!(
			resource_image_format_to_ghi(resource_management::types::Formats::R16F),
			ghi::Formats::R16F
		);
		assert_eq!(upload.source_bytes_per_row, 256);
		assert_eq!(upload.source_bytes_per_image, 512);
		assert_eq!(&data[..compact_row], &source[..compact_row]);
		assert_eq!(&data[256..256 + compact_row], &source[compact_row..]);
	}

	/// Ensures half-float HDR pixels reach the transfer buffer without normalization or channel conversion.
	#[test]
	fn texture_upload_preserves_rgba16f_environment_rows() {
		let extent = Extent::rectangle(2, 2);
		let compact_row = 2 * 8;
		let source = (0..compact_row * 2).map(|value| value as u8).collect::<Vec<_>>();

		let (data, upload) = staged_texture_bytes(ghi::Formats::RGBA16F, extent, 1, &source);

		assert_eq!(
			resource_image_format_to_ghi(resource_management::types::Formats::RGBA16F),
			ghi::Formats::RGBA16F
		);
		assert_eq!(upload.source_bytes_per_row, 256);
		assert_eq!(upload.source_bytes_per_image, 512);
		assert_eq!(&data[..compact_row], &source[..compact_row]);
		assert_eq!(&data[256..256 + compact_row], &source[compact_row..]);
	}

	#[test]
	fn cubemap_upload_preserves_every_face_and_image_pitch() {
		let extent = Extent::square(2);
		let compact_face_size = 2 * 2 * 8;
		let source = (0..compact_face_size * 6).map(|value| value as u8).collect::<Vec<_>>();
		let (data, upload) = staged_texture_bytes(ghi::Formats::RGBA16F, extent, 6, &source);

		assert_eq!(upload.source_bytes_per_image, 512);
		assert_eq!(data.len(), 512 * 6);
		for face in 0..6 {
			for row in 0..2 {
				let source_start = face * compact_face_size + row * 16;
				let upload_start = face * 512 + row * 256;

				assert_eq!(
					&data[upload_start..upload_start + 16],
					&source[source_start..source_start + 16]
				);
			}
		}
	}

	#[test]
	fn environment_specular_streams_form_one_gpu_mip_chain() {
		let extents: [Extent; IBL_SPECULAR_LEVEL_COUNT] =
			std::array::from_fn(|level| environment_mip_extent([256, 256, 0], level as u32));

		assert_eq!(extents[0], Extent::rectangle(256, 256));
		assert_eq!(extents[1], Extent::rectangle(128, 128));
		assert_eq!(extents[7], Extent::rectangle(2, 2));
		assert_eq!(compact_image_byte_size(ghi::Formats::RGBA16F, extents[0]), 256 * 256 * 8);
		assert_eq!(compact_image_byte_size(ghi::Formats::RGBA16F, extents[7]), 2 * 2 * 8);
	}
}
