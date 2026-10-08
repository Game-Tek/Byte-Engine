//! Compares startup gamepad discovery through hidapi with [`ghi::hid::scan`].
//!
//! Run it with `cargo bench -p byte-engine-ghi --bench hid_discovery`. Each measurement runs in a fresh process,
//! because the first call in a process pays for platform setup, which is the cost an application sees at startup.
//! Rounds alternate which side runs first so OS caches do not favor one of them.

use std::{process::Command, time::Instant};

use ghi::hid::Match;

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
	println!("{:<8} {:>14} {:>14} {:>14}", "side", "cold median", "cold min", "warm median");
	for (name, samples) in [("hidapi", &hidapi), ("native", &native)] {
		println!(
			"{:<8} {:>11.3} ms {:>11.3} ms {:>11.3} ms",
			name,
			median(&samples.cold) * 1e-6,
			samples.cold.iter().min().copied().unwrap_or_default() as f64 * 1e-6,
			median(&samples.warm) * 1e-6
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
	paths: Vec<String>,
}

impl Samples {
	fn add(&mut self, (cold, warm, paths): (u64, u64, Vec<String>)) {
		self.cold.push(cold);
		self.warm.push(warm);
		self.paths = paths;
	}
}

/// Runs one measurement in a fresh process and parses its `cold warm` line followed by one path per line.
fn run_child(side: &str) -> (u64, u64, Vec<String>) {
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
	let mut times = lines.next().unwrap().split(' ').map(|time| time.parse::<u64>().unwrap());
	let (cold, warm) = (times.next().unwrap(), times.next().unwrap());
	(cold, warm, lines.map(str::to_owned).collect())
}

/// Times the first discovery in this process, then the median of repeated ones, and prints them with the paths.
fn measure(side: &str) {
	let start = Instant::now();
	let (mut discover, paths): (Box<dyn FnMut() -> Vec<String>>, _) = match side {
		"hidapi" => {
			let mut api = hidapi::HidApi::new().unwrap();
			let paths = hidapi_gamepads(&api);
			let discover = move || {
				api.refresh_devices().unwrap();
				hidapi_gamepads(&api)
			};
			(Box::new(discover), paths)
		}
		"native" => {
			let paths = native_gamepads();
			(Box::new(native_gamepads), paths)
		}
		side => panic!("Unknown side {side}. The most likely cause is a typo in the child arguments."),
	};
	let cold = start.elapsed().as_nanos() as u64;

	let mut warm = (0..WARM_ITERATIONS)
		.map(|_| {
			let start = Instant::now();
			std::hint::black_box(discover());
			start.elapsed().as_nanos() as u64
		})
		.collect::<Vec<_>>();
	warm.sort_unstable();

	println!("{cold} {}", warm[warm.len() / 2]);
	for path in paths {
		println!("{path}");
	}
}

/// Applies the [`GAMEPADS`] rules to hidapi's device list, which holds one entry per top-level usage.
fn hidapi_gamepads(api: &hidapi::HidApi) -> Vec<String> {
	let mut paths = api
		.device_list()
		.filter(|device| {
			GAMEPADS.contains(&Match::Usage {
				page: device.usage_page(),
				usage: device.usage(),
			}) || GAMEPADS.contains(&Match::Vendor(device.vendor_id()))
		})
		.map(|device| device.path().to_string_lossy().into_owned())
		.collect::<Vec<_>>();
	paths.sort_unstable();
	paths.dedup();
	paths
}

fn native_gamepads() -> Vec<String> {
	let mut paths = ghi::hid::scan(GAMEPADS)
		.unwrap()
		.into_iter()
		.map(|device| device.path.as_str().to_owned())
		.collect::<Vec<_>>();
	paths.sort_unstable();
	paths
}

fn median(samples: &[u64]) -> f64 {
	let mut samples = samples.to_vec();
	samples.sort_unstable();
	samples.get(samples.len() / 2).copied().unwrap_or_default() as f64
}
