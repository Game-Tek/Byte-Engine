use std::{
	f32::consts::TAU,
	sync::{Arc, OnceLock},
};

use realfft::{ComplexToReal, RealFftPlanner, RealToComplex};
use rustfft::num_complex::Complex32;

use super::{AudioGraphTime, RuntimeAudioProcessor};

const WINDOW_SIZE: usize = 1024;
pub(super) const PITCH_SHIFT_LATENCY: usize = WINDOW_SIZE;
const HOP_SIZE: usize = WINDOW_SIZE / 8;
const BIN_COUNT: usize = WINDOW_SIZE / 2 + 1;
const WINDOW_MASK: usize = WINDOW_SIZE - 1;
const _: () = assert!(WINDOW_SIZE.is_power_of_two());
static SHARED: OnceLock<PitchShiftShared> = OnceLock::new();

/// The `PitchShiftShared` struct avoids rebuilding immutable FFT plans for
/// every pitch-shift playback.
struct PitchShiftShared {
	forward: Arc<dyn RealToComplex<f32>>,
	inverse: Arc<dyn ComplexToReal<f32>>,
	scratch_len: usize,
}

impl PitchShiftShared {
	/// Builds the reusable real forward and inverse transforms once.
	fn new() -> Self {
		let mut planner = RealFftPlanner::new();
		let forward = planner.plan_fft_forward(WINDOW_SIZE);
		let inverse = planner.plan_fft_inverse(WINDOW_SIZE);
		let scratch_len = forward.get_scratch_len().max(inverse.get_scratch_len());
		Self {
			forward,
			inverse,
			scratch_len,
		}
	}
}

/// The `PitchBinMapping` struct retains frequency-independent work for one
/// source bin throughout a pitch-shift playback.
struct PitchBinMapping {
	expected_advance: f32,
	target_bin: usize,
	fraction: f32,
	magnitude_scale: f32,
}

/// The `PitchShiftProcessor` struct owns reusable phase-vocoder state for one
/// real-time pitch-shift node.
pub(crate) struct PitchShiftProcessor {
	ratio: f32,
	forward: Arc<dyn RealToComplex<f32>>,
	inverse: Arc<dyn ComplexToReal<f32>>,
	input: Box<[f32]>,
	output: Box<[f32]>,
	normalization: Box<[f32]>,
	window: Box<[f32]>,
	transform_buffer: Box<[f32]>,
	spectrum: Box<[Complex32]>,
	shifted_spectrum: Box<[Complex32]>,
	scratch: Box<[Complex32]>,
	bin_mappings: Box<[PitchBinMapping]>,
	previous_phase: Box<[f32]>,
	output_phase: Box<[f32]>,
	cursor: usize,
	samples_until_transform: usize,
}

impl PitchShiftProcessor {
	/// Precomputes ratio-specific bin mappings and allocates all mutable DSP
	/// state before this processor reaches the audio worker.
	pub(super) fn new(ratio: f32) -> Self {
		let shared = SHARED.get_or_init(PitchShiftShared::new);
		let window = (0..WINDOW_SIZE)
			.map(|index| 0.5 - 0.5 * (TAU * index as f32 / WINDOW_SIZE as f32).cos())
			.collect::<Vec<_>>()
			.into_boxed_slice();
		// The ratio never changes during playback. Retain only source bins that
		// can contribute below Nyquist and precompute their fixed mapping work.
		let mut bin_mappings = Vec::with_capacity(BIN_COUNT);
		for bin in 0..BIN_COUNT {
			let target = bin as f32 * ratio;
			let target_bin = target.floor() as usize;
			if target_bin >= BIN_COUNT {
				break;
			}
			bin_mappings.push(PitchBinMapping {
				expected_advance: TAU * HOP_SIZE as f32 * bin as f32 / WINDOW_SIZE as f32,
				target_bin,
				fraction: target - target_bin as f32,
				magnitude_scale: nyquist_taper(target, ratio),
			});
		}
		let active_bin_count = bin_mappings.len();

		Self {
			ratio,
			forward: Arc::clone(&shared.forward),
			inverse: Arc::clone(&shared.inverse),
			input: vec![0.0; WINDOW_SIZE].into_boxed_slice(),
			output: vec![0.0; WINDOW_SIZE].into_boxed_slice(),
			normalization: vec![0.0; WINDOW_SIZE].into_boxed_slice(),
			window,
			transform_buffer: vec![0.0; WINDOW_SIZE].into_boxed_slice(),
			spectrum: vec![Complex32::ZERO; BIN_COUNT].into_boxed_slice(),
			shifted_spectrum: vec![Complex32::ZERO; BIN_COUNT].into_boxed_slice(),
			scratch: vec![Complex32::ZERO; shared.scratch_len].into_boxed_slice(),
			bin_mappings: bin_mappings.into_boxed_slice(),
			previous_phase: vec![0.0; active_bin_count].into_boxed_slice(),
			output_phase: vec![0.0; active_bin_count].into_boxed_slice(),
			cursor: 0,
			samples_until_transform: WINDOW_SIZE,
		}
	}

