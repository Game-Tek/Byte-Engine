//! Run with `cargo bench -p byte-engine --bench physics`.
//!
//! Each benchmark drops a block of bodies on a floor, lets them pile up, and then times steps of the pile. The pile
//! mixes spheres, boxes, capsules, boxes that rotate, and convex hulls, because each takes a different path through
//! contact detection.
//!
//! There are two piles. The one of 128 bodies shows what a step costs whatever its size, and how a small world
//! steps: it runs on the calling thread. The one of 65,536 bodies shows what a step costs as a world grows, on as
//! many lanes as the machine's default gives it.
//!
//! Each sample times 30 steps, so the item rate is steps per second and a sample's time divided by 30 is one step.
//! Compare the fastest samples of two or three runs: the system sometimes runs a whole row on its slower cores,
//! which more than doubles that row's times.
//!
//! A run takes about half a minute on an Apple M4.

use byte_engine::{
	application::Time,
	core::{
		alley::Alley,
		channel::{Channel as _, DefaultChannel},
		factory::Factory,
		listener::{DefaultListener, Listener as _},
		message::DeleteMessage,
		message_bus::{MessageBus, MessageBusConfig},
	},
	gameplay::transform::{Transform, TransformationUpdate},
	physics::{Body, BodyTypes, Constraint, ConvexHull, DynabitWorld, LocalSpace, Shapes, build_convex_hull},
	time::MediaTime,
};
use divan::{Bencher, counter::ItemsCount};
use math::{Point, Vector};

/// The steps a scene runs before it is timed, which is long enough for the top of the largest block to land: its
/// 40 layers reach 60 meters up, and a body falls that far in about 165 steps.
const SETTLING_STEPS: usize = 180;
/// The steps timed in each sample.
const TIMED_STEPS: usize = 30;
/// The distance between neighboring spawn positions, which leaves a gap between one-meter bodies.
const SPACING: f32 = 1.5;
/// The height of the lowest layer of bodies above the floor.
const FIRST_LAYER_HEIGHT: f32 = 2.0;
/// The bytes of messages a scene can hold at once. A scene publishes every body and where it starts before its
/// world reads any of them, which for the largest scene is more than a standalone channel's pool holds.
const MESSAGE_CAPACITY: usize = 64 * 1024 * 1024;

fn main() {
	divan::main();
}

/// Returns the shape of the body at `index`, which fits in a one-meter cube, and whether the body rotates.
///
/// Of every sixteen bodies, six are spheres, four are boxes, four are capsules, one is a box that rotates, and one is
/// a six-sided prism that rotates and collides as a convex hull. Spheres rotate too, which lets them roll. A hull
/// costs much more to test than the other shapes, so a larger share of them would leave little else to measure.
fn body(index: usize) -> (Shapes, bool) {
	let cube = || Shapes::cube(Vector::new(0.5, 0.5, 0.5));
	match index % 16 {
		0 | 1 | 2 | 8 | 9 | 10 => (Shapes::sphere(0.5), true),
		3 | 4 | 11 | 12 => (cube(), false),
		5 | 6 | 13 | 14 => (Shapes::capsule(0.25, 1.0, Point::origin()), false),
		7 => (cube(), true),
		_ => (Shapes::convex_hull(prism()), true),
	}
}

/// Returns the hull every prism in a scene shares, as bodies that collide as the same mesh would.
fn prism() -> std::sync::Arc<ConvexHull> {
	static PRISM: std::sync::OnceLock<std::sync::Arc<ConvexHull>> = std::sync::OnceLock::new();
	let prism = PRISM.get_or_init(|| {
		let corners: Vec<Point<LocalSpace>> = (0..6)
			.flat_map(|side| {
				let turn = side as f32 / 6.0 * std::f32::consts::TAU;
				[-0.5, 0.5].map(|height| Point::new(0.5 * turn.cos(), height, 0.5 * turn.sin()))
			})
			.collect();
		std::sync::Arc::new(build_convex_hull(&corners).expect("the corners of a prism enclose a volume"))
	});
	prism.clone()
}

/// The `Scene` struct holds a physics world with the channels a step reads from and publishes to.
struct Scene {
	world: DynabitWorld,
	authored: DefaultListener<TransformationUpdate>,
	published: DefaultChannel<TransformationUpdate>,
	published_reader: DefaultListener<TransformationUpdate>,
	elapsed: MediaTime,
}

