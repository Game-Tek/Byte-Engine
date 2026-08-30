//! Run with `cargo bench -p byte-engine --bench availability_graph`.

use divan::{Bencher, counter::ItemsCount};
use utils::{AvailabilityGraph, AvailabilityHandle};

fn main() {
	divan::main();
}

/// Creates `object_count` ready leaves backed by one shared resource.
fn fixture(object_count: usize) -> (AvailabilityGraph<u32>, AvailabilityHandle, Vec<AvailabilityHandle>) {
	let mut graph = AvailabilityGraph::with_capacity(object_count + 1, object_count);
	let resource = graph.get_or_insert(0, true);
	let objects = (1..=object_count as u32)
		.map(|key| {
			let object = graph.get_or_insert(key, true);
			graph.add_dependency(object, resource).unwrap();
			object
		})
		.collect();
	(graph, resource, objects)
}

#[divan::bench(args = [1, 128, 1024])]
fn cached_ready_reads(bencher: Bencher, object_count: usize) {
	let (graph, _, objects) = fixture(object_count);
	bencher.counter(ItemsCount::new(object_count)).bench_local(|| {
		for object in &objects {
			divan::black_box(graph.is_ready(*object));
		}
	});
}

#[divan::bench(args = [1, 128, 1024])]
fn shared_resource_transitions(bencher: Bencher, object_count: usize) {
	let (mut graph, resource, _) = fixture(object_count);
	bencher.counter(ItemsCount::new(object_count * 2)).bench_local(|| {
		graph.set_available(resource, false);
		graph.set_available(resource, true);
	});
}
