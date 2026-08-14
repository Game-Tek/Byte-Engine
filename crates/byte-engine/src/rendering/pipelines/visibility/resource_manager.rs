use std::collections::VecDeque;
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::Arc;
use std::time::Duration;

use ghi::context::{Context as _, ContextCreate as _};
use ghi::frame::Frame as _;
use ghi::Device as _;
use ghi::Queue as _;
use ghi::{
	command_buffer::{
		BoundComputePipelineMode as _, BoundPipelineLayoutMode as _, CommandBufferRecording as _, CommonCommandBufferMode as _,
	},
	Size as _,
};
use resource_management::resource::resource_manager::ResourceManager;
use resource_management::resource::ReadTargets;
use resource_management::resources::image::Image as ResourceImage;
use resource_management::resources::material::{Value, Variant as ResourceVariant};
use resource_management::resources::mesh::Mesh as ResourceMesh;
use resource_management::types::AlphaMode;
use resource_management::Reference;
use smallvec::SmallVec;
use utils::hash::{HashMap, HashMapExt};
use utils::Extent;

pub(super) use super::upload_staging;
use crate::core::EntityHandle;
use crate::rendering::pipelines::visibility::gpu_vertex_data_manager::{
	GPUVertexDataManager, MeshData as GpuMeshData, PreparedGpuMesh,
};
use crate::rendering::pipelines::visibility::{MAX_BINDLESS_TEXTURES, MAX_MATERIALS};
use crate::rendering::renderable::mesh::MeshSource;
use crate::resource_management::{self};

pub(crate) const IBL_SPECULAR_LEVEL_COUNT: usize =
	resource_management::resources::image::IBL_PREFILTERED_SPECULAR_MIP_COUNT as usize;
pub(crate) const ASYNC_UPLOAD_BUFFER_BYTE_COUNT: usize = 1024 * 1024 * 32;
type CompletionList = SmallVec<[VisibilityResourceCompletion; 16]>;
const ACTIVE_TRANSFER_POLL_INTERVAL: Duration = Duration::from_millis(1);

mod layouts;
mod preparation;
mod state;
mod worker;

pub(crate) use layouts::*;
pub(crate) use preparation::*;
pub use state::ResourceStates;
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
		let staging = super::super::upload_staging::UploadStagingArena::new(bytes);
		let executor = resource_management::r#async::Executor::new().expect("mesh metadata test executor");
		let mesh = executor
			.block_on(PreparedGpuMesh::prepare_generated_mesh(
				&crate::rendering::mesh::generator::BoxMeshGenerator::new(),
				staging,
			))
			.expect("generated mesh preparation");
		let mut material_indices = Vec::new();
		let mut primitive_skins = Vec::new();

		assert!(!VisibilityPipelineResourceManagerWorker::resource_mesh_metadata_is_valid(
			&mesh,
			&material_indices,
			&primitive_skins,
			0,
		));

		material_indices.push(0);
		primitive_skins.push(None);
		assert!(VisibilityPipelineResourceManagerWorker::resource_mesh_metadata_is_valid(
			&mesh,
			&material_indices,
			&primitive_skins,
			0,
		));
	}

	#[test]
	fn resource_commands_reach_the_async_worker_in_fifo_order() {
		let executor = resource_management::r#async::Executor::new().expect("expected test value");
		let (sender, receiver) = kanal::unbounded_async();
		let sender = sender.to_sync();

		for id in ["first", "second", "third"] {
			sender
				.send(VisibilityTransferCommand::RequestEnvironment { id: id.to_string() })
				.expect("expected test value");
		}

		let received = executor.block_on(async {
			let mut ids = Vec::new();
			for _ in 0..3 {
				let VisibilityTransferCommand::RequestEnvironment { id } = receiver.recv().await.expect("expected test value")
				else {
					panic!(
						"Unexpected visibility command. The most likely cause is that the FIFO test enqueued the wrong variant."
					);
				};
				ids.push(id);
			}
			ids
		});

		assert_eq!(received, ["first", "second", "third"]);
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
			std::array::from_fn(|level| environment_mip_extent([256, 256, 1], level as u32));

		assert_eq!(extents[0], Extent::new(256, 256, 1));
		assert_eq!(extents[1], Extent::new(128, 128, 1));
		assert_eq!(extents[7], Extent::new(2, 2, 1));
		assert_eq!(compact_image_byte_size(ghi::Formats::RGBA16F, extents[0]), 256 * 256 * 8);
		assert_eq!(compact_image_byte_size(ghi::Formats::RGBA16F, extents[7]), 2 * 2 * 8);
	}
}
