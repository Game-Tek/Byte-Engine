//! Run with `cargo bench -p byte-engine --bench pathfinding`.

use byte_engine::gameplay::pathfinding::{a_star, BitMatrixGraph, NodeHandle, TrivialGraph};
use divan::{counter::ItemsCount, Bencher};

fn main() {
	divan::main();
}

/// Builds a sparse edge-list graph with a ring and fixed-distance shortcuts.
fn sparse_trivial(node_count: usize) -> TrivialGraph<()> {
	let mut graph = TrivialGraph::new();
	for _ in 0..node_count {
		graph.push(());
	}
	connect_sparse(node_count, |edge| graph.connect(edge));
	graph
}

/// Builds a sparse bit-matrix graph with the same edges as [`sparse_trivial`].
fn sparse_bit_matrix(node_count: usize) -> BitMatrixGraph<()> {
	let mut graph = BitMatrixGraph::with_capacity(node_count);
	for _ in 0..node_count {
		graph.push(());
	}
	connect_sparse(node_count, |edge| graph.connect(edge));
	graph
}

/// Adds a connected ring and one longer-range edge per node.
fn connect_sparse(node_count: usize, mut connect: impl FnMut((NodeHandle, NodeHandle))) {
	for node in 0..node_count as NodeHandle {
		connect((node, (node + 1) % node_count as NodeHandle));
		connect((node, (node + 17) % node_count as NodeHandle));
	}
}

/// Builds a complete edge-list graph.
fn dense_trivial(node_count: usize) -> TrivialGraph<()> {
	let mut graph = TrivialGraph::new();
	for _ in 0..node_count {
		graph.push(());
	}
	connect_dense(node_count, |edge| graph.connect(edge));
	graph
}

/// Builds a complete bit-matrix graph.
fn dense_bit_matrix(node_count: usize) -> BitMatrixGraph<()> {
	let mut graph = BitMatrixGraph::with_capacity(node_count);
	for _ in 0..node_count {
		graph.push(());
	}
	connect_dense(node_count, |edge| graph.connect(edge));
	graph
}

/// Connects every distinct node pair once.
fn connect_dense(node_count: usize, mut connect: impl FnMut((NodeHandle, NodeHandle))) {
	for a in 0..node_count as NodeHandle {
		for b in a + 1..node_count as NodeHandle {
			connect((a, b));
		}
	}
}

/// Measures one zero-heuristic search without including fixture construction.
fn benchmark<T>(bencher: Bencher, graph: &impl byte_engine::gameplay::pathfinding::Graph<T>, node_count: usize) {
	let target = node_count as NodeHandle - 1;
	bencher.counter(ItemsCount::new(node_count)).bench_local(|| {
		divan::black_box(a_star(0, target, graph, |_, _| 1f32));
	});
}

mod sparse {
	use super::*;

	#[divan::bench(args = [128, 1024, 4096])]
	fn trivial(bencher: Bencher, node_count: usize) {
		let graph = sparse_trivial(node_count);
		benchmark(bencher, &graph, node_count);
	}

	#[divan::bench(args = [128, 1024, 4096])]
	fn bit_matrix(bencher: Bencher, node_count: usize) {
		let graph = sparse_bit_matrix(node_count);
		benchmark(bencher, &graph, node_count);
	}
}

mod dense {
	use super::*;

	#[divan::bench(args = [64, 256, 512])]
	fn trivial(bencher: Bencher, node_count: usize) {
		let graph = dense_trivial(node_count);
		benchmark(bencher, &graph, node_count);
	}

	#[divan::bench(args = [64, 256, 512])]
	fn bit_matrix(bencher: Bencher, node_count: usize) {
		let graph = dense_bit_matrix(node_count);
		benchmark(bencher, &graph, node_count);
	}
}
