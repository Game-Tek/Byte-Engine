use maths_rs::Vec3f;

use crate::{AABB, Capsule, OrientedBox, Plane, Point, Ray, Sphere, UnitVector, Vector, WorldSpace};

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
	/// Creates a contact from the unit `normal` from shape A toward shape B, the overlap `depth`, and the contact
	/// point on each shape.
	///
	/// The point on shape A is where it reaches furthest into shape B, and the other way around, so the two are
	/// `depth` apart along `normal`.
	pub fn new(normal: UnitVector<Space>, depth: f32, point_on_a: Point<Space>, point_on_b: Point<Space>) -> Self {
		Self {
			normal,
			depth,
			point_on_a,
			point_on_b,
		}
	}

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
// Always inlined because physics tests every pair of boxes with it, from a function that tests every other pair of
// shapes too and is large enough that the compiler would leave this as a call. With that call, physics steps of box
// piles were 3% to 4% slower on an Apple M4.
#[inline(always)]
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
	let axis = parallel_axis(a_along.to_array());
	if let (Some(axis), true) = (axis, axis == parallel_axis(b_along.to_array())) {
		// Both segments run along one axis, as those of upright capsules do, so only their coordinates on that axis
		// decide where they are closest: where they are side by side, or else at their facing ends.
		let reach = |segment: [Point<Space>; 2]| {
			let (first, second) = (segment[0].to_array()[axis], segment[1].to_array()[axis]);
			(first.min(second), first.max(second))
		};
		let ((a_low, a_high), (b_low, b_high)) = (reach(a), reach(b));
		let (low, high) = (a_low.max(b_low), a_high.min(b_high));
		let (on_a, on_b) = if low <= high {
			let middle = (low + high) * 0.5;
			(middle, middle)
		} else if a_low > b_high {
			(a_low, b_high)
		} else {
			(a_high, b_low)
		};
		let place = |point: Point<Space>, coordinate: f32| {
			let mut point = point.to_array();
			point[axis] = coordinate;
			Point::from_array(point)
		};
		return (place(a[0], on_a), place(b[0], on_b));
	}

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

/// Returns the axis that a segment with direction `along` runs along, when it has length and runs along one.
fn parallel_axis(along: [f32; 3]) -> Option<usize> {
	(0..3).find(|axis| along[*axis] != 0.0 && (0..3).all(|other| other == *axis || along[other] == 0.0))
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
	// How deep a point is inside the box is its distance to the nearest face plane, below zero outside.
	let depth = |time: f32| {
		(0..3)
			.map(|axis| {
				let position = origin[axis] + along[axis] * time;
				(position - min[axis]).min(max[axis] - position)
			})
			.fold(f32::INFINITY, f32::min)
	};
	// Returns the deepest of the two ends and the middle of a stretch, and the middle when none is deeper.
	let deepest = |from: f32, to: f32| {
		let mut deepest = (from + to) * 0.5;
		for time in [from, to] {
			if depth(time) > depth(deepest) {
				deepest = time;
			}
		}
		deepest
	};

	if let Some(axis) = parallel_axis(along) {
		// The segment runs along one axis, as that of an upright capsule does, so its distance to the box only
		// changes with its coordinate on that axis. It stays the same between the two faces across the axis, and
		// past them the nearer end is closest.
		let crossings = [min[axis], max[axis]].map(|face| (face - origin[axis]) / along[axis]);
		let (enters, leaves) = (crossings[0].min(crossings[1]), crossings[0].max(crossings[1]));
		let (from, to) = (enters.max(0.0), leaves.min(1.0));
		return if from <= to {
			at(deepest(from, to))
		} else if leaves < 0.0 {
			start
		} else {
			end
		};
	}

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
			return at(deepest(from, to));
		}

		let closest = -offset / rate;
		if closest < to {
			// A time before the stretch means the distance was already growing when the stretch started.
			return at(closest.max(from));
		}
	}
	end
}

/// Returns the contact between two boxes that are turned any way and overlap or touch.
///
/// The normal is the direction in which the boxes have the shortest way out of each other, which is either a
/// direction that a side of one of them faces, or one square to an edge of each, when two edges cross. The contact
/// points are in the middle of the part of each box that is deepest in the other: a corner, the middle of an edge or
/// of a side, or where the two edges that cross are closest. Boxes that rest on each other touch over a whole patch,
/// of which these are one pair of points.
pub fn obb_vs_obb<Space>(a: &OrientedBox<Space>, b: &OrientedBox<Space>) -> Option<Intersection<Space>> {
	obb_overlap(a, b).map(|overlap| overlap.contact(a, b))
}

