use std::{alloc::Allocator, io::Cursor};

use exr::prelude::{f16, ReadChannels as _, ReadImage as _, ReadLayers as _};
use utils::Extent;

use super::{
	asset_handler::{AssetHandler, BakeContext, LoadErrors},
	ResourceId,
};
use crate::{
	processors::ibl_processor::bake_image_ibl_in,
	resources::image::Image,
	types::{Formats, Gamma},
	ProcessedAsset,
};

/// The `DecodedExr` struct accumulates allocator-backed RGBA16F pixels while the EXR reader visits image blocks.
struct DecodedExr<'a> {
	data: Vec<u8, &'a dyn Allocator>,
	extent: Option<Extent>,
	width: usize,
	valid: bool,
}

impl<'a> DecodedExr<'a> {
	/// Allocates the exact half-float RGBA surface required by the decoded EXR layer.
	fn new(resolution: exr::prelude::Vec2<usize>, allocator: &'a dyn Allocator) -> Self {
		let extent = u32::try_from(resolution.width())
			.ok()
			.zip(u32::try_from(resolution.height()).ok())
			.map(|(width, height)| Extent::rectangle(width, height));
		let byte_len = resolution
			.width()
			.checked_mul(resolution.height())
			.and_then(|pixel_count| pixel_count.checked_mul(4 * std::mem::size_of::<f16>()));
		let mut data = Vec::new_in(allocator);
		let valid = extent.is_some()
			&& byte_len
				.map(|byte_len| {
					if data.try_reserve_exact(byte_len).is_err() {
						return false;
					}
					data.resize(byte_len, 0);
					true
				})
				.unwrap_or(false);

		Self {
			data,
			extent,
			width: resolution.width(),
			valid,
		}
	}

	/// Writes one decoded EXR pixel into its tightly packed little-endian RGBA16F location.
	fn set_pixel(&mut self, position: exr::prelude::Vec2<usize>, channels: (f16, f16, f16, f16)) {
		let Some(offset) = position
			.y()
			.checked_mul(self.width)
			.and_then(|row| row.checked_add(position.x()))
			.and_then(|pixel| pixel.checked_mul(4 * std::mem::size_of::<f16>()))
		else {
			self.valid = false;
			return;
		};
		let Some(end) = offset.checked_add(4 * std::mem::size_of::<f16>()) else {
			self.valid = false;
			return;
		};
		let Some(pixel) = self.data.get_mut(offset..end) else {
			self.valid = false;
			return;
		};

		for (destination, channel) in pixel
			.chunks_exact_mut(2)
			.zip([channels.0, channels.1, channels.2, channels.3])
		{
			destination.copy_from_slice(&channel.to_le_bytes());
		}
	}
}

/// The `EXRAssetHandler` struct provides linear HDR images to rendering pipelines without quantizing their radiance.
pub struct EXRAssetHandler;

impl Default for EXRAssetHandler {
	fn default() -> Self {
		Self::new()
	}
}

impl EXRAssetHandler {
	pub fn new() -> Self {
		Self
	}
}

impl AssetHandler for EXRAssetHandler {
	fn can_handle(&self, r#type: &str) -> bool {
		r#type.eq_ignore_ascii_case("exr")
			|| r#type == "Image"
			|| r#type.eq_ignore_ascii_case("image/exr")
			|| r#type.eq_ignore_ascii_case("image/x-exr")
	}

	async fn bake<'a>(&'a self, context: BakeContext<'a>, url: ResourceId<'a>) -> Result<(), LoadErrors> {
		if let Some(data_type) = context.resource_type(url) {
			if !self.can_handle(data_type) {
				return Err(LoadErrors::UnsupportedType);
			}
		}

		let (source, _, data_type) = context.resolve(url).await?;
		let allocator = context.allocator();
		if !self.can_handle(&data_type) {
			return Err(LoadErrors::UnsupportedType);
		}

		// EXR values are decoded directly to half floats so highlights above 1.0 remain available to lighting.
		let image = exr::prelude::read()
			.no_deep_data()
			.largest_resolution_level()
			.rgba_channels(
				|resolution, _| DecodedExr::new(resolution, allocator),
				|pixels, position, channels: (f16, f16, f16, f16)| pixels.set_pixel(position, channels),
			)
			.first_valid_layer()
			.all_attributes()
			.from_buffered(Cursor::new(source.as_slice()))
			.map_err(|_| LoadErrors::FailedToProcess)?;
		let decoded = image.layer_data.channel_data.pixels;
		let extent = decoded.extent.filter(|_| decoded.valid).ok_or(LoadErrors::FailedToProcess)?;

		let baked = bake_image_ibl_in(extent, &decoded.data, allocator).map_err(|_| LoadErrors::FailedToProcess)?;
		let image = Image {
			format: Formats::RGBA16F,
			gamma: Gamma::Linear,
			extent: baked.root_extent,
			mip_count: 1,
			ibl: Some(baked.ibl),
		};
		let asset = ProcessedAsset::new(url, image).with_streams(baked.streams);

		context.store_primary(asset, &baked.data)
	}
}

