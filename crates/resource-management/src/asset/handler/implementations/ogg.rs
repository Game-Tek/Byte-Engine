/// The `OGGAssetHandler` struct exists to decode OGG Vorbis assets into engine audio resources.
///
/// Decoded audio is stored as 16-bit PCM.
pub struct OGGAssetHandler;

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
		OGGAssetHandler
	}
}

impl AssetHandler for OGGAssetHandler {
	fn can_handle(&self, r#type: &str) -> bool {
		r#type == "ogg"
	}

	async fn bake<'a>(&'a self, context: BakeContext<'a>, url: ResourceId<'a>) -> Result<(), LoadErrors> {
		let (data, dt) = context.resolve(url).await?;

		if !self.can_handle(&dt) {
			return Err(LoadErrors::UnsupportedType);
		}

		// The decoder lends each planar block until the next decode call, so the
		// common sink consumes every block before requesting the next one.
		let (asset, data) = Self::decode_ogg(url, &data, BitDepths::Sixteen)?;

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
		asset::{ResourceId, handler::implementations::ogg::OGGAssetHandler},
		resources::audio::Audio,
		types::BitDepths,
	};

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
	processors::audio::{AudioDescription, process_audio},
	types::BitDepths,
};
