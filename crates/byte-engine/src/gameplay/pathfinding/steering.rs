//! Local crowd passing at a character's gait speed, with one retained passing side.

use math::{Point, UnitVector, Vector, distance_xz};
use smallvec::SmallVec;

use super::NavigationMesh;

/// Returns a passing velocity at the requested speed, or separation while standing, then applies the mesh boundary.
/// Overlapping blocked directions form one interval, so a group cannot produce competing sidesteps.
pub(super) fn steer(
	mesh: &NavigationMesh,
	(position, actual, radius, precedence): (Point, Vector, f32, f32),
	velocity: Vector,
	passing: &mut Option<(u32, bool)>,
	others: impl Iterator<Item = (u32, Point, Vector, f32, f32)>,
) -> (Vector, f32) {
	let mut following_speed = f32::INFINITY;
	let others = others.filter(|other| other.4 >= precedence);
	let stepped = if let Some(direction) = flat_unit(velocity) {
		let requested_speed = velocity.length();
		// A cone is the interval of headings that hit one predicted contact circle. Keeping the circles separate
		// avoids a discontinuous change of center and radius when two neighbors join or leave a group.
		let cones: SmallVec<[(u32, f32, f32); 16]> = others
			.filter_map(|(id, center, motion, other_radius, _)| {
				let contact = radius + other_radius + COMFORT_DISTANCE;
				let current = center - position;
				let along = current.dot(direction);
				let across = (current - direction * along).length();
				let lead_speed = motion.dot(direction);
				if along > 0.0 && across < contact && lead_speed > 0.2 && lead_speed >= motion.length() * 0.8 {
					// Follow someone travelling the same way instead of overtaking them into the next character. This is
					// a speed ceiling: a clip-driven caller can choose a slower gait or begin its stop with this headway.
					following_speed = following_speed.min((lead_speed + (along - contact) / HORIZON_SECONDS).max(0.0));
					return None;
				}
				let time = (distance_xz(position, center) / requested_speed).min(HORIZON_SECONDS);
				let offset = center + motion * time - position;
				let distance = offset.length();
				if distance > requested_speed * HORIZON_SECONDS + contact {
					return None;
				}
				let angle = (direction.x() * offset.z() - direction.z() * offset.x()).atan2(direction.dot(offset));
				let half = (contact / distance.max(contact)).asin();
				Some((id, angle - half, angle + half))
			})
			.collect();
		let speed = requested_speed.min(following_speed);
		if let Some([low, high]) = blocked_range(&cones) {
			let retained = passing.filter(|(id, _)| {
				cones
					.iter()
					.any(|&(other, from, to)| other == *id && from <= high.0 && to >= low.0)
			});
			let mut clockwise = retained.map(|(_, side)| side).unwrap_or_else(|| {
				let heading = flat_unit(actual).unwrap_or(direction);
				let angle = (direction.x() * heading.z() - direction.z() * heading.x()).atan2(direction.dot(heading));
				(angle - low.0).abs() <= (angle - high.0).abs()
			});
			let toward = |angle: f32| {
				let (sine, cosine) = angle.sin_cos();
				Vector::new(
					direction.x() * cosine - direction.z() * sine,
					0.0,
					direction.x() * sine + direction.z() * cosine,
				)
			};
			// Prefer the retained side. If its short lookahead leaves the mesh, try the other side instead. The
			// boundary constraint below still limits approach where neither passing direction is usable.
			let usable = |angle: f32| mesh.locate_polygon(position + toward(angle) * speed.min(1.0)).is_some();
			let angle = if clockwise { low.0 } else { high.0 };
			let alternate = if clockwise { high.0 } else { low.0 };
			if !usable(angle) && usable(alternate) {
				clockwise = !clockwise;
			}
			let (angle, owner) = if clockwise { low } else { high };
			*passing = Some((owner, clockwise));
			// A closed forward range is a queue, not a request to turn back into the following characters. Yield
			// until a forward pass opens; clip-driven callers can begin their stop instead of reversing the line.
			if angle.abs() >= std::f32::consts::FRAC_PI_2 {
				Vector::new(0.0, 0.0, 0.0)
			} else {
				toward(angle) * speed
			}
		} else {
			// Keep the decision while its neighbor is ahead, even through a brief clear route direction.
			if passing.is_some_and(|(id, _)| {
				!cones.iter().any(|&(other, low, high)| {
					id == other && low < std::f32::consts::FRAC_PI_2 && high > -std::f32::consts::FRAC_PI_2
				})
			}) {
				*passing = None;
			}
			if speed < requested_speed {
				direction * speed
			} else {
				velocity
			}
		}
	} else {
		*passing = None;
		// Resting characters react to present overlap only. Predicting a crossing here makes the separation reverse
		// as another character passes, although the resting character cannot execute such a finely controlled step.
		others.fold(velocity, |separated, (_, center, _, other_radius, _)| {
			let offset = position - center;
			let distance = offset.length();
			let shortfall = (radius + other_radius + COMFORT_DISTANCE - distance).max(0.0);
			let away = flat_unit(offset).unwrap_or(Vector::new(1.0, 0.0, 0.0));
			separated + away * (shortfall / LEAST_SECONDS)
		})
	};
	let constrained = match mesh.closest_boundary_point(position) {
		Some(boundary) => {
			let Some(toward) = flat_unit(boundary - position) else {
				return (stepped, following_speed);
			};
			let approach = stepped.dot(toward);
			if mesh.locate_polygon(position).is_some() {
				stepped - toward * (approach - distance_xz(position, boundary) / BOUNDARY_SECONDS).max(0.0)
			} else {
				stepped - toward * approach.min(0.0)
			}
		}
		None => stepped,
	};
	(constrained, following_speed)
}

/// Returns the connected range blocking the desired heading, retaining each endpoint's agent as it expands.
/// A gap wide enough for forward travel stays open, including ranges that only touch the desired heading.
fn blocked_range(cones: &[(u32, f32, f32)]) -> Option<[(f32, u32); 2]> {
	let &(id, from, to) = cones.iter().find(|&&(_, from, to)| from < 0.0 && to > 0.0)?;
	let (mut low, mut high) = ((from, id), (to, id));
	loop {
		let before = (low.0, high.0);
		for &(id, from, to) in cones {
			if from <= high.0 && to >= low.0 {
				if (from, id) < low {
					low = (from, id);
				}
				if to > high.0 || (to == high.0 && id < high.1) {
					high = (to, id);
				}
			}
		}
		if before == (low.0, high.0) {
			return Some([low, high]);
		}
	}
}

/// Returns `vector` flattened onto XZ at unit length, or `None` when it has no planar length.
fn flat_unit(vector: Vector) -> Option<Vector> {
	Vector::new(vector.x(), 0.0, vector.z())
		.normalized()
		.ok()
		.map(UnitVector::into_vector)
}

/// How far ahead, in seconds, a moving character anticipates another character.
const HORIZON_SECONDS: f32 = 2.0;
/// How soon, in seconds, an approach to the mesh boundary may reach it.
const BOUNDARY_SECONDS: f32 = 0.5;
/// The time, in seconds, allowed for separating an overlap.
const LEAST_SECONDS: f32 = 0.25;
/// The clearance, in meters, between passing agents' edges.
const COMFORT_DISTANCE: f32 = 0.15;
