use polodb_core::bson;
use serde::Deserialize;

use crate::{types::Audio, GenericResourceResponse, ResourceResponse, StorageBackend};

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

	fn read<'s, 'a, 'b>(&'s self, mut resource: GenericResourceResponse<'a>, reader: Option<Box<dyn ResourceReader>>, _: &'b dyn StorageBackend) -> utils::BoxedFuture<'b, Option<ResourceResponse<'a>>> where 'a: 'b {
		Box::pin(async move {
			let audio_resource = Audio::deserialize(bson::Deserializer::new(resource.resource.clone().into())).ok()?;

			if let Some(mut reader) = reader {
				if let Some(read_target) = &mut resource.read_target {
					match read_target {
						ReadTargets::Buffer(buffer) => {
							reader.read_into(0, buffer).await?;
						},
						ReadTargets::Box(buffer) => {
							reader.read_into(0, buffer).await?;
						},
						_ => {
							return None;
						}
						
					}
				} else {
					let mut buffer = Vec::with_capacity(resource.size);
					unsafe {
						buffer.set_len(resource.size);
					}
					reader.read_into(0, &mut buffer).await?;
					resource.set_box_buffer(buffer.into_boxed_slice());
				}
			}

			Some(ResourceResponse::new(resource, audio_resource))
		})
	}
}

#[cfg(test)]
mod tests {
	use crate::{asset::{asset_handler::AssetHandler, audio_asset_handler::AudioAssetHandler, tests::{TestAssetResolver, TestStorageBackend},}, types::BitDepths};

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

		let (resource, reader) = smol::block_on(storage_backend.read(url)).expect("Failed to read asset from storage");

		let resource = smol::block_on(audio_resource_handler.read(resource, Some(reader), &storage_backend)).unwrap();

		assert_eq!(resource.id(), "gun.wav");
		assert_eq!(resource.class, "Audio");

		let audio = resource.resource.downcast_ref::<Audio>().unwrap();

		assert_eq!(audio.bit_depth, BitDepths::Sixteen);
		assert_eq!(audio.channel_count, 1);
		assert_eq!(audio.sample_rate, 48000);
		assert_eq!(audio.sample_count, 152456 / 1 / (16 / 8));

		match &resource.read_target.expect("Expected read target") {
			ReadTargets::Box(buffer) => {
				assert_eq!(buffer.len(), audio.sample_count as usize * audio.channel_count as usize * (Into::<usize>::into(audio.bit_depth) / 8) as usize);
			},
			_ => {
				panic!("Expected read target to be a buffer");
			},
		}
	}
}