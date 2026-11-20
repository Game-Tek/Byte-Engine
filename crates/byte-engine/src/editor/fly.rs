//! How the editor's own camera flies: the model viewer's movement, where held keys push the camera along its view and
//! it glides to a stop once they're released, a drag of the pointer turns it, a scroll sets how fast it flies, and an
//! Alt+scroll how wide it sees.
//!
//! These are pure functions of the camera's state and its input; the editor feeds them the keys and the drag, and
//! sends the transform [`Fly::step`] returns to the camera it switched the game window to.

use std::f32::consts::{FRAC_PI_2, PI};

use crate::constants::up;
use crate::gameplay::Transform;
use crate::math::{Point, Scale, UnitVector, Vector, direction_from_orientation};
use math::Degrees;

/// How fast held keys speed the camera up, in meters per second squared.
const ACCELERATION: f32 = 100.0;
/// How much one scroll step speeds the camera up, as a factor of its speed.
const SPEED_STEP: f32 = 1.25;
/// The slowest and fastest the camera flies, as factors of its starting speed.
const SPEED_RANGE: [f32; 2] = [1.0 / 64.0, 64.0];
/// How much one Alt+scroll step narrows or widens the camera's vertical field of view, in degrees.
const FOV_STEP: f32 = 5.0;
/// The narrowest and widest vertical field of view an Alt+scroll sets, in degrees.
const FOV_RANGE: [f32; 2] = [10.0, 120.0];
/// The share of its speed the camera keeps after a second without keys, so it glides to a stop at any frame rate.
const DAMPING: f32 = 0.01;
/// How far short of straight up or down a camera can look, in radians, so its view keeps a direction to call right.
const PITCH_MARGIN: f32 = 0.01;
/// The steepest a camera can look up or down, in radians.
const PITCH_LIMIT: f32 = FRAC_PI_2 - PITCH_MARGIN;

/// The `Fly` struct is the editor camera's place and motion while it's flown, so each frame moves it on from the last.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Fly {
	position: Point,
	/// The view's turn about the vertical, from +Z toward +X, in radians.
	yaw: f32,
	/// The view's tilt above the horizon, in radians.
	pitch: f32,
	/// Meters per second, in the world.
	velocity: Vector,
}

impl Fly {
	/// Starts flying from where `transform` places a camera, at rest, so the view doesn't jump when the editor's camera
	/// takes over from the game's.
	pub(crate) fn from_transform(transform: &Transform) -> Self {
		let direction = direction_from_orientation(transform.get_orientation());
		Self {
			position: transform.get_position(),
			yaw: direction.x().atan2(direction.z()),
			pitch: direction.y().clamp(-1.0, 1.0).asin().clamp(-PITCH_LIMIT, PITCH_LIMIT),
			velocity: Vector::new(0.0, 0.0, 0.0),
		}
	}

	/// Turns the view by a pointer drag of `delta`, in normalized window coordinates: a drag across the whole window
	/// turns it a full circle, and one from bottom to top looks from straight down to straight up, as the model
	/// viewer's pointer does.
	pub(crate) fn turn(&mut self, delta: [f32; 2]) {
		self.yaw += delta[0] * PI;
		self.pitch = (self.pitch + delta[1] * FRAC_PI_2).clamp(-PITCH_LIMIT, PITCH_LIMIT);
	}

	/// Moves the camera on by `dt` seconds with `thrust` held, right, up, and forward along its view, each from -1 to
	/// 1, pushing `speed` times as hard as at the starting speed, and returns where it now is.
	pub(crate) fn step(&mut self, thrust: [f32; 3], speed: f32, dt: f32) -> Transform {
		let forward = heading(self.yaw, self.pitch);
		// Up across a view that tilts is shorter than a unit, so it's scaled back to one, and keys push alike at any tilt.
		let right = up()
			.cross(forward.into_vector())
			.normalized()
			.expect("The camera stops short of looking straight up or down.")
			.into_vector();
		let up = forward.cross(right);
		let push = thrust[0] * right + thrust[1] * up + thrust[2] * forward.into_vector();
		self.velocity += push * ACCELERATION * speed * dt;
		self.velocity = self.velocity * DAMPING.powf(dt);
		self.position += self.velocity * dt;
		self.transform()
	}

	/// Moves the camera by `offset` without turning it, as when it follows an entity a gizmo drag carries, and returns
	/// where it now is.
	pub(crate) fn carry(&mut self, offset: Vector) -> Transform {
		self.position += offset;
		self.transform()
	}

	/// Where the camera is, looking along its view.
	pub(crate) fn transform(&self) -> Transform {
		Transform::new(self.position, Scale::identity(), heading(self.yaw, self.pitch).into())
	}

	/// Whether the camera is still gliding, so the editor knows to keep moving it.
	pub(crate) fn moving(&self) -> bool {
		self.velocity.dot(self.velocity) > 1e-6
	}
}