#[cfg(test)]
mod tests {
	use std::io::Cursor;

	use exr::prelude::{SpecificChannels, WritableImage as _};

	use super::EXRAssetHandler;
	use crate::{
		asset::{asset_manager::AssetManager, storage_backend::tests::TestStorageBackend, ResourceId},
		r#async,
		resource::{storage_backend::tests::TestStorageBackend as TestResourceStorage, ReadStorageBackend as _},
		resources::image::Image,
		types::{Formats, Gamma},
	};

	fn hdr_fixture() -> Vec<u8> {
		let channels = SpecificChannels::rgb(|position: exr::prelude::Vec2<usize>| match position.x() {
			0 => (4.0_f32, 0.5_f32, -0.25_f32),
			_ => (16.0_f32, 2.0_f32, 8.0_f32),
		});
		let image = exr::prelude::Image::from_channels((2, 1), channels);
		let mut bytes = Vec::new();
		image
			.write()
			.non_parallel()
			.to_buffered(Cursor::new(&mut bytes))
			.expect("the in-memory EXR fixture must encode");
		bytes
	}

	#[test]
	fn handles_exr_extensions_and_mime_types_case_insensitively() {
		let handler = EXRAssetHandler::new();
		assert!(crate::AssetHandler::can_handle(&handler, "exr"));
		assert!(crate::AssetHandler::can_handle(&handler, "EXR"));
		assert!(crate::AssetHandler::can_handle(&handler, "image/x-exr"));
		assert!(crate::AssetHandler::can_handle(&handler, "Image"));
		assert!(!crate::AssetHandler::can_handle(&handler, "png"));
	}

	#[r#async::test]
	async fn asset_manager_bakes_linear_hdr_pixels_from_memory() {
		let source_storage = TestStorageBackend::new();
		source_storage.add_file("studio.exr", &hdr_fixture());
		let resource_storage = TestResourceStorage::new();
		let mut asset_manager = AssetManager::new(source_storage);
		asset_manager.add_asset_handler(EXRAssetHandler::new());

		asset_manager
			.bake("studio.exr", &resource_storage)
			.await
			.expect("the registered EXR handler must bake the image");
		let (stored, _) = resource_storage
			.read(ResourceId::new("studio.exr"))
			.expect("the baked EXR image must be stored");
		let image: Image = crate::from_slice(stored.resource()).expect("the stored EXR metadata must deserialize");
		let data = resource_storage
			.get_resource_data_by_name(ResourceId::new("studio.exr"))
			.expect("the stored EXR pixels must exist");
		let base_values = data[..16]
			.chunks_exact(2)
			.map(|bytes| exr::prelude::f16::from_le_bytes([bytes[0], bytes[1]]).to_f32())
			.collect::<Vec<_>>();

		assert_eq!(stored.class(), "Image");
		assert_eq!(image.format, Formats::RGBA16F);
		assert_eq!(image.gamma, Gamma::Linear);
		assert_eq!(image.extent, [2, 1, 1]);
		assert_eq!(image.mip_count, 1);
		let ibl = image.ibl.expect("EXR images must include baked IBL maps");
		assert_eq!(ibl.diffuse_irradiance.extent, [32, 16, 1]);
		assert_eq!(ibl.prefiltered_specular.extent, [2, 1, 1]);
		assert_eq!(ibl.prefiltered_specular.mip_count, 8);
		assert_eq!(base_values, vec![4.0, 0.5, -0.25, 1.0, 16.0, 2.0, 8.0, 1.0]);
		let streams = stored.streams().expect("the EXR image and IBL maps must be described");
		assert_eq!(streams.len(), 10);
		assert_eq!(streams[0].name(), crate::resources::image::IMAGE_BASE_MIP_STREAM_NAME);
		assert_eq!(streams[0].offset(), 0);
		assert_eq!(streams[0].size(), 16);
		assert_eq!(
			streams[1].name(),
			crate::resources::image::ibl_prefiltered_specular_stream_name(0)
		);
		assert_eq!(streams[1].offset(), 16);
		assert_eq!(streams[1].size(), 16);
		assert_eq!(
			streams.last().unwrap().name(),
			crate::resources::image::IBL_DIFFUSE_IRRADIANCE_STREAM_NAME
		);
		assert_eq!(data.len(), 4_184);
	}

	#[r#async::test]
	async fn malformed_exr_fails_without_storing_a_resource() {
		let source_storage = TestStorageBackend::new();
		source_storage.add_file("broken.exr", b"not an exr image");
		let resource_storage = TestResourceStorage::new();
		let mut asset_manager = AssetManager::new(source_storage);
		asset_manager.add_asset_handler(EXRAssetHandler::new());

		assert!(asset_manager.bake("broken.exr", &resource_storage).await.is_err());
		assert!(resource_storage.read(ResourceId::new("broken.exr")).is_none());
	}
}
