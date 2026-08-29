//! Run with `cargo bench -p byte-engine --bench channel --no-default-features`.

use std::{
	sync::{
		Arc, Barrier,
		atomic::{AtomicBool, Ordering},
	},
	thread::{self, JoinHandle},
};

use byte_engine::core::{
	channel::{Channel, DefaultChannel},
	factory::Factory,
	listener::{DefaultListener, Listener},
	message_bus::MessageBus,
};
use divan::{Bencher, counter::ItemsCount};

/// Draining below the current 128-message capacity prevents a blocking send.
const DRAIN_BATCH_SIZE: usize = 64;
const SCALAR_MESSAGE_COUNT: usize = 1_048_576;
const OBSERVED_FACTORY_CREATE_COUNT: usize = 262_144;
const BROADCAST_MESSAGE_COUNT: usize = 262_144;
const CONTENDED_MESSAGE_COUNT: usize = 1_048_576;

fn main() {
	divan::main();
}

/// Publishes bounded batches, then drains every listener before reusing a slot.
fn publish_and_drain<M: Clone + Send + Sync + 'static>(
	channel: &DefaultChannel<M>,
	listeners: &mut [DefaultListener<M>],
	message_count: usize,
	mut make_message: impl FnMut(usize) -> M,
) {
	for batch_start in (0..message_count).step_by(DRAIN_BATCH_SIZE) {
		let batch_end = (batch_start + DRAIN_BATCH_SIZE).min(message_count);

		for sequence in batch_start..batch_end {
			channel.send(divan::black_box(make_message(sequence)));
		}

		for listener in listeners.iter_mut() {
			for _ in batch_start..batch_end {
				let message = listener.read().expect("The bounded batch must be available");
				divan::black_box(message);
			}
		}
	}
}

/// Measures one producer and one consumer without producer contention.
#[divan::bench(sample_count = 10, sample_size = 1)]
fn single_producer_single_consumer(bencher: Bencher) {
	let channel = DefaultChannel::new();
	let mut listeners = [channel.listener()];

	bencher.counter(ItemsCount::new(SCALAR_MESSAGE_COUNT)).bench_local(|| {
		publish_and_drain(&channel, &mut listeners, SCALAR_MESSAGE_COUNT, |sequence| sequence as u64);
	});
}

/// Measures the application bus's default cell layout with one producer and consumer.
#[divan::bench(sample_count = 10, sample_size = 1)]
fn shared_bus_single_producer_single_consumer(bencher: Bencher) {
	let bus = MessageBus::default();
	let channel = bus.new_scope("benchmark").channel();
	let mut listeners = [channel.listener()];

	bencher.counter(ItemsCount::new(SCALAR_MESSAGE_COUNT)).bench_local(|| {
		publish_and_drain(&channel, &mut listeners, SCALAR_MESSAGE_COUNT, |sequence| sequence as u64);
	});
}

/// Measures enabled passive publication observation without including observer setup or teardown.
#[divan::bench(sample_count = 10, sample_size = 1)]
fn observed_shared_bus_single_producer_single_consumer(bencher: Bencher) {
	let bus = MessageBus::default();
	let _observer = bus.observe().expect("attach benchmark observer");
	let channel = bus.new_scope("benchmark").channel();
	let mut listeners = [channel.listener()];

	bencher.counter(ItemsCount::new(SCALAR_MESSAGE_COUNT)).bench_local(|| {
		publish_and_drain(&channel, &mut listeners, SCALAR_MESSAGE_COUNT, |sequence| sequence as u64);
	});
}

/// Measures handle generation, creation publication, and one consumer read.
#[divan::bench(sample_count = 10, sample_size = 1)]
fn factory_create_single_consumer(bencher: Bencher) {
	let factory = Factory::new();
	let mut listener = factory.listener();

	bencher.counter(ItemsCount::new(SCALAR_MESSAGE_COUNT)).bench_local(|| {
		for batch_start in (0..SCALAR_MESSAGE_COUNT).step_by(DRAIN_BATCH_SIZE) {
			let batch_end = batch_start + DRAIN_BATCH_SIZE;
			for sequence in batch_start..batch_end {
				divan::black_box(factory.create(divan::black_box(sequence as u64)));
			}
			for _ in batch_start..batch_end {
				let message = listener.read().expect("The bounded factory batch must be available");
				divan::black_box(message);
			}
		}
	});
}

/// Measures scoped factory acquisition storage with one creation consumer.
#[divan::bench(sample_count = 10, sample_size = 1)]
fn shared_bus_factory_create_single_consumer(bencher: Bencher) {
	let bus = MessageBus::default();
	let factory = bus.new_scope("benchmark").factory();
	let mut listener = factory.listener();

	bencher.counter(ItemsCount::new(SCALAR_MESSAGE_COUNT)).bench_local(|| {
		for batch_start in (0..SCALAR_MESSAGE_COUNT).step_by(DRAIN_BATCH_SIZE) {
			let batch_end = batch_start + DRAIN_BATCH_SIZE;
			for sequence in batch_start..batch_end {
				divan::black_box(factory.create(divan::black_box(sequence as u64)));
			}
			for _ in batch_start..batch_end {
				let message = listener.read().expect("The bounded factory batch must be available");
				divan::black_box(message);
			}
		}
	});
}

