//! Prepared runtime audio graph plans and processors.

use super::*;

/// The `CompiledAudioGraph` struct carries validated sample and processing
/// settings from graph creation to resource loading.
#[derive(Debug, Clone)]
pub(crate) struct CompiledAudioGraph {
	pub(crate) resource_id: String,
	pub(crate) playback_mode: SamplePlaybackMode,
	pub(crate) playback_rate: PlaybackRate,
	pub(crate) processors: AudioProcessors,
	pub(crate) muted: bool,
	pub(crate) muted_drain_latency: usize,
}

impl CompiledAudioGraph {
	/// Separates the resource request from the render plan retained while that
	/// resource loads.
	pub(crate) fn into_parts(self) -> (String, AudioGraphRenderPlan) {
		(
			self.resource_id,
			AudioGraphRenderPlan {
				playback_mode: self.playback_mode,
				playback_rate: self.playback_rate,
				processors: self.processors,
				muted: self.muted,
				muted_drain_latency: self.muted_drain_latency,
			},
		)
	}
}

/// The `AudioGraphRenderPlan` struct preserves validated playback and
/// processing state while the graph's sample resource loads.
#[derive(Debug)]
pub(crate) struct AudioGraphRenderPlan {
	pub(crate) playback_mode: SamplePlaybackMode,
	pub(crate) playback_rate: PlaybackRate,
	pub(crate) processors: AudioProcessors,
	pub(crate) muted: bool,
	pub(crate) muted_drain_latency: usize,
}

/// Selects what happens when the graph's sample reaches its final frame.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SamplePlaybackMode {
	Once,
	Loop,
}

/// The `PlaybackRate` struct keeps one authored varispeed rate as an exact
/// rational value for drift-free source-phase accumulation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct PlaybackRate {
	pub(crate) numerator: u64,
	pub(crate) denominator: u64,
}

impl PlaybackRate {
	pub(crate) const UNITY: Self = Self {
		numerator: 1,
		denominator: 1,
	};

	/// Converts the exact binary value of a validated positive `f32` rate into
	/// its smallest power-of-two fraction.
	pub(super) fn from_rate(rate: f32) -> Self {
		debug_assert!(rate.is_finite() && rate > 0.0);
		let bits = rate.to_bits();
		let significand = u64::from((bits & 0x7f_ffff) | 0x80_0000);
		let binary_exponent = ((bits >> 23) & 0xff) as i32 - 127 - 23;
		let (mut numerator, mut denominator) = if binary_exponent >= 0 {
			(significand << binary_exponent, 1)
		} else {
			(significand, 1_u64 << -binary_exponent)
		};
		let common_power_of_two = numerator.trailing_zeros().min(denominator.trailing_zeros());
		numerator >>= common_power_of_two;
		denominator >>= common_power_of_two;
		Self { numerator, denominator }
	}
}

/// Describes one allocation-free processor in a compiled graph.
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum AudioProcessor {
	Gain(f32),
	PitchShift(f32),
	Custom(CustomAudioFunction),
}

impl AudioProcessor {
	/// Returns the output tail that must elapse after the source ends.
	pub(super) fn latency(&self) -> usize {
		match self {
			Self::Gain(_) => 0,
			Self::PitchShift(_) => pitch_shift::PITCH_SHIFT_LATENCY,
			Self::Custom(_) => 0,
		}
	}

	/// Prepares this processor before it crosses to the audio worker.
	pub(crate) fn prepare(self) -> SmallBox<dyn RuntimeAudioProcessor + Send, S4> {
		match self {
			Self::Gain(gain) => smallbox!(GainProcessor(gain)),
			Self::PitchShift(ratio) => smallbox!(pitch_shift::PitchShiftProcessor::new(ratio)),
			Self::Custom(function) => smallbox!(CustomFunctionProcessor(function.create())),
		}
	}
}

/// The `RuntimeAudioProcessor` trait provides allocation-free block processing
/// after a graph has been prepared.
pub(crate) trait RuntimeAudioProcessor {
	fn process(&mut self, time: AudioGraphTime, samples: &mut [f32]);
}

/// The `GainProcessor` struct keeps one multiplier inline in its runtime node
/// box for block processing.
struct GainProcessor(f32);

impl RuntimeAudioProcessor for GainProcessor {
	fn process(&mut self, _time: AudioGraphTime, samples: &mut [f32]) {
		for sample in samples {
			*sample *= self.0;
		}
	}
}

/// The `CustomFunctionProcessor` struct owns one playback's mutable custom
/// closure state.
struct CustomFunctionProcessor(RuntimeCustomFunction);

impl RuntimeAudioProcessor for CustomFunctionProcessor {
	fn process(&mut self, time: AudioGraphTime, samples: &mut [f32]) {
		(self.0)(time, samples);
	}
}

impl AudioGraphRenderPlan {
	/// Allocates stateful processors on the loader task before playback.
	pub(crate) fn prepare(mut self) -> PreparedAudioGraphRenderPlan {
		// Keep latency accounting off the audio worker. Muted plans already
		// carry the latency of processors removed during compilation.
		let drain_latency = self.muted_drain_latency + self.processors.iter().map(AudioProcessor::latency).sum::<usize>();
		// The mixer can apply a terminal gain while adding the graph block to
		// the destination, avoiding a separate traversal of that block.
		let output_gain = match self.processors.last() {
			Some(AudioProcessor::Gain(gain)) => {
				let gain = *gain;
				self.processors.pop();
				gain
			}
			Some(AudioProcessor::PitchShift(_) | AudioProcessor::Custom(_)) | None => 1.0,
		};
		PreparedAudioGraphRenderPlan {
			playback_mode: self.playback_mode,
			playback_rate: self.playback_rate,
			processors: self.processors.into_iter().map(AudioProcessor::prepare).collect(),
			output_gain,
			muted: self.muted,
			drain_latency,
		}
	}
}

/// The `PreparedAudioGraphRenderPlan` struct owns initialized processing state
/// ready to move to the audio worker.
pub(crate) struct PreparedAudioGraphRenderPlan {
	pub(crate) playback_mode: SamplePlaybackMode,
	pub(crate) playback_rate: PlaybackRate,
	pub(crate) processors: RuntimeAudioProcessors,
	pub(crate) output_gain: f32,
	pub(crate) muted: bool,
	pub(crate) drain_latency: usize,
}
