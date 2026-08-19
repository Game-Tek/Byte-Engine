use crate::{Plane, Point, Ray, Sphere, UnitVector, Vector, WorldSpace, AABB};

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
	let a_min = a.min();
	let a_max = a.max();
	let b_min = b.min();
	let b_max = b.max();
	// Keep the overlap endpoints for both the separating-axis test and contact points.
	let overlap_min_x = a_min.x().max(b_min.x());
	let overlap_max_x = a_max.x().min(b_max.x());
	let overlap_min_y = a_min.y().max(b_min.y());
	let overlap_max_y = a_max.y().min(b_max.y());
	let overlap_min_z = a_min.z().max(b_min.z());
	let overlap_max_z = a_max.z().min(b_max.z());
	let overlap_x = overlap_max_x - overlap_min_x;
	let overlap_y = overlap_max_y - overlap_min_y;
	let overlap_z = overlap_max_z - overlap_min_z;
	if overlap_x < 0.0 || overlap_y < 0.0 || overlap_z < 0.0 {
		return None;
	}

	// Choose the shallowest axis, which is the minimum translation direction.
	let (axis, depth) = if overlap_y < overlap_x && overlap_y <= overlap_z {
		(1, overlap_y)
	} else if overlap_z < overlap_x && overlap_z < overlap_y {
		(2, overlap_z)
	} else {
		(0, overlap_x)
	};
	// Only the selected axis contributes to the normal's orientation.
	let sign = match axis {
		0 => signed_axis((b_min.x() + (b_max.x() - b_min.x()) * 0.5) - (a_min.x() + (a_max.x() - a_min.x()) * 0.5)),
		1 => signed_axis((b_min.y() + (b_max.y() - b_min.y()) * 0.5) - (a_min.y() + (a_max.y() - a_min.y()) * 0.5)),
		2 => signed_axis((b_min.z() + (b_max.z() - b_min.z()) * 0.5) - (a_min.z() + (a_max.z() - a_min.z()) * 0.5)),
		_ => unreachable!(),
	};
	let normal = axis_normal(axis, sign);
	let overlap_center: Point<Space> = Point::new(
		(overlap_min_x + overlap_max_x) * 0.5,
		(overlap_min_y + overlap_max_y) * 0.5,
		(overlap_min_z + overlap_max_z) * 0.5,
	);
	let (point_on_a, point_on_b) = match axis {
		0 => (
			Point::new(
				if sign > 0.0 { a_max.x() } else { a_min.x() },
				overlap_center.y(),
				overlap_center.z(),
			),
			Point::new(
				if sign > 0.0 { b_min.x() } else { b_max.x() },
				overlap_center.y(),
				overlap_center.z(),
			),
		),
		1 => (
			Point::new(
				overlap_center.x(),
				if sign > 0.0 { a_max.y() } else { a_min.y() },
				overlap_center.z(),
			),
			Point::new(
				overlap_center.x(),
				if sign > 0.0 { b_min.y() } else { b_max.y() },
				overlap_center.z(),
			),
		),
		2 => (
			Point::new(
				overlap_center.x(),
				overlap_center.y(),
				if sign > 0.0 { a_max.z() } else { a_min.z() },
			),
			Point::new(
				overlap_center.x(),
				overlap_center.y(),
				if sign > 0.0 { b_min.z() } else { b_max.z() },
			),
		),
		_ => unreachable!(),
	};

	Some(Intersection {
		normal,
		depth,
		point_on_a,
		point_on_b,
	})
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
	if value < 0.0 {
		-1.0
	} else {
		1.0
	}
}

fn axis_normal<Space>(axis: usize, sign: f32) -> UnitVector<Space> {
	match axis {
		0 if sign < 0.0 => -UnitVector::x_axis(),
		0 => UnitVector::x_axis(),
		1 if sign < 0.0 => -UnitVector::y_axis(),
		1 => UnitVector::y_axis(),
		2 if sign < 0.0 => -UnitVector::z_axis(),
		2 => UnitVector::z_axis(),
		_ => unreachable!(),
	}
}

