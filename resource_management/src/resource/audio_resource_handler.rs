use serde::Deserialize;
use smol::{fs::File, io::AsyncReadExt};

use crate::{types::Audio, Resource, Stream};

use super::resource_handler::ResourceHandler;

pub struct AudioResourceHandler {
}

impl AudioResourceHandler {
	pub fn new() -> Self {
		Self {
		}
	}
}

impl ResourceHandler for AudioResourceHandler {
	fn can_handle_type(&self, resource_type: &str) -> bool {
		return resource_type == "Audio" || resource_type == "wav";
	}

	fn get_deserializers(&self) -> Vec<(&'static str, Box<dyn Fn(&polodb_core::bson::Document) -> Box<dyn Resource> + Send>)> {
		vec![("Audio", Box::new(|document| {
			let audio = Audio::deserialize(polodb_core::bson::Deserializer::new(document.into())).unwrap();
			Box::new(audio)
		}))]
	}

	fn read<'a>(&'a self, _resource: &'a dyn Resource, file: &'a mut File, buffers: &'a mut [Stream<'a>]) -> utils::BoxedFuture<'a, ()> {
		Box::pin(async move {
			file.read_exact(buffers[0].buffer).await.unwrap();
		})
	}
}

#[cfg(test)]
mod tests {
	use crate::{resource::resource_manager::ResourceManager, types::{Audio, BitDepths}};

	use super::*;

	#[test]
	fn test_audio_resource_handler() {
		let audio_resource_handler = AudioResourceHandler::new();

		let mut resource_manager = ResourceManager::new();

		resource_manager.add_resource_handler(audio_resource_handler);

		let (response, buffer) = smol::block_on(resource_manager.get("gun")).unwrap();

		assert_eq!(response.resources.len(), 1);

		// let (generic_resource_serialization, data) = match processed_resources[0] {
		// 	ProcessedResources::Generated((ref generic_resource_serialization, ref data)) => (generic_resource_serialization, data),
		// 	_ => { panic!("Unexpected processed resource type"); }
		// };

		let resource = &response.resources[0];

		assert_eq!(resource.url, "gun");
		assert_eq!(resource.class, "Audio");

		// let audio_resource = (audio_resource_handler.get_deserializers().iter().find(|(class, _)| *class == "Audio").unwrap().1)(&resource.resource);

		let audio = resource.resource.downcast_ref::<Audio>().unwrap();

		assert_eq!(audio.bit_depth, BitDepths::Sixteen);
		assert_eq!(audio.channel_count, 1);
		assert_eq!(audio.sample_rate, 48000);
		assert_eq!(audio.sample_count, 152456 / 1 / (16 / 8));

		assert_eq!(buffer.len(), audio.sample_count as usize * audio.channel_count as usize * (Into::<usize>::into(audio.bit_depth) / 8) as usize);
	}
}