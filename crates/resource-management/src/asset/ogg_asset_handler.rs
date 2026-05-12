use super::{
	asset_handler::{AssetHandler, LoadErrors},
	asset_manager::AssetManager,
	ResourceId,
};
use crate::{
	asset,
	processors::audio_processor::process_audio,
	r#async::{spawn_cpu_task, BoxedFuture},
	resource,
	resources::audio::Audio,
	types::BitDepths,
	ProcessedAsset,
};

impl OGGAssetHandler {
	/// Decodes an OGG Vorbis buffer into audio metadata and PCM data.
	fn decode_ogg(data: &[u8], bit_depth: BitDepths) -> Result<(Audio, Vec<u8>), String> {
		use std::io::Cursor;

		let mut decoder = vorbis_rs::VorbisDecoder::new(Cursor::new(data))
			.map_err(|_| "Failed to decode OGG data. The file is likely corrupt or not Vorbis encoded.".to_string())?;

		let sample_rate = decoder.sampling_frequency().get();
		let channel_count = decoder.channels().get();

		let bytes_per_sample = usize::from(bit_depth) / 8;
		let mut data = Vec::with_capacity(channel_count as usize * sample_rate as usize * bytes_per_sample);

		while let Some(block) = decoder
			.decode_audio_block()
			.map_err(|_| "Failed to decode OGG data. The stream is likely corrupt.".to_string())?
		{
			let samples = block.samples();
			for &channel in samples {
				for sample in channel {
					push_pcm_sample(&mut data, *sample, bit_depth);
				}
			}
		}

		let sample_count = (data.len() / channel_count as usize / bytes_per_sample) as u32;
		let channel_count = channel_count as u16;

		let audio_resource = Audio {
			bit_depth,
			channel_count,
			sample_rate,
			sample_count,
		};

		Ok((audio_resource, data))
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

/// The `OGGAssetHandler` struct exists to decode OGG Vorbis assets into engine audio resources.
pub struct OGGAssetHandler {
	bit_depth: BitDepths,
}

fn push_pcm_sample(data: &mut Vec<u8>, sample: f32, bit_depth: BitDepths) {
	let sample = sample.clamp(-1.0, 1.0);

	match bit_depth {
		BitDepths::Eight => {
			let sample = ((sample * 0.5 + 0.5) * u8::MAX as f32).round() as u8;
			data.push(sample);
		}
		BitDepths::Sixteen => {
			let sample = (sample * i16::MAX as f32).round() as i16;
			data.extend_from_slice(&sample.to_le_bytes());
		}
		BitDepths::TwentyFour => {
			let sample = (sample * 8_388_607.0).round() as i32;
			let bytes = sample.to_le_bytes();
			data.extend_from_slice(&bytes[..3]);
		}
		BitDepths::ThirtyTwo => {
			let sample = (sample * i32::MAX as f32).round() as i32;
			data.extend_from_slice(&sample.to_le_bytes());
		}
	}
}

impl AssetHandler for OGGAssetHandler {
	fn can_handle(&self, r#type: &str) -> bool {
		r#type == "ogg"
	}

	fn bake<'a>(
		&'a self,
		_: &'a AssetManager,
		storage_backend: &'a dyn resource::StorageBackend,
		asset_storage_backend: &'a dyn asset::StorageBackend,
		url: ResourceId<'a>,
	) -> BoxedFuture<'a, Result<(ProcessedAsset, Box<[u8]>), LoadErrors>> {
		Box::pin(async move {
			if let Some(dt) = storage_backend.get_type(url) {
				if !self.can_handle(dt) {
					return Err(LoadErrors::UnsupportedType);
				}
			}

			let (data, _, dt) = asset_storage_backend
				.resolve(url)
				.await
				.or(Err(LoadErrors::AssetCouldNotBeLoaded))?;

			if !self.can_handle(&dt) {
				return Err(LoadErrors::UnsupportedType);
			}

			let bit_depth = self.bit_depth;
			let (audio_resource, data) = spawn_cpu_task(move || Self::decode_ogg(&data, bit_depth))
				.await
				.map_err(|_| LoadErrors::FailedToProcess)?
				.map_err(|_| LoadErrors::FailedToProcess)?;

			process_audio(url, audio_resource, data)
		})
	}
}

#[cfg(test)]
mod tests {
	use crate::{
		asset::{self, asset_manager::AssetManager, ogg_asset_handler::OGGAssetHandler, ResourceId},
		r#async, resource,
		resources::audio::Audio,
		types::BitDepths,
		AssetHandler,
	};

	#[r#async::test]
	async fn test_audio_asset_handler() {
		let audio_asset_handler = OGGAssetHandler::new();

		let asset_storage_backend = asset::storage_backend::tests::TestStorageBackend::new();
		let resource_storage_backend = resource::storage_backend::tests::TestStorageBackend::new();
		let asset_manager = AssetManager::new(asset_storage_backend.clone());

		let url = ResourceId::new("test-tone.ogg");
		asset_storage_backend.add_file("test-tone.ogg", &make_test_ogg());

		let (resource, data) = audio_asset_handler
			.bake(&asset_manager, &resource_storage_backend, &asset_storage_backend, url)
			.await
			.expect("Audio asset handler failed to load asset");

		crate::resource::WriteStorageBackend::store(&resource_storage_backend, &resource, &data)
			.expect("Audio asset failed to store");

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
			let (audio, data) = OGGAssetHandler::decode_ogg(&ogg, bit_depth).expect("Generated OGG should decode");

			assert_eq!(audio.bit_depth, bit_depth);
			assert_eq!(audio.channel_count, 1);
			assert_eq!(audio.sample_rate, 48_000);
			assert_eq!(audio.sample_count, 1024);
			assert_eq!(data.len(), 1024 * bytes_per_sample);
		}
	}

	/// Generates a deterministic OGG Vorbis fixture for the audio asset handler test.
	fn make_test_ogg() -> Vec<u8> {
		use std::num::{NonZeroU32, NonZeroU8};

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
