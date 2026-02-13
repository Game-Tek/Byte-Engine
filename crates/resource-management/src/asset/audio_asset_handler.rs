use std::sync::Arc;

use crate::{
    asset,
    r#async::{spawn_cpu_task, BoxedFuture},
    resource,
    resources::audio::Audio,
    types::BitDepths,
    ProcessedAsset,
};

use super::{
    asset_handler::{Asset, AssetHandler, LoadErrors},
    asset_manager::AssetManager,
    ResourceId,
};

pub struct AudioAsset {
    id: String,
    data: Arc<[u8]>,
}

impl Asset for AudioAsset {
    fn requested_assets(&self) -> Vec<String> {
        vec![]
    }

    fn load<'a>(
        &'a self,
        _: &'a AssetManager,
        storage_backend: &'a dyn resource::StorageBackend,
        _: &'a dyn asset::StorageBackend,
        id: ResourceId<'a>,
    ) -> BoxedFuture<'a, Result<(), String>> {
        Box::pin(async move {
            let extension = id.get_extension();

            let (audio_resource, data) = match extension {
                "wav" => Self::decode_wav(&self.data)?,
                "ogg" => {
                    let data = self.data.clone();
                    let decoded = spawn_cpu_task(move || Self::decode_ogg(&data))
                        .await
                        .or_else(|_| Err("Task panicked".to_string()))?;
                    decoded?
                }
                _ => {
                    return Err("Unsupported audio format. The asset extension is not handled by the audio loader.".to_string());
                }
            };

            let resource = ProcessedAsset::new(ResourceId::new(&self.id), audio_resource);
            storage_backend.store(&resource, &data).map_err(|_| {
                "Failed to store audio resource. The storage backend likely rejected the write."
                    .to_string()
            })?;
            Ok(())
        })
    }
}

impl AudioAsset {
    /// Parses a WAV buffer into audio metadata and PCM data.
    fn decode_wav(data: &[u8]) -> Result<(Audio, Vec<u8>), String> {
        let riff = data.get(0..4).ok_or_else(|| {
            "Invalid RIFF header. The file is likely truncated or not a WAV asset.".to_string()
        })?;
        if riff != b"RIFF" {
            return Err("Invalid RIFF header. The file is likely not a WAV asset.".to_string());
        }
        let format = data.get(8..12).ok_or_else(|| {
            "Invalid WAVE format. The file is likely truncated or not a WAV asset.".to_string()
        })?;
        if format != b"WAVE" {
            return Err("Invalid WAVE format. The file is likely not a WAV asset.".to_string());
        }
        let audio_format = data.get(20..22).ok_or_else(|| {
            "Invalid audio format. The WAV header is likely incomplete.".to_string()
        })?;
        if audio_format != b"\x01\x00" {
            return Err(
                "Unsupported audio format. The WAV file is likely not PCM encoded.".to_string(),
            );
        }
        let subchunk_1_size = data.get(16..20).ok_or_else(|| {
            "Invalid subchunk size. The WAV header is likely incomplete.".to_string()
        })?;
        let subchunk_1_size = u32::from_le_bytes([
            subchunk_1_size[0],
            subchunk_1_size[1],
            subchunk_1_size[2],
            subchunk_1_size[3],
        ]);
        if subchunk_1_size != 16 {
            return Err("Invalid subchunk size. The WAV header is likely malformed.".to_string());
        }
        let num_channels = data.get(22..24).ok_or_else(|| {
            "Invalid channel count. The WAV header is likely incomplete.".to_string()
        })?;
        let num_channels = u16::from_le_bytes([num_channels[0], num_channels[1]]);
        if num_channels != 1 && num_channels != 2 {
            return Err("Unsupported channel count. The WAV header likely reports an unsupported configuration.".to_string());
        }
        let sample_rate = data.get(24..28).ok_or_else(|| {
            "Invalid sample rate. The WAV header is likely incomplete.".to_string()
        })?;
        let sample_rate = u32::from_le_bytes([
            sample_rate[0],
            sample_rate[1],
            sample_rate[2],
            sample_rate[3],
        ]);
        let bits_per_sample = data.get(34..36).ok_or_else(|| {
            "Invalid bits per sample. The WAV header is likely incomplete.".to_string()
        })?;
        let bits_per_sample = u16::from_le_bytes([bits_per_sample[0], bits_per_sample[1]]);
        let bit_depth = match bits_per_sample {
            8 => BitDepths::Eight,
            16 => BitDepths::Sixteen,
            24 => BitDepths::TwentyFour,
            32 => BitDepths::ThirtyTwo,
            _ => {
                return Err(
                    "Unsupported bit depth. The WAV header likely reports an unsupported format."
                        .to_string(),
                );
            }
        };
        let data_header = data.get(36..40).ok_or_else(|| {
            "Invalid data header. The WAV header is likely incomplete.".to_string()
        })?;
        if data_header != b"data" {
            return Err("Invalid data header. The WAV header is likely malformed.".to_string());
        }
        let data_size = data
            .get(40..44)
            .ok_or_else(|| "Invalid data size. The WAV header is likely incomplete.".to_string())?;
        let data_size =
            u32::from_le_bytes([data_size[0], data_size[1], data_size[2], data_size[3]]);
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

    /// Decodes an OGG Vorbis buffer into audio metadata and PCM data.
    fn decode_ogg(data: &[u8]) -> Result<(Audio, Vec<u8>), String> {
        use std::io::Cursor;

        let mut decoder = vorbis_rs::VorbisDecoder::new(Cursor::new(data)).map_err(|_| {
            "Failed to decode OGG data. The file is likely corrupt or not Vorbis encoded."
                .to_string()
        })?;

        let sample_rate = decoder.sampling_frequency().get();
        let channel_count = decoder.channels().get();

        let mut data = Vec::with_capacity(channel_count as usize * sample_rate as usize * 4);

        // Force bit depth to 8-bit, TODO: support other bit depths
        let bit_depth = BitDepths::Eight;

        while let Some(block) = decoder
            .decode_audio_block()
            .map_err(|_| "Failed to decode OGG data. The stream is likely corrupt.".to_string())?
        {
            let samples = block.samples();
            for &x in samples {
                for y in x {
                    let sample = (y.clamp(-1.0, 1.0) * 127.0).round() as i8;
                    data.push(sample.cast_unsigned());
                }
            }
        }

        let sample_count = (data.len() / (channel_count as usize)) as u32;
        let channel_count = channel_count as u16;

        let audio_resource = Audio {
            bit_depth,
            channel_count,
            sample_rate,
            sample_count,
        };

        Ok((audio_resource, data))
    }
}

pub struct AudioAssetHandler {}

impl AudioAssetHandler {
    pub fn new() -> AudioAssetHandler {
        AudioAssetHandler {}
    }
}

impl AssetHandler for AudioAssetHandler {
    fn can_handle(&self, r#type: &str) -> bool {
        r#type == "wav" || r#type == "ogg"
    }