/// Measures enabled publication observation and semantic factory catalog updates.
#[divan::bench(sample_count = 10, sample_size = 1)]
fn observed_shared_bus_factory_create_single_consumer(bencher: Bencher) {
	bencher
		.counter(ItemsCount::new(OBSERVED_FACTORY_CREATE_COUNT))
		.with_inputs(|| {
			let bus = MessageBus::default();
			let observer = bus.observe().expect("attach benchmark observer");
			let factory = bus.new_scope("benchmark").factory();
			let listener = factory.listener();
			(factory, listener, observer)
		})
		.bench_local_values(|(factory, mut listener, observer)| {
			for batch_start in (0..OBSERVED_FACTORY_CREATE_COUNT).step_by(DRAIN_BATCH_SIZE) {
				let batch_end = batch_start + DRAIN_BATCH_SIZE;
				for sequence in batch_start..batch_end {
					divan::black_box(factory.create(divan::black_box(sequence as u64)));
				}
				for _ in batch_start..batch_end {
					let message = listener.read().expect("The bounded factory batch must be available");
					divan::black_box(message);
				}
			}
			// Return owned state so its entity catalog is destroyed outside the timed region.
			(factory, listener, observer)
		});
}

/// Measures one scalar publication delivered to every registered consumer.
#[divan::bench(args = [4, 16, 64], sample_count = 10, sample_size = 1)]
fn broadcast_fanout(bencher: Bencher, listener_count: usize) {
	let channel = DefaultChannel::new();
	let mut listeners: Vec<_> = (0..listener_count).map(|_| channel.listener()).collect();

	bencher.counter(ItemsCount::new(BROADCAST_MESSAGE_COUNT)).bench_local(|| {
		publish_and_drain(&channel, &mut listeners, BROADCAST_MESSAGE_COUNT, |sequence| sequence as u64);
	});
}

/// Measures application-bus publication delivered to every registered consumer.
#[divan::bench(args = [4, 16, 64], sample_count = 10, sample_size = 1)]
fn shared_bus_broadcast_fanout(bencher: Bencher, listener_count: usize) {
	let bus = MessageBus::default();
	let channel = bus.new_scope("benchmark").channel();
	let mut listeners: Vec<_> = (0..listener_count).map(|_| channel.listener()).collect();

	bencher.counter(ItemsCount::new(BROADCAST_MESSAGE_COUNT)).bench_local(|| {
		publish_and_drain(&channel, &mut listeners, BROADCAST_MESSAGE_COUNT, |sequence| sequence as u64);
	});
}

/// The `ContendedFixture` struct keeps producer and consumer threads alive across Divan samples.
struct ContendedFixture {
	start: Arc<Barrier>,
	finish: Arc<Barrier>,
	stop: Arc<AtomicBool>,
	workers: Vec<JoinHandle<()>>,
}

impl ContendedFixture {
	/// Starts one consumer and the requested number of producers on a shared channel.
	fn new(producer_count: usize) -> Self {
		let channel = DefaultChannel::new();
		let listener = channel.listener();
		let participant_count = producer_count + 2;
		let start = Arc::new(Barrier::new(participant_count));
		let finish = Arc::new(Barrier::new(participant_count));
		let stop = Arc::new(AtomicBool::new(false));
		let mut workers = Vec::with_capacity(producer_count + 1);

		workers.push(Self::spawn_consumer(
			listener,
			Arc::clone(&start),
			Arc::clone(&finish),
			Arc::clone(&stop),
		));

		let messages_per_producer = CONTENDED_MESSAGE_COUNT / producer_count;
		for producer_index in 0..producer_count {
			workers.push(Self::spawn_producer(
				channel.clone(),
				producer_index,
				messages_per_producer,
				Arc::clone(&start),
				Arc::clone(&finish),
				Arc::clone(&stop),
			));
		}

		Self {
			start,
			finish,
			stop,
			workers,
		}
	}

	/// Releases every worker for one measured publication and drain cycle.
	fn run(&self) {
		self.start.wait();
		self.finish.wait();
	}

	/// Runs the consumer until it has observed every message in the current cycle.
	fn spawn_consumer(
		mut listener: DefaultListener<u64>,
		start: Arc<Barrier>,
		finish: Arc<Barrier>,
		stop: Arc<AtomicBool>,
	) -> JoinHandle<()> {
		thread::spawn(move || {
			loop {
				start.wait();
				if stop.load(Ordering::Acquire) {
					break;
				}

				let mut received = 0;
				while received < CONTENDED_MESSAGE_COUNT {
					if let Some(message) = listener.read() {
						divan::black_box(message);
						received += 1;
					} else {
						std::hint::spin_loop();
					}
				}
				finish.wait();
			}
		})
	}

	/// Publishes one disjoint portion of the cycle's messages.
	fn spawn_producer(
		channel: DefaultChannel<u64>,
		producer_index: usize,
		message_count: usize,
		start: Arc<Barrier>,
		finish: Arc<Barrier>,
		stop: Arc<AtomicBool>,
	) -> JoinHandle<()> {
		thread::spawn(move || {
			loop {
				start.wait();
				if stop.load(Ordering::Acquire) {
					break;
				}

				for sequence in 0..message_count {
					let message = ((producer_index as u64) << 48) | sequence as u64;
					channel.send(divan::black_box(message));
				}
				finish.wait();
			}
		})
	}
}

impl Drop for ContendedFixture {
	fn drop(&mut self) {
		// Workers are waiting at `start` between samples, so one release is enough
		// to make each of them observe the stop flag and exit.
		self.stop.store(true, Ordering::Release);
		self.start.wait();
		for worker in self.workers.drain(..) {
			worker.join().expect("Channel benchmark worker must exit cleanly");
		}
	}
}

/// Measures producers contending for the same publication stream.
#[divan::bench(args = [2, 4, 8], sample_count = 10, sample_size = 1)]
fn contended_producers(bencher: Bencher, producer_count: usize) {
	assert_eq!(CONTENDED_MESSAGE_COUNT % producer_count, 0);
	let fixture = ContendedFixture::new(producer_count);

	bencher
		.counter(ItemsCount::new(CONTENDED_MESSAGE_COUNT))
		.bench_local(|| fixture.run());
}
