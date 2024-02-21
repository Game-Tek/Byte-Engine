use polodb_core::bson;
use serde::Deserialize;

use crate::{types::Audio, GenericResourceSerialization, ResourceResponse};

use super::resource_handler::{ReadTargets, ResourceHandler, ResourceReader};

pub struct AudioResourceHandler {
}

impl AudioResourceHandler {
	pub fn new() -> Self {
		Self {
		}
	}
}

impl ResourceHandler for AudioResourceHandler {
	fn get_handled_resource_classes<'a>(&self,) -> &'a [&'a str] {
		&["Audio"]
	}

	fn read<'a>(&'a self, resource: &'a GenericResourceSerialization, file: &'a mut dyn ResourceReader, buffers: &'a mut ReadTargets<'a>) -> utils::BoxedFuture<'a, Option<ResourceResponse>> {
		Box::pin(async move {
			let audio_resource = Audio::deserialize(bson::Deserializer::new(resource.resource.clone().into())).ok()?;

			match buffers {
				ReadTargets::Buffer(buffer) => {
					file.read_into(0, buffer).await?;
				},
				_ => {
					return None;
				}
			}

			Some(ResourceResponse::new(resource, audio_resource))
		})
	}
}

#[cfg(test)]
mod tests {
	use crate::{asset::{asset_handler::AssetHandler, audio_asset_handler::AudioAssetHandler, tests::{TestAssetResolver, TestStorageBackend}, StorageBackend}, resource::tests::TestResourceReader, types::{Audio, BitDepths}};

	use super::*;

	#[test]
	fn test_audio_resource_handler() {
		// Create resource from asset

		let audio_asset_handler = AudioAssetHandler::new();

		let url = "gun.wav";
		let doc = json::object! {
			"url": url,
		};

		let asset_resolver = TestAssetResolver::new();
		let storage_backend = TestStorageBackend::new();

		smol::block_on(audio_asset_handler.load(&asset_resolver, &storage_backend, url, &doc)).expect("Audio asset handler did not handle asset").expect("Audio asset handler failed to load asset");

		// Load resource from storage

		let audio_resource_handler = AudioResourceHandler::new();

		let (resource, data) = storage_backend.read(url).expect("Failed to read asset from storage");

		let mut resource_reader = TestResourceReader::new(data);

		let mut buffer = vec![0; 152456];

		unsafe {
			buffer.set_len(152456);
		}

		let resource = smol::block_on(audio_resource_handler.read(&resource, &mut resource_reader, &mut ReadTargets::Buffer(&mut buffer))).unwrap();

		assert_eq!(resource.url, "gun.wav");
		assert_eq!(resource.class, "Audio");

		let audio = resource.resource.downcast_ref::<Audio>().unwrap();

		assert_eq!(audio.bit_depth, BitDepths::Sixteen);
		assert_eq!(audio.channel_count, 1);
		assert_eq!(audio.sample_rate, 48000);
		assert_eq!(audio.sample_count, 152456 / 1 / (16 / 8));

		assert_eq!(buffer.len(), audio.sample_count as usize * audio.channel_count as usize * (Into::<usize>::into(audio.bit_depth) / 8) as usize);
	}
}