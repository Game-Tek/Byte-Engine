//! Local avoidance: the velocity an agent picks to follow its direction without running into its neighbors.

/// Returns the velocity an agent moves at to head along `direction` at one of `speeds` without running into `others`
/// or off `mesh`.
///
/// Every candidate velocity is scored on its own, against where the others are and how fast they move now, so agents
/// steer without coordinating; see the navigation Reference page for the candidates and their costs. `agent` and each
/// item of `others` are a position, a velocity, and a radius; `others` is cloned once per candidate. The agent keeps
/// its velocity for `response_time` seconds before a candidate takes effect, so candidates start where that leaves
/// it, against the others carried along by theirs for as long.
pub(super) fn steer(
	mesh: &NavigationMesh,
	(position, velocity, radius): (Point, Vector, f32),
	response_time: f32,
	direction: Vector,
	speeds: &[f32],
	others: impl Iterator<Item = (Point, Vector, f32)> + Clone,
) -> Vector {
	let still = Vector::new(0.0, 0.0, 0.0);
	let (Some(heading), Some(&preferred_speed)) = (heading_xz(direction), speeds.first()) else {
		return still;
	};
	// Speed changes count against the fastest speed, so an agent that prefers standing weighs moving like a walking
	// one weighs stopping.
	let fastest = speeds.iter().copied().fold(0.0, f32::max);
	if fastest <= 0.0 {
		return still;
	}
	let speed_change = |speed: f32| (speed - preferred_speed).abs() / fastest;

	let start = position + velocity * response_time;
	let cost = |candidate: Vector, turn_and_speed_change: f32| {
		let contact = others
			.clone()
			.filter_map(|(other, other_velocity, other_radius)| {
				let other = other + other_velocity * response_time;
				time_to_contact_xz(start, candidate, other, other_velocity, radius + other_radius)
			})
			.map(|seconds| response_time + seconds)
			.filter(|&seconds| seconds < CONTACT_HORIZON_SECONDS)
			.min_by(f32::total_cmp)
			.map_or(0.0, |seconds| CONTACT_COST * (1.0 - seconds / CONTACT_HORIZON_SECONDS));
		let ahead = start + candidate * MESH_LOOKAHEAD_SECONDS;
		let leaving = if mesh.locate_polygon(ahead).is_some() {
			0.0
		} else {
			LEAVING_MESH_COST
		};
		turn_and_speed_change + contact + leaving
	};

	// Standing is its own candidate, so zero speeds pair with no heading.
	let moving = speeds.iter().filter(|&&speed| speed > 0.0).flat_map(|&speed| {
		TURN_DEGREES.iter().map(move |&turn| {
			let (sine, cosine) = turn.to_radians().sin_cos();
			let [x, z] = heading;
			// A positive turn is counterclockwise on XZ, the winding `signed_area_xz` calls positive.
			let candidate = Vector::new(x * cosine - z * sine, 0.0, x * sine + z * cosine) * speed;
			(candidate, turn.abs() / 180.0 + speed_change(speed))
		})
	});
	// Standing last, so it only wins a tie against nothing.
	let candidates = moving.chain(std::iter::once((still, speed_change(0.0))));

	let mut best = (still, f32::INFINITY);
	for (candidate, turn_and_speed_change) in candidates {
		let total = cost(candidate, turn_and_speed_change);
		if total < best.1 {
			best = (candidate, total);
		}
	}
	best.0
}

/// Returns the velocity of an agent that ignores its neighbors: along `direction` at the first of `speeds`.
pub(super) fn preferred_velocity(direction: Vector, speeds: &[f32]) -> Vector {
	match (heading_xz(direction), speeds.first()) {
		(Some([x, z]), Some(&speed)) => Vector::new(x, 0.0, z) * speed,
		_ => Vector::new(0.0, 0.0, 0.0),
	}
}

/// Returns `direction` projected onto XZ at unit length, or [`None`] when it has no length there.
fn heading_xz(direction: Vector) -> Option<[f32; 2]> {
	let length = direction.x().hypot(direction.z());
	(length > 0.0 && length.is_finite()).then(|| [direction.x() / length, direction.z() / length])
}

/// The turns from the direction steering tries at each speed, in the order that breaks ties.
const TURN_DEGREES: [f32; 13] = [
	0.0, 15.0, -15.0, 30.0, -30.0, 45.0, -45.0, 60.0, -60.0, 90.0, -90.0, 120.0, -120.0,
];

/// How far ahead, in seconds, steering looks for contacts.
const CONTACT_HORIZON_SECONDS: f32 = 2.0;

/// The cost of a contact happening now, falling to zero at the horizon.
const CONTACT_COST: f32 = 2.0;

/// How far ahead, in seconds, a velocity must stay on the mesh.
const MESH_LOOKAHEAD_SECONDS: f32 = 0.5;

/// The cost of a velocity that leaves the mesh.
const LEAVING_MESH_COST: f32 = 3.0;

use math::{Point, Vector, time_to_contact_xz};

use super::NavigationMesh;