impl Scene {
	/// Creates a floor with a block of `bodies` one-meter bodies above it, as wide as it is tall, and steps the
	/// world with `step` until they have landed.
	fn settled(bodies: usize, mut step: impl FnMut(&mut Self, Time)) -> Self {
		// One pool for every channel, without ticks, so its storage is free again once the world has read it.
		let messages = MessageBus::new(MessageBusConfig {
			capacity: MESSAGE_CAPACITY,
			ticks: false,
		})
		.expect("the capacity holds more than two chunks");
		let authoring = messages.new_scope("authoring");
		let body_factory: Factory<Body> = authoring.factory();
		let handles: Factory<()> = authoring.factory();
		let deletes: DefaultChannel<DeleteMessage> = authoring.channel();
		let authored: DefaultChannel<TransformationUpdate> = authoring.channel();
		// A scope of its own keeps what the world publishes apart from what it reads, which is the same message type.
		let published: DefaultChannel<TransformationUpdate> = messages.new_scope("simulation").channel();
		let mut scene = Self {
			world: DynabitWorld::new(
				body_factory.listener(),
				authoring.factory::<Constraint>().listener(),
				deletes.listener(),
			),
			authored: authored.listener(),
			published_reader: published.listener(),
			published,
			elapsed: MediaTime::ZERO,
		};

		let floor = handles.create(());
		body_factory.derive(
			floor,
			Body::new(BodyTypes::Static, Shapes::cube(Vector::<LocalSpace>::new(200.0, 0.5, 200.0))),
		);
		authored.send(TransformationUpdate::new(
			floor,
			Transform::from_position(Point::new(0.0, -0.5, 0.0)),
		));

		let side = (bodies as f32).cbrt().ceil() as usize;
		let reach = (side - 1) as f32 * SPACING * 0.5;
		for index in 0..bodies {
			let (column, row, layer) = (index % side, index / side % side, index / (side * side));
			// A cheap integer hash nudges each body sideways, so columns topple instead of stacking exactly.
			let hash = (index as u32).wrapping_mul(0x9E37_79B9);
			let nudge = |shift: u32| (((hash >> shift) & 0xFF) as f32 / 255.0 - 0.5) * 0.2;
			let position = Point::new(
				column as f32 * SPACING - reach + nudge(8),
				FIRST_LAYER_HEIGHT + layer as f32 * SPACING,
				row as f32 * SPACING - reach + nudge(16),
			);
			let handle = handles.create(());
			let (shape, rotates) = body(index);
			body_factory.derive(handle, Body::new(BodyTypes::Dynamic, shape).with_rotation(rotates));
			authored.send(TransformationUpdate::new(handle, Transform::from_position(position)));
		}

		scene.run(SETTLING_STEPS, &mut step);
		scene
	}

	/// Advances the scene by `steps` sixtieths of a second and reads what each step published, as a renderer would.
	fn run(&mut self, steps: usize, mut step: impl FnMut(&mut Self, Time)) {
		let delta = MediaTime::from_seconds_f32(1.0 / 60.0);
		for _ in 0..steps {
			self.elapsed += delta;
			step(self, Time::new(self.elapsed, delta));
			while self.published_reader.read().is_some() {}
		}
	}
}

/// Measures a step of a small pile, which shows the cost a step has whatever its size.
#[divan::bench(args = [128], sample_count = 64, sample_size = 1)]
fn small(bencher: Bencher, bodies: usize) {
	time(bencher, bodies);
}

/// Measures a step of a large pile, which shows the cost a step has for each body and contact.
#[divan::bench(args = [65_536], sample_count = 4, sample_size = 1)]
fn large(bencher: Bencher, bodies: usize) {
	time(bencher, bodies);
}

/// Times steps of a settled pile of `bodies` bodies on an alley with one lane for each hardware thread, as an
/// application runs them.
fn time(bencher: Bencher, bodies: usize) {
	let lanes = std::cell::RefCell::new(Alley::new());
	let step = |scene: &mut Scene, time: Time| {
		scene
			.world
			.update(time, &mut scene.authored, &scene.published, &mut lanes.borrow_mut());
	};

	bencher
		.counter(ItemsCount::new(TIMED_STEPS))
		.with_inputs(|| Scene::settled(bodies, step))
		.bench_local_values(|mut scene| {
			scene.run(TIMED_STEPS, step);
			scene
		});
}
