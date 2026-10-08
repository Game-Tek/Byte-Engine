//! Compares startup gamepad discovery through hidapi with [`ghi::hid::Scanner`].
//!
//! Run it with `cargo bench -p byte-engine-ghi --bench hid_discovery`. Each measurement runs in a fresh process,
//! because the first call in a process pays for platform setup, which is the cost an application sees at startup.
//! Rounds alternate which side runs first so OS caches do not favor one of them.
//!
//! Allocation counts cover the Rust global allocator only. hidapi's C backend and the platform frameworks
//! allocate outside it, so the counts show what each side adds on the Rust side.

use std::{
	alloc::{GlobalAlloc, Layout, System},
	process::Command,
	sync::atomic::{AtomicU64, Ordering},
	time::Instant,
};

use ghi::hid::{Scanner, Usage};

/// Joysticks and gamepads, the usages the input system reads.
const GAMEPADS: &[Usage] = &[Usage { page: 0x01, usage: 0x04 }, Usage { page: 0x01, usage: 0x05 }];

const ROUNDS: usize = 7;
const WARM_ITERATIONS: usize = 25;

/// The `CountingAllocator` struct counts allocations and reallocations so each side's allocations can be reported.
struct CountingAllocator;

static ALLOCATIONS: AtomicU64 = AtomicU64::new(0);

// SAFETY: every call forwards to the system allocator unchanged; counting has no effect on the memory returned.
unsafe impl GlobalAlloc for CountingAllocator {
	unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
		ALLOCATIONS.fetch_add(1, Ordering::Relaxed);
		// SAFETY: the caller upholds `GlobalAlloc::alloc`'s contract, which `System` shares.
		unsafe { System.alloc(layout) }
	}

	unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
		ALLOCATIONS.fetch_add(1, Ordering::Relaxed);
		// SAFETY: `ptr` came from this allocator, which hands out `System` memory.
		unsafe { System.realloc(ptr, layout, new_size) }
	}

	unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
		// SAFETY: `ptr` came from this allocator, which hands out `System` memory.
		unsafe { System.dealloc(ptr, layout) }
	}
}

#[global_allocator]
static ALLOCATOR: CountingAllocator = CountingAllocator;

/// The `Measurement` struct carries one child process's results back to the parent.
struct Measurement {
	cold: u64,
	warm: u64,
	cold_allocations: u64,
	warm_allocations: u64,
	paths: Vec<String>,
}

fn main() {
	let mut args = std::env::args().skip(1);
	if args.next().as_deref() == Some("--measure") {
		match args.next().as_deref() {
			Some("hidapi") => measure_hidapi(),
			Some("native") => measure_native(),
			side => panic!("Unknown side {side:?}. The most likely cause is a typo in the child arguments."),
		}
		return;
	}

	let mut hidapi = Vec::new();
	let mut native = Vec::new();
	for round in 0..ROUNDS {
		// Alternate the order so neither side always runs on warmer OS caches.
		if round % 2 == 0 {
			hidapi.push(run_child("hidapi"));
			native.push(run_child("native"));
		} else {
			native.push(run_child("native"));
			hidapi.push(run_child("hidapi"));
		}
	}

	println!("HID gamepad discovery, {ROUNDS} fresh processes per side, {WARM_ITERATIONS} warm scans each");
	println!(
		"{:<8} {:>14} {:>14} {:>12} {:>12}",
		"side", "cold median", "warm median", "cold allocs", "warm allocs"
	);
	let medians = [("hidapi", &hidapi), ("native", &native)].map(|(name, runs)| {
		let cold = median(runs.iter().map(|run| run.cold));
		let warm = median(runs.iter().map(|run| run.warm));
		// Allocations are deterministic for a device set, so any run stands for all of them.
		let last = runs.last().unwrap();
		println!(
			"{name:<8} {:>11.3} ms {:>11.3} ms {:>12} {:>12}",
			cold * 1e-6,
			warm * 1e-6,
			last.cold_allocations,
			last.warm_allocations
		);
		(cold, warm)
	});
	println!(
		"cold speedup: {:.1}x, warm speedup: {:.1}x",
		medians[0].0 / medians[1].0,
		medians[0].1 / medians[1].1
	);

	let hidapi = &hidapi.last().unwrap().paths;
	let native = &native.last().unwrap().paths;
	println!("hidapi found {} device(s), native found {}", hidapi.len(), native.len());
	for path in hidapi.iter().filter(|path| !native.contains(path)) {
		println!("  only hidapi: {path}");
	}
	for path in native.iter().filter(|path| !hidapi.contains(path)) {
		println!("  only native: {path}");
	}
}

