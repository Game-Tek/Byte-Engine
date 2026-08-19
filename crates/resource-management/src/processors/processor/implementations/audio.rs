mod source;

pub use source::{AudioDescription, AudioSink};

/// Processes audio supplied by one statically dispatched format producer.
///
/// The producer may lend canonical interleaved PCM to [`AudioSink`] or append
/// decoded planar blocks. After processing, pass the returned bytes to
/// [`crate::asset::handler::BakeContext::store_primary`].
pub fn process_audio<'source>(
	id: ResourceId<'_>,
	description: AudioDescription,
	produce: impl FnOnce(&mut AudioSink<'source>) -> Result<(), LoadErrors>,
) -> Result<(ProcessedAsset, Cow<'source, [u8]>), LoadErrors> {
	let mut sink = AudioSink::new(description)?;
	produce(&mut sink)?;

	let (description, data) = sink.finish()?;
	Ok((ProcessedAsset::new(id, description), data))
}

#[cfg(test)]
mod tests {
	use std::borrow::Cow;

	use super::{process_audio, AudioDescription};
	use crate::{
		asset::{handler::LoadErrors, ResourceId},
		resources::audio::Audio,
		types::BitDepths,
	};

	fn description(bit_depth: BitDepths, channel_count: u16) -> AudioDescription {
		AudioDescription {
			bit_depth,
			channel_count,
			sample_rate: 48_000,
		}
	}

	#[test]
	fn process_audio_borrows_canonical_interleaved_pcm() {
		let pcm = [1_u8, 2, 3, 4];
		let (asset, data) = process_audio(
			ResourceId::new("audio/test.wav"),
			description(BitDepths::Sixteen, 1),
			|sink| sink.set_interleaved_pcm(&pcm),
		)
		.expect("Audio processing should succeed");

		let audio: Audio = crate::from_slice(&asset.resource).expect("Processed asset should deserialize as audio");

		assert!(matches!(data, Cow::Borrowed(_)));
		assert_eq!(data.as_ptr(), pcm.as_ptr());
		assert_eq!(audio.bit_depth, BitDepths::Sixteen);
		assert_eq!(audio.channel_count, 1);
		assert_eq!(audio.sample_rate, 48_000);
		assert_eq!(audio.sample_count, 2);
	}

	#[test]
	fn process_audio_interleaves_consecutive_planar_blocks() {
		let left_a = [-1.0, 0.0];
		let right_a = [0.5, 0.0];
		let left_b = [1.0];
		let right_b = [-0.5];
		let (asset, data) = process_audio(
			ResourceId::new("audio/test.ogg"),
			description(BitDepths::Sixteen, 2),
			|sink| {
				sink.append_planar_f32(&[&left_a, &right_a])?;
				sink.append_planar_f32(&[&left_b, &right_b])
			},
		)
		.expect("Planar audio should process");

		let samples: Vec<i16> = data
			.chunks_exact(2)
			.map(|sample| i16::from_le_bytes([sample[0], sample[1]]))
			.collect();
		let audio: Audio = crate::from_slice(&asset.resource).unwrap();

		assert!(matches!(data, Cow::Owned(_)));
		assert_eq!(samples, [-32_767, 16_384, 0, 0, 32_767, -16_384]);
		assert_eq!(audio.sample_count, 3);
	}

	#[test]
	fn process_audio_encodes_every_supported_bit_depth() {
		for (bit_depth, expected) in [
			(BitDepths::Eight, vec![0, 255]),
			(
				BitDepths::Sixteen,
				[-32_767_i16, 32_767_i16].into_iter().flat_map(i16::to_le_bytes).collect(),
			),
			(BitDepths::TwentyFour, vec![1, 0, 128, 255, 255, 127]),
			(
				BitDepths::ThirtyTwo,
				[i32::MIN, i32::MAX].into_iter().flat_map(i32::to_le_bytes).collect(),
			),
		] {
			let samples = [-1.0, 1.0];
			let (_, data) = process_audio(ResourceId::new("audio/test.ogg"), description(bit_depth, 1), |sink| {
				sink.append_planar_f32(&[&samples])
			})
			.unwrap();

			assert_eq!(data.as_ref(), expected);
		}
	}

	#[test]
	fn process_audio_rejects_invalid_layouts_and_payloads() {
		for description in [
			AudioDescription {
				bit_depth: BitDepths::Sixteen,
				channel_count: 0,
				sample_rate: 48_000,
			},
			AudioDescription {
				bit_depth: BitDepths::Sixteen,
				channel_count: 3,
				sample_rate: 48_000,
			},
			AudioDescription {
				bit_depth: BitDepths::Sixteen,
				channel_count: 1,
				sample_rate: 0,
			},
		] {
			assert_eq!(
				process_audio(ResourceId::new("audio/invalid"), description, |_| Ok(())).unwrap_err(),
				LoadErrors::FailedToProcess
			);
		}

		assert_eq!(
			process_audio(
				ResourceId::new("audio/partial.wav"),
				description(BitDepths::Sixteen, 2),
				|sink| sink.set_interleaved_pcm(&[0, 1, 2]),
			)
			.unwrap_err(),
			LoadErrors::FailedToProcess
		);

		let left = [0.0, 1.0];
		let right = [0.0];
		assert_eq!(
			process_audio(
				ResourceId::new("audio/mismatched.ogg"),
				description(BitDepths::Sixteen, 2),
				|sink| sink.append_planar_f32(&[&left, &right]),
			)
			.unwrap_err(),
			LoadErrors::FailedToProcess
		);
	}
}

use std::borrow::Cow;

use crate::{
	asset::{handler::LoadErrors, ResourceId},
	ProcessedAsset,
};
