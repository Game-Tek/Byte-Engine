use maths_rs::Vec3f;

use crate::{AABB, Capsule, Plane, Point, Ray, Sphere, UnitVector, Vector, WorldSpace};

/// The `Intersection` struct records a static contact using a normal that points from shape A toward shape B.
pub struct Intersection<Space = WorldSpace> {
	normal: UnitVector<Space>,
	depth: f32,
	point_on_a: Point<Space>,
	point_on_b: Point<Space>,
}

impl<Space> Clone for Intersection<Space> {
	fn clone(&self) -> Self {
		Self {
			normal: self.normal,
			depth: self.depth,
			point_on_a: self.point_on_a,
			point_on_b: self.point_on_b,
		}
	}
}

impl<Space> std::fmt::Debug for Intersection<Space> {
	fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		formatter
			.debug_struct("Intersection")
			.field("normal", &self.normal)
			.field("depth", &self.depth)
			.field("point_on_a", &self.point_on_a)
			.field("point_on_b", &self.point_on_b)
			.finish()
	}
}

impl<Space> PartialEq for Intersection<Space> {
	fn eq(&self, other: &Self) -> bool {
		self.normal == other.normal
			&& self.depth == other.depth
			&& self.point_on_a == other.point_on_a
			&& self.point_on_b == other.point_on_b
	}
}

impl<Space> Intersection<Space> {
	/// Returns the unit normal from shape A toward shape B.
	pub fn normal(&self) -> UnitVector<Space> {
		self.normal
	}

	/// Returns the overlap depth in world-space units.
	pub fn depth(&self) -> f32 {
		self.depth
	}

	/// Returns the contact point on shape A.
	pub fn point_on_a(&self) -> Point<Space> {
		self.point_on_a
	}

	/// Returns the contact point on shape B.
	pub fn point_on_b(&self) -> Point<Space> {
		self.point_on_b
	}

	/// Returns the same contact with the shape order reversed.
	pub fn swap(self) -> Self {
		Self {
			normal: -self.normal,
			depth: self.depth,
			point_on_a: self.point_on_b,
			point_on_b: self.point_on_a,
		}
	}
}

/// The `DynamicIntersection` struct records a contact reached during a time step using a normal from shape A toward shape B.
pub struct DynamicIntersection<Space = WorldSpace> {
	toi: f32,
	contact: Intersection<Space>,
}

impl<Space> Clone for DynamicIntersection<Space> {
	fn clone(&self) -> Self {
		Self {
			toi: self.toi,
			contact: self.contact.clone(),
		}
	}
}

impl<Space> std::fmt::Debug for DynamicIntersection<Space> {
	fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		formatter
			.debug_struct("DynamicIntersection")
			.field("toi", &self.toi)
			.field("contact", &self.contact)
			.finish()
	}
}

impl<Space> PartialEq for DynamicIntersection<Space> {
	fn eq(&self, other: &Self) -> bool {
		self.toi == other.toi && self.contact == other.contact
	}
}

impl<Space> DynamicIntersection<Space> {
	/// Returns the time of impact in seconds from the start of the simulated step.
	pub fn toi(&self) -> f32 {
		self.toi
	}

	/// Returns the contact generated at the time of impact.
	pub fn contact(&self) -> &Intersection<Space> {
		&self.contact
	}

	/// Consumes this result and returns its static contact data.
	pub fn into_contact(self) -> Intersection<Space> {
		self.contact
	}
}

impl<Space> From<DynamicIntersection<Space>> for Intersection<Space> {
	fn from(intersection: DynamicIntersection<Space>) -> Self {
		intersection.contact
	}
}

impl<Space> PartialOrd for DynamicIntersection<Space> {
	fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
		self.toi.partial_cmp(&other.toi)
	}
}

/// Returns the first non-negative distance where `ray` intersects `aabb`.
pub fn ray_aabb_intersection<Space>(ray: &Ray<Space>, aabb: &AABB<Space>) -> Option<f32> {
	let origin = ray.origin();
	let direction = ray.direction();
	let min = aabb.min();
	let max = aabb.max();
	let mut entry = f32::NEG_INFINITY;
	let mut exit = f32::INFINITY;

	// Each slab restricts the interval of ray distances that can remain inside the box.
	for (origin, direction, min, max) in [
		(origin.x(), direction.x(), min.x(), max.x()),
		(origin.y(), direction.y(), min.y(), max.y()),
		(origin.z(), direction.z(), min.z(), max.z()),
	] {
		if direction == 0.0 {
			if origin < min || origin > max {
				return None;
			}
			continue;
		}

		let first = (min - origin) / direction;
		let second = (max - origin) / direction;
		entry = entry.max(first.min(second));
		exit = exit.min(first.max(second));
	}

	(entry <= exit && exit >= 0.0).then(|| entry.max(0.0))
}

/// Returns whether `sphere` is inside or touches every plane of a frustum.
///
/// Each plane normal must point toward the inside of the frustum.
pub fn sphere_in_frustum<Space>(sphere: &Sphere<Space>, frustum_planes: &[Plane<Space>; 6]) -> bool {
	frustum_planes.iter().all(|plane| plane.is_sphere_in_half_space(sphere))
}

