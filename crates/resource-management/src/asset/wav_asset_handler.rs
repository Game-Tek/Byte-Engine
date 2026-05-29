use super::{
	asset_handler::{AssetHandler, LoadErrors},
	asset_manager::AssetManager,
	audio_utils::{bit_depth_from_bits_per_sample, sample_count_from_pcm_len},
	ResourceId,
};
use crate::{
	asset, processors::audio_processor::process_audio, r#async::BoxedFuture, resource, resources::audio::Audio, ProcessedAsset,
};

impl WAVAssetHandler {
	/// Parses a WAV buffer into audio metadata and PCM data.
	fn decode_wav(data: &[u8]) -> Result<(Audio, Vec<u8>), String> {
		fn read_u16(bytes: &[u8], offset: usize, name: &str) -> Result<u16, String> {
			let bytes = bytes
				.get(offset..offset + 2)
				.ok_or_else(|| format!("Invalid {name}. The WAV file is likely truncated."))?;
			Ok(u16::from_le_bytes([bytes[0], bytes[1]]))
		}

		fn read_u32(bytes: &[u8], offset: usize, name: &str) -> Result<u32, String> {
			let bytes = bytes
				.get(offset..offset + 4)
				.ok_or_else(|| format!("Invalid {name}. The WAV file is likely truncated."))?;
			Ok(u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]))
		}

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

		let mut offset = 12;
		let mut wav_format = None;
		let mut pcm_data = None;

		while offset + 8 <= data.len() {
			let chunk_id = data.get(offset..offset + 4).expect("chunk header was bounds checked");
			let chunk_size = read_u32(data, offset + 4, "chunk size")? as usize;
			let chunk_start = offset + 8;
			let chunk_end = chunk_start
				.checked_add(chunk_size)
				.ok_or_else(|| "Invalid chunk size. The WAV chunk size likely overflowed.".to_string())?;
			let chunk = data
				.get(chunk_start..chunk_end)
				.ok_or_else(|| "Invalid chunk data. The WAV file is likely truncated.".to_string())?;

			match chunk_id {
				b"fmt " => {
					if chunk_size < 16 {
						return Err("Invalid fmt chunk. The WAV format chunk is likely incomplete.".to_string());
					}

					let audio_format = read_u16(chunk, 0, "audio format")?;
					let num_channels = read_u16(chunk, 2, "channel count")?;
					let sample_rate = read_u32(chunk, 4, "sample rate")?;
					let bits_per_sample = read_u16(chunk, 14, "bits per sample")?;

					let is_pcm = match audio_format {
						0x0001 => true,
						0xfffe => {
							if chunk_size < 40 {
								return Err(
									"Invalid extensible fmt chunk. The WAV format chunk is likely incomplete.".to_string()
								);
							}
							let sub_format = chunk.get(24..40).ok_or_else(|| {
								"Invalid extensible format. The WAV format chunk is likely incomplete.".to_string()
							})?;
							sub_format
								== [
									0x01, 0x00, 0x00, 0x00, 0x00, 0x00, 0x10, 0x00, 0x80, 0x00, 0x00, 0xaa, 0x00, 0x38, 0x9b,
									0x71,
								]
						}
						_ => false,
					};
					if !is_pcm {
						return Err("Unsupported audio format. The WAV file is likely not PCM encoded.".to_string());
					}
					if num_channels != 1 && num_channels != 2 {
						return Err(
							"Unsupported channel count. The WAV header likely reports an unsupported configuration."
								.to_string(),
						);
					}

					let bit_depth = bit_depth_from_bits_per_sample(bits_per_sample).ok_or_else(|| {
						"Unsupported bit depth. The WAV header likely reports an unsupported format.".to_string()
					})?;

					wav_format = Some((bit_depth, num_channels, sample_rate, bits_per_sample));
				}
				b"data" => pcm_data = Some(chunk),
				_ => {}
			}

			offset = chunk_end + (chunk_size % 2);
		}

		let (bit_depth, num_channels, sample_rate) = wav_format
			.map(|(bit_depth, num_channels, sample_rate, _bits_per_sample)| (bit_depth, num_channels, sample_rate))
			.ok_or_else(|| "Missing fmt chunk. The WAV file likely has no format description.".to_string())?;
		let data = pcm_data.ok_or_else(|| "Missing data chunk. The WAV file likely has no PCM payload.".to_string())?;
		let sample_count = sample_count_from_pcm_len(data.len(), num_channels, bit_depth);
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

