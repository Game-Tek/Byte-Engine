//! Local avoidance: the least change to an agent's velocity that keeps it clear of its neighbors and on the mesh.

/// Returns `velocity` with a sidestep added for each crowd among `others` and the slide along the mesh boundary
/// removed.
///
/// Every term is a continuous function of where the agents are and how fast they move, so an agent that turns toward
/// the result at a bounded rate moves smoothly; see the navigation Reference page for the terms. `agent` and each item
/// of `others` are a position, a velocity, a radius, and a precedence. Others that touch each other are avoided as one
/// crowd, so an agent goes around a group instead of between its members; the agent holds its course for crowds of
/// less precedence, which step aside for it.
pub(super) fn steer(
	mesh: &NavigationMesh,
	(position, radius, precedence): (Point, f32, f32),
	velocity: Vector,
	others: impl Iterator<Item = (Point, Vector, f32, f32)>,
) -> Vector {
	let stepped = crowds(others)
		.filter(|crowd| crowd.precedence >= precedence)
		.fold(velocity, |stepped, crowd| {
			stepped
				+ sidestep(
					position,
					velocity,
					radius + crowd.radius + COMFORT_DISTANCE,
					crowd.center,
					crowd.velocity,
				)
		});
	// The slide applies to the velocity with the sidesteps in, so no sidestep leads off the mesh. Off the mesh, the
	// boundary is one way: nothing may lead farther from it, so an agent pushed off can come back but never deeper.
	match mesh.closest_boundary_point(position) {
		Some(boundary) if mesh.locate_polygon(position).is_some() => stepped - slide(position, stepped, boundary),
		Some(boundary) => stepped - receding(position, stepped, boundary),
		None => stepped,
	}
}

/// Returns the part of `velocity` that carries an agent at `position`, off the mesh, farther from `boundary`.
fn receding(position: Point, velocity: Vector, boundary: Point) -> Vector {
	let Some(toward) = flat_unit(boundary - position) else {
		return Vector::new(0.0, 0.0, 0.0);
	};
	toward * velocity.dot(toward).min(0.0)
}

/// The `Crowd` struct is one circle an agent steers around: a group of others that touch each other, or one alone.
pub(super) struct Crowd {
	pub(super) center: Point,
	pub(super) velocity: Vector,
	pub(super) radius: f32,
	pub(super) precedence: f32,
}

/// Groups `others` into the crowds an agent steers around: others within contact distance of each other join one
/// crowd, avoided as one circle around them all that moves at their mean velocity and has their greatest precedence.
///
/// Sidesteps from the members of a group an agent heads into would push it to either side and cancel out, so it
/// would walk in between; one circle sends it around.
pub(super) fn crowds(others: impl Iterator<Item = (Point, Vector, f32, f32)>) -> impl Iterator<Item = Crowd> {
	// Few agents are ever near each other, so a small buffer holds them without reaching for the heap.
	let mut members: SmallVec<[(Point, Vector, f32, f32, usize); 16]> = others
		.enumerate()
		.map(|(index, (position, velocity, radius, precedence))| (position, velocity, radius, precedence, index))
		.collect();
	// Each member joins the group of any member it touches, until no two touching members are apart.
	let mut merged = true;
	while merged {
		merged = false;
		for first in 0..members.len() {
			for second in first + 1..members.len() {
				let (a, b) = (members[first], members[second]);
				if a.4 != b.4 && distance_xz(a.0, b.0) <= a.2 + b.2 + COMFORT_DISTANCE {
					let (from, to) = (a.4.max(b.4), a.4.min(b.4));
					members
						.iter_mut()
						.filter(|member| member.4 == from)
						.for_each(|member| member.4 = to);
					merged = true;
				}
			}
		}
	}
	let groups: SmallVec<[usize; 16]> = members.iter().map(|member| member.4).collect();
	let mut seen: SmallVec<[usize; 16]> = SmallVec::new();
	groups.into_iter().filter_map(move |group| {
		if seen.contains(&group) {
			return None;
		}
		seen.push(group);
		let crowd = members.iter().filter(|member| member.4 == group);
		let count = crowd.clone().count() as f32;
		let (x, z, velocity) = crowd.clone().fold((0.0, 0.0, Vector::new(0.0, 0.0, 0.0)), |(x, z, v), m| {
			(x + m.0.x(), z + m.0.z(), v + m.1)
		});
		let center = Point::new(x / count, 0.0, z / count);
		Some(Crowd {
			center,
			velocity: velocity / count,
			radius: crowd.clone().map(|m| distance_xz(center, m.0) + m.2).fold(0.0, f32::max),
			precedence: crowd.map(|m| m.3).fold(f32::NEG_INFINITY, f32::max),
		})
	})
}