/// Returns the contact between two overlapping or tangent spheres.
// Inlined because the capsule tests end in this one, which would otherwise leave every caller with a call to a shared
// copy. With that call, physics steps of capsule piles were 2% to 3% slower on an Apple M4.
#[inline]
pub fn sphere_vs_sphere<Space>(sphere_a: &Sphere<Space>, sphere_b: &Sphere<Space>) -> Option<Intersection<Space>> {
	let offset = sphere_b.center() - sphere_a.center();
	let radius_sum = sphere_a.radius() + sphere_b.radius();
	let distance_squared = offset.length_squared();
	if distance_squared > radius_sum * radius_sum {
		return None;
	}

	// Coincident centers have no geometric normal, so use a stable axis instead of producing NaNs.
	let normal = offset.normalized().unwrap_or_else(|_| UnitVector::x_axis());
	let distance = distance_squared.sqrt();
	Some(Intersection {
		normal,
		depth: (radius_sum - distance).max(0.0),
		point_on_a: sphere_a.center() + normal * sphere_a.radius(),
		point_on_b: sphere_b.center() - normal * sphere_b.radius(),
	})
}

/// Returns the contact between two overlapping axis-aligned boxes.
pub fn aabb_vs_aabb<Space>(a: &AABB<Space>, b: &AABB<Space>) -> Option<Intersection<Space>> {
	let (a_min, a_max) = (a.min().to_array(), a.max().to_array());
	let (b_min, b_max) = (b.min().to_array(), b.max().to_array());
	// Keep the overlap endpoints for both the separating-axis test and contact points.
	let overlap_min: [f32; 3] = std::array::from_fn(|axis| a_min[axis].max(b_min[axis]));
	let overlap_max: [f32; 3] = std::array::from_fn(|axis| a_max[axis].min(b_max[axis]));
	let overlap: [f32; 3] = std::array::from_fn(|axis| overlap_max[axis] - overlap_min[axis]);
	if overlap[0] < 0.0 || overlap[1] < 0.0 || overlap[2] < 0.0 {
		return None;
	}

	// Choose the shallowest axis, which is the minimum translation direction. Everything after this reads that
	// axis by index instead of branching on it: the axis changes from one pair of boxes to the next with no
	// pattern, so a processor would guess such branches wrong about as often as right.
	let axis = shallowest_axis(overlap);
	// Only the selected axis contributes to the normal's orientation. The sign points from A's center to B's.
	let a_center = a_min[axis] + (a_max[axis] - a_min[axis]) * 0.5;
	let b_center = b_min[axis] + (b_max[axis] - b_min[axis]) * 0.5;
	let sign = signed_axis(b_center - a_center);
	// The contact points lie on the facing surfaces of the boxes, in the middle of the overlap.
	let mut point_on_a: [f32; 3] = std::array::from_fn(|axis| (overlap_min[axis] + overlap_max[axis]) * 0.5);
	let mut point_on_b = point_on_a;
	(point_on_a[axis], point_on_b[axis]) = if sign > 0.0 {
		(a_max[axis], b_min[axis])
	} else {
		(a_min[axis], b_max[axis])
	};

	Some(Intersection {
		normal: axis_normal(axis, sign),
		depth: overlap[axis],
		point_on_a: Point::from_array(point_on_a),
		point_on_b: Point::from_array(point_on_b),
	})
}

// Selects the minimum-translation axis with stable tie-breaking toward X, then Y.
fn shallowest_axis([overlap_x, overlap_y, overlap_z]: [f32; 3]) -> usize {
	let y_is_shallowest = (overlap_y < overlap_x) & (overlap_y <= overlap_z);
	let z_is_shallowest = !y_is_shallowest & (overlap_z < overlap_x) & (overlap_z < overlap_y);
	usize::from(y_is_shallowest) + 2 * usize::from(z_is_shallowest)
}

/// Returns the contact between a sphere and an axis-aligned box.
pub fn sphere_vs_aabb<Space>(sphere: &Sphere<Space>, aabb: &AABB<Space>) -> Option<Intersection<Space>> {
	let center = sphere.center();
	let closest = Point::new(
		center.x().clamp(aabb.min().x(), aabb.max().x()),
		center.y().clamp(aabb.min().y(), aabb.max().y()),
		center.z().clamp(aabb.min().z(), aabb.max().z()),
	);
	let toward_box = closest - center;
	let distance_squared = toward_box.length_squared();
	if distance_squared > sphere.radius() * sphere.radius() {
		return None;
	}

	if let Ok(normal) = toward_box.normalized() {
		let distance = distance_squared.sqrt();
		return Some(Intersection {
			normal,
			depth: sphere.radius() - distance,
			point_on_a: center + normal * sphere.radius(),
			point_on_b: closest,
		});
	}

	// A center inside the box clamps to itself; select the nearest face for a stable separation direction.
	let half_extents = aabb.half_extents();
	let delta = center - aabb.center();
	let face_distance: Vector<Space> = Vector::new(
		half_extents.x() - delta.x().abs(),
		half_extents.y() - delta.y().abs(),
		half_extents.z() - delta.z().abs(),
	);
	let (axis, nearest_face) = if face_distance.y() < face_distance.x() && face_distance.y() <= face_distance.z() {
		(1, face_distance.y())
	} else if face_distance.z() < face_distance.x() && face_distance.z() < face_distance.y() {
		(2, face_distance.z())
	} else {
		(0, face_distance.x())
	};
	let sign = match axis {
		0 => signed_axis(delta.x()),
		1 => signed_axis(delta.y()),
		2 => signed_axis(delta.z()),
		_ => unreachable!(),
	};
	let normal = axis_normal(axis, sign);
	let point_on_b = center + normal * nearest_face;

	Some(Intersection {
		normal,
		depth: sphere.radius() + nearest_face,
		point_on_a: center + normal * sphere.radius(),
		point_on_b,
	})
}

