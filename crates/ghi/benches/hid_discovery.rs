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

use ghi::hid::{Match, Scanner};

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

	unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
		ALLOCATIONS.fetch_add(1, Ordering::Relaxed);
		// SAFETY: the caller upholds `GlobalAlloc::alloc_zeroed`'s contract, which `System` shares.
		unsafe { System.alloc_zeroed(layout) }
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

/// The gamepad rules the input system scans for: joysticks, gamepads, and Sony controllers by vendor.
const GAMEPADS: &[Match] = &[
	Match::Usage { page: 0x01, usage: 0x04 },
	Match::Usage { page: 0x01, usage: 0x05 },
	Match::Vendor(0x054C),
];

const ROUNDS: usize = 7;
const WARM_ITERATIONS: usize = 25;

fn main() {
	let mut args = std::env::args().skip(1);
	if args.next().as_deref() == Some("--measure") {
		measure(
			&args
				.next()
				.expect("Missing the side to measure. The most likely cause is a manual run of the child mode."),
		);
		return;
	}

	let mut hidapi = Samples::default();
	let mut native = Samples::default();
	for round in 0..ROUNDS {
		if round % 2 == 0 {
			hidapi.add(run_child("hidapi"));
			native.add(run_child("native"));
		} else {
			native.add(run_child("native"));
			hidapi.add(run_child("hidapi"));
		}
	}

	println!("HID gamepad discovery, {ROUNDS} fresh processes per side, {WARM_ITERATIONS} warm scans each");
	println!(
		"{:<8} {:>14} {:>14} {:>14} {:>12} {:>12}",
		"side", "cold median", "cold min", "warm median", "cold allocs", "warm allocs"
	);
	for (name, samples) in [("hidapi", &hidapi), ("native", &native)] {
		println!(
			"{:<8} {:>11.3} ms {:>11.3} ms {:>11.3} ms {:>12} {:>12}",
			name,
			median(&samples.cold) * 1e-6,
			samples.cold.iter().min().copied().unwrap_or_default() as f64 * 1e-6,
			median(&samples.warm) * 1e-6,
			samples.cold_allocations,
			samples.warm_allocations
		);
	}
	println!(
		"cold speedup: {:.1}x, warm speedup: {:.1}x",
		median(&hidapi.cold) / median(&native.cold),
		median(&hidapi.warm) / median(&native.warm)
	);

	println!(
		"hidapi found {} device(s), native found {}",
		hidapi.paths.len(),
		native.paths.len()
	);
	for path in hidapi.paths.iter().filter(|path| !native.paths.contains(path)) {
		println!("  only hidapi: {path}");
	}
	for path in native.paths.iter().filter(|path| !hidapi.paths.contains(path)) {
		println!("  only native: {path}");
	}
	for path in hidapi.paths.iter().filter(|path| native.paths.contains(path)) {
		println!("  both:        {path}");
	}
}

/// The `Samples` struct collects one side's results across child processes.
#[derive(Default)]
struct Samples {
	cold: Vec<u64>,
	warm: Vec<u64>,
	/// Allocations are deterministic for a device set, so the last process's counts stand for all of them.
	cold_allocations: u64,
	warm_allocations: u64,
	paths: Vec<String>,
}

impl Samples {
	fn add(&mut self, measurement: Measurement) {
		self.cold.push(measurement.cold);
		self.warm.push(measurement.warm);
		self.cold_allocations = measurement.cold_allocations;
		self.warm_allocations = measurement.warm_allocations;
		self.paths = measurement.paths;
	}
}

/// The `Measurement` struct carries one child process's results back to the parent.
struct Measurement {
	cold: u64,
	warm: u64,
	cold_allocations: u64,
	warm_allocations: u64,
	paths: Vec<String>,
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

/// Times and counts the first discovery in this process, then repeated ones, and prints them with the paths.
///
/// Discovery only counts matches; the paths are collected afterwards so collecting them is not measured.
fn measure(side: &str) {
	let (cold, cold_allocations, warm, warm_allocations, paths) = match side {
		"hidapi" => {
			let (mut api, cold, cold_allocations) = sample(|| {
				let api = hidapi::HidApi::new().unwrap();
				std::hint::black_box(hidapi_gamepads(&api).count());
				api
			});
			let (warm, warm_allocations) = sample_warm(|| {
				api.refresh_devices().unwrap();
				std::hint::black_box(hidapi_gamepads(&api).count());
			});
			let mut paths = hidapi_gamepads(&api)
				.map(|device| device.path().to_string_lossy().into_owned())
				.collect::<Vec<_>>();
			// hidapi lists a device once per top-level usage.
			paths.sort_unstable();
			paths.dedup();
			(cold, cold_allocations, warm, warm_allocations, paths)
		}
		"native" => {
			let (mut scanner, cold, cold_allocations) = sample(|| {
				let mut scanner = Scanner::new(GAMEPADS).unwrap();
				let mut count = 0;
				scanner.scan(|_| count += 1).unwrap();
				std::hint::black_box(count);
				scanner
			});
			let (warm, warm_allocations) = sample_warm(|| {
				let mut count = 0;
				scanner.scan(|_| count += 1).unwrap();
				std::hint::black_box(count);
			});
			let mut paths = Vec::new();
			scanner.scan(|device| paths.push(device.path.to_string())).unwrap();
			(cold, cold_allocations, warm, warm_allocations, paths)
		}
		side => panic!("Unknown side {side}. The most likely cause is a typo in the child arguments."),
	};

	println!("{cold} {warm} {cold_allocations} {warm_allocations}");
	let mut paths = paths;
	paths.sort_unstable();
	for path in paths {
		println!("{path}");
	}
}

/// Runs `work` once and returns its result with the time and allocations it took.
fn sample<T>(work: impl FnOnce() -> T) -> (T, u64, u64) {
	let allocations = ALLOCATIONS.load(Ordering::Relaxed);
	let start = Instant::now();
	let result = work();
	let time = start.elapsed().as_nanos() as u64;
	(result, time, ALLOCATIONS.load(Ordering::Relaxed) - allocations)
}

/// Runs `work` [`WARM_ITERATIONS`] times and returns the median time and the allocations of the last run.
fn sample_warm(mut work: impl FnMut()) -> (u64, u64) {
	let mut times = [0u64; WARM_ITERATIONS];
	let mut allocations = 0;
	for time in &mut times {
		((), *time, allocations) = sample(&mut work);
	}
	times.sort_unstable();
	(times[WARM_ITERATIONS / 2], allocations)
}

/// Applies the [`GAMEPADS`] rules to hidapi's device list, which holds one entry per top-level usage.
fn hidapi_gamepads(api: &hidapi::HidApi) -> impl Iterator<Item = &hidapi::DeviceInfo> {
	api.device_list().filter(|device| {
		GAMEPADS.contains(&Match::Usage {
			page: device.usage_page(),
			usage: device.usage(),
		}) || GAMEPADS.contains(&Match::Vendor(device.vendor_id()))
	})
}

fn median(samples: &[u64]) -> f64 {
	let mut samples = samples.to_vec();
	samples.sort_unstable();
	samples.get(samples.len() / 2).copied().unwrap_or_default() as f64
}
