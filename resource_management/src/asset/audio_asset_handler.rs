use smol::future::FutureExt;

use crate::types::{Audio, BitDepths};

use super::{asset_handler::AssetHandler, AssetResolver};

struct AudioAssetHandler {

}

impl AudioAssetHandler {
	fn new() -> AudioAssetHandler {
		AudioAssetHandler {}
	}
}

impl AssetHandler for AudioAssetHandler {
	fn load<'a>(&'a self, asset_resolver: &'a dyn AssetResolver, url: &'a str, json: &'a json::JsonValue) -> utils::BoxedFuture<'a, Option<Result<(), String>>> {
		async move {
			if let Some(dt) = asset_resolver.get_type(url) {
				if dt != "wav" { return None; }
			}

			let (data, dt) = asset_resolver.resolve(url).await?;

			if dt != "wav" { return None; }

			let riff = &data[0..4];

			if riff != b"RIFF" {
				return Some(Err("Invalid RIFF header".to_string()));
			}

			let format = &data[8..12];

			if format != b"WAVE" {
				return Some(Err("Invalid WAVE format".to_string()));
			}

			let audio_format = &data[20..22];

			if audio_format != b"\x01\x00" {
				return Some(Err("Invalid audio format".to_string()));
			}

			let subchunk_1_size = &data[16..20];

			let subchunk_1_size = u32::from_le_bytes([subchunk_1_size[0], subchunk_1_size[1], subchunk_1_size[2], subchunk_1_size[3]]);

			if subchunk_1_size != 16 {
				return Some(Err("Invalid subchunk 1 size".to_string()));
			}

			let num_channels = &data[22..24];

			let num_channels = u16::from_le_bytes([num_channels[0], num_channels[1]]);

			if num_channels != 1 && num_channels != 2 {
				return Some(Err("Invalid number of channels".to_string()));
			}

			let sample_rate = &data[24..28];

			let sample_rate = u32::from_le_bytes([sample_rate[0], sample_rate[1], sample_rate[2], sample_rate[3]]);

			let bits_per_sample = &data[34..36];

			let bits_per_sample = u16::from_le_bytes([bits_per_sample[0], bits_per_sample[1]]);

			let bit_depth = match bits_per_sample {
				8 => BitDepths::Eight,
				16 => BitDepths::Sixteen,
				24 => BitDepths::TwentyFour,
				32 => BitDepths::ThirtyTwo,
				_ => { return Some(Err("Invalid bits per sample".to_string())); }
			};

			let data_header = &data[36..40];

			if data_header != b"data" {
				return Some(Err("Invalid data header".to_string()));
			}

			let data_size = &data[40..44];

			let data_size = u32::from_le_bytes([data_size[0], data_size[1], data_size[2], data_size[3]]);

			let sample_count = data_size / (bits_per_sample / 8) as u32 / num_channels as u32;

			let data = &data[44..][..data_size as usize];

			let audio_resource = Audio {
				bit_depth,
				channel_count: num_channels,
				sample_rate,
				sample_count,
			};

			// Ok(
			// 	vec![
			// 		ProcessedResources::Generated((
			// 			GenericResourceSerialization::new(asset_url.to_string(), audio_resource),
			// 			Vec::from(data),
			// 		))
			// 	]
			
			// )

			Some(Ok(()))
		}.boxed()
	}
}

#[cfg(test)]
mod tests {
	use crate::asset::tests::TestAssetResolver;

	use super::*;

	#[test]
	fn test_audio_asset_handler() {
		let audio_asset_handler = AudioAssetHandler::new();

		let url = "gun.wav";
		let doc = json::object! {
			"url": url,
		};

		let asset_resolver = TestAssetResolver::new();

		smol::block_on(audio_asset_handler.load(&asset_resolver, url, &doc)).expect("Audio asset handler did not handle asset");

		// assert_eq!(resource.url, "gun");
		// assert_eq!(resource.class, "Audio");

		// let audio = resource.resource.downcast_ref::<Audio>().unwrap();

		// assert_eq!(audio.bit_depth, BitDepths::Sixteen);
		// assert_eq!(audio.channel_count, 1);
		// assert_eq!(audio.sample_rate, 48000);
		// assert_eq!(audio.sample_count, 152456 / 1 / (16 / 8));

		// assert_eq!(buffer.len(), audio.sample_count as usize * audio.channel_count as usize * (Into::<usize>::into(audio.bit_depth) / 8) as usize);
	}
}