/// Returns the contact between a capsule and a sphere that overlap or touch.
///
/// The capsule is shape A, so the normal points from the capsule toward the sphere.
pub fn capsule_vs_sphere<Space>(capsule: &Capsule<Space>, sphere: &Sphere<Space>) -> Option<Intersection<Space>> {
	// A capsule is a sphere swept along its segment, so it touches another shape where the sphere around the
	// segment's nearest point does.
	let nearest = closest_point_on_segment(capsule.start(), capsule.end(), sphere.center());
	sphere_vs_sphere(&Sphere::new(nearest, capsule.radius()), sphere)
}

/// Returns the contact between two capsules that overlap or touch.
///
/// Capsules that lie side by side touch along a stretch. Their contact is in the middle of it.
pub fn capsule_vs_capsule<Space>(a: &Capsule<Space>, b: &Capsule<Space>) -> Option<Intersection<Space>> {
	let (on_a, on_b) = closest_points_between_segments([a.start(), a.end()], [b.start(), b.end()]);
	sphere_vs_sphere(&Sphere::new(on_a, a.radius()), &Sphere::new(on_b, b.radius()))
}

/// Returns the contact between a capsule and an axis-aligned box that overlap or touch.
///
/// The capsule is shape A, so the normal points from the capsule toward the box. A capsule that lies along a face
/// touches it along a stretch, and its contact is in the middle of it. A capsule whose segment enters the box is
/// pushed out through the face nearest to the deepest of the segment's two ends and the middle of the part inside.
pub fn capsule_vs_aabb<Space>(capsule: &Capsule<Space>, aabb: &AABB<Space>) -> Option<Intersection<Space>> {
	let nearest = closest_point_on_segment_to_aabb(capsule.start(), capsule.end(), aabb);
	sphere_vs_aabb(&Sphere::new(nearest, capsule.radius()), aabb)
}

/// Returns the point of the segment from `start` to `end` that is closest to `point`.
fn closest_point_on_segment<Space>(start: Point<Space>, end: Point<Space>, point: Point<Space>) -> Point<Space> {
	let along = end - start;
	let length_squared = along.length_squared();
	// A segment without length is a single point.
	if length_squared == 0.0 {
		return start;
	}
	start + along * ((point - start).dot(along) / length_squared).clamp(0.0, 1.0)
}

/// Returns the closest pair of points between two segments, each given as its two ends: first the point on `a`,
/// then the point on `b`.
///
/// Parallel segments are equally close along a stretch. The points are then in the middle of it.
fn closest_points_between_segments<Space>(a: [Point<Space>; 2], b: [Point<Space>; 2]) -> (Point<Space>, Point<Space>) {
	let (a_along, b_along) = (a[1] - a[0], b[1] - b[0]);
	let (a_length, b_length) = (a_along.length_squared(), b_along.length_squared());
	let between = a[0] - b[0];
	let (a_offset, b_offset) = (a_along.dot(between), b_along.dot(between));
	let alignment = a_along.dot(b_along);

	// Where the lines through the segments pass closest, as a share of the way along `a`. The divisor is the squared
	// area the two segments span, which is zero when they are parallel or one of them has no length.
	let spanned = a_length * b_length - alignment * alignment;
	let share = if spanned > f32::EPSILON * a_length * b_length {
		((alignment * b_offset - a_offset * b_length) / spanned).clamp(0.0, 1.0)
	} else if a_length > 0.0 {
		// The stretch of `a` that lies beside `b` runs between where `b`'s two ends project onto it.
		let beside = [-a_offset, alignment - a_offset].map(|projection| (projection / a_length).clamp(0.0, 1.0));
		(beside[0] + beside[1]) * 0.5
	} else {
		// `a` is a single point.
		0.0
	};

	// The closest point on `b` to that point may be one of `b`'s ends, which in turn may be closer to another point
	// of `a`.
	let on_b = closest_point_on_segment(b[0], b[1], a[0] + a_along * share);
	(closest_point_on_segment(a[0], a[1], on_b), on_b)
}

