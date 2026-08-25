//! Run with `cargo bench -p byte-engine --bench animation_graph`.

use byte_engine::animation::graph::benchmarks::{AnimationGraphBenchmark, AnimationGraphBenchmarkFixture};
use divan::{Bencher, counter::ItemsCount};

fn main() {
	divan::main();
}

fn benchmark(bencher: Bencher, benchmark: AnimationGraphBenchmark, node_count: usize) {
	let mut fixture = AnimationGraphBenchmarkFixture::new(benchmark, node_count);
	let mut state = fixture.prepare();
	bencher.counter(ItemsCount::new(node_count)).bench_local(|| {
		divan::black_box(state.advance());
	});
}

#[divan::bench(args = [1, 32, 128])]
fn active_pose(bencher: Bencher, node_count: usize) {
	benchmark(bencher, AnimationGraphBenchmark::ActivePose, node_count);
}

#[divan::bench(args = [1, 32, 128])]
fn active_root_motion(bencher: Bencher, node_count: usize) {
	benchmark(bencher, AnimationGraphBenchmark::ActiveRootMotion, node_count);
}

#[divan::bench(args = [1, 32, 128])]
fn inertialized_transition(bencher: Bencher, node_count: usize) {
	benchmark(bencher, AnimationGraphBenchmark::InertializedTransition, node_count);
}