	/// Processes one block through the persistent phase-vocoder state. Every
	/// working buffer is allocated during construction.
	pub(super) fn process(&mut self, samples: &mut [f32]) {
		for sample in samples {
			*sample = self.process_sample(*sample);
		}
	}

	/// Buffers one sample and periodically transforms a complete overlapping
	/// frame.
	fn process_sample(&mut self, sample: f32) -> f32 {
		let divisor = self.normalization[self.cursor];
		let output = if divisor > f32::EPSILON {
			self.output[self.cursor] / divisor
		} else {
			0.0
		};
		self.output[self.cursor] = 0.0;
		self.normalization[self.cursor] = 0.0;
		self.input[self.cursor] = sample;
		self.cursor = (self.cursor + 1) & WINDOW_MASK;
		self.samples_until_transform -= 1;

		if self.samples_until_transform == 0 {
			self.transform_frame();
			self.samples_until_transform = HOP_SIZE;
		}
		output
	}

	/// Estimates each source bin's true frequency, maps it by the requested
	/// ratio, and overlap-adds the reconstructed frame into the output ring.
	fn transform_frame(&mut self) {
		for index in 0..WINDOW_SIZE {
			let source_index = (self.cursor + index) & WINDOW_MASK;
			self.transform_buffer[index] = self.input[source_index] * self.window[index];
		}
		self.shifted_spectrum.fill(Complex32::ZERO);
		self.forward
			.process_with_scratch(&mut self.transform_buffer, &mut self.spectrum, &mut self.scratch)
			.expect("Preallocated real FFT buffers must retain their planned lengths.");

		for (bin, mapping) in self.bin_mappings.iter().enumerate() {
			let value = self.spectrum[bin];
			let phase = value.arg();
			let residual = wrap_phase(phase - self.previous_phase[bin] - mapping.expected_advance);
			self.previous_phase[bin] = phase;
			let true_advance = mapping.expected_advance + residual;
			// Track synthesis phase per source bin. Several source bins map to
			// one destination while pitching down, so destination-owned phase
			// would advance several times during the same analysis frame.
			self.output_phase[bin] += true_advance * self.ratio;
			let magnitude = value.norm() * mapping.magnitude_scale;
			let shifted = Complex32::from_polar(magnitude, self.output_phase[bin]);
			self.shifted_spectrum[mapping.target_bin] += shifted * (1.0 - mapping.fraction);
			if mapping.fraction > 0.0 && mapping.target_bin + 1 < BIN_COUNT {
				self.shifted_spectrum[mapping.target_bin + 1] += shifted * mapping.fraction;
			}
		}

		// A real inverse transform requires real DC and Nyquist values. The old
		// complex transform discarded their imaginary output components too.
		self.shifted_spectrum[0].im = 0.0;
		self.shifted_spectrum[BIN_COUNT - 1].im = 0.0;
		self.inverse
			.process_with_scratch(&mut self.shifted_spectrum, &mut self.transform_buffer, &mut self.scratch)
			.expect("Preallocated inverse real FFT buffers must retain their planned lengths and real endpoints.");
		let fft_scale = 1.0 / WINDOW_SIZE as f32;
		for index in 0..WINDOW_SIZE {
			let destination = (self.cursor + index) & WINDOW_MASK;
			let window = self.window[index];
			self.output[destination] += self.transform_buffer[index] * fft_scale * window;
			self.normalization[destination] += window * window;
		}
	}
}

impl RuntimeAudioProcessor for PitchShiftProcessor {
	fn process(&mut self, _time: AudioGraphTime, samples: &mut [f32]) {
		PitchShiftProcessor::process(self, samples);
	}
}

fn wrap_phase(phase: f32) -> f32 {
	(phase + std::f32::consts::PI).rem_euclid(TAU) - std::f32::consts::PI
}

/// Softens the upper tenth of the output spectrum only while pitching up.
/// This avoids the ringing caused by cutting partials off abruptly at Nyquist.
fn nyquist_taper(target_bin: f32, ratio: f32) -> f32 {
	if ratio <= 1.0 {
		return 1.0;
	}
	let normalized = target_bin / (BIN_COUNT - 1) as f32;
	if normalized <= 0.9 {
		1.0
	} else {
		((normalized.min(1.0) - 0.9) * 10.0 * std::f32::consts::PI)
			.cos()
			.mul_add(0.5, 0.5)
	}
}

#[cfg(test)]
mod tests {
	use std::f32::consts::TAU;

	use super::{PitchShiftProcessor, WINDOW_SIZE};

