//! Audio graph playback-time values.

use std::f32::consts::TAU;

/// The `AudioGraphTime` struct identifies when a processed block starts in one
/// graph playback.
///
/// Use [`Self::seconds_at`] for non-periodic timing inside [`crate::audio::graph::fns::custom`]. For
/// periodic waveforms, use [`Self::periodic_phase_step`] and
/// [`Self::advance_periodic_phase`] to retain a bounded phase.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AudioGraphTime {
	sample_index: u64,
	sample_rate: u32,
}

impl AudioGraphTime {
	/// Creates a block time at the audio-system boundary.
	pub(crate) const fn new(sample_index: u64, sample_rate: u32) -> Self {
		Self {
			sample_index,
			sample_rate,
		}
	}

	/// Returns the output sample index at the start of the block.
	pub const fn sample_index(self) -> u64 {
		self.sample_index
	}

	/// Returns the output sample rate in samples per second.
	pub const fn sample_rate(self) -> u32 {
		self.sample_rate
	}

	/// Returns the time at the start of the block in seconds.
	pub fn seconds(self) -> f64 {
		debug_assert!(
			self.sample_rate > 0,
			"Audio sample rate is zero. The most likely cause is constructing graph time outside the audio system."
		);
		self.sample_index as f64 / f64::from(self.sample_rate)
	}

	/// Returns the time of one sample in the block in seconds.
	pub fn seconds_at(self, sample_offset: usize) -> f64 {
		debug_assert!(
			self.sample_rate > 0,
			"Audio sample rate is zero. The most likely cause is constructing graph time outside the audio system."
		);
		(self.sample_index as f64 + sample_offset as f64) / f64::from(self.sample_rate)
	}

	/// Returns one bounded phase step for a periodic waveform.
	///
	/// Calculate this once per custom-processor block, then pass it to
	/// [`Self::advance_periodic_phase`] for every sample. This avoids deriving a
	/// periodic phase from an ever-growing absolute time value.
	pub fn periodic_phase_step(self, frequency_hz: f32) -> f32 {
		debug_assert!(
			self.sample_rate > 0,
			"Audio sample rate is zero. The most likely cause is constructing graph time outside the audio system."
		);
		debug_assert!(
			frequency_hz.is_finite(),
			"Periodic waveform frequency is not finite. The most likely cause is a NaN or infinite custom-processor parameter."
		);
		let sample_rate = self.sample_rate as f32;
		TAU * frequency_hz.rem_euclid(sample_rate) / sample_rate
	}

	/// Advances a periodic phase while retaining it in `0.0..TAU`.
	///
	/// Pass a phase step returned by [`Self::periodic_phase_step`]. The function
	/// uses only one addition and one conditional correction per sample.
	pub fn advance_periodic_phase(phase: f32, phase_step: f32) -> f32 {
		debug_assert!(
			phase.is_finite() && (0.0..TAU).contains(&phase),
			"Periodic waveform phase is outside 0.0..TAU. The most likely cause is bypassing AudioGraphTime::advance_periodic_phase."
		);
		debug_assert!(
			phase_step.is_finite() && (0.0..TAU).contains(&phase_step),
			"Periodic waveform phase step is outside 0.0..TAU. The most likely cause is bypassing AudioGraphTime::periodic_phase_step."
		);
		let phase = phase + phase_step;
		if phase >= TAU { phase - TAU } else { phase }
	}
}

#[cfg(test)]
mod tests {
	use super::AudioGraphTime;

	#[test]
	fn periodic_phase_remains_bounded_after_many_samples() {
		let time = AudioGraphTime::new(0, 48_000);
		let phase_step = time.periodic_phase_step(20_000.0);
		let mut phase = 0.0;
		for _ in 0..1_000_000 {
			phase = AudioGraphTime::advance_periodic_phase(phase, phase_step);
		}

		assert!((0.0..std::f32::consts::TAU).contains(&phase));
	}
}
