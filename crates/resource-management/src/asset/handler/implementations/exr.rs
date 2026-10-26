/// The `EXRAssetHandler` struct lets an asset manager bake EXR files into linear HDR image resources.
///
/// It only decodes; the shared [`ImageProcessor`] stores the pixels as an
/// `RGBA16F` mip chain, as it would any other half-float source.
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

	/// Decodes one EXR and stores its linear image without environment lighting maps.
	async fn bake<'a>(&'a self, context: BakeContext<'a>, url: ResourceId<'a>) -> Result<(), LoadErrors> {
		let (source, data_type) = context.resolve(url).await?;

		if !self.can_handle(&data_type) {
			return Err(LoadErrors::UnsupportedType);
		}

		// Decoded EXR pixels are already linear, so only the semantic comes from the file name.
		let semantic = guess_semantic_from_name(url.get_base());
		let spec = context.load_sidecar(url).await?;
		let settings = image_settings(&context, url, spec.as_ref(), IMAGE_SETTINGS_KEY, semantic)?;

		// Decoding reserves the whole stored chain, so the processor keeps the decoded pixels as the stored image.
		let capacity = |extent| {
			ImageProcessor::in_place_capacity(
				semantic,
				Gamma::Linear,
				settings,
				extent,
				SourceChannels::RGBA,
				SourceEncoding::F16LittleEndian,
			)
			.unwrap_or(0)
		};
		let decoded = decode_rgba16f_in(source.as_slice(), context.allocator(), capacity).map_err(|error| {
			context.error(format_args!(
				"EXR image '{}' could not be decoded. The most likely cause is invalid or unsupported EXR data. Decoder: {error}",
				url.as_ref()
			));

			LoadErrors::FailedToProcess
		})?;

		if decoded.format() != image::ImageFormat::OpenExr {
			return Err(LoadErrors::UnsupportedType);
		}

		let source = OwnedImageSource {
			extent: decoded.extent(),
			channels: SourceChannels::RGBA,
			encoding: SourceEncoding::F16LittleEndian,
			data: decoded.into_data(),
		};
		let image = context
			.image_processor()
			.process_owned_in(semantic, Gamma::Linear, settings, source, context.allocator())
			.await?;
		let (asset, data) = image.into_asset(url);

		context.store_primary(asset, &data).await
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
		resource::{ReadStorageBackend as _, storage_backend::MemoryStorageBackend},
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

	#[r#async::test]
	async fn exr_bakes_a_linear_hdr_mip_chain() {
		let source_storage = TestStorageBackend::new();

		source_storage.add_file("studio.exr", &hdr_fixture());

		let resource_storage = MemoryStorageBackend::new();
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
			.as_chunks::<2>()
			.0
			.iter()
			.map(|bytes| exr::prelude::f16::from_le_bytes([bytes[0], bytes[1]]).to_f32())
			.collect::<Vec<_>>();

		assert_eq!(stored.class(), "Image");
		assert_eq!(image.format, Formats::RGBA16F);
		assert_eq!(image.gamma, Gamma::Linear);
		assert_eq!(image.extent, [2, 1, 0]);
		assert_eq!(image.mip_count, 2);
		assert!(image.ibl.is_none());
		assert!(image.photometry.is_none());
		// The base level keeps the HDR values, and the 1x1 level averages them without clamping.
		assert_eq!(
			values,
			vec![4.0, 0.5, -0.25, 1.0, 16.0, 2.0, 8.0, 1.0, 10.0, 1.25, 3.875, 1.0]
		);

		let streams = stored.streams().expect("the EXR image must describe its mips");

		assert_eq!(streams.len(), 2);
		assert_eq!(streams[0].name(), crate::resources::image::IMAGE_BASE_MIP_STREAM_NAME);
		assert_eq!((streams[0].offset(), streams[0].size()), (0, 16));
		assert_eq!((streams[1].offset(), streams[1].size()), (16, 8));
		assert_eq!(data.len(), 24);
	}

	#[r#async::test]
	async fn malformed_exr_fails_without_storing_a_resource() {
		let source_storage = TestStorageBackend::new();

		source_storage.add_file("broken.exr", b"not an exr image");

		let resource_storage = MemoryStorageBackend::new();
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
	asset::{IMAGE_SETTINGS_KEY, image_settings},
	processors::image::{
		ImageProcessor, OwnedImageSource, SourceChannels, SourceEncoding, decode_rgba16f_in, guess_semantic_from_name,
	},
	types::Gamma,
};