	const SAMPLE_RATE: f32 = 48_000.0;
	const SAMPLE_COUNT: usize = 16_384;
	const SOURCE_FREQUENCY: f32 = 750.0;

	fn sine_wave() -> Vec<f32> {
		(0..SAMPLE_COUNT)
			.map(|index| (TAU * SOURCE_FREQUENCY * index as f32 / SAMPLE_RATE).sin())
			.collect()
	}

	fn render_samples(input: &[f32], ratio: f32, chunks: &[usize]) -> Vec<f32> {
		let mut processor = PitchShiftProcessor::new(ratio);
		let mut output = Vec::with_capacity(input.len() + WINDOW_SIZE);
		let mut cursor = 0;
		for &chunk_size in chunks {
			let end = (cursor + chunk_size).min(input.len());
			let mut block = input[cursor..end].to_vec();
			processor.process(&mut block);
			output.extend(block);
			cursor = end;
			if cursor == input.len() {
				break;
			}
		}
		let mut remainder = input[cursor..].to_vec();
		processor.process(&mut remainder);
		output.extend(remainder);
		let mut tail = vec![0.0; WINDOW_SIZE];
		processor.process(&mut tail);
		output.extend(tail);
		output
	}

	fn render(ratio: f32, chunks: &[usize]) -> Vec<f32> {
		render_samples(&sine_wave(), ratio, chunks)
	}

	#[test]
	fn processors_share_immutable_fft_plans() {
		let first = PitchShiftProcessor::new(0.5);
		let second = PitchShiftProcessor::new(2.0);

		assert!(std::sync::Arc::ptr_eq(&first.forward, &second.forward));
		assert!(std::sync::Arc::ptr_eq(&first.inverse, &second.inverse));
	}

	fn magnitude_at(samples: &[f32], frequency: f32) -> f32 {
		let start = WINDOW_SIZE * 2;
		let samples = &samples[start..start + 8192];
		let mut real = 0.0;
		let mut imaginary = 0.0;
		for (index, sample) in samples.iter().enumerate() {
			let phase = TAU * frequency * index as f32 / SAMPLE_RATE;
			real += sample * phase.cos();
			imaginary -= sample * phase.sin();
		}
		real.hypot(imaginary)
	}

	#[test]
	fn pitch_shift_moves_a_tone_up_and_down_without_changing_content_length() {
		let shifted_up = render(2.0, &[SAMPLE_COUNT]);
		let shifted_down = render(0.5, &[SAMPLE_COUNT]);

		assert_eq!(shifted_up.len(), SAMPLE_COUNT + WINDOW_SIZE);
		assert_eq!(shifted_down.len(), SAMPLE_COUNT + WINDOW_SIZE);
		assert!(magnitude_at(&shifted_up, 1500.0) > magnitude_at(&shifted_up, SOURCE_FREQUENCY) * 4.0);
		assert!(magnitude_at(&shifted_down, 375.0) > magnitude_at(&shifted_down, SOURCE_FREQUENCY) * 4.0);
	}

	#[test]
	fn rendering_is_identical_across_period_boundaries() {
		let contiguous = render(1.5, &[SAMPLE_COUNT]);
		let chunked = render(1.5, &[127; 130]);
		assert_eq!(contiguous, chunked);
	}

	#[test]
	fn downshift_preserves_two_distinct_low_tones() {
		let input = (0..SAMPLE_COUNT)
			.map(|index| {
				let time = index as f32 / SAMPLE_RATE;
				(TAU * 300.0 * time).sin() + (TAU * 420.0 * time).sin()
			})
			.collect::<Vec<_>>();
		let output = render_samples(&input, 0.5, &[SAMPLE_COUNT]);

		let first = magnitude_at(&output, 150.0);
		let second = magnitude_at(&output, 210.0);
		let valley = magnitude_at(&output, 180.0);
		assert!(first > valley * 1.8, "the 150 Hz tone was smeared into the spectral valley");
		assert!(second > valley * 1.8, "the 210 Hz tone was smeared into the spectral valley");
	}

	#[test]
	fn pitch_up_rejects_energy_that_would_cross_nyquist() {
		let input = (0..SAMPLE_COUNT)
			.map(|index| {
				let time = index as f32 / SAMPLE_RATE;
				(TAU * 10_000.0 * time).sin() + (TAU * 14_000.0 * time).sin()
			})
			.collect::<Vec<_>>();
		let output = render_samples(&input, 2.0, &[SAMPLE_COUNT]);
		let wanted = magnitude_at(&output, 20_000.0);
		let strongest_alias = (1..18)
			.map(|kilohertz| magnitude_at(&output, kilohertz as f32 * 1000.0))
			.fold(0.0, f32::max);

		assert!(
			wanted > strongest_alias * 20.0,
			"pitch-up output contains strong aliased energy"
		);
	}
}
