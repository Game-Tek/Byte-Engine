//! Run with `cargo bench -p byte-engine --bench physics`.
//!
//! Every benchmark drops a block of bodies on a floor, lets them pile up, and then times steps of the pile. Scenes
//! differ in the number of bodies and in their shapes, because boxes, spheres, and capsules take different paths
//! through contact detection. Boxes and capsules that rotate take another one again, which costs much more for each
//! pair, so their scenes are smaller.
//!
//! Each sample times 30 steps, so the item rate is steps per second and a sample's time divided by 30 is one step.
//! Compare the fastest samples of two or three runs: the system sometimes runs a whole row on its slower cores,
//! which more than doubles that row's times. The largest scenes are the ones to compare two builds by: on an Apple
//! M4 their times repeat to within about 1%, where a scene of 1,000 bodies varied by up to 4%.
//!
//! A run takes about six minutes on an Apple M4. Add `-- --sample-count 4` to halve that.

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
	physics::{Body, BodyTypes, DynabitWorld, LocalSpace, Shapes},
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

/// The `Mix` trait selects the shapes of a scene's bodies.
trait Mix {
	/// Returns the shape of the body at `index`, which fits in a one-meter cube.
	fn shape(index: usize) -> Shapes;

	/// Returns whether the body at `index` rotates. Spheres do, which lets them roll. Boxes and capsules that do not
	/// are tested along the axes, whatever happens to them.
	fn rotates(index: usize) -> bool {
		matches!(Self::shape(index), Shapes::Sphere { .. })
	}
}

/// Every body is a box.
struct Boxes;
/// Every other body is a sphere, and the rest are boxes.
struct Mixed;
/// Every body is a sphere.
struct Spheres;
/// Every body is a capsule.
struct Capsules;
/// Every body is a box that rotates.
struct TurningBoxes;

impl Mix for Boxes {
	fn shape(_: usize) -> Shapes {
		Shapes::cube(Vector::new(0.5, 0.5, 0.5))
	}
}

impl Mix for Mixed {
	fn shape(index: usize) -> Shapes {
		if index.is_multiple_of(2) {
			Spheres::shape(index)
		} else {
			Boxes::shape(index)
		}
	}
}

impl Mix for Spheres {
	fn shape(_: usize) -> Shapes {
		Shapes::sphere(0.5)
	}
}

impl Mix for Capsules {
	fn shape(_: usize) -> Shapes {
		Shapes::capsule(0.25, 1.0, Point::origin())
	}
}

impl Mix for TurningBoxes {
	fn shape(index: usize) -> Shapes {
		Boxes::shape(index)
	}

	fn rotates(_: usize) -> bool {
		true
	}
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
	fn settled<M: Mix>(bodies: usize, mut step: impl FnMut(&mut Self, Time)) -> Self {
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
			world: DynabitWorld::new(body_factory.listener(), deletes.listener()),
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
			body_factory.derive(
				handle,
				Body::new(BodyTypes::Dynamic, M::shape(index)).with_rotation(M::rotates(index)),
			);
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

/// Measures a step of a pile on the calling thread.
#[divan::bench(types = [Boxes, Mixed, Spheres, Capsules], args = [4_000, 16_000, 64_000], sample_count = 8, sample_size = 1)]
fn serial<M: Mix>(bencher: Bencher, bodies: usize) {
	time_serial::<M>(bencher, bodies);
}

/// Measures a step of a pile of bodies that rotate on the calling thread.
#[divan::bench(types = [TurningBoxes], args = [1_000, 4_000], sample_count = 8, sample_size = 1)]
fn serial_turning<M: Mix>(bencher: Bencher, bodies: usize) {
	time_serial::<M>(bencher, bodies);
}

/// Measures a step of a pile on an alley with one lane for each hardware thread, as an application runs it.
#[divan::bench(types = [Boxes, Mixed, Spheres, Capsules], args = [4_000, 16_000, 64_000], sample_count = 8, sample_size = 1)]
fn alley<M: Mix>(bencher: Bencher, bodies: usize) {
	time_alley::<M>(bencher, bodies);
}

/// Measures a step of a pile of bodies that rotate on an alley.
#[divan::bench(types = [TurningBoxes], args = [1_000, 4_000], sample_count = 8, sample_size = 1)]
fn alley_turning<M: Mix>(bencher: Bencher, bodies: usize) {
	time_alley::<M>(bencher, bodies);
}

fn time_serial<M: Mix>(bencher: Bencher, bodies: usize) {
	let step = |scene: &mut Scene, time: Time| scene.world.update(time, &mut scene.authored, &scene.published);

	bencher
		.counter(ItemsCount::new(TIMED_STEPS))
		.with_inputs(|| Scene::settled::<M>(bodies, step))
		.bench_local_values(|mut scene| {
			scene.run(TIMED_STEPS, step);
			scene
		});
}

fn time_alley<M: Mix>(bencher: Bencher, bodies: usize) {
	let lanes = std::cell::RefCell::new(Alley::new());
	let step = |scene: &mut Scene, time: Time| {
		scene
			.world
			.update_with_alley(time, &mut scene.authored, &scene.published, &mut lanes.borrow_mut());
	};

	bencher
		.counter(ItemsCount::new(TIMED_STEPS))
		.with_inputs(|| Scene::settled::<M>(bodies, step))
		.bench_local_values(|mut scene| {
			scene.run(TIMED_STEPS, step);
			scene
		});
}