/// The speed one scroll of `delta` from `speed` sets: faster for a scroll up, `delta` above zero, or slower for a scroll
/// down, one step per scroll, so a trackpad and a wheel change it alike. The editor keeps the speed apart from any one
/// flight, and passes it to [`Fly::step`].
pub(crate) fn throttle(speed: f32, delta: f32) -> f32 {
	let factor = if delta > 0.0 { SPEED_STEP } else { 1.0 / SPEED_STEP };
	(speed * factor).clamp(SPEED_RANGE[0], SPEED_RANGE[1])
}

/// The vertical field of view one Alt+scroll of `delta` from `fov` sets: narrower for a scroll up, `delta` above zero,
/// so the view zooms in as a scroll over a texture does, or wider for a scroll down, one step per scroll.
pub(crate) fn widen(fov: Degrees, delta: f32) -> Degrees {
	let step = if delta > 0.0 { -FOV_STEP } else { FOV_STEP };
	Degrees::new((fov.value() + step).clamp(FOV_RANGE[0], FOV_RANGE[1]))
}

/// The direction a camera turned `yaw` about the vertical, from +Z toward +X, and tilted `pitch` above the horizon
/// looks along, both in radians.
pub(crate) fn heading(yaw: f32, pitch: f32) -> UnitVector {
	Vector::new(yaw.sin() * pitch.cos(), pitch.sin(), yaw.cos() * pitch.cos())
		.normalized()
		.expect("A direction from angles has unit length.")
}

#[cfg(test)]
mod tests {
	use super::*;
	use crate::math::orientation_from_direction;

	fn looking(direction: Vector) -> Transform {
		let direction = direction.normalized().unwrap();
		Transform::new(
			Point::new(1.0, 2.0, 3.0),
			Scale::identity(),
			orientation_from_direction(direction),
		)
	}

	fn view(transform: &Transform) -> Vector {
		direction_from_orientation(transform.get_orientation()).into_vector()
	}

	#[test]
	fn flying_starts_where_the_camera_stands_and_looks() {
		let start = looking(Vector::new(1.0, 0.5, -1.0));
		let fly = Fly::from_transform(&start);
		let shown = fly.transform();
		assert!((shown.get_position() - start.get_position()).length() < 1e-5);
		assert!((view(&shown) - view(&start)).length() < 1e-4);
	}

	#[test]
	fn held_keys_push_along_the_view_and_the_camera_glides_to_a_stop() {
		let mut fly = Fly::from_transform(&looking(Vector::new(1.0, 0.0, 0.0)));
		let start = fly.transform().get_position();
		for _ in 0..10 {
			fly.step([0.0, 0.0, 1.0], 1.0, 1.0 / 60.0);
		}
		let pushed = fly.transform().get_position() - start;
		assert!(
			pushed.x() > 0.0 && pushed.y().abs() < 1e-4 && pushed.z().abs() < 1e-4,
			"forward is the view: {pushed:?}"
		);
		for _ in 0..600 {
			fly.step([0.0; 3], 1.0, 1.0 / 60.0);
		}
		assert!(!fly.moving(), "released keys let the camera stop");
	}

	#[test]
	fn a_scroll_up_flies_faster_and_a_scroll_down_slower() {
		let distance = |scrolls: f32| {
			let speed = (0..scrolls.abs() as usize).fold(1.0, |speed, _| throttle(speed, scrolls));
			let mut fly = Fly::from_transform(&looking(Vector::new(0.0, 0.0, 1.0)));
			let start = fly.transform().get_position();
			fly.step([0.0, 0.0, 1.0], speed, 0.1);
			(fly.transform().get_position() - start).length()
		};
		assert!(distance(3.0) > distance(0.0) && distance(0.0) > distance(-3.0));
		assert_eq!(distance(1000.0), distance(100.0), "the speed stops at its fastest");
	}

	#[test]
	fn an_alt_scroll_up_narrows_the_view_and_down_widens_it_within_its_range() {
		let fov = Degrees::new(45.0);
		assert!(widen(fov, 1.0) < fov && widen(fov, -1.0) > fov);
		let widest = (0..100).fold(fov, |fov, _| widen(fov, -1.0));
		assert_eq!(widest, Degrees::new(FOV_RANGE[1]));
	}

	#[test]
	fn a_drag_turns_the_view_and_stops_short_of_straight_up() {
		let mut fly = Fly::from_transform(&looking(Vector::new(0.0, 0.0, 1.0)));
		fly.turn([0.5, 0.0]);
		let turned = view(&fly.transform());
		assert!(
			(turned - Vector::new(1.0, 0.0, 0.0)).length() < 1e-4,
			"half a window turns a quarter: {turned:?}"
		);
		fly.turn([0.0, 5.0]);
		let up = view(&fly.transform());
		assert!(up.y() < 1.0 && up.y() > 0.99, "the view stops short of straight up: {up:?}");
	}
}