/// Returns the point of the segment from `start` to `end` that is closest to `aabb`.
///
/// A segment that runs beside a face, or through the box, has a stretch of points that are all as close as each
/// other. Of the two ends and the middle of that stretch, this returns the one deepest inside the box, and the
/// middle when none is deeper than the others.
fn closest_point_on_segment_to_aabb<Space>(start: Point<Space>, end: Point<Space>, aabb: &AABB<Space>) -> Point<Space> {
	let (origin, along) = (start.to_array(), (end - start).to_array());
	let (min, max) = (aabb.min().to_array(), aabb.max().to_array());
	let at = |time: f32| start + (end - start) * time;

	// The times, from zero at `start` to one at `end`, at which the segment crosses the plane of a box face. Between
	// two neighboring times the segment stays on one side of every face.
	let mut times = [0.0f32; 8];
	let mut count = 1;
	for axis in 0..3 {
		for face in [min[axis], max[axis]] {
			// A segment that runs along a plane never crosses it. The division then gives no time between zero and one.
			let time = (face - origin[axis]) / along[axis];
			if time > 0.0 && time < 1.0 {
				times[count] = time;
				count += 1;
			}
		}
	}
	times[count] = 1.0;
	let times = &mut times[..=count];
	times.sort_unstable_by(f32::total_cmp);

	// The distance to a box never falls again once it has started to grow along a line. So the closest point is in
	// the first stretch between two times where the distance stops falling.
	for stretch in times.windows(2) {
		let (from, to) = (stretch[0], stretch[1]);
		if to <= from {
			// The segment crosses two planes at once, through an edge or a corner of the box.
			continue;
		}
		// Half the rate at which the squared distance changes is `offset + rate * time` over this stretch. Only the
		// axes along which the segment is outside the box add to it, and the middle of the stretch tells which those
		// are without landing on a face.
		let middle = (from + to) * 0.5;
		let (mut offset, mut rate) = (0.0, 0.0);
		for axis in 0..3 {
			let position = origin[axis] + along[axis] * middle;
			let face = if position < min[axis] {
				min[axis]
			} else if position > max[axis] {
				max[axis]
			} else {
				continue;
			};
			offset += along[axis] * (origin[axis] - face);
			rate += along[axis] * along[axis];
		}

		if rate == 0.0 {
			// The distance stays the same over the whole stretch: the segment runs beside a face or inside the box.
			// How deep a point is inside the box is its distance to the nearest face plane, below zero outside.
			let depth = |time: f32| {
				(0..3)
					.map(|axis| {
						let position = origin[axis] + along[axis] * time;
						(position - min[axis]).min(max[axis] - position)
					})
					.fold(f32::INFINITY, f32::min)
			};
			let mut deepest = middle;
			for time in [from, to] {
				if depth(time) > depth(deepest) {
					deepest = time;
				}
			}
			return at(deepest);
		}

		let closest = -offset / rate;
		if closest < to {
			// A time before the stretch means the distance was already growing when the stretch started.
			return at(closest.max(from));
		}
	}
	end
}

/// Returns the entry and exit distances where `ray` intersects `sphere`.
pub fn ray_vs_sphere<Space>(ray: &Ray<Space>, sphere: &Sphere<Space>) -> Option<(f32, f32)> {
	let offset = sphere.center() - ray.origin();
	let projection = ray.direction().dot(offset);
	let perpendicular_squared = offset.length_squared() - projection * projection;
	let radius_squared = sphere.radius() * sphere.radius();
	if perpendicular_squared > radius_squared {
		return None;
	}

	let half_chord = (radius_squared - perpendicular_squared).max(0.0).sqrt();
	Some((projection - half_chord, projection + half_chord))
}

/// Returns the first sphere contact reached while both spheres move for `dt` seconds.
pub fn sphere_vs_sphere_dynamic<Space>(
	sphere_a: &Sphere<Space>,
	sphere_b: &Sphere<Space>,
	a_velocity: Vector<Space>,
	b_velocity: Vector<Space>,
	dt: f32,
) -> Option<DynamicIntersection<Space>> {
	let displacement = (a_velocity - b_velocity) * dt;
	let Ok((direction, travel_distance)) = displacement.normalize_with_length() else {
		return collision_at_time(sphere_a, sphere_b, a_velocity, b_velocity, 0.0);
	};
	if !travel_distance.is_finite() {
		return collision_at_time(sphere_a, sphere_b, a_velocity, b_velocity, 0.0);
	}
	let expanded = Sphere::new(sphere_b.center(), sphere_a.radius() + sphere_b.radius());
	let (entry, exit) = ray_vs_sphere(&Ray::new(sphere_a.center(), direction), &expanded)?;
	if exit < 0.0 {
		return None;
	}
	let fraction = (entry.max(0.0) / travel_distance).min(1.0);
	if entry > travel_distance {
		return None;
	}

	collision_at_time(sphere_a, sphere_b, a_velocity, b_velocity, fraction * dt)
}

