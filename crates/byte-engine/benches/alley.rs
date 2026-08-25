//! Run with `cargo bench -p byte-engine --bench alley`.

use std::panic::resume_unwind;

use byte_engine::core::alley::{Alley, Lane};
use divan::{Bencher, counter::ItemsCount};

/// Sixteen rounds expose collective costs without exceeding one dispatch's capacity.
const COLLECTIVE_ROUNDS: usize = 16;
const SHARED_ITEM_COUNT: usize = 4_096;

fn main() {
	divan::main();
}

/// Returns lane results or resumes a panic captured by `Alley::execute`.
fn finish_dispatch<R>(result: std::thread::Result<Vec<R>>) -> Vec<R> {
	result.unwrap_or_else(|payload| resume_unwind(payload))
}

/// Measures minimal lane dispatch and ordered result collection.
#[divan::bench(args = [1, 2, 4, 8])]
fn execute_dispatch(bencher: Bencher, parallelism: usize) {
	let mut alley = Alley::with_parallelism(parallelism);

	bencher.counter(ItemsCount::new(parallelism)).bench_local(|| {
		let results = finish_dispatch(alley.execute(|lane| divan::black_box(lane.idx())));
		divan::black_box(results);
	});
}

/// Measures one single-runner claim across all lanes.
#[divan::bench(args = [1, 2, 4, 8])]
fn only_one_runs(bencher: Bencher, parallelism: usize) {
	let mut alley = Alley::with_parallelism(parallelism);

	bencher.counter(ItemsCount::new(parallelism)).bench_local(|| {
		let results = finish_dispatch(alley.execute(|lane| {
			lane.only_one_runs(|| {
				divan::black_box(());
			});
			divan::black_box(lane.idx())
		}));
		divan::black_box(results);
	});
}

/// Measures one claim section limited to half the lanes, with at least one runner.
#[divan::bench(args = [1, 2, 4, 8])]
fn with_limited_parallelism(bencher: Bencher, parallelism: usize) {
	let mut alley = Alley::with_parallelism(parallelism);
	let limit = (parallelism / 2).max(1);

	bencher.counter(ItemsCount::new(parallelism)).bench_local(|| {
		let results = finish_dispatch(alley.execute(|lane| {
			lane.with_limited_parallelism(limit, || {
				divan::black_box(());
			});
			divan::black_box(lane.idx())
		}));
		divan::black_box(results);
	});
}

/// Measures publication and collection of one broadcast scalar.
#[divan::bench(args = [1, 2, 4, 8])]
fn broadcast_scalar(bencher: Bencher, parallelism: usize) {
	let mut alley = Alley::with_parallelism(parallelism);

	bencher.counter(ItemsCount::new(parallelism)).bench_local(|| {
		let results = finish_dispatch(alley.execute(|lane| lane.broadcast(|| divan::black_box(1_u64))));
		divan::black_box(results);
	});
}

/// Measures one scalar publication from every lane and its collective barrier.
#[divan::bench(args = [1, 2, 4, 8])]
fn each_scalar(bencher: Bencher, parallelism: usize) {
	let mut alley = Alley::with_parallelism(parallelism);

	bencher.counter(ItemsCount::new(parallelism)).bench_local(|| {
		let results = finish_dispatch(alley.execute(|lane| {
			let lane_idx = lane.idx();
			let values = lane.each(|| divan::black_box(lane_idx as u64));
			divan::black_box(values).next().expect("Each must return one value per lane")
		}));
		divan::black_box(results);
	});
}

/// Measures chained broadcast synchronization after paying dispatch overhead once.
#[divan::bench(args = [1, 2, 4, 8])]
fn chained_broadcasts(bencher: Bencher, parallelism: usize) {
	let mut alley = Alley::with_parallelism(parallelism);

	bencher
		.counter(ItemsCount::new(parallelism * COLLECTIVE_ROUNDS))
		.bench_local(|| {
			let results = finish_dispatch(alley.execute(|lane| {
				let mut checksum = 0_u64;
				for round in 0..COLLECTIVE_ROUNDS {
					checksum ^= lane.broadcast(|| divan::black_box(round as u64));
				}
				divan::black_box(checksum)
			}));
			divan::black_box(results);
		});
}

/// Measures chained each barriers after paying dispatch overhead once.
#[divan::bench(args = [1, 2, 4, 8])]
fn chained_each(bencher: Bencher, parallelism: usize) {
	let mut alley = Alley::with_parallelism(parallelism);

	bencher
		.counter(ItemsCount::new(parallelism * COLLECTIVE_ROUNDS))
		.bench_local(|| {
			let results = finish_dispatch(alley.execute(|lane| {
				let lane_idx = lane.idx();
				let mut checksum = 0_u64;
				for round in 0..COLLECTIVE_ROUNDS {
					let values = lane.each(|| divan::black_box((round + lane_idx) as u64));
					let first_value = divan::black_box(values).next().expect("Each must return one value per lane");
					checksum = checksum.wrapping_add(first_value);
				}
				divan::black_box(checksum)
			}));
			divan::black_box(results);
		});
}

/// Measures two independent mutable-resource owner branches without synchronization.
#[divan::bench(args = [1, 2, 4, 8])]
fn mutable_resource_branches(bencher: Bencher, parallelism: usize) {
	let mut alley = Alley::with_parallelism(parallelism);
	let mut first = 1_u64;
	let mut second = 2_u64;

	bencher.counter(ItemsCount::new(parallelism * 2)).bench_local(|| {
		let results = finish_dispatch(alley.execute_with_mut((&mut first, &mut second), |lane, (first, second)| {
			let first = lane.only_one_runs_mut(first, |value| divan::black_box(*value));
			let second = lane.only_one_runs_mut(second, |value| divan::black_box(*value));
			divan::black_box((first, second))
		}));
		divan::black_box(results);
	});
}

/// Measures balanced shared-slice partitioning, lane-local checksums, and collective publication.
#[divan::bench(args = [1, 2, 4, 8])]
fn each_shared_checksum(bencher: Bencher, parallelism: usize) {
	let mut alley = Alley::with_parallelism(parallelism);
	let input: Vec<u64> = (0..SHARED_ITEM_COUNT as u64).collect();

	bencher.counter(ItemsCount::new(input.len())).bench_local(|| {
		let results = finish_dispatch(alley.execute(|lane| {
			let partials = lane.each_shared(divan::black_box(&input), |partition| {
				partition.iter().copied().fold(0_u64, u64::wrapping_add)
			});
			divan::black_box(partials).fold(0_u64, u64::wrapping_add)
		}));
		divan::black_box(results);
	});
}