/// Returns the velocity an agent at `position` moving at `velocity` adds to pass `other` at `contact` distance.
///
/// The sidestep is the shortfall at the closest approach, taken in the time left until then. Two that are already too
/// close are closest now, so they step apart at once.
fn sidestep(position: Point, velocity: Vector, contact: f32, other: Point, other_velocity: Vector) -> Vector {
	let time = closest_approach_time_xz(position, velocity, other, other_velocity).clamp(0.0, HORIZON_SECONDS);
	let offset = (position + velocity * time) - (other + other_velocity * time);
	let distance = offset.x().hypot(offset.z());
	let shortfall = contact - distance;
	if shortfall <= 0.0 {
		return Vector::new(0.0, 0.0, 0.0);
	}
	let away = if distance > NAVIGATION_TOLERANCE {
		Vector::new(offset.x() / distance, 0.0, offset.z() / distance)
	} else {
		// Dead on course: step to the right of the relative motion, as people do.
		let relative = velocity - other_velocity;
		flat_unit(Vector::new(-relative.z(), 0.0, relative.x())).unwrap_or(Vector::new(1.0, 0.0, 0.0))
	};
	// The shortfall over the time left is the rate that clears it in time; far-off approaches count less.
	away * (shortfall / time.max(LEAST_SECONDS) * (1.0 - time / HORIZON_SECONDS))
}

/// Returns the part of `velocity` an agent at `position` gives up as it nears `boundary`: whatever approach would
/// reach it sooner than [`BOUNDARY_SECONDS`], so it arrives sliding along the boundary instead of through it.
fn slide(position: Point, velocity: Vector, boundary: Point) -> Vector {
	let Some(toward) = flat_unit(boundary - position) else {
		return Vector::new(0.0, 0.0, 0.0);
	};
	let allowed = distance_xz(position, boundary) / BOUNDARY_SECONDS;
	toward * (velocity.dot(toward) - allowed).max(0.0)
}

/// Returns `vector` flattened onto XZ at unit length, or [`None`] when it has no length there.
fn flat_unit(vector: Vector) -> Option<Vector> {
	Vector::new(vector.x(), 0.0, vector.z())
		.normalized()
		.ok()
		.map(UnitVector::into_vector)
}

/// How far ahead, in seconds, steering looks for other agents.
pub(super) const HORIZON_SECONDS: f32 = 2.0;

/// The soonest, in seconds, an agent may be on course to reach the boundary; nearer it, it slows its approach so it
/// arrives sliding. Paths run along the boundary without crossing it, and lose nothing to this.
const BOUNDARY_SECONDS: f32 = 0.5;

/// The least time, in seconds, a sidestep is given, so two already too close step apart at a bounded speed.
const LEAST_SECONDS: f32 = 0.25;

/// How much room, in meters, agents keep between their edges when they pass.
pub(super) const COMFORT_DISTANCE: f32 = 0.15;

use math::{Point, UnitVector, Vector, closest_approach_time_xz, distance_xz};
use smallvec::SmallVec;

use super::{NAVIGATION_TOLERANCE, NavigationMesh};
