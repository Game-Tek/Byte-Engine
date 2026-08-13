//! Run with `cargo bench -p byte-engine --bench audio_graph`.

use byte_engine::audio::audio_system::benchmarks::{AudioGraphBenchmark, AudioGraphBenchmarkState, PERIOD_SIZE};
use divan::{counter::ItemsCount, Bencher};

fn main() {
	divan::main();
}

fn benchmark(bencher: Bencher, benchmark: AudioGraphBenchmark) {
	let mut state = AudioGraphBenchmarkState::new(benchmark);
	bencher
		.counter(ItemsCount::new(PERIOD_SIZE))
		.bench_local(|| state.render_period());
}

#[divan::bench]
fn direct_unity(bencher: Bencher) {
	benchmark(bencher, AudioGraphBenchmark::DirectUnity);
}

#[divan::bench]
fn resample_44100_to_48000(bencher: Bencher) {
	benchmark(bencher, AudioGraphBenchmark::Resample44100To48000);
}

#[divan::bench]
fn custom_processor(bencher: Bencher) {
	benchmark(bencher, AudioGraphBenchmark::CustomProcessor);
}

#[divan::bench]
fn pitch_shift_up_1_5x(bencher: Bencher) {
	benchmark(bencher, AudioGraphBenchmark::PitchShiftUp);
}

#[divan::bench]
fn pitch_shift_down_0_5x(bencher: Bencher) {
	benchmark(bencher, AudioGraphBenchmark::PitchShiftDown);
}