fn collision_at_time<Space>(
	sphere_a: &Sphere<Space>,
	sphere_b: &Sphere<Space>,
	a_velocity: Vector<Space>,
	b_velocity: Vector<Space>,
	toi: f32,
) -> Option<DynamicIntersection<Space>> {
	let moved_a = Sphere::new(sphere_a.center() + a_velocity * toi, sphere_a.radius());
	let moved_b = Sphere::new(sphere_b.center() + b_velocity * toi, sphere_b.radius());
	if moved_a.center() == moved_b.center() {
		// Coincident dynamic centers have no geometric normal. Use relative motion directly instead
		// of creating and replacing the static contact's known fallback normal.
		let normal = (b_velocity - a_velocity)
			.normalized()
			.unwrap_or_else(|_| UnitVector::x_axis());
		return Some(DynamicIntersection {
			toi,
			contact: Intersection {
				normal,
				depth: (moved_a.radius() + moved_b.radius()).max(0.0),
				point_on_a: moved_a.center() + normal * moved_a.radius(),
				point_on_b: moved_b.center() - normal * moved_b.radius(),
			},
		});
	}

	Some(DynamicIntersection {
		toi,
		contact: sphere_vs_sphere(&moved_a, &moved_b)?,
	})
}

fn signed_axis(value: f32) -> f32 {
	if value < 0.0 { -1.0 } else { 1.0 }
}

fn axis_normal<Space>(axis: usize, sign: f32) -> UnitVector<Space> {
	// Scaling the whole axis by the sign keeps the direction unit length without a branch on the axis.
	let mut components = [0.0; 3];
	components[axis] = 1.0;
	UnitVector::unchecked(Vec3f::new(components[0] * sign, components[1] * sign, components[2] * sign))
}

#[cfg(test)]
mod tests {
	use super::*;
	use crate::{Point, UnitVector, WorldSpace, assert_float_eq_with_epsilon, assert_geometry_near};

	#[test]
	fn ray_aabb_intersection_handles_axis_aligned_diagonal_and_parallel_rays() {
		let aabb: AABB<WorldSpace> = AABB::new(Point::new(-0.5, -0.5, -0.5), Point::new(0.5, 0.5, 0.5));

		let from_above = Ray::new(Point::new(0.0, 2.0, 0.0), -UnitVector::y_axis());

		assert_eq!(ray_aabb_intersection(&from_above, &aabb), Some(1.5));
		let from_front = Ray::new(Point::new(0.0, 0.0, -2.0), UnitVector::z_axis());

		assert_eq!(ray_aabb_intersection(&from_front, &aabb), Some(1.5));
		let diagonal = Ray::new(Point::new(0.0, 1.0, -1.0), Vector::new(0.0, -1.0, 1.0).normalized().unwrap());

		assert_float_eq_with_epsilon!(ray_aabb_intersection(&diagonal, &aabb).unwrap(), 0.5_f32.sqrt(), 0.000001);

		let parallel_miss = Ray::new(Point::new(2.0, 2.0, 0.0), -UnitVector::y_axis());

		assert_eq!(ray_aabb_intersection(&parallel_miss, &aabb), None);
	}

	#[test]
	fn sphere_in_frustum_includes_touching_spheres() {
		let planes: [Plane<WorldSpace>; 6] = [
			Plane::new(UnitVector::x_axis(), 1.0),
			Plane::new(-UnitVector::x_axis(), 1.0),
			Plane::new(UnitVector::y_axis(), 1.0),
			Plane::new(-UnitVector::y_axis(), 1.0),
			Plane::new(UnitVector::z_axis(), 1.0),
			Plane::new(-UnitVector::z_axis(), 1.0),
		];

		assert!(sphere_in_frustum(&Sphere::new(Point::origin(), 1.0), &planes));
		assert!(!sphere_in_frustum(&Sphere::new(Point::new(5.0, 5.0, 5.0), 1.5), &planes));
		assert!(sphere_in_frustum(&Sphere::new(Point::new(2.0, 0.0, 0.0), 1.0), &planes));
	}

	#[test]
	fn coincident_spheres_produce_a_finite_stable_contact() {
		let first: Sphere<WorldSpace> = Sphere::new(Point::origin(), 1.0);
		let second = Sphere::new(Point::origin(), 1.0);
		let contact = sphere_vs_sphere(&first, &second).unwrap();

		assert_eq!(contact.normal(), UnitVector::x_axis());
		assert_eq!(contact.depth(), 2.0);
		assert_eq!(contact.point_on_a(), Point::new(1.0, 0.0, 0.0));
		assert_eq!(contact.point_on_b(), Point::new(-1.0, 0.0, 0.0));
	}

	#[test]
	fn aabb_contacts_use_stable_surface_points() {
		let first: AABB<WorldSpace> = AABB::from_center_and_half_extents(Point::origin(), Vector::new(1.0, 1.0, 1.0));
		let second = AABB::from_center_and_half_extents(Point::new(1.0, 0.0, 0.0), Vector::new(1.0, 1.0, 1.0));
		let aabb_contact = aabb_vs_aabb(&first, &second).unwrap();

		assert_eq!(aabb_contact.normal(), UnitVector::x_axis());
		assert_eq!(aabb_contact.depth(), 1.0);
		assert_eq!(aabb_contact.point_on_a(), Point::new(1.0, 0.0, 0.0));
		assert_eq!(aabb_contact.point_on_b(), Point::new(0.0, 0.0, 0.0));
	}

