use std::borrow::Cow;

use crate::{asset::handler::LoadErrors, resources::audio::Audio, types::BitDepths};

/// The `AudioDescription` struct selects the stored PCM layout for one decoded audio source.
///
/// Pass the description to [`super::process_audio`] before lending or decoding samples.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct AudioDescription {
	/// Number of bits stored for each channel sample.
	pub bit_depth: BitDepths,
	/// Number of interleaved channels in each stored frame.
	pub channel_count: u16,
	/// Number of stored frames played per second.
	pub sample_rate: u32,
}

/// The `AudioSink` struct provides a common destination for borrowed PCM and decoded sample blocks.
///
/// Call [`Self::set_interleaved_pcm`] once for already-canonical PCM, or call
/// [`Self::append_planar_f32`] for each block produced by a compressed decoder.
pub struct AudioSink<'source> {
	description: AudioDescription,
	data: Cow<'source, [u8]>,
	frame_count: u64,
	mode: AudioSinkMode,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum AudioSinkMode {
	Empty,
	Borrowed,
	Generated,
}

impl<'source> AudioSink<'source> {
	/// Creates a sink after validating the stored channel and sample-rate contract.
	pub(crate) fn new(description: AudioDescription) -> Result<Self, LoadErrors> {
		if !matches!(description.channel_count, 1 | 2) || description.sample_rate == 0 {
			return Err(LoadErrors::FailedToProcess);
		}

		Ok(Self {
			description,
			data: Cow::Borrowed(&[]),
			frame_count: 0,
			mode: AudioSinkMode::Empty,
		})
	}

	/// Lends a complete little-endian, interleaved integer PCM payload to the processor.
	pub fn set_interleaved_pcm(&mut self, data: &'source [u8]) -> Result<(), LoadErrors> {
		if self.mode != AudioSinkMode::Empty {
			return Err(LoadErrors::FailedToProcess);
		}

		let frame_width = frame_width(self.description)?;
		if data.is_empty() || !data.len().is_multiple_of(frame_width) {
			return Err(LoadErrors::FailedToProcess);
		}

		self.frame_count = u64::try_from(data.len() / frame_width).map_err(|_| LoadErrors::FailedToProcess)?;
		self.data = Cow::Borrowed(data);
		self.mode = AudioSinkMode::Borrowed;
		Ok(())
	}

	/// Appends one borrowed planar float block in canonical interleaved frame order.
	pub fn append_planar_f32(&mut self, channels: &[&[f32]]) -> Result<(), LoadErrors> {
		if self.mode == AudioSinkMode::Borrowed || channels.len() != usize::from(self.description.channel_count) {
			return Err(LoadErrors::FailedToProcess);
		}

		let Some(frame_count) = channels.first().map(|channel| channel.len()) else {
			return Err(LoadErrors::FailedToProcess);
		};
		if channels.iter().any(|channel| channel.len() != frame_count) {
			return Err(LoadErrors::FailedToProcess);
		}
		if frame_count == 0 {
			return Ok(());
		}

		let frame_width = frame_width(self.description)?;
		let additional_bytes = frame_count.checked_mul(frame_width).ok_or(LoadErrors::FailedToProcess)?;
		let data = self.data.to_mut();
		data.try_reserve(additional_bytes).map_err(|_| LoadErrors::FailedToProcess)?;

		for frame in 0..frame_count {
			for channel in channels {
				push_pcm_sample(data, channel[frame], self.description.bit_depth);
			}
		}

		self.frame_count = self
			.frame_count
			.checked_add(u64::try_from(frame_count).map_err(|_| LoadErrors::FailedToProcess)?)
			.ok_or(LoadErrors::FailedToProcess)?;
		self.mode = AudioSinkMode::Generated;
		Ok(())
	}

	/// Finishes the payload and derives metadata from the samples actually written.
	pub(crate) fn finish(self) -> Result<(Audio, Cow<'source, [u8]>), LoadErrors> {
		if self.mode == AudioSinkMode::Empty || self.frame_count == 0 {
			return Err(LoadErrors::FailedToProcess);
		}

		let sample_count = u32::try_from(self.frame_count).map_err(|_| LoadErrors::FailedToProcess)?;
		Ok((
			Audio {
				bit_depth: self.description.bit_depth,
				channel_count: self.description.channel_count,
				sample_rate: self.description.sample_rate,
				sample_count,
			},
			self.data,
		))
	}
}

/// Calculates the complete interleaved byte width of one audio frame.
fn frame_width(description: AudioDescription) -> Result<usize, LoadErrors> {
	usize::from(description.channel_count)
		.checked_mul(usize::from(description.bit_depth) / 8)
		.filter(|width| *width != 0)
		.ok_or(LoadErrors::FailedToProcess)
}

/// Appends one normalized float sample using the runtime PCM representation.
fn push_pcm_sample(data: &mut Vec<u8>, sample: f32, bit_depth: BitDepths) {
	let sample = sample.clamp(-1.0, 1.0);

	match bit_depth {
		BitDepths::Eight => data.push(((sample * 0.5 + 0.5) * u8::MAX as f32).round() as u8),
		BitDepths::Sixteen => data.extend_from_slice(&((sample * i16::MAX as f32).round() as i16).to_le_bytes()),
		BitDepths::TwentyFour => {
			let bytes = ((sample * 8_388_607.0).round() as i32).to_le_bytes();
			data.extend_from_slice(&bytes[..3]);
		}
		BitDepths::ThirtyTwo => data.extend_from_slice(&((sample * i32::MAX as f32).round() as i32).to_le_bytes()),
	}
}
