use std::{f32::consts::TAU, sync::Arc};

use rustfft::{num_complex::Complex32, Fft, FftPlanner};

use super::RuntimeAudioProcessor;

const WINDOW_SIZE: usize = 1024;
pub(super) const PITCH_SHIFT_LATENCY: usize = WINDOW_SIZE;
const HOP_SIZE: usize = WINDOW_SIZE / 8;
const BIN_COUNT: usize = WINDOW_SIZE / 2 + 1;

/// The `PitchShiftProcessor` struct owns reusable phase-vocoder state for one
/// real-time pitch-shift node.
pub(crate) struct PitchShiftProcessor {
	ratio: f32,
	forward: Arc<dyn Fft<f32>>,
	inverse: Arc<dyn Fft<f32>>,
	input: Box<[f32]>,
	output: Box<[f32]>,
	normalization: Box<[f32]>,
	window: Box<[f32]>,
	spectrum: Box<[Complex32]>,
	shifted_spectrum: Box<[Complex32]>,
	scratch: Box<[Complex32]>,
	previous_phase: Box<[f32]>,
	output_phase: Box<[f32]>,
	cursor: usize,
	samples_seen: usize,
}

impl PitchShiftProcessor {
	pub(super) fn new(ratio: f32) -> Self {
		let mut planner = FftPlanner::new();
		let forward = planner.plan_fft_forward(WINDOW_SIZE);
		let inverse = planner.plan_fft_inverse(WINDOW_SIZE);
		let scratch_len = forward.get_inplace_scratch_len().max(inverse.get_inplace_scratch_len());
		let window = (0..WINDOW_SIZE)
			.map(|index| 0.5 - 0.5 * (TAU * index as f32 / WINDOW_SIZE as f32).cos())
			.collect::<Vec<_>>()
			.into_boxed_slice();

		Self {
			ratio,
			forward,
			inverse,
			input: vec![0.0; WINDOW_SIZE].into_boxed_slice(),
			output: vec![0.0; WINDOW_SIZE].into_boxed_slice(),
			normalization: vec![0.0; WINDOW_SIZE].into_boxed_slice(),
			window,
			spectrum: vec![Complex32::ZERO; WINDOW_SIZE].into_boxed_slice(),
			shifted_spectrum: vec![Complex32::ZERO; WINDOW_SIZE].into_boxed_slice(),
			scratch: vec![Complex32::ZERO; scratch_len].into_boxed_slice(),
			previous_phase: vec![0.0; BIN_COUNT].into_boxed_slice(),
			output_phase: vec![0.0; BIN_COUNT].into_boxed_slice(),
			cursor: 0,
			samples_seen: 0,
		}
	}

	/// Buffers one sample and periodically transforms a complete overlapping
	/// frame. Every buffer is allocated during construction.
	pub(super) fn process(&mut self, sample: f32) -> f32 {
		let divisor = self.normalization[self.cursor];
		let output = if divisor > f32::EPSILON {
			self.output[self.cursor] / divisor
		} else {
			0.0
		};
		self.output[self.cursor] = 0.0;
		self.normalization[self.cursor] = 0.0;
		self.input[self.cursor] = sample;
		self.cursor = (self.cursor + 1) % WINDOW_SIZE;
		self.samples_seen += 1;

		if self.samples_seen >= WINDOW_SIZE && (self.samples_seen - WINDOW_SIZE) % HOP_SIZE == 0 {
			self.transform_frame();
		}
		output
	}

	/// Estimates each source bin's true frequency, maps it by the requested
	/// ratio, and overlap-adds the reconstructed frame into the output ring.
	fn transform_frame(&mut self) {
		for index in 0..WINDOW_SIZE {
			let source_index = (self.cursor + index) % WINDOW_SIZE;
			self.spectrum[index] = Complex32::new(self.input[source_index] * self.window[index], 0.0);
			self.shifted_spectrum[index] = Complex32::ZERO;
		}
		self.forward.process_with_scratch(&mut self.spectrum, &mut self.scratch);

		for bin in 0..BIN_COUNT {
			let value = self.spectrum[bin];
			let phase = value.arg();
			let expected_advance = TAU * HOP_SIZE as f32 * bin as f32 / WINDOW_SIZE as f32;
			let residual = wrap_phase(phase - self.previous_phase[bin] - expected_advance);
			self.previous_phase[bin] = phase;
			let true_advance = expected_advance + residual;
			let target = bin as f32 * self.ratio;
			let target_bin = target.floor() as usize;
			if target_bin >= BIN_COUNT {
				continue;
			}
			// Track synthesis phase per source bin. Several source bins map to
			// one destination while pitching down, so destination-owned phase
			// would advance several times during the same analysis frame.
			self.output_phase[bin] += true_advance * self.ratio;
			let magnitude = value.norm() * nyquist_taper(target, self.ratio);
			let shifted = Complex32::from_polar(magnitude, self.output_phase[bin]);
			let fraction = target - target_bin as f32;
			self.shifted_spectrum[target_bin] += shifted * (1.0 - fraction);
			if fraction > 0.0 && target_bin + 1 < BIN_COUNT {
				self.shifted_spectrum[target_bin + 1] += shifted * fraction;
			}
		}

		for bin in 1..WINDOW_SIZE / 2 {
			self.shifted_spectrum[WINDOW_SIZE - bin] = self.shifted_spectrum[bin].conj();
		}
		self.inverse
			.process_with_scratch(&mut self.shifted_spectrum, &mut self.scratch);
		let fft_scale = 1.0 / WINDOW_SIZE as f32;
		for index in 0..WINDOW_SIZE {
			let destination = (self.cursor + index) % WINDOW_SIZE;
			let window = self.window[index];
			self.output[destination] += self.shifted_spectrum[index].re * fft_scale * window;
			self.normalization[destination] += window * window;
		}
	}
}

impl RuntimeAudioProcessor for PitchShiftProcessor {
	fn process(&mut self, sample: f32) -> f32 {
		PitchShiftProcessor::process(self, sample)
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
			for &sample in &input[cursor..(cursor + chunk_size).min(input.len())] {
				output.push(processor.process(sample));
			}
			cursor = (cursor + chunk_size).min(input.len());
			if cursor == input.len() {
				break;
			}
		}
		for &sample in &input[cursor..] {
			output.push(processor.process(sample));
		}
		output.extend((0..WINDOW_SIZE).map(|_| processor.process(0.0)));
		output
	}

	fn render(ratio: f32, chunks: &[usize]) -> Vec<f32> {
		render_samples(&sine_wave(), ratio, chunks)
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
