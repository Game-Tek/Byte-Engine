use crate::{
	asset,
	r#async::{spawn_cpu_task, BoxedFuture},
	resource,
	resources::audio::Audio,
	types::BitDepths,
	ProcessedAsset,
};

use super::{
	asset_handler::{AssetHandler, LoadErrors},
	asset_manager::AssetManager,
	ResourceId,
};

impl OGGAssetHandler {
	/// Decodes an OGG Vorbis buffer into audio metadata and PCM data.
	fn decode_ogg(data: &[u8]) -> Result<(Audio, Vec<u8>), String> {
		use std::io::Cursor;

		let mut decoder = vorbis_rs::VorbisDecoder::new(Cursor::new(data))
			.map_err(|_| "Failed to decode OGG data. The file is likely corrupt or not Vorbis encoded.".to_string())?;

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

	pub fn new() -> OGGAssetHandler {
		OGGAssetHandler {}
	}
}

pub struct OGGAssetHandler {}

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

			let (audio_resource, data) = spawn_cpu_task(move || Self::decode_ogg(&data))
				.await
				.map_err(|_| LoadErrors::FailedToProcess)?
				.map_err(|_| LoadErrors::FailedToProcess)?;

			Ok((ProcessedAsset::new(url, audio_resource), data.into_boxed_slice()))
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
	#[ignore = "Test uses data not pushed to the repository"]
	async fn test_audio_asset_handler() {
		let audio_asset_handler = OGGAssetHandler::new();

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
