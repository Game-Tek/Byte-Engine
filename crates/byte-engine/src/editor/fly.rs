//! How the editor's own camera flies: the model viewer's movement, where held keys push the camera along its view and
//! it glides to a stop once they're released, and a drag of the pointer turns it.
//!
//! These are pure functions of the camera's state and its input; the editor feeds them the keys and the drag, and
//! sends the transform [`Fly::step`] returns to the camera it switched the game window to.

use std::f32::consts::{FRAC_PI_2, PI};

use crate::constants::up;
use crate::gameplay::Transform;
use crate::math::{Point, Scale, UnitVector, Vector, direction_from_orientation};

/// How fast held keys speed the camera up, in meters per second squared.
const ACCELERATION: f32 = 100.0;
/// The share of its speed the camera keeps after a second without keys, so it glides to a stop at any frame rate.
const DAMPING: f32 = 0.01;
/// How far short of straight up or down the camera can look, in radians, so its view keeps a direction to call right.
const PITCH_MARGIN: f32 = 0.01;

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
		let limit = FRAC_PI_2 - PITCH_MARGIN;
		Self {
			position: transform.get_position(),
			yaw: direction.x().atan2(direction.z()),
			pitch: direction.y().clamp(-1.0, 1.0).asin().clamp(-limit, limit),
			velocity: Vector::new(0.0, 0.0, 0.0),
		}
	}

	/// Turns the view by a pointer drag of `delta`, in normalized window coordinates: a drag across the whole window
	/// turns it a full circle, and one from bottom to top looks from straight down to straight up, as the model
	/// viewer's pointer does.
	pub(crate) fn turn(&mut self, delta: [f32; 2]) {
		let limit = FRAC_PI_2 - PITCH_MARGIN;
		self.yaw += delta[0] * PI;
		self.pitch = (self.pitch + delta[1] * FRAC_PI_2).clamp(-limit, limit);
	}

	/// Moves the camera on by `dt` seconds with `thrust` held, right, up, and forward along its view, each from -1 to
	/// 1, and returns where it now is.
	pub(crate) fn step(&mut self, thrust: [f32; 3], dt: f32) -> Transform {
		let forward = self.direction();
		let right = up().cross(forward.into_vector());
		let up = forward.cross(right);
		let push = thrust[0] * right + thrust[1] * up + thrust[2] * forward.into_vector();
		self.velocity += push * ACCELERATION * dt;
		self.velocity = self.velocity * DAMPING.powf(dt);
		self.position += self.velocity * dt;
		self.transform()
	}

	/// Where the camera is, looking along its view.
	pub(crate) fn transform(&self) -> Transform {
		Transform::new(self.position, Scale::identity(), self.direction().into())
	}

	/// Whether the camera is still gliding, so the editor knows to keep moving it.
	pub(crate) fn moving(&self) -> bool {
		self.velocity.dot(self.velocity) > 1e-6
	}

	fn direction(&self) -> UnitVector {
		let (yaw, pitch) = (self.yaw, self.pitch);
		Vector::new(yaw.sin() * pitch.cos(), pitch.sin(), yaw.cos() * pitch.cos())
			.normalized()
			.expect("A direction from angles has unit length.")
	}
}

#[cfg(test)]
mod tests {
	use super::*;
	use crate::math::orientation_from_direction;

	fn looking(direction: Vector) -> Transform {
		let direction = direction.normalized().unwrap();
		Transform::new(Point::new(1.0, 2.0, 3.0), Scale::identity(), orientation_from_direction(direction))
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
			fly.step([0.0, 0.0, 1.0], 1.0 / 60.0);
		}
		let pushed = fly.transform().get_position() - start;
		assert!(pushed.x() > 0.0 && pushed.y().abs() < 1e-4 && pushed.z().abs() < 1e-4, "forward is the view: {pushed:?}");
		for _ in 0..600 {
			fly.step([0.0; 3], 1.0 / 60.0);
		}
		assert!(!fly.moving(), "released keys let the camera stop");
	}

	#[test]
	fn a_drag_turns_the_view_and_stops_short_of_straight_up() {
		let mut fly = Fly::from_transform(&looking(Vector::new(0.0, 0.0, 1.0)));
		fly.turn([0.5, 0.0]);
		let turned = view(&fly.transform());
		assert!((turned - Vector::new(1.0, 0.0, 0.0)).length() < 1e-4, "half a window turns a quarter: {turned:?}");
		fly.turn([0.0, 5.0]);
		let up = view(&fly.transform());
		assert!(up.y() < 1.0 && up.y() > 0.99, "the view stops short of straight up: {up:?}");
	}
}
