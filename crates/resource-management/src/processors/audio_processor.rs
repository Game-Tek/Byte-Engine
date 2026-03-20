use crate::{
	asset::{asset_handler::LoadErrors, ResourceId},
	resources::audio::Audio,
	ProcessedAsset,
};

pub fn process_audio<'a>(
	id: ResourceId<'a>,
	description: Audio,
	buffer: impl Into<Box<[u8]>>,
) -> Result<(ProcessedAsset, Box<[u8]>), LoadErrors> {
	Ok((ProcessedAsset::new(id, description), buffer.into()))
}

#[cfg(test)]
mod tests {
	use crate::{asset::ResourceId, resources::audio::Audio, types::BitDepths};

	use super::process_audio;

	#[test]
	fn process_audio_serializes_audio_metadata_and_preserves_pcm_data() {
		let description = Audio {
			bit_depth: BitDepths::Sixteen,
			channel_count: 2,
			sample_rate: 48_000,
			sample_count: 128,
		};

		let (asset, data) = process_audio(ResourceId::new("audio/test.wav"), description, vec![1_u8, 2, 3, 4])
			.expect("Audio processing should succeed");

		let audio: Audio = crate::from_slice(&asset.resource).expect("Processed asset should deserialize as audio");

		assert_eq!(asset.id, "audio/test.wav");
		assert_eq!(asset.class, "Audio");
		assert_eq!(audio.bit_depth, BitDepths::Sixteen);
		assert_eq!(audio.channel_count, 2);
		assert_eq!(audio.sample_rate, 48_000);
		assert_eq!(audio.sample_count, 128);
		assert_eq!(&*data, &[1, 2, 3, 4]);
	}
}