/// The `ObbOverlap` struct holds the direction two oriented boxes overlap least along and how much, for code that
/// builds its own contact points from that, as the patch two boxes touch over is built.
///
/// Get one from [`obb_overlap`]. [`Self::contact`] returns the contact [`obb_vs_obb`] would.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ObbOverlap<Space = WorldSpace> {
	normal: UnitVector<Space>,
	depth: f32,
	separation: ObbSeparation,
}

impl<Space> ObbOverlap<Space> {
	/// Returns the unit normal from the first box toward the second.
	pub fn normal(&self) -> UnitVector<Space> {
		self.normal
	}

	/// Returns how far the boxes overlap along the normal.
	pub fn depth(&self) -> f32 {
		self.depth
	}

	/// Returns what the boxes touch with: a side of one of them, or an edge of each.
	pub fn separation(&self) -> ObbSeparation {
		self.separation
	}

	/// Returns the contact between `a` and `b`, which have to be the boxes this overlap was found for.
	pub fn contact(&self, a: &OrientedBox<Space>, b: &OrientedBox<Space>) -> Intersection<Space> {
		let (normal, depth) = (self.normal, self.depth);
		match self.separation {
			ObbSeparation::SideOfA(_) => {
				let point_on_b = b.furthest_middle(-normal.into_vector());
				Intersection {
					normal,
					depth,
					point_on_a: point_on_b + normal * depth,
					point_on_b,
				}
			}
			ObbSeparation::SideOfB(_) => {
				let point_on_a = a.furthest_middle(normal.into_vector());
				Intersection {
					normal,
					depth,
					point_on_a,
					point_on_b: point_on_a - normal * depth,
				}
			}
			ObbSeparation::Edges(i, j) => {
				let (a_axis, b_axis) = (a.axes()[i].into_vector(), b.axes()[j].into_vector());
				// The two edges that cross: on each box, the edge along that axis that is furthest toward the other
				// box. Leaving the axis out of the direction gives the middle of that edge.
				let across = |axis: Vector<Space>, toward: Vector<Space>| toward - axis * axis.dot(toward);
				let a_middle = a.furthest_middle(across(a_axis, normal.into_vector()));
				let b_middle = b.furthest_middle(across(b_axis, -normal.into_vector()));
				// Where the lines through the two edges pass closest, as distances along each from its middle, kept
				// within the edge.
				let apart = b_middle - a_middle;
				let (along_a, along_b) = (apart.dot(a_axis), apart.dot(b_axis));
				let cosine = a_axis.dot(b_axis);
				let (a_half, b_half) = (a.half_extents()[i], b.half_extents()[j]);
				let on_a = ((along_a - cosine * along_b) / length_squared_of(cosine)).clamp(-a_half, a_half);
				let on_b = ((cosine * along_a - along_b) / length_squared_of(cosine)).clamp(-b_half, b_half);
				Intersection {
					normal,
					depth,
					point_on_a: a_middle + a_axis * on_a,
					point_on_b: b_middle + b_axis * on_b,
				}
			}
		}
	}
}

