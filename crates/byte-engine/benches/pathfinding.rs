//! Run with `cargo bench -p byte-engine --bench pathfinding`.

use byte_engine::gameplay::pathfinding::{NavigationMesh, NavigationPortal, string_pull, string_pull_into};
use divan::{Bencher, counter::ItemsCount};
use math::Point;

fn main() {
	divan::main();
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

/// Measures a six-agent pursuit plan, including retained path storage and crowd steering.
#[divan::bench]
fn crowd_navigation(bencher: Bencher) {
	use byte_engine::{
		MediaTime,
		core::{
			factory::Creator as _,
			message_bus::{MessageBus, MessageBusConfig},
		},
		gameplay::{
			Transform,
			pathfinding::{Navigation, NavigationAgent, NavigationArea},
		},
		math::{Orientation, Scale, Vector},
	};
	let bus = MessageBus::new(MessageBusConfig::new(1 << 20)).unwrap();
	let world = bus.new_scope("benchmark");
	let mut navigation = Navigation::new(&world, &[][..] as &[byte_engine::application::Parameter]);
	bus.begin_tick();
	world.create(NavigationArea::walkable(20.0, 20.0)).with(Transform::identity());
	let agents: Vec<_> = (0..6)
		.map(|i| {
			let position = Point::new(-3.0 + i as f32, 0.0, 0.0);
			world
				.create(NavigationAgent::new(0.35))
				.with(Transform::new(position, Scale::identity(), Orientation::identity()))
				.handle()
		})
		.collect();
	navigation.update(MediaTime::from_seconds_f32(1.0 / 60.0)).unwrap();
	let mut path = Vec::new();
	bencher.counter(ItemsCount::new(6usize)).bench_local(|| {
		for &agent in &agents {
			divan::black_box(navigation.find_path(agent, Point::new(10.0, 0.0, 0.0), &mut path).unwrap());
			divan::black_box(navigation.steer(agent, Vector::new(1.4, 0.0, 0.0)));
		}
	});
}
