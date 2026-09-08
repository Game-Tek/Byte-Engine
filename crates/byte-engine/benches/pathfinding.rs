//! Run with `cargo bench -p byte-engine --bench pathfinding`.

use byte_engine::gameplay::pathfinding::{
	BitMatrixGraph, Graph, NavigationMesh, NavigationPortal, NodeHandle, PathSearch, TrivialGraph, a_star, a_star_batch,
	string_pull, string_pull_into,
};
use divan::{Bencher, counter::ItemsCount};
use math::Point;

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

/// The `Lattice` struct is a square four-neighbor grid, the shape a tile-based scene searches.
struct Lattice {
	side: usize,
}

impl Graph<()> for Lattice {
	fn node_count(&self) -> usize {
		self.side * self.side
	}

	fn neighbors(&self, node: NodeHandle) -> impl Iterator<Item = NodeHandle> + '_ {
		let side = self.side as NodeHandle;
		let (x, z) = (node % side, node / side);
		[
			(x > 0).then(|| node - 1),
			(x + 1 < side).then(|| node + 1),
			(z > 0).then(|| node - side),
			(z + 1 < side).then(|| node + side),
		]
		.into_iter()
		.flatten()
	}
}

impl Lattice {
	fn manhattan(&self, from: NodeHandle, to: NodeHandle) -> f32 {
		let side = self.side as NodeHandle;
		((from % side).abs_diff(to % side) + (from / side).abs_diff(to / side)) as f32
	}

	/// Spreads requests across the lattice so consecutive searches touch different cells.
	fn requests(&self, count: usize) -> Vec<(NodeHandle, NodeHandle)> {
		let nodes = self.node_count() as NodeHandle;
		(0..count as NodeHandle)
			.map(|index| ((index * 7919) % nodes, nodes - 1 - (index * 104_729) % nodes))
			.collect()
	}
}

/// Compares one search per request against sharing search storage across a request batch.
///
/// The lattice is the 32 by 32 grid of a small scene; requests grow like the actor count.
mod batch {
	use super::*;

	const SIDE: usize = 32;

	#[divan::bench(args = [1, 16, 128])]
	fn individual(bencher: Bencher, request_count: usize) {
		let lattice = Lattice { side: SIDE };
		let requests = lattice.requests(request_count);
		bencher.counter(ItemsCount::new(request_count)).bench_local(|| {
			for &(start, target) in &requests {
				divan::black_box(a_star(start, target, &lattice, |from, to| lattice.manhattan(from, to)));
			}
		});
	}

	#[divan::bench(args = [1, 16, 128])]
	fn batched(bencher: Bencher, request_count: usize) {
		let lattice = Lattice { side: SIDE };
		let requests = lattice.requests(request_count);
		bencher.counter(ItemsCount::new(request_count)).bench_local(|| {
			a_star_batch(
				requests.iter().copied(),
				&lattice,
				|from, to| lattice.manhattan(from, to),
				|_, path| {
					divan::black_box(path);
				},
			);
		});
	}

	/// A system that searches every tick keeps one search alive, so nothing allocates per batch either.
	#[divan::bench(args = [1, 16, 128])]
	fn retained(bencher: Bencher, request_count: usize) {
		let lattice = Lattice { side: SIDE };
		let requests = lattice.requests(request_count);
		let mut search = PathSearch::new();
		let mut path = Vec::new();
		bencher.counter(ItemsCount::new(request_count)).bench_local(|| {
			for &(start, target) in &requests {
				search.find_into(start, target, &lattice, |from, to| lattice.manhattan(from, to), &mut path);
				divan::black_box(&path);
			}
		});
	}
}

/// Measures string pulling through a straight corridor without fixture construction.
#[divan::bench(args = [16, 128, 1024])]
fn funnel(bencher: Bencher, portal_count: usize) {
	let (portals, start, target) = straight_corridor(portal_count);

	bencher.counter(ItemsCount::new(portal_count)).bench_local(|| {
		divan::black_box(string_pull(start, target, &portals).unwrap());
	});
}

/// Measures straight-corridor string pulling with reusable output storage.
#[divan::bench(args = [16, 128, 1024])]
fn funnel_reused(bencher: Bencher, portal_count: usize) {
	let (portals, start, target) = straight_corridor(portal_count);
	let mut path = Vec::with_capacity(2);

	bencher.counter(ItemsCount::new(portal_count)).bench_local(|| {
		divan::black_box(string_pull_into(start, target, &portals, &mut path).unwrap());
	});
}

/// Measures string pulling through forced alternating corners without fixture construction.
#[divan::bench(args = [16, 128, 1024])]
fn funnel_corners(bencher: Bencher, portal_count: usize) {
	let (portals, start, target) = corner_corridor(portal_count);

	bencher.counter(ItemsCount::new(portal_count)).bench_local(|| {
		divan::black_box(string_pull(start, target, &portals).unwrap());
	});
}

/// Measures forced-corner string pulling with reusable output storage.
#[divan::bench(args = [16, 128, 1024])]
fn funnel_corners_reused(bencher: Bencher, portal_count: usize) {
	let (portals, start, target) = corner_corridor(portal_count);
	let mut path = Vec::with_capacity(portal_count + 2);

	bencher.counter(ItemsCount::new(portal_count)).bench_local(|| {
		divan::black_box(string_pull_into(start, target, &portals, &mut path).unwrap());
	});
}

/// Builds a corridor whose portals do not constrain the direct path.
fn straight_corridor(portal_count: usize) -> (Vec<NavigationPortal>, Point, Point) {
	let portals = (1..=portal_count)
		.map(|x| {
			let x = x as f32;
			NavigationPortal::new(Point::new(x, 0.0, 1.0), Point::new(x, 0.0, -1.0))
		})
		.collect();
	(portals, Point::origin(), Point::new(portal_count as f32 + 1.0, 0.0, 0.0))
}

/// Builds a corridor whose zero-width portals force every alternating corner.
fn corner_corridor(portal_count: usize) -> (Vec<NavigationPortal>, Point, Point) {
	let portals = (1..=portal_count)
		.map(|x| {
			let point = Point::new(x as f32, 0.0, if x % 2 == 0 { 1.0 } else { -1.0 });
			NavigationPortal::new(point, point)
		})
		.collect();
	(portals, Point::origin(), Point::new(portal_count as f32 + 1.0, 0.0, 0.0))
}

/// Builds a regular quad navigation mesh with shared indexed edges.
fn navigation_grid(side: usize) -> NavigationMesh {
	let vertices = (0..=side)
		.flat_map(|z| (0..=side).map(move |x| Point::new(x as f32, 0.0, z as f32)))
		.collect();
	let vertex = |x: usize, z: usize| (z * (side + 1) + x) as u32;
	let polygons = (0..side)
		.flat_map(|z| (0..side).map(move |x| vec![vertex(x, z), vertex(x + 1, z), vertex(x + 1, z + 1), vertex(x, z + 1)]))
		.collect();
	NavigationMesh::new(vertices, polygons).unwrap()
}

/// Measures endpoint location, A*, and funneling across a square navigation grid.
#[divan::bench(args = [4, 16, 32])]
fn navigation_mesh_path(bencher: Bencher, side: usize) {
	let mesh = navigation_grid(side);
	let start = Point::new(0.25, 0.0, 0.25);
	let target = Point::new(side as f32 - 0.25, 0.0, side as f32 - 0.25);

	bencher.counter(ItemsCount::new(side * side)).bench_local(|| {
		divan::black_box(mesh.find_path(start, target).unwrap());
	});
}
