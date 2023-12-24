use std::io::Read;

use serde::{Serialize, Deserialize};

use super::{resource_handler::ResourceHandler, Resource, ProcessedResources, GenericResourceSerialization};

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

	fn process(&self, resource_manager: &super::resource_manager::ResourceManager, asset_url: &str) -> Result<Vec<ProcessedResources>, String> {
		let (bytes, _) = resource_manager.read_asset_from_source(asset_url).unwrap();

		let riff = &bytes[0..4];

		if riff != b"RIFF" {
			return Err("Invalid RIFF header".to_string());
		}

		let format = &bytes[8..12];

		if format != b"WAVE" {
			return Err("Invalid WAVE format".to_string());
		}

		let audio_format = &bytes[20..22];

		if audio_format != b"\x01\x00" {
			return Err("Invalid audio format".to_string());
		}

		let subchunk_1_size = &bytes[16..20];

		let subchunk_1_size = u32::from_le_bytes([subchunk_1_size[0], subchunk_1_size[1], subchunk_1_size[2], subchunk_1_size[3]]);

		if subchunk_1_size != 16 {
			return Err("Invalid subchunk 1 size".to_string());
		}

		let num_channels = &bytes[22..24];

		let num_channels = u16::from_le_bytes([num_channels[0], num_channels[1]]);

		if num_channels != 1 && num_channels != 2 {
			return Err("Invalid number of channels".to_string());
		}

		let sample_rate = &bytes[24..28];

		let sample_rate = u32::from_le_bytes([sample_rate[0], sample_rate[1], sample_rate[2], sample_rate[3]]);

		let bits_per_sample = &bytes[34..36];

		let bits_per_sample = u16::from_le_bytes([bits_per_sample[0], bits_per_sample[1]]);

		let bit_depth = match bits_per_sample {
			8 => BitDepths::Eight,
			16 => BitDepths::Sixteen,
			24 => BitDepths::TwentyFour,
			32 => BitDepths::ThirtyTwo,
			_ => { return Err("Invalid bits per sample".to_string()); }
		};

		let data_header = &bytes[36..40];

		if data_header != b"data" {
			return Err("Invalid data header".to_string());
		}

		let data_size = &bytes[40..44];

		let data_size = u32::from_le_bytes([data_size[0], data_size[1], data_size[2], data_size[3]]);

		let sample_count = data_size / (bits_per_sample / 8) as u32 / num_channels as u32;

		let data = &bytes[44..][..data_size as usize];

		let audio_resource = Audio {
			bit_depth,
			channel_count: num_channels,
			sample_rate,
			sample_count,
		};

		Ok(
			vec![
				ProcessedResources::Generated((
					GenericResourceSerialization::new(asset_url.to_string(), audio_resource),
					Vec::from(data),
				))
			]
		
		)
	}

	fn read(&self, _resource: &Box<dyn Resource>, file: &mut std::fs::File, buffers: &mut [super::Stream]) {
		file.read_exact(buffers[0].buffer).unwrap();
	}
}

#[derive(Debug, Serialize, Deserialize, PartialEq, Clone, Copy)]
pub enum BitDepths {
	Eight,
	Sixteen,
	TwentyFour,
	ThirtyTwo,
}

impl From<BitDepths> for usize {
	fn from(bit_depth: BitDepths) -> Self {
		match bit_depth {
			BitDepths::Eight => 8,
			BitDepths::Sixteen => 16,
			BitDepths::TwentyFour => 24,
			BitDepths::ThirtyTwo => 32,
		}
	}
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct Audio {
	pub bit_depth: BitDepths,
	pub channel_count: u16,
	pub sample_rate: u32,
	pub sample_count: u32,
}

impl Resource for Audio {
	fn get_class(&self) -> &'static str { "Audio" }
}

#[cfg(test)]
mod tests {
	use super::*;

	use crate::resource_management::resource_manager::ResourceManager;

	#[test]
	fn test_audio_resource_handler() {
		let audio_resource_handler = AudioResourceHandler::new();

		let resource_manager = ResourceManager::new();

		let processed_resources = audio_resource_handler.process(&resource_manager, "gun").unwrap();

		assert_eq!(processed_resources.len(), 1);

		let (generic_resource_serialization, data) = match processed_resources[0] {
			ProcessedResources::Generated((ref generic_resource_serialization, ref data)) => (generic_resource_serialization, data),
			_ => { panic!("Unexpected processed resource type"); }
		};

		assert_eq!(generic_resource_serialization.url, "gun");
		assert_eq!(generic_resource_serialization.class, "Audio");

		let audio_resource = (audio_resource_handler.get_deserializers().iter().find(|(class, _)| *class == "Audio").unwrap().1)(&generic_resource_serialization.resource);

		let audio = audio_resource.downcast_ref::<Audio>().unwrap();

		assert_eq!(audio.bit_depth, BitDepths::Sixteen);
		assert_eq!(audio.channel_count, 1);
		assert_eq!(audio.sample_rate, 48000);
		assert_eq!(audio.sample_count, 152456 / 1 / (16 / 8));

		assert_eq!(data.len(), audio.sample_count as usize * audio.channel_count as usize * (Into::<usize>::into(audio.bit_depth) / 8) as usize);
	}
}