	#[test]
	fn aabb_contacts_face_the_other_box_along_the_shallowest_axis() {
		let first: AABB<WorldSpace> = AABB::from_center_and_half_extents(Point::origin(), Vector::new(1.0, 1.0, 1.0));
		let axes = [UnitVector::x_axis(), UnitVector::y_axis(), UnitVector::z_axis()];

		for (axis, direction) in axes.into_iter().enumerate() {
			for sign in [1.0, -1.0] {
				// The second box overlaps the first by a quarter along this axis and by more along the others.
				let mut center = [0.5, 0.5, 0.5];
				center[axis] = 1.75 * sign;
				let second = AABB::from_center_and_half_extents(Point::from_array(center), Vector::new(1.0, 1.0, 1.0));
				let contact = aabb_vs_aabb(&first, &second).unwrap();

				let mut on_first = [0.25, 0.25, 0.25];
				on_first[axis] = sign;
				let mut on_second = on_first;
				on_second[axis] = 0.75 * sign;
				assert_eq!(contact.normal(), if sign > 0.0 { direction } else { -direction });
				assert_eq!(contact.depth(), 0.25);
				assert_eq!(contact.point_on_a(), Point::from_array(on_first));
				assert_eq!(contact.point_on_b(), Point::from_array(on_second));
			}
		}
	}

	#[test]
	fn aabb_contacts_break_depth_ties_toward_x_then_y() {
		let first: AABB<WorldSpace> = AABB::from_center_and_half_extents(Point::origin(), Vector::new(1.0, 1.0, 1.0));
		let corner = AABB::from_center_and_half_extents(Point::new(1.5, 1.5, 1.5), Vector::new(1.0, 1.0, 1.0));
		let edge = AABB::from_center_and_half_extents(Point::new(0.0, 1.5, 1.5), Vector::new(1.0, 1.0, 1.0));

		assert_eq!(aabb_vs_aabb(&first, &corner).unwrap().normal(), UnitVector::x_axis());
		assert_eq!(aabb_vs_aabb(&first, &edge).unwrap().normal(), UnitVector::y_axis());
	}

	#[test]
	fn sphere_contacts_include_tangency() {
		let aabb: AABB<WorldSpace> = AABB::from_center_and_half_extents(Point::origin(), Vector::new(0.5, 0.5, 0.5));
		let surface_sphere = Sphere::new(Point::new(0.0, 0.9, 0.0), 0.5);
		let surface_contact = sphere_vs_aabb(&surface_sphere, &aabb).unwrap();

		assert_eq!(surface_contact.normal(), -UnitVector::y_axis());
		assert_eq!(surface_contact.point_on_b(), Point::new(0.0, 0.5, 0.0));
		assert_float_eq_with_epsilon!(surface_contact.depth(), 0.1, 0.000001);
	}

	#[test]
	fn sphere_contacts_inside_boxes_use_stable_surface_points() {
		let aabb: AABB<WorldSpace> = AABB::from_center_and_half_extents(Point::origin(), Vector::new(0.5, 0.5, 0.5));
		let inside_sphere = Sphere::new(Point::new(0.0, 0.49, 0.0), 0.5);
		let inside_contact = sphere_vs_aabb(&inside_sphere, &aabb).unwrap();

		assert_eq!(inside_contact.normal(), UnitVector::y_axis());
		assert_eq!(inside_contact.point_on_b(), Point::new(0.0, 0.5, 0.0));
		assert_float_eq_with_epsilon!(inside_contact.depth(), 0.51, 0.000001);
	}

	/// Returns a capsule that stands on the origin: 0.5 in radius and 2 tall, so its cap centers are at heights 0.5
	/// and 1.5.
	fn standing_capsule() -> Capsule<WorldSpace> {
		Capsule::new(Point::new(0.0, 0.5, 0.0), Point::new(0.0, 1.5, 0.0), 0.5)
	}

	#[test]
	fn a_sphere_beside_a_capsule_touches_its_side() {
		let sphere = Sphere::new(Point::new(0.9, 1.2, 0.0), 0.5);
		let contact = capsule_vs_sphere(&standing_capsule(), &sphere).unwrap();

		assert_eq!(contact.normal(), UnitVector::x_axis());
		assert_float_eq_with_epsilon!(contact.depth(), 0.1, 0.000001);
		assert_eq!(contact.point_on_a(), Point::new(0.5, 1.2, 0.0));
		assert_geometry_near!(contact.point_on_b(), Point::<WorldSpace>::new(0.4, 1.2, 0.0));
	}

	#[test]
	fn a_sphere_past_the_end_of_a_capsule_touches_its_round_cap() {
		let capsule = standing_capsule();
		let above = Sphere::new(Point::new(0.0, 2.4, 0.0), 0.5);
		let contact = capsule_vs_sphere(&capsule, &above).unwrap();

		assert_eq!(contact.normal(), UnitVector::y_axis());
		assert_float_eq_with_epsilon!(contact.depth(), 0.1, 0.000001);
		assert_eq!(contact.point_on_a(), Point::new(0.0, 2.0, 0.0));

		// A cylinder of the same size would reach this sphere with its rim. The cap curves away from it.
		let beside_the_cap = Sphere::new(Point::new(0.8, 2.3, 0.0), 0.5);
		assert_eq!(capsule_vs_sphere(&capsule, &beside_the_cap), None);
	}

