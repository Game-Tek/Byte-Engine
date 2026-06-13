use crate::types::BitDepths;

/// Converts a PCM bits-per-sample value into a supported audio bit depth.
pub(super) fn bit_depth_from_bits_per_sample(bits_per_sample: u16) -> Option<BitDepths> {
	match bits_per_sample {
		8 => Some(BitDepths::Eight),
		16 => Some(BitDepths::Sixteen),
		24 => Some(BitDepths::TwentyFour),
		32 => Some(BitDepths::ThirtyTwo),
		_ => None,
	}
}

/// Returns the number of bytes used by one sample at the given bit depth.
pub(super) fn bytes_per_sample(bit_depth: BitDepths) -> usize {
	usize::from(bit_depth) / 8
}

/// Counts audio frames from interleaved PCM byte length and channel layout.
pub(super) fn sample_count_from_pcm_len(byte_len: usize, channel_count: u16, bit_depth: BitDepths) -> u32 {
	(byte_len / channel_count as usize / bytes_per_sample(bit_depth)) as u32
}

/// Appends one normalized float sample as PCM at the requested bit depth.
pub(super) fn push_pcm_sample<A: std::alloc::Allocator>(data: &mut Vec<u8, A>, sample: f32, bit_depth: BitDepths) {
	let sample = sample.clamp(-1.0, 1.0);

	match bit_depth {
		BitDepths::Eight => {
			let sample = ((sample * 0.5 + 0.5) * u8::MAX as f32).round() as u8;
			data.push(sample);
		}
		BitDepths::Sixteen => {
			let sample = (sample * i16::MAX as f32).round() as i16;
			data.extend_from_slice(&sample.to_le_bytes());
		}
		BitDepths::TwentyFour => {
			let sample = (sample * 8_388_607.0).round() as i32;
			let bytes = sample.to_le_bytes();
			data.extend_from_slice(&bytes[..3]);
		}
		BitDepths::ThirtyTwo => {
			let sample = (sample * i32::MAX as f32).round() as i32;
			data.extend_from_slice(&sample.to_le_bytes());
		}
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn bit_depth_from_bits_per_sample_maps_supported_pcm_depths() {
		assert_eq!(bit_depth_from_bits_per_sample(8), Some(BitDepths::Eight));
		assert_eq!(bit_depth_from_bits_per_sample(16), Some(BitDepths::Sixteen));
		assert_eq!(bit_depth_from_bits_per_sample(24), Some(BitDepths::TwentyFour));
		assert_eq!(bit_depth_from_bits_per_sample(32), Some(BitDepths::ThirtyTwo));
		assert_eq!(bit_depth_from_bits_per_sample(12), None);
	}

	#[test]
	fn bytes_per_sample_reports_pcm_byte_width() {
		assert_eq!(bytes_per_sample(BitDepths::Eight), 1);
		assert_eq!(bytes_per_sample(BitDepths::Sixteen), 2);
		assert_eq!(bytes_per_sample(BitDepths::TwentyFour), 3);
		assert_eq!(bytes_per_sample(BitDepths::ThirtyTwo), 4);
	}

	#[test]
	fn sample_count_from_pcm_len_counts_interleaved_frames() {
		assert_eq!(sample_count_from_pcm_len(16, 2, BitDepths::Sixteen), 4);
		assert_eq!(sample_count_from_pcm_len(18, 2, BitDepths::TwentyFour), 3);
	}

	#[test]
	fn push_pcm_sample_writes_supported_bit_depths() {
		let mut data = Vec::new();
		push_pcm_sample(&mut data, -1.0, BitDepths::Eight);
		push_pcm_sample(&mut data, 1.0, BitDepths::Eight);
		assert_eq!(data, [0, 255]);

		let mut data = Vec::new();
		push_pcm_sample(&mut data, 1.0, BitDepths::Sixteen);
		assert_eq!(data, i16::MAX.to_le_bytes());

		let mut data = Vec::new();
		push_pcm_sample(&mut data, 1.0, BitDepths::TwentyFour);
		assert_eq!(data, [0xff, 0xff, 0x7f]);

		let mut data = Vec::new();
		push_pcm_sample(&mut data, 1.0, BitDepths::ThirtyTwo);
		assert_eq!(data, i32::MAX.to_le_bytes());
	}
}