/// Runs one measurement in a fresh process and parses its `cold warm cold-allocs warm-allocs` line followed by one
/// path per line.
fn run_child(side: &str) -> Measurement {
	let output = Command::new(std::env::current_exe().unwrap())
		.args(["--measure", side])
		.output()
		.expect("Failed to start the measurement process. The most likely cause is that the benchmark binary moved.");
	assert!(
		output.status.success(),
		"The {side} measurement failed: {}. The most likely cause is that HID enumeration is unavailable.",
		String::from_utf8_lossy(&output.stderr)
	);
	let stdout = String::from_utf8(output.stdout).unwrap();
	let mut lines = stdout.lines();
	let mut numbers = lines.next().unwrap().split(' ').map(|number| number.parse::<u64>().unwrap());
	let mut next = || numbers.next().unwrap();
	Measurement {
		cold: next(),
		warm: next(),
		cold_allocations: next(),
		warm_allocations: next(),
		paths: lines.map(str::to_owned).collect(),
	}
}

/// Measures `HidApi::new` plus filtering, then repeated refreshes, and prints the results with the found paths.
fn measure_hidapi() {
	let matches = |device: &&hidapi::DeviceInfo| {
		GAMEPADS.contains(&Usage {
			page: device.usage_page(),
			usage: device.usage(),
		})
	};
	let (mut api, cold, cold_allocations) = sample(|| {
		let api = hidapi::HidApi::new().unwrap();
		std::hint::black_box(api.device_list().filter(matches).count());
		api
	});
	let (warm, warm_allocations) = sample_warm(|| {
		api.refresh_devices().unwrap();
		std::hint::black_box(api.device_list().filter(matches).count());
	});

	let paths = api
		.device_list()
		.filter(matches)
		.map(|device| device.path().to_string_lossy().into_owned())
		.collect();
	print_measurement(cold, warm, cold_allocations, warm_allocations, paths);
}

/// Measures [`Scanner::new`] plus a scan, then repeated scans, and prints the results with the found paths.
fn measure_native() {
	let (mut scanner, cold, cold_allocations) = sample(|| {
		let mut scanner = Scanner::new(GAMEPADS);
		scanner.scan(|device| _ = std::hint::black_box(device)).unwrap();
		scanner
	});
	let (warm, warm_allocations) = sample_warm(|| scanner.scan(|device| _ = std::hint::black_box(device)).unwrap());

	let mut paths = Vec::new();
	scanner.scan(|device| paths.push(device.path.to_string())).unwrap();
	print_measurement(cold, warm, cold_allocations, warm_allocations, paths);
}

fn print_measurement(cold: u64, warm: u64, cold_allocations: u64, warm_allocations: u64, mut paths: Vec<String>) {
	println!("{cold} {warm} {cold_allocations} {warm_allocations}");
	// hidapi lists a device once per top-level usage.
	paths.sort_unstable();
	paths.dedup();
	for path in paths {
		println!("{path}");
	}
}

/// Runs `work` once and returns its result with the nanoseconds and allocations it took.
fn sample<T>(work: impl FnOnce() -> T) -> (T, u64, u64) {
	let allocations = ALLOCATIONS.load(Ordering::Relaxed);
	let start = Instant::now();
	let result = work();
	let time = start.elapsed().as_nanos() as u64;
	(result, time, ALLOCATIONS.load(Ordering::Relaxed) - allocations)
}

/// Runs `work` [`WARM_ITERATIONS`] times and returns the median nanoseconds and the allocations of the last run.
fn sample_warm(mut work: impl FnMut()) -> (u64, u64) {
	let mut times = [0u64; WARM_ITERATIONS];
	let mut allocations = 0;
	for time in &mut times {
		((), *time, allocations) = sample(&mut work);
	}
	times.sort_unstable();
	(times[WARM_ITERATIONS / 2], allocations)
}

fn median(samples: impl Iterator<Item = u64>) -> f64 {
	let mut samples = samples.collect::<Vec<_>>();
	samples.sort_unstable();
	samples[samples.len() / 2] as f64
}