#[cfg(test)]
mod tests {
	use super::*;
	use crate::{assert_float_eq_with_epsilon, Point, UnitVector, WorldSpace};

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
	fn aabb_and_sphere_contacts_include_tangency_and_use_stable_surface_points() {
		let first: AABB<WorldSpace> = AABB::from_center_and_half_extents(Point::origin(), Vector::new(1.0, 1.0, 1.0));
		let second = AABB::from_center_and_half_extents(Point::new(1.0, 0.0, 0.0), Vector::new(1.0, 1.0, 1.0));
		let aabb_contact = aabb_vs_aabb(&first, &second).unwrap();

		assert_eq!(aabb_contact.normal(), UnitVector::x_axis());
		assert_eq!(aabb_contact.depth(), 1.0);
		assert_eq!(aabb_contact.point_on_a(), Point::new(1.0, 0.0, 0.0));
		assert_eq!(aabb_contact.point_on_b(), Point::new(0.0, 0.0, 0.0));

		let aabb: AABB<WorldSpace> = AABB::from_center_and_half_extents(Point::origin(), Vector::new(0.5, 0.5, 0.5));
		let surface_sphere = Sphere::new(Point::new(0.0, 0.9, 0.0), 0.5);
		let surface_contact = sphere_vs_aabb(&surface_sphere, &aabb).unwrap();

		assert_eq!(surface_contact.normal(), -UnitVector::y_axis());
		assert_eq!(surface_contact.point_on_b(), Point::new(0.0, 0.5, 0.0));
		assert_float_eq_with_epsilon!(surface_contact.depth(), 0.1, 0.000001);

		let inside_sphere = Sphere::new(Point::new(0.0, 0.49, 0.0), 0.5);
		let inside_contact = sphere_vs_aabb(&inside_sphere, &aabb).unwrap();

		assert_eq!(inside_contact.normal(), UnitVector::y_axis());
		assert_eq!(inside_contact.point_on_b(), Point::new(0.0, 0.5, 0.0));
		assert_float_eq_with_epsilon!(inside_contact.depth(), 0.51, 0.000001);
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
	fn dynamic_spheres_detect_approach_and_report_initial_overlap() {
		let first: Sphere<WorldSpace> = Sphere::new(Point::new(-2.0, 0.0, 0.0), 1.0);
		let second = Sphere::new(Point::new(2.0, 0.0, 0.0), 1.0);
		let hit = sphere_vs_sphere_dynamic(&first, &second, Vector::new(3.0, 0.0, 0.0), Vector::zero(), 1.0).unwrap();

		assert_float_eq_with_epsilon!(hit.toi(), 2.0 / 3.0, 0.0001);

		let first: Sphere<WorldSpace> = Sphere::new(Point::origin(), 1.0);
		let second = Sphere::new(Point::new(4.0, 0.0, 0.0), 1.0);
		let hit =
			sphere_vs_sphere_dynamic(&first, &second, Vector::new(1.0, 0.0, 0.0), Vector::new(-1.0, 0.0, 0.0), 2.0).unwrap();

		assert_float_eq_with_epsilon!(hit.toi(), 1.0, 0.000001);
		assert_float_eq_with_epsilon!(hit.contact().depth(), 0.0, 0.000001);
		assert_eq!(hit.contact().normal(), UnitVector::x_axis());

		let overlapping = Sphere::new(Point::new(1.5, 0.0, 0.0), 1.0);
		let hit = sphere_vs_sphere_dynamic(&first, &overlapping, Vector::zero(), Vector::zero(), 1.0).unwrap();

		assert_eq!(hit.toi(), 0.0);
		assert_float_eq_with_epsilon!(hit.contact().depth(), 0.5, 0.000001);
		assert_eq!(hit.contact().normal(), UnitVector::x_axis());

		let coincident: Sphere<WorldSpace> = Sphere::new(Point::origin(), 1.0);
		let hit = sphere_vs_sphere_dynamic(&coincident, &coincident, Vector::new(1.0, 0.0, 0.0), Vector::zero(), 1.0).unwrap();

		assert_eq!(hit.toi(), 0.0);
		assert_eq!(hit.contact().normal(), -UnitVector::x_axis());
		assert_eq!(hit.contact().depth(), 2.0);
		assert_eq!(hit.contact().point_on_a(), Point::new(-1.0, 0.0, 0.0));
		assert_eq!(hit.contact().point_on_b(), Point::new(1.0, 0.0, 0.0));
	}
}
