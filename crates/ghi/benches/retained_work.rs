//! Measures repeated public GHI workflows on the same native backend as the rendering tests.
//! See `benches/README.md` for timing boundaries and comparison guidance.

use divan::Bencher;
use ghi::{
	context::{Context as _, ContextCreate as _},
	implementation::{Context, Device, Instance},
	queue::{FrameRequest, Queue as _, QueueExecution as _},
	*,
};

#[path = "retained_work/raster.rs"]
mod raster;

// Count transient Rust allocations along with time; native driver allocations are outside this allocator.
#[global_allocator]
static ALLOC: divan::AllocProfiler = divan::AllocProfiler::system();

fn main() {
	divan::main();
}

/// Creates the native raster setup used by the GPU tests, with validation opt-in for smoke runs.
/// Bind this tuple in order so the context drops before the device and instance.
fn setup() -> (Instance, Device, Context, QueueHandle) {
	let features = ghi::device::Features::new().validation(std::env::var_os("GHI_BENCH_VALIDATION").is_some());
	let mut instance = Instance::new(features).expect(
		"Failed to create the benchmark instance. The most likely cause is that the native GPU backend is unavailable.",
	);
	let mut queue = None;
	let device = instance
		.create_device(
			features,
			&mut [(QueueSelection::new(ghi::types::WorkloadTypes::RASTER), &mut queue)],
		)
		.expect("Failed to create the benchmark device. The most likely cause is unavailable raster queue support.");
	let mut context = ghi::device::Device::create_context(&device)
		.expect("Failed to create the benchmark context. The most likely cause is unavailable backend command support.");
	context.set_frames_in_flight(2);
	(instance, device, context, queue.unwrap())
}

/// Warms both frame slots, then measures completed iterations with all fixture setup excluded.
fn measure(bencher: Bencher, mut iteration: impl FnMut()) {
	for _ in 0..32 {
		iteration();
	}
	bencher.bench_local(iteration);
}
