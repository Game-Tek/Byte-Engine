/// The `OGGAssetHandler` struct exists to decode OGG Vorbis assets into engine audio resources.
pub struct OGGAssetHandler {
	bit_depth: BitDepths,
}

impl OGGAssetHandler {
	/// Decodes an OGG Vorbis buffer through the common audio processor.
	fn decode_ogg<'a>(
		id: ResourceId<'_>,
		data: &'a [u8],
		bit_depth: BitDepths,
	) -> Result<(ProcessedAsset, Cow<'a, [u8]>), LoadErrors> {
		use std::io::Cursor;

		let mut decoder = vorbis_rs::VorbisDecoder::new(Cursor::new(data)).map_err(|_| LoadErrors::FailedToProcess)?;

		let sample_rate = decoder.sampling_frequency().get();

		let description = AudioDescription {
			bit_depth,
			channel_count: u16::from(decoder.channels().get()),
			sample_rate,
		};

		process_audio(id, description, |sink| {
			while let Some(block) = decoder.decode_audio_block().map_err(|_| LoadErrors::FailedToProcess)? {
				sink.append_planar_f32(block.samples())?;
			}

			Ok(())
		})
	}

	pub fn new() -> OGGAssetHandler {
		OGGAssetHandler {
			bit_depth: BitDepths::Sixteen,
		}
	}

	/// Creates an OGG asset handler that outputs PCM at the requested bit depth.
	pub fn with_bit_depth(bit_depth: BitDepths) -> OGGAssetHandler {
		OGGAssetHandler { bit_depth }
	}
}

impl AssetHandler for OGGAssetHandler {
	fn can_handle(&self, r#type: &str) -> bool {
		r#type == "ogg"
	}

	async fn bake<'a>(&'a self, context: BakeContext<'a>, url: ResourceId<'a>) -> Result<(), LoadErrors> {
		if let Some(dt) = context.resource_type(url) {
			if !self.can_handle(dt) {
				return Err(LoadErrors::UnsupportedType);
			}
		}

		let (data, _, dt) = context.resolve(url).await?;

		if !self.can_handle(&dt) {
			return Err(LoadErrors::UnsupportedType);
		}

		// The decoder lends each planar block until the next decode call, so the
		// common sink consumes every block before requesting the next one.
		let (asset, data) = Self::decode_ogg(url, &data, self.bit_depth)?;

		match data {
			Cow::Borrowed(data) => context.store_primary(asset, data).await,
			Cow::Owned(data) => context.store_primary_owned(asset, data).await,
		}
	}
}

impl Default for OGGAssetHandler {
	fn default() -> Self {
		Self::new()
	}
}

#[cfg(test)]
mod tests {
	use crate::{
		AssetHandler,
		asset::{self, ResourceId, handler::implementations::ogg::OGGAssetHandler, manager::AssetManager},
		r#async, resource,
		resources::audio::Audio,
		types::BitDepths,
	};

	#[r#async::test]
	async fn test_audio_asset_handler() {
		let asset_storage_backend = asset::storage_backend::tests::TestStorageBackend::new();

		let resource_storage_backend = resource::storage_backend::tests::TestStorageBackend::new();

		asset_storage_backend.add_file("test-tone.ogg", &make_test_ogg());

		let mut asset_manager = AssetManager::new(asset_storage_backend, resource_storage_backend.clone());

		asset_manager.add_asset_handler(OGGAssetHandler::new());

		asset_manager
			.bake("test-tone.ogg")
			.await
			.expect("Audio asset handler failed to load asset");

		let generated_resources = resource_storage_backend.get_resources();

		assert_eq!(generated_resources.len(), 1);

		let resource = &generated_resources[0];

		assert_eq!(resource.id, "test-tone.ogg");
		assert_eq!(resource.class, "Audio");

		let resource: Audio = crate::from_slice(&resource.resource).unwrap();

		assert_eq!(resource.bit_depth, BitDepths::Sixteen);
		assert_eq!(resource.channel_count, 1);
		assert_eq!(resource.sample_rate, 48_000);
		assert_eq!(resource.sample_count, 1024);

		let data = resource_storage_backend
			.get_resource_data_by_name(ResourceId::new("test-tone.ogg"))
			.expect("Audio resource data should exist");

		assert_eq!(data.len(), 1024 * 2);
	}

	#[test]
	fn decode_ogg_supports_configured_output_bit_depths() {
		let ogg = make_test_ogg();

		for (bit_depth, bytes_per_sample) in [
			(BitDepths::Eight, 1),
			(BitDepths::Sixteen, 2),
			(BitDepths::TwentyFour, 3),
			(BitDepths::ThirtyTwo, 4),
		] {
			let (asset, data) = OGGAssetHandler::decode_ogg(ResourceId::new("generated.ogg"), &ogg, bit_depth)
				.expect("Generated OGG should decode");
			let audio: Audio = crate::from_slice(&asset.resource).unwrap();

			assert_eq!(audio.bit_depth, bit_depth);
			assert_eq!(audio.channel_count, 1);
			assert_eq!(audio.sample_rate, 48_000);
			assert_eq!(audio.sample_count, 1024);
			assert_eq!(data.len(), 1024 * bytes_per_sample);
		}
	}

	/// Generates a deterministic OGG Vorbis fixture for the audio asset handler test.
	fn make_test_ogg() -> Vec<u8> {
		use std::num::{NonZeroU8, NonZeroU32};

		let sample_rate = NonZeroU32::new(48_000).unwrap();

		let channels = NonZeroU8::new(1).unwrap();

		let sink = Vec::new();

		let mut builder = vorbis_rs::VorbisEncoderBuilder::new_with_serial(sample_rate, channels, sink, 1);

		let mut encoder = builder.build().expect("Test OGG encoder should initialize");

		let samples: Vec<f32> = (0..1024)
			.map(|index| ((index as f32 / 48_000.0) * 440.0 * std::f32::consts::TAU).sin() * 0.25)
			.collect();

		encoder.encode_audio_block([samples]).expect("Test OGG samples should encode");

		encoder.finish().expect("Test OGG stream should finish")
	}
}

use std::borrow::Cow;

use super::{
	ResourceId,
	handler::{AssetHandler, BakeContext, LoadErrors},
};
use crate::{
	ProcessedAsset,
	processors::processor::implementations::audio::{AudioDescription, process_audio},
	types::BitDepths,
};