/// Returns the direction two oriented boxes overlap least along and how much, or `None` when they are apart.
///
/// This is all of [`obb_vs_obb`] but its contact points. Use it to find the whole patch that two boxes touch over,
/// which starts from the side that faces the other box.
pub fn obb_overlap<Space>(a: &OrientedBox<Space>, b: &OrientedBox<Space>) -> Option<ObbOverlap<Space>> {
	let (a_axes, b_axes) = (a.axes().map(UnitVector::into_vector), b.axes().map(UnitVector::into_vector));
	let (a_half, b_half) = (a.half_extents(), b.half_extents());
	let between = b.center() - a.center();

	// Two boxes are apart exactly when some direction has a gap between how far they reach along it, and only
	// fifteen directions have to be tried: the three that the sides of each box face, and the nine that are square
	// to one edge of each. Everything is worked out in the first box's own frame, where its axes are the coordinate
	// axes: `turn` holds the second box's axes there, and `offset` the way to its center.
	let turn: [[f32; 3]; 3] = std::array::from_fn(|i| std::array::from_fn(|j| a_axes[i].dot(b_axes[j])));
	let spread = turn.map(|row| row.map(f32::abs));
	let offset: [f32; 3] = std::array::from_fn(|i| between.dot(a_axes[i]));

	// The direction with the least overlap is the shortest way out. Each is kept with how far the second box's
	// center is along it, whose sign tells which way along it leads from the first box to the second.
	// A direction only replaces an earlier one when it overlaps less by more than rounding. Boxes that rest side on
	// side overlap as little along the direction either side faces, and the earlier one is kept from step to step.
	let clearly_under = |least: f32| least - 1e-5 * (1.0 + least);
	let mut side: Option<(f32, f32, ObbSeparation)> = None;
	let mut shallowest_side = |depth: f32, along: f32, separation: ObbSeparation| {
		if side.is_none_or(|(least, ..)| depth < clearly_under(least)) {
			side = Some((depth, along, separation));
		}
	};

	for i in 0..3 {
		let reach = a_half[i] + (0..3).map(|j| b_half[j] * spread[i][j]).sum::<f32>();
		let depth = reach - offset[i].abs();
		if depth < 0.0 {
			return None;
		}
		shallowest_side(depth, offset[i], ObbSeparation::SideOfA(i));
	}
	for j in 0..3 {
		let along: f32 = (0..3).map(|i| offset[i] * turn[i][j]).sum();
		let reach = (0..3).map(|i| a_half[i] * spread[i][j]).sum::<f32>() + b_half[j];
		let depth = reach - along.abs();
		if depth < 0.0 {
			return None;
		}
		shallowest_side(depth, along, ObbSeparation::SideOfB(j));
	}
	let side = side?;

	// The cross product of two edges is square to both, and as long as the sine of the angle between them, which
	// the overlap along it has to be divided by. The directions are compared before that division, by their squares,
	// so that only one that turns out to be the least pays for a square root. A direction between two edges has to
	// overlap clearly less than the sides do to count at all: where sides face each other, one often overlaps as
	// little as the sides' own direction, give or take rounding, and the sides' direction is kept then, so that boxes
	// that rest on each other do not get a normal that flickers between the two.
	let mut least = side.0 - 1e-4 * (1.0 + side.0);
	let mut crossing: Option<(f32, f32, ObbSeparation)> = None;
	for i in 0..3 {
		let (i1, i2) = ((i + 1) % 3, (i + 2) % 3);
		for j in 0..3 {
			let (j1, j2) = ((j + 1) % 3, (j + 2) % 3);
			// Edges that run the same way have no direction square to both that the sides do not already cover.
			let length_squared = length_squared_of(turn[i][j]);
			if length_squared < 1e-6 {
				continue;
			}
			let along = offset[i2] * turn[i1][j] - offset[i1] * turn[i2][j];
			let reach = a_half[i1] * spread[i2][j]
				+ a_half[i2] * spread[i1][j]
				+ b_half[j1] * spread[i][j2]
				+ b_half[j2] * spread[i][j1];
			let overlap = reach - along.abs();
			if overlap < 0.0 {
				return None;
			}
			if least > 0.0 && overlap * overlap < least * least * length_squared {
				let depth = overlap / length_squared.sqrt();
				crossing = Some((depth, along, ObbSeparation::Edges(i, j)));
				least = clearly_under(depth);
			}
		}
	}

	let (depth, along, separation) = crossing.unwrap_or(side);
	let sign = signed_axis(along);
	let normal = match separation {
		// A box's axis is of unit length either way around.
		ObbSeparation::SideOfA(i) => UnitVector::unchecked((a.axes()[i] * sign).into_maths()),
		ObbSeparation::SideOfB(j) => UnitVector::unchecked((b.axes()[j] * sign).into_maths()),
		ObbSeparation::Edges(i, j) => (a_axes[i].cross(b_axes[j]) * sign).normalized().ok()?,
	};
	Some(ObbOverlap {
		normal,
		depth,
		separation,
	})
}

/// The `ObbSeparation` enum names the direction two oriented boxes overlap least along, which says what they touch
/// with: a side of one box, or an edge of each.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ObbSeparation {
	/// The direction that a pair of the first box's sides faces, by the index of its axis.
	SideOfA(usize),
	/// The direction that a pair of the second box's sides faces, by the index of its axis.
	SideOfB(usize),
	/// The direction square to an edge of each box, by the indices of the axes the edges run along.
	Edges(usize, usize),
}

