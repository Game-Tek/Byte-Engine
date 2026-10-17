//! Run with `cargo bench -p byte-engine --bench alley`.

use std::{
	hint::spin_loop,
	panic::resume_unwind,
	time::{Duration, Instant},
};

use byte_engine::core::alley::{Alley, Lane};
use divan::{Bencher, counter::ItemsCount};

/// Sixteen rounds expose collective costs without exceeding one dispatch's capacity.
const COLLECTIVE_ROUNDS: usize = 16;
const SHARED_ITEM_COUNT: usize = 4_096;
/// Enough sections without values that their cost hides the one dispatch they run in.
const SECTION_ROUNDS: usize = 1_024;
/// Rounds of a lane function in which a different lane is busy before each broadcast.
const BUSY_ROUNDS: usize = 32;

fn main() {
	divan::main();
}

/// Returns lane results or resumes a panic captured by `Alley::execute`.
fn finish_dispatch<R>(result: std::thread::Result<Vec<R>>) -> Vec<R> {
	result.unwrap_or_else(|payload| resume_unwind(payload))
}

/// Keeps the calling lane busy for `duration`, as uneven work between sections does.
fn busy(duration: Duration) {
	let start = Instant::now();
	while start.elapsed() < duration {
		spin_loop();
	}
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

/// Measures a dispatch whose workers went to sleep, as a frame's first dispatch finds them.
#[divan::bench(args = [2, 4, 8], sample_count = 64, sample_size = 1)]
fn execute_dispatch_after_idle(bencher: Bencher, parallelism: usize) {
	let mut alley = Alley::with_parallelism(parallelism);

	bencher
		.counter(ItemsCount::new(parallelism))
		.with_inputs(|| std::thread::sleep(Duration::from_millis(2)))
		.bench_local_values(|()| {
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

/// Measures `each` sections without values inside one dispatch, which is the cost of one barrier.
#[divan::bench(args = [1, 2, 4, 8])]
fn sections_each(bencher: Bencher, parallelism: usize) {
	let mut alley = Alley::with_parallelism(parallelism);

	bencher.counter(ItemsCount::new(SECTION_ROUNDS)).bench_local(|| {
		finish_dispatch(alley.execute(|lane| {
			for _ in 0..SECTION_ROUNDS {
				let _ = lane.each(|| divan::black_box(()));
			}
		}));
	});
}

/// Measures broadcast sections without values inside one dispatch.
#[divan::bench(args = [1, 2, 4, 8])]
fn sections_broadcast(bencher: Bencher, parallelism: usize) {
	let mut alley = Alley::with_parallelism(parallelism);

	bencher.counter(ItemsCount::new(SECTION_ROUNDS)).bench_local(|| {
		finish_dispatch(alley.execute(|lane| {
			for _ in 0..SECTION_ROUNDS {
				lane.broadcast(|| divan::black_box(()));
			}
		}));
	});
}

/// Measures a phased algorithm's rounds: one lane changes the resource, then every lane reads its share.
///
/// Each round is one exclusive and one shared stage, as a physics step alternates them.
#[divan::bench(args = [1, 2, 4, 8])]
fn sections_phased(bencher: Bencher, parallelism: usize) {
	let mut alley = Alley::with_parallelism(parallelism);
	let mut values = vec![0_u64; SHARED_ITEM_COUNT];

	bencher.counter(ItemsCount::new(SECTION_ROUNDS)).bench_local(|| {
		finish_dispatch(alley.execute_phased(&mut values, parallelism, |lane, values| {
			for round in 0..SECTION_ROUNDS as u64 {
				lane.broadcast_mut(values, |values| values[0] = round);
				let _ = lane.each_ref(values, |values, lane| {
					divan::black_box(lane.share(values).first().copied());
				});
			}
		}));
	});
}

/// Measures keyed shared stages, as a solver runs one stage per batch.
#[divan::bench(args = [1, 2, 4, 8])]
fn sections_keyed(bencher: Bencher, parallelism: usize) {
	let mut alley = Alley::with_parallelism(parallelism);
	let mut value = 0_u64;

	bencher.counter(ItemsCount::new(SECTION_ROUNDS)).bench_local(|| {
		finish_dispatch(alley.execute_phased(&mut value, parallelism, |lane, value| {
			for round in 0..SECTION_ROUNDS as u32 {
				let _ = lane.each_ref_keyed(value, round, |value, stage| {
					divan::black_box((*value, stage.key()));
				});
			}
		}));
	});
}

/// Measures broadcasts when a different lane is busy before each one, as uneven work makes lanes arrive apart.
///
/// Every lane has 4 µs of extra work every few rounds and 1 µs of work after each broadcast. When the first lane to
/// arrive runs each broadcast, the lanes' busy spells overlap, so a dispatch takes about one lane's share of them.
#[divan::bench(args = [2, 4, 8])]
fn broadcasts_with_a_busy_lane(bencher: Bencher, parallelism: usize) {
	let mut alley = Alley::with_parallelism(parallelism);

	bencher.counter(ItemsCount::new(BUSY_ROUNDS)).bench_local(|| {
		let results = finish_dispatch(alley.execute(|lane| {
			let lane_idx = lane.idx();
			let mut checksum = 0_u64;
			for round in 0..BUSY_ROUNDS {
				if round % parallelism == lane_idx {
					busy(Duration::from_micros(4));
				}
				checksum ^= lane.broadcast(|| divan::black_box(round as u64));
				busy(Duration::from_micros(1));
			}
			checksum
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
