//! Run with `cargo bench -p byte-engine --bench physics`.
//!
//! Every benchmark drops a block of bodies on a floor, lets them pile up, and then times steps of the pile. Scenes
//! differ in the number of bodies and in their shapes, because boxes and spheres take different paths through
//! contact detection.
//!
//! Each sample times 30 steps, so the item rate is steps per second and a sample's time divided by 30 is one step.
//! Compare the fastest samples of two or three runs: the system sometimes runs a whole row on its slower cores,
//! which more than doubles that row's times.

use byte_engine::{
	application::Time,
	core::{
		alley::Alley,
		channel::{Channel as _, DefaultChannel},
		factory::Factory,
		listener::{DefaultListener, Listener as _},
		message::DeleteMessage,
	},
	gameplay::transform::{Transform, TransformationUpdate},
	physics::{Body, BodyTypes, DynabitWorld, LocalSpace, Shapes},
	time::MediaTime,
};
use divan::{Bencher, counter::ItemsCount};
use math::{Point, Vector};

/// The steps a scene runs before it is timed, which is long enough for most of the bodies to land.
const SETTLING_STEPS: usize = 120;
/// The steps timed in each sample.
const TIMED_STEPS: usize = 30;
/// The distance between neighboring spawn positions, which leaves a gap between one-meter bodies.
const SPACING: f32 = 1.5;
/// The height of the lowest layer of bodies above the floor.
const FIRST_LAYER_HEIGHT: f32 = 2.0;

fn main() {
	divan::main();
}

/// The `Mix` trait selects which bodies of a scene are spheres.
trait Mix {
	/// Returns whether the body at `index` is a sphere instead of a box.
	fn is_sphere(index: usize) -> bool;
}

/// Every body is a box.
struct Boxes;
/// Every other body is a sphere.
struct Mixed;
/// Every body is a sphere.
struct Spheres;

impl Mix for Boxes {
	fn is_sphere(_: usize) -> bool {
		false
	}
}

impl Mix for Mixed {
	fn is_sphere(index: usize) -> bool {
		index.is_multiple_of(2)
	}
}

impl Mix for Spheres {
	fn is_sphere(_: usize) -> bool {
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
	/// world with `step` until most of them have landed.
	fn settled<M: Mix>(bodies: usize, mut step: impl FnMut(&mut Self, Time)) -> Self {
		let body_factory = Factory::<Body>::new();
		let handles = Factory::<()>::new();
		let deletes = DefaultChannel::<DeleteMessage>::new();
		let authored = DefaultChannel::<TransformationUpdate>::new();
		let published = DefaultChannel::<TransformationUpdate>::new();
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
			let shape = if M::is_sphere(index) {
				Shapes::sphere(0.5)
			} else {
				Shapes::cube(Vector::new(0.5, 0.5, 0.5))
			};
			let handle = handles.create(());
			body_factory.derive(handle, Body::new(BodyTypes::Dynamic, shape));
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
#[divan::bench(types = [Boxes, Mixed, Spheres], args = [1_000, 4_000, 16_000], sample_count = 8, sample_size = 1)]
fn serial<M: Mix>(bencher: Bencher, bodies: usize) {
	let step = |scene: &mut Scene, time: Time| scene.world.update(time, &mut scene.authored, &scene.published);

	bencher
		.counter(ItemsCount::new(TIMED_STEPS))
		.with_inputs(|| Scene::settled::<M>(bodies, step))
		.bench_local_values(|mut scene| {
			scene.run(TIMED_STEPS, step);
			scene
		});
}

/// Measures a step of a pile on an alley with one lane for each hardware thread, as an application runs it.
#[divan::bench(types = [Boxes, Mixed, Spheres], args = [1_000, 4_000, 16_000], sample_count = 8, sample_size = 1)]
fn alley<M: Mix>(bencher: Bencher, bodies: usize) {
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
