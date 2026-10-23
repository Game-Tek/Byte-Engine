//! Local avoidance: the least change to an agent's velocity that keeps it clear of its neighbors and on the mesh.

/// Returns `velocity` with a sidestep added for each of `others` and the slide along the mesh boundary removed.
///
/// Every term is a continuous function of where the agents are and how fast they move, so an agent that turns toward
/// the result at a bounded rate moves smoothly; see the navigation Reference page for the terms. `agent` and each item
/// of `others` are a position, a velocity, a radius, and a precedence; the agent holds its course for others of less
/// precedence, which step aside for it.
pub(super) fn steer(
	mesh: &NavigationMesh,
	(position, radius, precedence): (Point, f32, f32),
	velocity: Vector,
	others: impl Iterator<Item = (Point, Vector, f32, f32)>,
) -> Vector {
	let stepped = others
		.filter(|&(_, _, _, other_precedence)| other_precedence >= precedence)
		.fold(velocity, |stepped, (other, other_velocity, other_radius, _)| {
			stepped
				+ sidestep(
					position,
					velocity,
					radius + other_radius + COMFORT_DISTANCE,
					other,
					other_velocity,
				)
		});
	// The slide applies to the velocity with the sidesteps in, so no sidestep leads off the mesh. Off the mesh, the
	// nearest boundary is the way back on.
	match mesh.closest_boundary_point(position) {
		Some(boundary) if mesh.locate_polygon(position).is_some() => stepped - slide(position, stepped, boundary),
		_ => stepped,
	}
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
const HORIZON_SECONDS: f32 = 2.0;

/// The soonest, in seconds, an agent may be on course to reach the boundary; nearer it, it slows its approach so it
/// arrives sliding. Paths run along the boundary without crossing it, and lose nothing to this.
const BOUNDARY_SECONDS: f32 = 0.5;

/// The least time, in seconds, a sidestep is given, so two already too close step apart at a bounded speed.
const LEAST_SECONDS: f32 = 0.25;

/// How much room, in meters, agents keep between their edges when they pass.
const COMFORT_DISTANCE: f32 = 0.15;

use math::{Point, UnitVector, Vector, closest_approach_time_xz, distance_xz};

use super::{NAVIGATION_TOLERANCE, NavigationMesh};