	#[test]
	fn capsules_side_by_side_touch_in_the_middle_of_the_stretch_they_share() {
		let first = standing_capsule();
		// The second capsule stands half a unit higher, so the segments are side by side from heights 1 to 1.5.
		let second = Capsule::new(Point::new(0.8, 1.0, 0.0), Point::new(0.8, 2.0, 0.0), 0.5);
		let contact = capsule_vs_capsule(&first, &second).unwrap();

		assert_eq!(contact.normal(), UnitVector::x_axis());
		assert_float_eq_with_epsilon!(contact.depth(), 0.2, 0.000001);
		assert_geometry_near!(contact.point_on_a(), Point::<WorldSpace>::new(0.5, 1.25, 0.0));
		assert_geometry_near!(contact.point_on_b(), Point::<WorldSpace>::new(0.3, 1.25, 0.0));
		assert_eq!(capsule_vs_capsule(&second, &first).unwrap(), contact.swap());
	}

	#[test]
	fn crossed_capsules_touch_where_their_segments_pass_closest() {
		let upright = standing_capsule();
		// Lies across the upright capsule, 0.8 in front of it.
		let lying = Capsule::new(Point::new(-2.0, 1.0, 0.8), Point::new(2.0, 1.0, 0.8), 0.5);
		let contact = capsule_vs_capsule(&upright, &lying).unwrap();

		assert_eq!(contact.normal(), UnitVector::z_axis());
		assert_float_eq_with_epsilon!(contact.depth(), 0.2, 0.000001);
		assert_geometry_near!(contact.point_on_a(), Point::<WorldSpace>::new(0.0, 1.0, 0.5));
		assert_geometry_near!(contact.point_on_b(), Point::<WorldSpace>::new(0.0, 1.0, 0.3));
	}

	#[test]
	fn capsules_end_to_end_touch_at_their_caps() {
		let lower = standing_capsule();
		let upper = Capsule::new(Point::new(0.0, 2.4, 0.0), Point::new(0.0, 3.4, 0.0), 0.5);
		let contact = capsule_vs_capsule(&lower, &upper).unwrap();

		assert_eq!(contact.normal(), UnitVector::y_axis());
		assert_float_eq_with_epsilon!(contact.depth(), 0.1, 0.000001);
		assert_eq!(contact.point_on_a(), Point::new(0.0, 2.0, 0.0));

		let apart = Capsule::new(Point::new(0.0, 2.6, 0.0), Point::new(0.0, 3.6, 0.0), 0.5);
		assert_eq!(capsule_vs_capsule(&lower, &apart), None);
	}

	#[test]
	fn a_capsule_standing_on_a_box_touches_it_under_its_lower_cap() {
		let floor: AABB<WorldSpace> = AABB::new(Point::new(-5.0, -1.0, -5.0), Point::new(5.0, 0.1, 5.0));
		let contact = capsule_vs_aabb(&standing_capsule(), &floor).unwrap();

		assert_eq!(contact.normal(), -UnitVector::y_axis());
		assert_float_eq_with_epsilon!(contact.depth(), 0.1, 0.000001);
		assert_eq!(contact.point_on_a(), Point::new(0.0, 0.0, 0.0));
		assert_eq!(contact.point_on_b(), Point::new(0.0, 0.1, 0.0));
	}

	#[test]
	fn a_capsule_beside_a_box_touches_it_with_its_side_and_misses_it_past_a_corner() {
		let capsule = standing_capsule();
		// A wall whose near face is 0.4 from the capsule's segment, from height 1 up.
		let wall = AABB::new(Point::new(0.4, 1.0, -5.0), Point::new(1.0, 5.0, 5.0));
		let contact = capsule_vs_aabb(&capsule, &wall).unwrap();

		assert_eq!(contact.normal(), UnitVector::x_axis());
		assert_float_eq_with_epsilon!(contact.depth(), 0.1, 0.000001);
		// The segment runs beside the wall from heights 1 to 1.5.
		assert_geometry_near!(contact.point_on_b(), Point::<WorldSpace>::new(0.4, 1.25, 0.0));

		// The bounds of the capsule and this box overlap, but the cap curves away from the box's edge.
		let past_the_cap = AABB::new(Point::new(0.4, 1.9, -5.0), Point::new(1.0, 5.0, 5.0));
		assert_eq!(capsule_vs_aabb(&capsule, &past_the_cap), None);
	}

	#[test]
	fn a_capsule_lying_on_a_box_touches_it_in_the_middle_of_the_stretch_above_it() {
		// The capsule sticks out past the box's edge at x = 1, so only the stretch from x = -1 to 1 is above the box.
		let lying: Capsule<WorldSpace> = Capsule::new(Point::new(-1.0, 0.4, 0.0), Point::new(3.0, 0.4, 0.0), 0.5);
		let floor = AABB::new(Point::new(-5.0, -1.0, -5.0), Point::new(1.0, 0.0, 5.0));
		let contact = capsule_vs_aabb(&lying, &floor).unwrap();

		assert_eq!(contact.normal(), -UnitVector::y_axis());
		assert_float_eq_with_epsilon!(contact.depth(), 0.1, 0.000001);
		assert_geometry_near!(contact.point_on_b(), Point::<WorldSpace>::new(0.0, 0.0, 0.0));
	}

