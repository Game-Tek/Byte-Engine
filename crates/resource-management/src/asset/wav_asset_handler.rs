use crate::{asset, r#async::BoxedFuture, resource, resources::audio::Audio, types::BitDepths, ProcessedAsset};

use super::{
	asset_handler::{AssetHandler, LoadErrors},
	asset_manager::AssetManager,
	ResourceId,
};

impl WAVAssetHandler {
	/// Parses a WAV buffer into audio metadata and PCM data.
	fn decode_wav(data: &[u8]) -> Result<(Audio, Vec<u8>), String> {
		let riff = data
			.get(0..4)
			.ok_or_else(|| "Invalid RIFF header. The file is likely truncated or not a WAV asset.".to_string())?;
		if riff != b"RIFF" {
			return Err("Invalid RIFF header. The file is likely not a WAV asset.".to_string());
		}
		let format = data
			.get(8..12)
			.ok_or_else(|| "Invalid WAVE format. The file is likely truncated or not a WAV asset.".to_string())?;
		if format != b"WAVE" {
			return Err("Invalid WAVE format. The file is likely not a WAV asset.".to_string());
		}
		let audio_format = data
			.get(20..22)
			.ok_or_else(|| "Invalid audio format. The WAV header is likely incomplete.".to_string())?;
		if audio_format != b"\x01\x00" {
			return Err("Unsupported audio format. The WAV file is likely not PCM encoded.".to_string());
		}
		let subchunk_1_size = data
			.get(16..20)
			.ok_or_else(|| "Invalid subchunk size. The WAV header is likely incomplete.".to_string())?;
		let subchunk_1_size =
			u32::from_le_bytes([subchunk_1_size[0], subchunk_1_size[1], subchunk_1_size[2], subchunk_1_size[3]]);
		if subchunk_1_size != 16 {
			return Err("Invalid subchunk size. The WAV header is likely malformed.".to_string());
		}
		let num_channels = data
			.get(22..24)
			.ok_or_else(|| "Invalid channel count. The WAV header is likely incomplete.".to_string())?;
		let num_channels = u16::from_le_bytes([num_channels[0], num_channels[1]]);
		if num_channels != 1 && num_channels != 2 {
			return Err("Unsupported channel count. The WAV header likely reports an unsupported configuration.".to_string());
		}
		let sample_rate = data
			.get(24..28)
			.ok_or_else(|| "Invalid sample rate. The WAV header is likely incomplete.".to_string())?;
		let sample_rate = u32::from_le_bytes([sample_rate[0], sample_rate[1], sample_rate[2], sample_rate[3]]);
		let bits_per_sample = data
			.get(34..36)
			.ok_or_else(|| "Invalid bits per sample. The WAV header is likely incomplete.".to_string())?;
		let bits_per_sample = u16::from_le_bytes([bits_per_sample[0], bits_per_sample[1]]);
		let bit_depth = match bits_per_sample {
			8 => BitDepths::Eight,
			16 => BitDepths::Sixteen,
			24 => BitDepths::TwentyFour,
			32 => BitDepths::ThirtyTwo,
			_ => {
				return Err("Unsupported bit depth. The WAV header likely reports an unsupported format.".to_string());
			}
		};
		let data_header = data
			.get(36..40)
			.ok_or_else(|| "Invalid data header. The WAV header is likely incomplete.".to_string())?;
		if data_header != b"data" {
			return Err("Invalid data header. The WAV header is likely malformed.".to_string());
		}
		let data_size = data
			.get(40..44)
			.ok_or_else(|| "Invalid data size. The WAV header is likely incomplete.".to_string())?;
		let data_size = u32::from_le_bytes([data_size[0], data_size[1], data_size[2], data_size[3]]);
		let sample_count = data_size / (bits_per_sample / 8) as u32 / num_channels as u32;
		let data = data
			.get(44..)
			.ok_or_else(|| "Invalid PCM data. The WAV file is likely truncated.".to_string())?;
		let data = data
			.get(..data_size as usize)
			.ok_or_else(|| "Invalid PCM data. The WAV file is likely truncated.".to_string())?;
		let audio_resource = Audio {
			bit_depth,
			channel_count: num_channels,
			sample_rate,
			sample_count,
		};
		Ok((audio_resource, data.to_vec()))
	}

	pub fn new() -> WAVAssetHandler {
		WAVAssetHandler {}
	}
}

pub struct WAVAssetHandler {}

impl AssetHandler for WAVAssetHandler {
	fn can_handle(&self, r#type: &str) -> bool {
		r#type == "wav"
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

			let (audio_resource, data) = Self::decode_wav(&data).map_err(|_| LoadErrors::FailedToProcess)?;

			Ok((ProcessedAsset::new(url, audio_resource), data.into_boxed_slice()))
		})
	}
}

#[cfg(test)]
mod tests {
	use crate::{
		asset::{self, asset_manager::AssetManager, wav_asset_handler::WAVAssetHandler, ResourceId},
		r#async, resource,
		resources::audio::Audio,
		types::BitDepths,
		AssetHandler,
	};

	#[r#async::test]
	#[ignore = "Test uses data not pushed to the repository"]
	async fn test_audio_asset_handler() {
		let audio_asset_handler = WAVAssetHandler::new();

		let asset_storage_backend = asset::storage_backend::tests::TestStorageBackend::new();
		let resource_storage_backend = resource::storage_backend::tests::TestStorageBackend::new();
		let asset_manager = AssetManager::new(asset_storage_backend.clone());

		let url = ResourceId::new("gun.wav");

		let (resource, data) = audio_asset_handler
			.bake(&asset_manager, &resource_storage_backend, &asset_storage_backend, url)
			.await
			.expect("Audio asset handler failed to load asset");

		crate::resource::WriteStorageBackend::store(&resource_storage_backend, &resource, &data)
			.expect("Audio asset failed to store");

		let generated_resources = resource_storage_backend.get_resources();

		assert_eq!(generated_resources.len(), 1);

		let resource = &generated_resources[0];

		assert_eq!(resource.id, "gun.wav");
		assert_eq!(resource.class, "Audio");
		let resource: Audio = pot::from_slice(&resource.resource).unwrap();
		assert_eq!(resource.bit_depth, BitDepths::Sixteen);
		assert_eq!(resource.channel_count, 1);
		assert_eq!(resource.sample_rate, 48000);
		assert_eq!(resource.sample_count, 152456 / 1 / (16 / 8));
	}
}