    fn load<'a>(
        &'a self,
        _: &'a AssetManager,
        storage_backend: &'a dyn resource::StorageBackend,
        asset_storage_backend: &'a dyn asset::StorageBackend,
        url: ResourceId<'a>,
    ) -> BoxedFuture<'a, Result<Box<dyn Asset>, LoadErrors>> {
        Box::pin(async move {
            if let Some(dt) = storage_backend.get_type(url) {
                if dt != "wav" && dt != "ogg" {
                    return Err(LoadErrors::UnsupportedType);
                }
            }

            let (data, _, dt) = asset_storage_backend
                .resolve(url)
                .await
                .or(Err(LoadErrors::AssetCouldNotBeLoaded))?;

            if dt != "wav" && dt != "ogg" {
                return Err(LoadErrors::UnsupportedType);
            }

            Ok(Box::new(AudioAsset {
                id: url.to_string(),
                data: Arc::from(data),
            }) as Box<dyn Asset>)
        })
    }
}

#[cfg(test)]
mod tests {
    use crate::{
        asset::{
            self, asset_manager::AssetManager, audio_asset_handler::AudioAssetHandler, ResourceId,
        },
        r#async, resource,
        resources::audio::Audio,
        types::BitDepths,
        AssetHandler,
    };

    #[r#async::test]
    async fn test_audio_asset_handler() {
        let audio_asset_handler = AudioAssetHandler::new();

        let asset_storage_backend = asset::storage_backend::tests::TestStorageBackend::new();
        let resource_storage_backend = resource::storage_backend::tests::TestStorageBackend::new();
        let asset_manager = AssetManager::new(asset_storage_backend.clone());

        let url = ResourceId::new("gun.wav");

        let asset = audio_asset_handler
            .load(
                &asset_manager,
                &resource_storage_backend,
                &asset_storage_backend,
                url,
            )
            .await
            .expect("Audio asset handler failed to load asset");

        let _ = asset
            .load(
                &asset_manager,
                &resource_storage_backend,
                &asset_storage_backend,
                url,
            )
            .await
            .expect("Audio asset failed to load");

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