	#[test]
	fn a_capsule_leaning_toward_a_box_touches_it_with_its_nearest_end() {
		// The lower end is the closest point of the segment to the floor, and the upper end leans away over it.
		let leaning: Capsule<WorldSpace> = Capsule::new(Point::new(0.0, 0.4, 0.0), Point::new(1.0, 2.0, 0.0), 0.5);
		let floor = AABB::new(Point::new(-5.0, -1.0, -5.0), Point::new(5.0, 0.0, 5.0));
		let contact = capsule_vs_aabb(&leaning, &floor).unwrap();

		assert_eq!(contact.normal(), -UnitVector::y_axis());
		assert_float_eq_with_epsilon!(contact.depth(), 0.1, 0.000001);
		assert_eq!(contact.point_on_b(), Point::new(0.0, 0.0, 0.0));
	}

	#[test]
	fn a_capsule_with_an_end_inside_a_box_leaves_by_the_face_nearest_to_that_end() {
		let floor: AABB<WorldSpace> = AABB::new(Point::new(-5.0, -4.0, -5.0), Point::new(5.0, 0.0, 5.0));
		// The lower cap center is 0.3 under the floor's top, and the upper one is above it.
		let sunk = Capsule::new(Point::new(0.0, -0.3, 0.0), Point::new(0.0, 0.7, 0.0), 0.5);
		let contact = capsule_vs_aabb(&sunk, &floor).unwrap();

		// Moving the capsule up by the depth leaves it standing on the floor's top.
		assert_float_eq_with_epsilon!(contact.depth(), 0.8, 0.000001);
		assert_geometry_near!(contact.point_on_b(), Point::<WorldSpace>::new(0.0, 0.0, 0.0));
	}

	#[test]
	fn a_capsule_that_passes_through_a_box_leaves_by_the_face_nearest_to_the_middle_of_the_part_inside() {
		let slab: AABB<WorldSpace> = AABB::new(Point::new(-5.0, 0.1, -5.0), Point::new(5.0, 0.3, 5.0));
		// Both cap centers are outside the slab, and the segment between them crosses it from one side to the other.
		let through = Capsule::new(Point::new(0.0, -0.3, 0.0), Point::new(0.0, 0.7, 0.0), 0.5);
		let contact = capsule_vs_aabb(&through, &slab).unwrap();

		// The middle of the part inside is 0.1 from both the top and the bottom of the slab.
		assert_float_eq_with_epsilon!(contact.depth(), 0.6, 0.000001);
	}

	#[test]
	fn ray_vs_sphere_returns_entry_and_exit_distances() {
		let ray: Ray<WorldSpace> = Ray::new(Point::origin(), UnitVector::z_axis());
		let sphere = Sphere::new(Point::new(0.0, 0.0, 10.0), 1.0);

		assert_eq!(ray_vs_sphere(&ray, &sphere), Some((9.0, 11.0)));

		let miss = Sphere::new(Point::new(0.0, 4.0, 10.0), 1.0);

		assert_eq!(ray_vs_sphere(&ray, &miss), None);
	}

	#[test]
	fn dynamic_spheres_report_tangent_contact() {
		let first: Sphere<WorldSpace> = Sphere::new(Point::origin(), 1.0);
		let second = Sphere::new(Point::new(4.0, 0.0, 0.0), 1.0);
		let hit =
			sphere_vs_sphere_dynamic(&first, &second, Vector::new(1.0, 0.0, 0.0), Vector::new(-1.0, 0.0, 0.0), 2.0).unwrap();

		assert_float_eq_with_epsilon!(hit.toi(), 1.0, 0.000001);
		assert_float_eq_with_epsilon!(hit.contact().depth(), 0.0, 0.000001);
		assert_eq!(hit.contact().normal(), UnitVector::x_axis());
	}

	#[test]
	fn dynamic_spheres_report_initial_overlap() {
		let first: Sphere<WorldSpace> = Sphere::new(Point::origin(), 1.0);
		let overlapping = Sphere::new(Point::new(1.5, 0.0, 0.0), 1.0);
		let hit = sphere_vs_sphere_dynamic(&first, &overlapping, Vector::zero(), Vector::zero(), 1.0).unwrap();

		assert_eq!(hit.toi(), 0.0);
		assert_float_eq_with_epsilon!(hit.contact().depth(), 0.5, 0.000001);
		assert_eq!(hit.contact().normal(), UnitVector::x_axis());
	}

	#[test]
	fn dynamic_coincident_spheres_use_a_stable_contact() {
		let coincident: Sphere<WorldSpace> = Sphere::new(Point::origin(), 1.0);
		let hit = sphere_vs_sphere_dynamic(&coincident, &coincident, Vector::new(1.0, 0.0, 0.0), Vector::zero(), 1.0).unwrap();

		assert_eq!(hit.toi(), 0.0);
		assert_eq!(hit.contact().normal(), -UnitVector::x_axis());
		assert_eq!(hit.contact().depth(), 2.0);
		assert_eq!(hit.contact().point_on_a(), Point::new(-1.0, 0.0, 0.0));
		assert_eq!(hit.contact().point_on_b(), Point::new(1.0, 0.0, 0.0));
	}
}
