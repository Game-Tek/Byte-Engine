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

		let data = &bytes[44..data_size as usize];

		let audio_resource = Audio {
			bit_depth,
			channel_count: num_channels,
			sample_rate,
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

#[derive(Debug, Serialize, Deserialize)]
enum BitDepths {
	Eight,
	Sixteen,
	TwentyFour,
	ThirtyTwo,
}

#[derive(Debug, Serialize, Deserialize)]
struct Audio {
	bit_depth: BitDepths,
	channel_count: u16,
	sample_rate: u32,
}

impl Resource for Audio {
	fn get_class(&self) -> &'static str { "Audio" }
}