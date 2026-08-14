//! Audio graph playback-time values.

/// The `AudioGraphTime` struct identifies when a processed block starts in one
/// graph playback.
///
/// Use [`Self::seconds_at`] to calculate the time of each sample when generating
/// a waveform inside [`fns::custom`].
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
}