/// The `WAVAssetHandler` struct exists to load WAV audio assets into engine audio resources.
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
		allocator: &'a dyn std::alloc::Allocator,
	) -> BoxedFuture<'a, Result<(ProcessedAsset, Box<[u8]>), LoadErrors>> {
		Box::pin(async move {
			if let Some(dt) = storage_backend.get_type(url) {
				if !self.can_handle(dt) {
					return Err(LoadErrors::UnsupportedType);
				}
			}

			let (data, _, dt) = asset_storage_backend
				.resolve_in(url, allocator)
				.await
				.or(Err(LoadErrors::AssetCouldNotBeLoaded))?;

			if !self.can_handle(&dt) {
				return Err(LoadErrors::UnsupportedType);
			}

			let (audio_resource, data) = Self::decode_wav(&data).map_err(|_| LoadErrors::FailedToProcess)?;

			process_audio(url, audio_resource, data)
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

	fn chunk(id: &[u8; 4], payload: &[u8]) -> Vec<u8> {
		let mut chunk = Vec::new();
		chunk.extend_from_slice(id);
		chunk.extend_from_slice(&(payload.len() as u32).to_le_bytes());
		chunk.extend_from_slice(payload);
		if payload.len() % 2 == 1 {
			chunk.push(0);
		}
		chunk
	}

	fn riff(chunks: &[Vec<u8>]) -> Vec<u8> {
		let mut wav = Vec::new();
		wav.extend_from_slice(b"RIFF");
		let size = 4 + chunks.iter().map(Vec::len).sum::<usize>() as u32;
		wav.extend_from_slice(&size.to_le_bytes());
		wav.extend_from_slice(b"WAVE");
		for chunk in chunks {
			wav.extend_from_slice(chunk);
		}
		wav
	}

	fn pcm_fmt() -> Vec<u8> {
		let mut fmt = Vec::new();
		fmt.extend_from_slice(&1u16.to_le_bytes());
		fmt.extend_from_slice(&2u16.to_le_bytes());
		fmt.extend_from_slice(&44_100u32.to_le_bytes());
		fmt.extend_from_slice(&176_400u32.to_le_bytes());
		fmt.extend_from_slice(&4u16.to_le_bytes());
		fmt.extend_from_slice(&16u16.to_le_bytes());
		fmt
	}

	#[test]
	fn decode_wav_skips_extra_metadata_chunks() {
		let pcm = [1, 2, 3, 4, 5, 6, 7, 8];
		let wav = riff(&[
			chunk(b"JUNK", b"abc"),
			chunk(b"fmt ", &pcm_fmt()),
			chunk(b"LIST", b"INFOISFTByte Engine"),
			chunk(b"data", &pcm),
		]);

		let (audio, data) = WAVAssetHandler::decode_wav(&wav).expect("WAV should decode");

		assert_eq!(audio.bit_depth, BitDepths::Sixteen);
		assert_eq!(audio.channel_count, 2);
		assert_eq!(audio.sample_rate, 44_100);
		assert_eq!(audio.sample_count, 2);
		assert_eq!(data, pcm);
	}

	#[test]
	fn decode_wav_allows_reordered_chunks_and_extensible_pcm() {
		let pcm = [1, 2, 3, 4];
		let mut fmt = pcm_fmt();
		fmt[0..2].copy_from_slice(&0xfffeu16.to_le_bytes());
		fmt.extend_from_slice(&22u16.to_le_bytes());
		fmt.extend_from_slice(&16u16.to_le_bytes());
		fmt.extend_from_slice(&3u32.to_le_bytes());
		fmt.extend_from_slice(&[
			0x01, 0x00, 0x00, 0x00, 0x00, 0x00, 0x10, 0x00, 0x80, 0x00, 0x00, 0xaa, 0x00, 0x38, 0x9b, 0x71,
		]);
		let wav = riff(&[chunk(b"data", &pcm), chunk(b"fmt ", &fmt)]);

		let (audio, data) = WAVAssetHandler::decode_wav(&wav).expect("WAV should decode");

		assert_eq!(audio.bit_depth, BitDepths::Sixteen);
		assert_eq!(audio.sample_count, 1);
		assert_eq!(data, pcm);
	}

	#[r#async::test]
	#[ignore = "Test uses data not pushed to the repository"]
	async fn test_audio_asset_handler() {
		let audio_asset_handler = WAVAssetHandler::new();

		let asset_storage_backend = asset::storage_backend::tests::TestStorageBackend::new();
		let resource_storage_backend = resource::storage_backend::tests::TestStorageBackend::new();
		let asset_manager = AssetManager::new(asset_storage_backend.clone());

		let url = ResourceId::new("gun.wav");

		let (resource, data) = audio_asset_handler
			.bake(
				&asset_manager,
				&resource_storage_backend,
				&asset_storage_backend,
				url,
				&std::alloc::Global,
			)
			.await
			.expect("Audio asset handler failed to load asset");

		crate::resource::WriteStorageBackend::store(&resource_storage_backend, &resource, &data)
			.expect("Audio asset failed to store");

		let generated_resources = resource_storage_backend.get_resources();

		assert_eq!(generated_resources.len(), 1);

		let resource = &generated_resources[0];

		assert_eq!(resource.id, "gun.wav");
		assert_eq!(resource.class, "Audio");
		let resource: Audio = crate::from_slice(&resource.resource).unwrap();
		assert_eq!(resource.bit_depth, BitDepths::Sixteen);
		assert_eq!(resource.channel_count, 1);
		assert_eq!(resource.sample_rate, 48000);
		assert_eq!(resource.sample_count, 152456 / 1 / (16 / 8));
	}
}