/// Returns the squared length of the cross product of two unit directions, from the cosine of the angle between them.
fn length_squared_of(cosine: f32) -> f32 {
	1.0 - cosine * cosine
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
	use crate::{Point, UnitVector, WorldSpace, assert_float_eq, assert_float_eq_with_epsilon, assert_geometry_near};

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

	/// Returns a capsule that stands on the origin: 0.5 in radius and 2 tall, so its cap centers are at heights 0.5
	/// and 1.5.
	fn standing_capsule() -> Capsule<WorldSpace> {
		Capsule::new(Point::new(0.0, 0.5, 0.0), Point::new(0.0, 1.5, 0.0), 0.5)
	}

	/// Asserts that `contact` has the expected normal, depth, and points, to within a hundred-thousandth, leaving out
	/// what is `None`.
	fn assert_contact(
		case: &str,
		contact: Option<Intersection<WorldSpace>>,
		normal: Option<[f32; 3]>,
		depth: f32,
		[on_a, on_b]: [Option<[f32; 3]>; 2],
	) {
		let contact = contact.unwrap_or_else(|| panic!("{case}: the shapes touch"));
		let near = |found: [f32; 3], expected: [f32; 3]| {
			found
				.iter()
				.zip(expected)
				.all(|(found, expected)| (found - expected).abs() <= 1e-5)
		};
		assert!(
			normal.is_none_or(|normal| near(contact.normal().into_vector().to_array(), normal)),
			"{case}: {contact:?}"
		);
		assert!((contact.depth() - depth).abs() <= 1e-5, "{case}: {contact:?}");
		assert!(
			on_a.is_none_or(|on_a| near(contact.point_on_a().to_array(), on_a)),
			"{case}: {contact:?}"
		);
		assert!(
			on_b.is_none_or(|on_b| near(contact.point_on_b().to_array(), on_b)),
			"{case}: {contact:?}"
		);
	}

	#[test]
	fn touching_shapes_get_the_contact_of_their_nearest_parts() {
		let unit_box = |center: [f32; 3], half: f32| {
			AABB::<WorldSpace>::from_center_and_half_extents(Point::from_array(center), Vector::new(half, half, half))
		};
		let span = |min: [f32; 3], max: [f32; 3]| AABB::<WorldSpace>::new(Point::from_array(min), Point::from_array(max));
		let capsule =
			|start: [f32; 3], end: [f32; 3]| Capsule::<WorldSpace>::new(Point::from_array(start), Point::from_array(end), 0.5);
		let sphere = |center: [f32; 3], radius: f32| Sphere::<WorldSpace>::new(Point::from_array(center), radius);
		let standing = standing_capsule();
		// A floor whose top is at height 0.1 or 0, a wall whose near face is 0.4 from the standing capsule's segment from
		// height 1 up, a capsule that stands half a unit higher beside the standing one, so their segments are side by
		// side from heights 1 to 1.5, and capsules lying across, above, leaning, sunk 0.3 under a floor's top, and through
		// a slab whose top and bottom are both 0.1 from the middle of the part inside.
		let (floor, low_floor, short_floor) = (
			span([-5.0, -1.0, -5.0], [5.0, 0.1, 5.0]),
			span([-5.0, -1.0, -5.0], [5.0, 0.0, 5.0]),
			span([-5.0, -1.0, -5.0], [1.0, 0.0, 5.0]),
		);
		let wall = span([0.4, 1.0, -5.0], [1.0, 5.0, 5.0]);
		let sunk = capsule([0.0, -0.3, 0.0], [0.0, 0.7, 0.0]);
		let (x, y, z) = (Some([1.0, 0.0, 0.0]), Some([0.0, 1.0, 0.0]), Some([0.0, 0.0, 1.0]));
		let down = Some([0.0, -1.0, 0.0]);
		let cases = [
			// Coincident centers have no direction between them, so they take +x.
			(
				"coincident spheres",
				sphere_vs_sphere(&sphere([0.0; 3], 1.0), &sphere([0.0; 3], 1.0)),
				x,
				2.0,
				[Some([1.0, 0.0, 0.0]), Some([-1.0, 0.0, 0.0])],
			),
			(
				"overlapping boxes",
				aabb_vs_aabb(&unit_box([0.0; 3], 1.0), &unit_box([1.0, 0.0, 0.0], 1.0)),
				x,
				1.0,
				[Some([1.0, 0.0, 0.0]), Some([0.0; 3])],
			),
			(
				"a sphere on a box",
				sphere_vs_aabb(&sphere([0.0, 0.9, 0.0], 0.5), &unit_box([0.0; 3], 0.5)),
				down,
				0.1,
				[None, Some([0.0, 0.5, 0.0])],
			),
			(
				"a sphere in a box",
				sphere_vs_aabb(&sphere([0.0, 0.49, 0.0], 0.5), &unit_box([0.0; 3], 0.5)),
				y,
				0.51,
				[None, Some([0.0, 0.5, 0.0])],
			),
			(
				"a sphere beside a capsule",
				capsule_vs_sphere(&standing, &sphere([0.9, 1.2, 0.0], 0.5)),
				x,
				0.1,
				[Some([0.5, 1.2, 0.0]), Some([0.4, 1.2, 0.0])],
			),
			(
				"a sphere on a capsule's cap",
				capsule_vs_sphere(&standing, &sphere([0.0, 2.4, 0.0], 0.5)),
				y,
				0.1,
				[Some([0.0, 2.0, 0.0]), None],
			),
			(
				"capsules side by side",
				capsule_vs_capsule(&standing, &capsule([0.8, 1.0, 0.0], [0.8, 2.0, 0.0])),
				x,
				0.2,
				[Some([0.5, 1.25, 0.0]), Some([0.3, 1.25, 0.0])],
			),
			(
				"crossed capsules",
				capsule_vs_capsule(&standing, &capsule([-2.0, 1.0, 0.8], [2.0, 1.0, 0.8])),
				z,
				0.2,
				[Some([0.0, 1.0, 0.5]), Some([0.0, 1.0, 0.3])],
			),
			(
				"capsules end to end",
				capsule_vs_capsule(&standing, &capsule([0.0, 2.4, 0.0], [0.0, 3.4, 0.0])),
				y,
				0.1,
				[Some([0.0, 2.0, 0.0]), None],
			),
			(
				"a capsule standing on a box",
				capsule_vs_aabb(&standing, &floor),
				down,
				0.1,
				[Some([0.0; 3]), Some([0.0, 0.1, 0.0])],
			),
			(
				"a capsule beside a box",
				capsule_vs_aabb(&standing, &wall),
				x,
				0.1,
				[None, Some([0.4, 1.25, 0.0])],
			),
			(
				"a capsule lying on a box's edge",
				capsule_vs_aabb(&capsule([-1.0, 0.4, 0.0], [3.0, 0.4, 0.0]), &short_floor),
				down,
				0.1,
				[None, Some([0.0; 3])],
			),
			(
				"a capsule leaning over a box",
				capsule_vs_aabb(&capsule([0.0, 0.4, 0.0], [1.0, 2.0, 0.0]), &low_floor),
				down,
				0.1,
				[None, Some([0.0; 3])],
			),
			// Moving the capsule up by the depth leaves it standing on the floor's top.
			(
				"a capsule with an end in a box",
				capsule_vs_aabb(&sunk, &span([-5.0, -4.0, -5.0], [5.0, 0.0, 5.0])),
				None,
				0.8,
				[None, Some([0.0; 3])],
			),
			(
				"a capsule through a box",
				capsule_vs_aabb(&sunk, &span([-5.0, 0.1, -5.0], [5.0, 0.3, 5.0])),
				None,
				0.6,
				[None, None],
			),
		];
		for (case, contact, normal, depth, points) in cases {
			assert_contact(case, contact, normal, depth, points);
		}
		assert_eq!(
			capsule_vs_capsule(&capsule([0.8, 1.0, 0.0], [0.8, 2.0, 0.0]), &standing),
			capsule_vs_capsule(&standing, &capsule([0.8, 1.0, 0.0], [0.8, 2.0, 0.0])).map(Intersection::swap)
		);

		// A cylinder of the same size would reach each of these with its rim. The cap curves away from them.
		assert_eq!(capsule_vs_sphere(&standing, &sphere([0.8, 2.3, 0.0], 0.5)), None);
		assert_eq!(
			capsule_vs_capsule(&standing, &capsule([0.0, 2.6, 0.0], [0.0, 3.6, 0.0])),
			None
		);
		assert_eq!(capsule_vs_aabb(&standing, &span([0.4, 1.9, -5.0], [1.0, 5.0, 5.0])), None);
	}

	#[test]
	fn a_barely_tilted_capsule_touches_like_an_upright_one() {
		// An upright capsule's segment runs along one axis, which has closed forms. A tilt too small to matter takes
		// the general search instead, and both have to agree. The tilt is along the wall below, so it brings neither
		// end closer to it.
		let tilted = |capsule: &Capsule<WorldSpace>| {
			Capsule::new(capsule.start(), capsule.end() + Vector::new(0.0, 0.0, 1e-4), capsule.radius())
		};
		let agree = |upright: Intersection<WorldSpace>, tilted: Intersection<WorldSpace>| {
			assert_float_eq!(upright.depth(), tilted.depth());
			assert_geometry_near!(upright.normal(), tilted.normal());
			assert_geometry_near!(upright.point_on_a(), tilted.point_on_a());
			assert_geometry_near!(upright.point_on_b(), tilted.point_on_b());
		};

		let standing = standing_capsule();
		let sunk = Capsule::new(Point::new(0.0, -0.3, 0.0), Point::new(0.0, 0.7, 0.0), 0.5);
		let floor = AABB::new(Point::new(-5.0, -4.0, -5.0), Point::new(5.0, 0.1, 5.0));
		let wall = AABB::new(Point::new(0.4, 1.0, -5.0), Point::new(1.0, 5.0, 5.0));
		let ceiling = AABB::new(Point::new(-5.0, 1.9, -5.0), Point::new(5.0, 4.0, 5.0));
		for (capsule, aabb) in [
			(&standing, &floor),
			(&standing, &wall),
			(&standing, &ceiling),
			(&sunk, &floor),
		] {
			agree(
				capsule_vs_aabb(capsule, aabb).unwrap(),
				capsule_vs_aabb(&tilted(capsule), aabb).unwrap(),
			);
		}

		let beside = Capsule::new(Point::new(0.8, 1.0, 0.0), Point::new(0.8, 2.0, 0.0), 0.5);
		let above = Capsule::new(Point::new(0.3, 2.4, 0.0), Point::new(0.3, 3.4, 0.0), 0.5);
		let below = Capsule::new(Point::new(0.3, -1.4, 0.0), Point::new(0.3, -0.4, 0.0), 0.5);
		for other in [&beside, &above, &below] {
			agree(
				capsule_vs_capsule(&standing, other).unwrap(),
				capsule_vs_capsule(&tilted(&standing), other).unwrap(),
			);
		}
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

	/// Returns a turn of `angle` radians about the axis `axis`.
	fn turned_about(axis: [f32; 3], angle: f32) -> crate::Orientation {
		let [x, y, z] = axis.map(|part| part * angle);
		crate::Orientation::try_from_rotation_vector(Vector::<WorldSpace>::new(x, y, z)).unwrap()
	}

	fn oriented(center: [f32; 3], half_extents: [f32; 3], orientation: crate::Orientation) -> OrientedBox<WorldSpace> {
		OrientedBox::new(Point::from_array(center), half_extents, orientation)
	}

	#[test]
	fn boxes_along_the_axes_get_the_contact_of_their_bounds() {
		let level = crate::Orientation::identity();
		let (a, b) = (
			oriented([0.0, 0.0, 0.0], [1.0, 1.0, 1.0], level),
			oriented([1.75, 0.5, -0.25], [1.0, 1.0, 1.0], level),
		);
		let expected = aabb_vs_aabb(
			&AABB::new(Point::new(-1.0, -1.0, -1.0), Point::new(1.0, 1.0, 1.0)),
			&AABB::new(Point::new(0.75, -0.5, -1.25), Point::new(2.75, 1.5, 0.75)),
		)
		.unwrap();

		let contact = obb_vs_obb(&a, &b).expect("the boxes overlap");

		assert!((contact.normal().into_vector() - expected.normal().into_vector()).length() <= 1e-6);
		assert_float_eq_with_epsilon!(contact.depth(), expected.depth(), 1e-6);
		// The points are on the sides that face each other, as far apart as the boxes are deep in each other.
		assert_float_eq_with_epsilon!(contact.point_on_a().x(), 1.0, 1e-6);
		assert_float_eq_with_epsilon!(contact.point_on_b().x(), 0.75, 1e-6);
		assert!(obb_vs_obb(&a, &oriented([2.1, 0.5, -0.25], [1.0, 1.0, 1.0], level)).is_none());

		// The other way around, the same contact is seen from the other side.
		let swapped = obb_vs_obb(&b, &a).expect("the boxes overlap");
		assert!((swapped.normal().into_vector() - -contact.normal().into_vector()).length() <= 1e-6);
		assert_float_eq_with_epsilon!(swapped.depth(), contact.depth(), 1e-6);
	}

	#[test]
	fn a_turned_box_touches_a_side_with_its_edge() {
		// A cube turned an eighth of a turn about z stands on an edge, the square root of two below its middle, which
		// is 0.1 into the top of a floor.
		let floor = oriented([0.0, -0.5, 0.0], [4.0, 0.5, 4.0], crate::Orientation::identity());
		let cube = oriented(
			[1.0, std::f32::consts::SQRT_2 - 0.1, 0.5],
			[1.0, 1.0, 1.0],
			turned_about([0.0, 0.0, 1.0], std::f32::consts::FRAC_PI_4),
		);

		let contact = obb_vs_obb(&floor, &cube).expect("the edge is in the floor");

		assert!((contact.normal().into_vector() - Vector::new(0.0, 1.0, 0.0)).length() <= 1e-5);
		assert_float_eq_with_epsilon!(contact.depth(), 0.1, 1e-5);
		// The middle of the edge, and the point of the floor's top above it.
		assert!((contact.point_on_b() - Point::new(1.0, -0.1, 0.5)).length() <= 1e-5);
		assert!((contact.point_on_a() - Point::new(1.0, 0.0, 0.5)).length() <= 1e-5);

		let lifted = oriented(
			[1.0, std::f32::consts::SQRT_2 + 0.1, 0.5],
			[1.0, 1.0, 1.0],
			turned_about([0.0, 0.0, 1.0], std::f32::consts::FRAC_PI_4),
		);
		assert!(obb_vs_obb(&floor, &lifted).is_none());
	}

	#[test]
	fn boxes_whose_edges_cross_touch_where_the_edges_are_closest() {
		// Two cubes turned an eighth of a turn, one about z and one about x, so that the first has an edge along z on
		// top and the second an edge along x underneath. The edges cross, 0.2 into each other.
		let reach = std::f32::consts::SQRT_2;
		let lower = oriented(
			[0.0, 0.0, 0.0],
			[1.0, 1.0, 1.0],
			turned_about([0.0, 0.0, 1.0], std::f32::consts::FRAC_PI_4),
		);
		let upper = oriented(
			[0.25, 2.0 * reach - 0.2, -0.5],
			[1.0, 1.0, 1.0],
			turned_about([1.0, 0.0, 0.0], std::f32::consts::FRAC_PI_4),
		);

		let contact = obb_vs_obb(&lower, &upper).expect("the edges are in each other");

		assert!((contact.normal().into_vector() - Vector::new(0.0, 1.0, 0.0)).length() <= 1e-5);
		assert_float_eq_with_epsilon!(contact.depth(), 0.2, 1e-5);
		// The lower cube's edge runs along z at x = 0, and the upper one's along x at z = -0.5.
		assert!((contact.point_on_a() - Point::new(0.0, reach, -0.5)).length() <= 1e-5);
		assert!((contact.point_on_b() - Point::new(0.0, reach - 0.2, -0.5)).length() <= 1e-5);

		// No side of either cube faces a direction with as little overlap.
		let apart = oriented(
			[0.25, 2.0 * reach + 0.1, -0.5],
			[1.0, 1.0, 1.0],
			turned_about([1.0, 0.0, 0.0], std::f32::consts::FRAC_PI_4),
		);
		assert!(obb_vs_obb(&lower, &apart).is_none());
	}

	#[test]
	fn a_box_inside_another_gets_the_shortest_way_out() {
		// The small box is turned and 0.25 from the large one's side at x = 2 at its nearest corner.
		let large = oriented([0.0, 0.0, 0.0], [2.0, 3.0, 4.0], crate::Orientation::identity());
		let small = oriented(
			[1.5, 0.5, -1.0],
			[0.25, 0.25, 0.25],
			turned_about([0.0, 0.0, 1.0], std::f32::consts::FRAC_PI_4),
		);

		let contact = obb_vs_obb(&large, &small).expect("one box is inside the other");

		// It leaves through that side once all of it has passed: its far edge is the square root of an eighth
		// behind its middle.
		assert!((contact.normal().into_vector() - Vector::new(1.0, 0.0, 0.0)).length() <= 1e-5);
		assert_float_eq_with_epsilon!(contact.depth(), 0.5 + 0.125f32.sqrt(), 1e-5);
	}
}
