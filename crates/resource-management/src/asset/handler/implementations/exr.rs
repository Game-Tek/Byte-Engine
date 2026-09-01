/// The `EXRAssetHandler` struct provides standalone linear HDR image resources.
pub struct EXRAssetHandler;

impl EXRAssetHandler {
	/// Creates a handler that stores EXR pixels as a plain RGBA16F image.
	///
	/// Next, register this handler with [`crate::asset::manager::AssetManager::add_asset_handler`].
	pub fn new() -> Self {
		Self
	}
}

impl Default for EXRAssetHandler {
	fn default() -> Self {
		Self::new()
	}
}

impl AssetHandler for EXRAssetHandler {
	fn can_handle(&self, r#type: &str) -> bool {
		r#type.eq_ignore_ascii_case("exr")
			|| r#type == "Image"
			|| r#type.eq_ignore_ascii_case("image/exr")
			|| r#type.eq_ignore_ascii_case("image/x-exr")
	}

	/// Decodes one EXR and stores only its linear base image without environment lighting maps.
	async fn bake<'a>(&'a self, context: BakeContext<'a>, url: ResourceId<'a>) -> Result<(), LoadErrors> {
		if let Some(data_type) = context.resource_type(url)
			&& !self.can_handle(data_type)
		{
			return Err(LoadErrors::UnsupportedType);
		}

		let (source, _, data_type) = context.resolve(url).await?;

		if !self.can_handle(&data_type) {
			return Err(LoadErrors::UnsupportedType);
		}

		let decoded = decode_rgba16f_in(source.as_slice(), context.allocator()).map_err(|error| {
			context.error(format_args!(
				"EXR image '{}' could not be decoded. The most likely cause is invalid or unsupported EXR data. Decoder: {error}",
				url.as_ref()
			));

			LoadErrors::FailedToProcess
		})?;

		if decoded.format() != image::ImageFormat::OpenExr {
			return Err(LoadErrors::UnsupportedType);
		}

		let image = Image {
			format: Formats::RGBA16F,
			gamma: Gamma::Linear,
			extent: decoded.extent().as_array(),
			mip_count: 1,
			ibl: None,
			photometry: None,
		};
		let streams = vec![StreamDescription::new(IMAGE_BASE_MIP_STREAM_NAME, decoded.data().len(), 0)];

		context
			.store_primary(ProcessedAsset::new(url, image).with_streams(streams), decoded.data())
			.await
	}
}

#[cfg(test)]
mod tests {
	use std::io::Cursor;

	use exr::prelude::{SpecificChannels, WritableImage as _};

	use super::EXRAssetHandler;
	use crate::{
		asset::{ResourceId, manager::AssetManager, storage_backend::tests::TestStorageBackend},
		r#async,
		resource::{ReadStorageBackend as _, storage_backend::tests::TestStorageBackend as TestResourceStorage},
		resources::image::Image,
		types::{Formats, Gamma},
	};

	/// Encodes a tiny HDR fixture with values outside normalized image range.
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
	async fn exr_bakes_a_plain_linear_hdr_image() {
		let source_storage = TestStorageBackend::new();

		source_storage.add_file("studio.exr", &hdr_fixture());

		let resource_storage = TestResourceStorage::new();
		let mut asset_manager = AssetManager::new(source_storage, resource_storage.clone());

		asset_manager.add_asset_handler(EXRAssetHandler::new());
		asset_manager
			.bake("studio.exr")
			.await
			.expect("the registered EXR handler must bake the image");

		let (stored, _) = resource_storage
			.read(ResourceId::new("studio.exr"))
			.await
			.expect("the baked EXR image must be stored");
		let image: Image = crate::from_slice(stored.resource()).expect("the stored EXR metadata must deserialize");
		let data = resource_storage
			.get_resource_data_by_name(ResourceId::new("studio.exr"))
			.expect("the stored EXR pixels must exist");
		let values = data
			.chunks_exact(2)
			.map(|bytes| exr::prelude::f16::from_le_bytes([bytes[0], bytes[1]]).to_f32())
			.collect::<Vec<_>>();

		assert_eq!(stored.class(), "Image");
		assert_eq!(image.format, Formats::RGBA16F);
		assert_eq!(image.gamma, Gamma::Linear);
		assert_eq!(image.extent, [2, 1, 0]);
		assert_eq!(image.mip_count, 1);
		assert!(image.ibl.is_none());
		assert!(image.photometry.is_none());
		assert_eq!(values, vec![4.0, 0.5, -0.25, 1.0, 16.0, 2.0, 8.0, 1.0]);

		let streams = stored.streams().expect("the EXR image must describe its base mip");

		assert_eq!(streams.len(), 1);
		assert_eq!(streams[0].name(), crate::resources::image::IMAGE_BASE_MIP_STREAM_NAME);
		assert_eq!(streams[0].offset(), 0);
		assert_eq!(streams[0].size(), 16);
		assert_eq!(data.len(), 16);
	}

	#[r#async::test]
	async fn malformed_exr_fails_without_storing_a_resource() {
		let source_storage = TestStorageBackend::new();

		source_storage.add_file("broken.exr", b"not an exr image");

		let resource_storage = TestResourceStorage::new();
		let mut asset_manager = AssetManager::new(source_storage, resource_storage.clone());

		asset_manager.add_asset_handler(EXRAssetHandler::new());

		assert!(asset_manager.bake("broken.exr").await.is_err());
		assert!(resource_storage.read(ResourceId::new("broken.exr")).await.is_none());
	}
}

use super::{
	ResourceId,
	handler::{AssetHandler, BakeContext, LoadErrors},
};
use crate::{
	ProcessedAsset, StreamDescription,
	processors::processor::implementations::image::decode_rgba16f_in,
	resources::image::{IMAGE_BASE_MIP_STREAM_NAME, Image},
	types::{Formats, Gamma},
};
