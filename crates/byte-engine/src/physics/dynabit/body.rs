use math::{
	AABB, Capsule, Orientation, OrientedBox, Point, Scale, Sphere, Vector,
	collision::{
		Intersection, aabb_vs_aabb, capsule_vs_aabb, capsule_vs_capsule, capsule_vs_sphere, obb_vs_obb, sphere_vs_aabb,
		sphere_vs_sphere, sphere_vs_sphere_dynamic,
	},
};

use crate::{
	core::factory::Handle,
	physics::{
		LocalSpace,
		body::BodyTypes,
		collider::{Shapes, capsule_reach, sphere_moment},
		contact_patch::{FEATURE_SLACK, Feature, patch_corners},
		dynabit::contact::Contact,
		epa::ContactScratch,
		gjk::rounded_shapes_contact,
	},
	time::MediaTime,
};

/// The `PhysicsBody` struct stores the mutable world-space state used by Dynabit.
pub struct PhysicsBody {
	pub(crate) body_type: BodyTypes,
	pub(crate) collision_shape: Shapes,
	pub(crate) position: Point,
	pub(crate) orientation: Orientation,
	pub(crate) scale: Scale,
	pub(crate) linear_velocity: Vector,
	pub(crate) angular_velocity: Vector,
	/// Reciprocal mass in kilograms.
	pub(crate) inv_mass: f32,
	/// Center of mass relative to the collider origin.
	pub(crate) center_of_mass: Point<LocalSpace>,
	pub(crate) elasticity: f32,
	pub(crate) friction: f32,
	/// Whether contacts turn the body and its shape follows its orientation. See
	/// [`crate::physics::Body::with_rotation`].
	pub(crate) rotates: bool,
	pub(crate) handle: Handle,
}

impl PhysicsBody {
	/// Applies a world-space linear impulse.
	pub fn apply_linear_impulse(&mut self, impulse: Vector) {
		if self.inv_mass != 0.0 {
			self.linear_velocity += impulse * self.inv_mass;
		}
	}

	/// Returns the mass in kilograms.
	pub fn mass(&self) -> f32 {
		1.0 / self.inv_mass
	}

	/// Returns how easily contacts turn the body about each of its local axes: its inverse mass over each moment of
	/// inertia, or zero for a body that does not rotate.
	// The solver asks this for both bodies of every contact, in a loop that keeps many values in registers. When this
	// was a call there, or had one in it for the shapes that are not spheres, steps of sphere piles took 4% longer
	// on an Apple M4, though spheres never reach that call. So this and everything it needs is made part of that
	// loop.
	#[inline(always)]
	pub(crate) fn inverse_moments(&self) -> [f32; 3] {
		match self.collision_shape {
			_ if !self.rotates => [0.0; 3],
			Shapes::Sphere { radius } => [self.inv_mass / sphere_moment(radius); 3],
			_ => {
				let moments = self.collision_shape.principal_inertia();
				[
					self.inv_mass / moments.x,
					self.inv_mass / moments.y,
					self.inv_mass / moments.z,
				]
			}
		}
	}

	/// Returns whether the body's shape has to be treated as turned the way the body is.
	///
	/// A sphere is the same whichever way it is turned, and the shape of a body that does not rotate collides as it
	/// was made. So does that of a body that could rotate but is not turned and never will be by a contact, as a
	/// level floor is, which lets what rests on it use the quicker tests that shapes along the axes have.
	pub(crate) fn turns_shape(&self) -> bool {
		self.rotates && !matches!(self.collision_shape, Shapes::Sphere { .. }) && self.is_or_can_be_turned()
	}

	/// Returns whether a body that rotates is turned now or can be turned by a contact later.
	fn is_or_can_be_turned(&self) -> bool {
		self.inv_mass != 0.0 || self.orientation.to_array() != Orientation::identity().to_array()
	}

	/// Returns whether contacts turn the body.
	fn is_turned_by_contacts(&self) -> bool {
		self.rotates && self.inv_mass != 0.0
	}

	/// Returns the center of mass in world coordinates.
	pub fn world_space_center_of_mass(&self) -> Point {
		let local_offset = self.center_of_mass - Point::origin();
		// Rotating a local offset into the world frame crosses a spatial math boundary.
		self.position + Vector::from_maths(self.orientation.into_maths() * local_offset.into_maths())
	}

	/// Advances the body by `dt` using its current linear and angular velocities.
	pub fn update(&mut self, dt: MediaTime) {
		let motion = self.advanced(dt);
		self.apply(motion);
	}

	/// Returns where the body is, and how it spins, after moving for `dt` at its current velocities.
	///
	/// This reads the body without changing it, so lanes can advance bodies side by side. Write the result back
	/// with [`Self::apply`].
	pub(crate) fn advanced(&self, dt: MediaTime) -> Motion {
		let seconds = dt.as_seconds_f32();
		let travel = self.linear_velocity * seconds;
		let position = self.position + travel;
		let center_of_mass = self.world_space_center_of_mass() + travel;
		let center_offset = position - center_of_mass;

		// The spin changes when the body's moments of inertia differ. That change is worked out in the body's own
		// frame, where the inertia tensor is the diagonal of those moments, and then rotated into the world. With no
		// torque acting, angular momentum stays as it is, so the spin changes by the inverse inertia times the cross
		// product of the momentum with the spin. A body that is not spinning, as boxes and capsules never are, has none to
		// change.
		let angular_velocity = if self.angular_velocity.to_array() == [0.0; 3] {
			self.angular_velocity
		} else {
			let moments = self.collision_shape.principal_inertia();
			let local_velocity = self.orientation.inverse().rotate_vector(self.angular_velocity).into_maths();
			let local_acceleration = maths_rs::cross(local_velocity * moments, local_velocity) / moments;
			self.angular_velocity + self.orientation.rotate_vector(Vector::from_maths(local_acceleration)) * seconds
		};

		// A spin that is not finite turns the body by nothing.
		let delta_orientation =
			Orientation::try_from_rotation_vector(angular_velocity * seconds).unwrap_or_else(|_| Orientation::identity());
		Motion {
			position: center_of_mass + delta_orientation.rotate_vector(center_offset),
			orientation: delta_orientation.compose(self.orientation),
			angular_velocity,
		}
	}

	/// Moves the body to where [`Self::advanced`] found it to be.
	pub(crate) fn apply(&mut self, motion: Motion) {
		self.position = motion.position;
		self.orientation = motion.orientation;
		self.angular_velocity = motion.angular_velocity;
	}

	/// Returns the current world-space axis-aligned bounds.
	pub fn bounds(&self) -> AABB {
		if self.turns_shape() {
			return self.turned_bounds();
		}
		let local = self.collision_shape.bounds();
		// Crossing from authored local geometry to world simulation requires an explicit raw boundary conversion.
		AABB::new(
			self.position + Vector::from_maths(local.min().into_maths()),
			self.position + Vector::from_maths(local.max().into_maths()),
		)
	}

	/// Returns the world-space bounds of a shape that is turned the way the body is.
	fn turned_bounds(&self) -> AABB {
		// Every shape is a box around its middle, made thicker all around by a radius: a sphere is only the radius, a
		// capsule is a box without width around its segment, and a hull is the box around it, which is a little
		// more than the hull needs once it is turned.
		let (middle, half_sizes, radius) = match &self.collision_shape {
			Shapes::Sphere { radius } => (Point::origin(), Vector::zero(), *radius),
			Shapes::Cube { size } => (Point::origin(), *size, 0.0),
			Shapes::Capsule { radius, height, center } => {
				(*center, Vector::new(0.0, capsule_reach(*radius, *height), 0.0), *radius)
			}
			Shapes::ConvexHull(hull) => (hull.bounds().center(), hull.bounds().half_extents(), 0.0),
		};

		// A turned box reaches along each world axis as far as its three half sizes do together, each turned.
		let turned = |local: Vector<LocalSpace>| self.orientation.rotate_vector(Vector::from_maths(local.into_maths()));
		let reach = [
			Vector::new(half_sizes.x(), 0.0, 0.0),
			Vector::new(0.0, half_sizes.y(), 0.0),
			Vector::new(0.0, 0.0, half_sizes.z()),
		]
		.into_iter()
		.fold(Vector::new(radius, radius, radius), |reach, side| {
			let side = turned(side);
			reach + Vector::new(side.x().abs(), side.y().abs(), side.z().abs())
		});
		AABB::from_center_and_half_extents(self.position + turned(middle - Point::origin()), reach)
	}

	/// Returns whether the body travels far enough in the next `dt` seconds that [`intersect`] tests it along its
	/// path instead of where it is now.
	///
	/// Only a sphere is tested along its path, and only once a step carries it farther than half its radius. A
	/// slower sphere cannot pass through another body between two steps: whatever it reaches, it still overlaps
	/// when the step ends. Testing it in place lets its contacts be found and resolved together with the others.
	pub fn outruns_its_shape(&self, dt: f32) -> bool {
		match self.collision_shape {
			Shapes::Sphere { radius } => {
				let reach = radius * 0.5;
				self.linear_velocity.length_squared() * dt * dt > reach * reach
			}
			Shapes::Cube { .. } | Shapes::Capsule { .. } | Shapes::ConvexHull(_) => false,
		}
	}

	/// Returns world-space bounds around everywhere [`intersect`] can find this body in contact during the next
	/// `dt` seconds.
	///
	/// A body that [`Self::outruns_its_shape`] is tested along its path, so its bounds cover the path at its
	/// current linear velocity, slightly enlarged so bodies that are about to touch still count as overlapping.
	/// Any other body is only tested where it is now, so its bounds are exact and no pair is tested that cannot be
	/// in contact. Pass the bounds to [`crate::physics::SweepBounds::rebuild`] to find the bodies this one may
	/// collide with.
	pub fn swept_bounds(&self, dt: f32) -> AABB {
		debug_assert!(
			dt.is_finite() && dt >= 0.0,
			"Swept bounds interval is invalid. The most likely cause is passing a negative or non-finite frame interval."
		);
		let current = self.bounds();
		if !self.outruns_its_shape(dt) {
			return current;
		}
		const MARGIN: f32 = 0.01;
		let margin = Vector::new(MARGIN, MARGIN, MARGIN);
		let travel = self.linear_velocity * dt;
		let min = maths_rs::min(current.min().into_maths(), (current.min() + travel).into_maths());
		let max = maths_rs::max(current.max().into_maths(), (current.max() + travel).into_maths());
		AABB::new(Point::from_maths(min) - margin, Point::from_maths(max) + margin)
	}
}

/// The `Motion` struct holds the state a body has after moving for a while, between working it out and writing it
/// to the body.
pub(crate) struct Motion {
	position: Point,
	orientation: Orientation,
	angular_velocity: Vector,
}

/// Adds the contacts between two world-space bodies to `contacts`: none when they are apart, one where they touch
/// at a point, and up to four where they touch along a stretch or over a flat side and contacts turn one of them.
///
/// `scratch` is working storage for the pairs that go through GJK. Pass the same one for every pair that one thread
/// tests, so that testing them does not allocate.
pub fn intersect(
	(a, i): (&PhysicsBody, usize),
	(b, j): (&PhysicsBody, usize),
	dt: f32,
	contacts: &mut Vec<Contact>,
	scratch: &mut ContactScratch,
) {
	match own_contact((a, i), (b, j), dt) {
		Some(contact) => contacts.extend(contact),
		None => convex_contacts((a, i), (b, j), contacts, scratch),
	}
}

/// Returns the contact between two bodies whose shapes have a test of their own for each other, which may find
/// them apart. Returns `None` for a pair that has no such test: one with a hull, or with a shape that is turned the
/// way its body is.
// This hands its contact back, where it could add it to the list of contacts that its caller fills. Adding it in
// here made steps of every kind of pile 2% to 3% slower on an Apple M4.
fn own_contact((a, i): (&PhysicsBody, usize), (b, j): (&PhysicsBody, usize), dt: f32) -> Option<Option<Contact>> {
	let touching = match (&a.collision_shape, &b.collision_shape) {
		(Shapes::Sphere { radius: a_radius }, Shapes::Sphere { radius: b_radius }) => {
			let (a_sphere, b_sphere) = (Sphere::new(a.position, *a_radius), Sphere::new(b.position, *b_radius));
			if a.outruns_its_shape(dt) || b.outruns_its_shape(dt) {
				// A fast sphere could pass through the other one between two steps, so find where their paths meet.
				let impact = sphere_vs_sphere_dynamic(&a_sphere, &b_sphere, a.linear_velocity, b.linear_velocity, dt);
				return Some(impact.map(|impact| {
					let mut contact = Contact::from_dynamic(i, j, impact);
					// The points are where the spheres will touch. Contacts are solved from where the bodies are now,
					// so the points move back to where they are on the spheres now.
					contact.a.point = contact.a.point - a.linear_velocity * contact.toi;
					contact.b.point = contact.b.point - b.linear_velocity * contact.toi;
					contact
				}));
			}
			sphere_vs_sphere(&a_sphere, &b_sphere)
		}
		(Shapes::ConvexHull(_), _) | (_, Shapes::ConvexHull(_)) => return None,
		_ if a.turns_shape() || b.turns_shape() => return None,
		(Shapes::Cube { size: a_size }, Shapes::Cube { size: b_size }) => {
			aabb_vs_aabb(&upright_box(a.position, *a_size), &upright_box(b.position, *b_size))
		}
		(Shapes::Sphere { radius }, Shapes::Cube { size }) => {
			sphere_vs_aabb(&Sphere::new(a.position, *radius), &upright_box(b.position, *size))
		}
		(Shapes::Cube { size }, Shapes::Sphere { radius }) => {
			sphere_vs_aabb(&Sphere::new(b.position, *radius), &upright_box(a.position, *size)).map(Intersection::swap)
		}
		(Shapes::Capsule { radius, height, center }, _) => {
			capsule_contact(&world_capsule(a.position, *radius, *height, *center), b)
		}
		(_, Shapes::Capsule { radius, height, center }) => {
			capsule_contact(&world_capsule(b.position, *radius, *height, *center), a).map(Intersection::swap)
		}
	};
	Some(touching.map(|touching| Contact::from_static(i, j, touching)))
}

/// Returns where the upright `capsule` touches the body `other`, with the capsule as the first shape of the
/// intersection. Neither shape is turned the way its body is, and `other` is not a hull.
fn capsule_contact(capsule: &Capsule, other: &PhysicsBody) -> Option<Intersection> {
	match &other.collision_shape {
		Shapes::Sphere { radius } => capsule_vs_sphere(capsule, &Sphere::new(other.position, *radius)),
		Shapes::Cube { size } => capsule_vs_aabb(capsule, &upright_box(other.position, *size)),
		Shapes::Capsule { radius, height, center } => {
			capsule_vs_capsule(capsule, &world_capsule(other.position, *radius, *height, *center))
		}
		Shapes::ConvexHull(_) => unreachable!(
			"A capsule was tested against a hull directly. The most likely cause is a change to `intersect` that no longer sends every pair with a hull through GJK first."
		),
	}
}

/// Adds to `contacts` where the shapes of two bodies touch, for any two shapes, each placed and turned as its body
/// has it. It serves the pairs that the tests along the axes cannot: those with a hull, and those with a shape that
/// is turned the way its body is.
///
/// Two boxes have a test of their own for this. Every other pair goes through GJK, which only asks each shape for
/// its furthest point along a direction.
fn convex_contacts(
	(a, i): (&PhysicsBody, usize),
	(b, j): (&PhysicsBody, usize),
	contacts: &mut Vec<Contact>,
	scratch: &mut ContactScratch,
) {
	let (placed_a, placed_b) = (Placement::of(a), Placement::of(b));
	let (radius_a, radius_b) = (core_radius(&a.collision_shape), core_radius(&b.collision_shape));
	let deepest = match (&a.collision_shape, &b.collision_shape) {
		(Shapes::Cube { size: a_size }, Shapes::Cube { size: b_size }) => {
			obb_vs_obb(&placed_a.oriented_box(*a_size), &placed_b.oriented_box(*b_size))
		}
		_ => rounded_shapes_contact(
			|direction| placed_a.core_support(direction),
			radius_a,
			|direction| placed_b.core_support(direction),
			radius_b,
			scratch,
		),
	};
	let Some(deepest) = deepest else {
		return;
	};

	// The two points found are one pair that is as deep in each other as the shapes are. Where one of the shapes touches
	// with a single corner, that corner is the pair, and so it is where contacts turn neither body, because a body
	// that cannot be turned is held as well at one point as at several. Otherwise the shapes touch along a stretch
	// or over a patch, and a body held at one point of it would rock on that point, so it is held at the patch's
	// corners.
	if a.is_turned_by_contacts() || b.is_turned_by_contacts() {
		let reach = deepest.depth() + FEATURE_SLACK;
		let feature_a = placed_a.core_feature(deepest.normal().into(), reach);
		let feature_b = placed_b.core_feature(-deepest.normal().into_vector(), reach);
		if feature_a.len() > 1 && feature_b.len() > 1 {
			let corners = patch_corners(&feature_a, radius_a, &feature_b, radius_b, deepest.normal());
			if !corners.is_empty() {
				contacts.extend(corners.into_iter().map(|held| Contact {
					corner: held.number,
					..Contact::from_static(i, j, Intersection::new(deepest.normal(), held.depth, held.on_a, held.on_b))
				}));
				return;
			}
		}
	}
	contacts.push(Contact::from_static(i, j, deepest));
}

/// The `Placement` struct answers questions about the core of a body's shape in world space: the shape itself for a
/// box and a hull, the center of a sphere, and the segment between the centers of a capsule's caps. [`core_radius`]
/// is how far the shape reaches beyond its core.
struct Placement<'a> {
	body: &'a PhysicsBody,
	/// For a shape that is turned the way its body is: the turn from world space into the shape's own frame, and the
	/// turn back.
	turn: Option<(Orientation, Orientation)>,
}

impl<'a> Placement<'a> {
	fn of(body: &'a PhysicsBody) -> Self {
		Self {
			body,
			turn: body.turns_shape().then(|| (body.orientation.inverse(), body.orientation)),
		}
	}

	/// Returns the body's box, for a collider that is a [`Shapes::Cube`] with these half sizes.
	fn oriented_box(&self, half_sizes: Vector<LocalSpace>) -> OrientedBox {
		let orientation = self.turn.map_or(Orientation::identity(), |(_, out_of_shape)| out_of_shape);
		OrientedBox::new(self.body.position, half_sizes.to_array(), orientation)
	}

	/// Returns a world-space direction in the shape's own frame.
	fn to_shape(&self, direction: Vector) -> Vector<LocalSpace> {
		let direction = self
			.turn
			.map_or(direction, |(into_shape, _)| into_shape.rotate_vector(direction));
		// Crossing from world simulation to authored local geometry requires an explicit raw boundary conversion.
		Vector::from_maths(direction.into_maths())
	}

	/// Returns the world-space position of a point of the shape.
	fn to_world(&self, local: Point<LocalSpace>) -> Point {
		// Crossing from authored local geometry to world simulation requires an explicit raw boundary conversion.
		let offset = Vector::from_maths(local.into_maths());
		self.body.position
			+ self
				.turn
				.map_or(offset, |(_, out_of_shape)| out_of_shape.rotate_vector(offset))
	}

	/// Returns the point of the core that is furthest along `direction`.
	fn core_support(&self, direction: Vector) -> Point {
		let direction = self.to_shape(direction);
		self.to_world(match &self.body.collision_shape {
			Shapes::Sphere { .. } => Point::origin(),
			Shapes::Cube { size } => {
				let corner = |half_extent: f32, along: f32| half_extent.abs().copysign(along);
				Point::new(
					corner(size.x(), direction.x()),
					corner(size.y(), direction.y()),
					corner(size.z(), direction.z()),
				)
			}
			Shapes::Capsule { radius, height, center } => {
				*center + Vector::new(0.0, capsule_reach(*radius, *height).copysign(direction.y()), 0.0)
			}
			Shapes::ConvexHull(hull) => hull.support_point(direction),
		})
	}

	/// Returns the corners of the core that are within `reach` of as far along `direction` as the core gets: the
	/// part of the shape that touches another shape in that direction.
	fn core_feature(&self, direction: Vector, reach: f32) -> Feature<math::WorldSpace> {
		let direction = self.to_shape(direction);
		let along = |corner: Point<LocalSpace>| direction.dot(corner - Point::origin());
		let mut feature = Feature::new();
		let mut keep = |corners: &mut dyn Iterator<Item = Point<LocalSpace>>, furthest: f32| {
			feature.extend(
				corners
					.filter(|&corner| along(corner) >= furthest - reach)
					.map(|corner| self.to_world(corner)),
			);
		};
		match &self.body.collision_shape {
			Shapes::Sphere { .. } => keep(&mut std::iter::once(Point::origin()), 0.0),
			Shapes::Cube { size } => {
				let half = size.to_array().map(f32::abs);
				let furthest = half[0] * direction.x().abs() + half[1] * direction.y().abs() + half[2] * direction.z().abs();
				let mut corners = (0..8).map(|corner| {
					let side = |axis: usize| if corner >> axis & 1 == 0 { -half[axis] } else { half[axis] };
					Point::new(side(0), side(1), side(2))
				});
				keep(&mut corners, furthest);
			}
			Shapes::Capsule { radius, height, center } => {
				let reach = Vector::new(0.0, capsule_reach(*radius, *height), 0.0);
				let ends = [*center - reach, *center + reach];
				keep(&mut ends.into_iter(), along(ends[0]).max(along(ends[1])));
			}
			Shapes::ConvexHull(hull) => {
				let furthest = along(hull.support_point(direction));
				keep(&mut hull.vertices().iter().copied(), furthest);
			}
		}
		feature
	}
}

/// Returns how far `shape` reaches beyond its core in every direction, which is the radius of a sphere and of a
/// capsule, and nothing for a shape with flat sides.
fn core_radius(shape: &Shapes) -> f32 {
	match shape {
		Shapes::Sphere { radius } | Shapes::Capsule { radius, .. } => *radius,
		Shapes::Cube { .. } | Shapes::ConvexHull(_) => 0.0,
	}
}

/// Returns the world-space box of a body at `position` that does not rotate and whose collider is a
/// [`Shapes::Cube`] with these half sizes.
///
/// The box of such a body lies along the axes whatever the body's orientation, so placing it only moves it.
fn upright_box(position: Point, half_sizes: Vector<LocalSpace>) -> AABB {
	// Crossing from authored local geometry to world simulation requires an explicit raw boundary conversion.
	AABB::from_center_and_half_extents(position, Vector::from_maths(half_sizes.into_maths()))
}

/// Returns the world-space capsule of a body at `position` that does not rotate and whose collider is a
/// [`Shapes::Capsule`] with these properties.
///
/// The capsule of such a body is upright whatever the body's orientation, so placing it only moves it.
fn world_capsule(position: Point, radius: f32, height: f32, center: Point<LocalSpace>) -> Capsule {
	// Crossing from authored local geometry to world simulation requires an explicit raw boundary conversion.
	let middle = position + Vector::from_maths(center.into_maths());
	let reach = Vector::new(0.0, capsule_reach(radius, height), 0.0);
	Capsule::new(middle - reach, middle + reach, radius)
}

#[cfg(test)]
mod tests {
	use super::*;
	use crate::physics::Shapes;

	/// Returns the contact between two bodies that touch at one place, or none when they are apart.
	fn touch(a: (&PhysicsBody, usize), b: (&PhysicsBody, usize), dt: f32) -> Option<Contact> {
		let mut contacts = Vec::new();
		intersect(a, b, dt, &mut contacts, &mut ContactScratch::default());
		assert!(contacts.len() <= 1, "the bodies touch at {} places", contacts.len());
		contacts.pop()
	}

	fn body() -> PhysicsBody {
		PhysicsBody {
			body_type: BodyTypes::Dynamic,
			collision_shape: Shapes::Sphere { radius: 1.0 },
			position: Point::origin(),
			orientation: Orientation::identity(),
			scale: Scale::identity(),
			linear_velocity: Vector::zero(),
			angular_velocity: Vector::new(0.0, 1.0, 0.0),
			inv_mass: 1.0,
			center_of_mass: Point::origin(),
			elasticity: 0.0,
			friction: 1.0,
			rotates: false,
			handle: crate::core::factory::Factory::<()>::new().create(()),
		}
	}

	#[test]
	fn swept_bounds_reach_where_a_fast_body_ends_the_step() {
		let mut body = body();
		body.position = Point::new(-5.0, -5.0, -5.0);
		body.linear_velocity = Vector::new(10.0, 10.0, 10.0);

		let swept = body.swept_bounds(1.0);

		// The body has a radius of one and ends the step centered five units past the origin.
		assert!(swept.min().x() <= -6.0 && swept.max().x() >= 6.0);
		assert!(swept.min().y() <= -6.0 && swept.max().y() >= 6.0);
		assert!(swept.min().z() <= -6.0 && swept.max().z() >= 6.0);
	}

	#[test]
	fn swept_bounds_of_a_slow_sphere_are_its_bounds() {
		let mut body = body();
		body.position = Point::new(2.0, 0.0, 0.0);
		// The sphere has a radius of one and moves less than half of it in the step.
		body.linear_velocity = Vector::new(0.3, 0.0, 0.0);

		let swept = body.swept_bounds(1.0);

		assert_eq!(swept.min(), body.bounds().min());
		assert_eq!(swept.max(), body.bounds().max());
	}

	#[test]
	fn slow_spheres_touch_only_where_they_are_now() {
		let mut first = body();
		first.linear_velocity = Vector::new(0.3, 0.0, 0.0);
		let mut apart = body();
		apart.position = Point::new(2.2, 0.0, 0.0);
		let mut overlapping = body();
		overlapping.position = Point::new(1.5, 0.0, 0.0);

		// The first sphere reaches the second during the step, but it is tested where it starts.
		assert!(touch((&first, 0), (&apart, 1), 1.0).is_none());
		let contact = touch((&first, 0), (&overlapping, 1), 1.0).expect("overlapping spheres are in contact");
		assert_eq!(contact.toi, 0.0);
		assert!((contact.depth - 0.5).abs() <= 1e-6);
	}

	#[test]
	fn a_fast_sphere_touches_where_its_path_meets_another() {
		let mut first = body();
		first.linear_velocity = Vector::new(4.0, 0.0, 0.0);
		let mut apart = body();
		apart.position = Point::new(4.0, 0.0, 0.0);

		// The spheres have a radius of one, so they touch after the first has moved two units.
		let contact = touch((&first, 0), (&apart, 1), 1.0).expect("the path reaches the other sphere");
		assert!((contact.toi - 0.5).abs() <= 1e-6);
		// The contact points are the facing points of the spheres where they are now.
		assert!((contact.a.point - Point::new(1.0, 0.0, 0.0)).length() <= 1e-5);
		assert!((contact.b.point - Point::new(3.0, 0.0, 0.0)).length() <= 1e-5);
	}

	#[test]
	fn swept_bounds_of_a_box_are_its_bounds() {
		let mut body = body();
		body.collision_shape = Shapes::cube(Vector::new(0.5, 1.0, 1.5));
		body.position = Point::new(2.0, 0.0, 0.0);
		body.linear_velocity = Vector::new(10.0, 10.0, 10.0);

		let swept = body.swept_bounds(1.0);

		assert_eq!(swept.min(), body.bounds().min());
		assert_eq!(swept.max(), body.bounds().max());
	}

	#[test]
	fn a_capsule_stands_upright_on_its_body_whatever_the_orientation() {
		let mut character = body();
		// A character 1.8 tall whose origin is at its feet.
		character.collision_shape = Shapes::capsule(0.3, 1.8, Point::new(0.0, 0.9, 0.0));
		character.position = Point::new(2.0, 1.0, 0.0);
		// Lying on its side, as an animation might leave a character's transform.
		let quarter_turn = Vector::<math::WorldSpace>::new(0.0, 0.0, std::f32::consts::FRAC_PI_2);
		character.orientation = Orientation::try_from_rotation_vector(quarter_turn).unwrap();

		assert!((character.bounds().min() - Point::new(1.7, 1.0, -0.3)).length() <= 1e-5);
		assert!((character.bounds().max() - Point::new(2.3, 2.8, 0.3)).length() <= 1e-5);

		// A box whose top is 0.05 above the capsule's feet touches it from below, under its lower cap.
		let mut floor = body();
		floor.collision_shape = Shapes::cube(Vector::new(4.0, 0.5, 4.0));
		floor.position = Point::new(0.0, 0.55, 0.0);
		let contact = touch((&character, 0), (&floor, 1), 1.0 / 60.0).expect("the capsule stands in the box");
		assert!((contact.a.point - Point::new(2.0, 1.0, 0.0)).length() <= 1e-5);
		assert!((contact.b.point - Point::new(2.0, 1.05, 0.0)).length() <= 1e-5);
		assert!((contact.depth - 0.05).abs() <= 1e-5);
	}

	#[test]
	fn contacts_between_two_shapes_are_the_same_for_both_body_orders() {
		let mut capsule = body();
		capsule.collision_shape = Shapes::capsule(0.3, 1.8, Point::new(0.0, 0.9, 0.0));
		let mut sphere = body();
		sphere.position = Point::new(1.0, 1.0, 0.0);
		let mut cube = body();
		cube.collision_shape = Shapes::cube(Vector::new(0.5, 0.5, 0.5));
		cube.position = Point::new(-0.7, 1.0, 0.0);

		for other in [&sphere, &cube] {
			let forward = touch((&capsule, 0), (other, 1), 1.0 / 60.0).expect("the shapes overlap");
			let backward = touch((other, 0), (&capsule, 1), 1.0 / 60.0).expect("the shapes overlap");

			assert_eq!(forward.normal, -backward.normal);
			assert_eq!(forward.depth, backward.depth);
			assert_eq!((forward.a.point, forward.b.point), (backward.b.point, backward.a.point));
			// The capsule reaches 0.3 to each side. The sphere reaches back to 0 and the cube forward to -0.2.
			assert!(forward.depth > 0.0 && forward.depth <= 0.3 + 1e-6);
		}
	}

	#[test]
	fn a_spinning_body_with_unequal_moments_keeps_its_angular_momentum() {
		let mut body = body();
		// A box of 1 by 2 by 3 units has unit-mass moments of 13/12, 10/12, and 5/12.
		body.collision_shape = Shapes::cube(Vector::new(0.5, 1.0, 1.5));
		body.angular_velocity = Vector::new(1.0, 1.0, 0.0);

		body.update(MediaTime::from_seconds_f32(0.01));

		// With no torque, the spin about the third axis changes at (13/12 - 10/12) / (5/12) = 0.6 per second.
		assert!((body.angular_velocity.z() - 0.006).abs() <= 1e-5);
	}

	#[test]
	fn bounds_are_world_space_even_when_shape_data_is_local() {
		let mut body = body();
		body.position = Point::new(3.0, 0.0, 0.0);

		assert_eq!(body.bounds().min(), Point::new(2.0, -1.0, -1.0));
	}
	/// Returns a hull with a corner one unit from its middle along each axis, both ways.
	fn diamond() -> Shapes {
		let tips = [
			[1.0, 0.0, 0.0],
			[-1.0, 0.0, 0.0],
			[0.0, 1.0, 0.0],
			[0.0, -1.0, 0.0],
			[0.0, 0.0, 1.0],
			[0.0, 0.0, -1.0],
		]
		.map(Point::from_array);
		Shapes::convex_hull(crate::physics::build_convex_hull(&tips).expect("the tips enclose a volume"))
	}

	#[test]
	fn a_hull_touches_a_sphere_a_box_and_a_capsule_with_its_tip() {
		let mut hull = body();
		hull.collision_shape = diamond();
		// Each shape's near side is 0.25 inside the hull's tip at x = 1, and lower than it, so the tip touches a flat
		// or upright part of each.
		let mut sphere = body();
		sphere.position = Point::new(1.75, 0.0, 0.0);
		let mut cube = body();
		cube.collision_shape = Shapes::cube(Vector::new(0.5, 0.5, 0.5));
		cube.position = Point::new(1.25, -0.25, 0.25);
		let mut capsule = body();
		capsule.collision_shape = Shapes::capsule(0.3, 1.8, Point::new(0.0, 0.9, 0.0));
		capsule.position = Point::new(1.05, -1.0, 0.0);

		for (other, on_other) in [
			(&sphere, Point::new(0.75, 0.0, 0.0)),
			(&cube, Point::new(0.75, 0.0, 0.0)),
			(&capsule, Point::new(0.75, 0.0, 0.0)),
		] {
			let contact = touch((&hull, 0), (other, 1), 1.0 / 60.0).expect("the tip is inside the shape");

			assert!(
				(contact.normal.into_vector() - Vector::new(1.0, 0.0, 0.0)).length() <= 1e-5,
				"{contact:?}"
			);
			assert!((contact.depth - 0.25).abs() <= 1e-5, "{contact:?}");
			assert!((contact.a.point - Point::new(1.0, 0.0, 0.0)).length() <= 1e-5, "{contact:?}");
			assert!((contact.b.point - on_other).length() <= 1e-5, "{contact:?}");
			assert_eq!(contact.toi, 0.0);

			// The other way around, the same contact is seen from the other side.
			let backward = touch((other, 0), (&hull, 1), 1.0 / 60.0).expect("the tip is inside the shape");
			assert!((backward.normal.into_vector() + contact.normal.into_vector()).length() <= 1e-5);
			assert!((backward.depth - contact.depth).abs() <= 1e-5);
			assert!((backward.a.point - contact.b.point).length() <= 1e-5);
			assert!((backward.b.point - contact.a.point).length() <= 1e-5);
		}

		// Moved 0.3 further off, none of them reaches the tip.
		for other in [&mut sphere, &mut cube, &mut capsule] {
			other.position += Vector::new(0.3, 0.0, 0.0);
			assert!(touch((&hull, 0), (other, 1), 1.0 / 60.0).is_none());
			assert!(touch((other, 0), (&hull, 1), 1.0 / 60.0).is_none());
		}
	}

	#[test]
	fn hulls_touch_along_their_shallowest_overlap() {
		let mut first = body();
		first.collision_shape = diamond();
		let mut second = body();
		second.collision_shape = diamond();
		second.position = Point::new(1.5, 0.0, 0.0);

		let contact = touch((&first, 0), (&second, 1), 1.0 / 60.0).expect("the tips overlap");

		// The tips are 0.5 into each other, but each has slanted sides that are nearer: sliding apart along one takes
		// 0.5 over the square root of three. Four sides are as near, and the contact is on one of them.
		let slant = 1.0 / 3f32.sqrt();
		assert!((contact.depth - 0.5 * slant).abs() <= 1e-5, "{contact:?}");
		let normal = contact.normal.into_vector().to_array();
		assert!(
			(normal[0] - slant).abs() <= 1e-5 && (normal[1].abs() - slant).abs() <= 1e-5,
			"{contact:?}"
		);
		assert!((contact.a.point - contact.b.point - contact.normal.into_vector() * contact.depth).length() <= 1e-5);

		second.position = Point::new(2.1, 0.0, 0.0);
		assert!(touch((&first, 0), (&second, 1), 1.0 / 60.0).is_none());
	}

	#[test]
	fn a_hull_collides_as_it_was_built_whatever_the_orientation() {
		let mut hull = body();
		hull.collision_shape = diamond();
		hull.position = Point::new(2.0, 1.0, 0.0);
		let mut floor = body();
		floor.collision_shape = Shapes::cube(Vector::new(4.0, 0.5, 4.0));
		floor.position = Point::new(0.0, -0.4, 0.0);
		let upright = touch((&hull, 0), (&floor, 1), 1.0 / 60.0).expect("the hull's lower tip is in the floor");

		// Turned an eighth of a turn about z, which would lift the tip out of the floor if the hull followed.
		let eighth_turn = Vector::<math::WorldSpace>::new(0.0, 0.0, std::f32::consts::FRAC_PI_4);
		hull.orientation = Orientation::try_from_rotation_vector(eighth_turn).unwrap();
		let turned = touch((&hull, 0), (&floor, 1), 1.0 / 60.0).expect("the hull's lower tip is in the floor");

		assert!((upright.depth - 0.1).abs() <= 1e-5);
		assert_eq!(
			(turned.depth, turned.a.point, turned.b.point),
			(upright.depth, upright.a.point, upright.b.point)
		);
		assert_eq!(hull.bounds().min(), Point::new(1.0, 0.0, -1.0));
		assert_eq!(hull.bounds().max(), Point::new(3.0, 2.0, 1.0));
		assert_eq!(hull.swept_bounds(1.0).min(), hull.bounds().min());
	}
	/// Returns the contacts between two bodies.
	fn touches(a: (&PhysicsBody, usize), b: (&PhysicsBody, usize)) -> Vec<Contact> {
		let mut contacts = Vec::new();
		intersect(a, b, 1.0 / 60.0, &mut contacts, &mut ContactScratch::default());
		contacts
	}

	/// Returns a turn of `angle` radians about the z-axis.
	fn about_z(angle: f32) -> Orientation {
		Orientation::try_from_rotation_vector(Vector::<math::WorldSpace>::new(0.0, 0.0, angle)).unwrap()
	}

	/// Returns a static floor whose top is at height zero.
	fn floor() -> PhysicsBody {
		let mut floor = body();
		floor.body_type = BodyTypes::Static;
		floor.inv_mass = 0.0;
		floor.collision_shape = Shapes::cube(Vector::new(4.0, 0.5, 4.0));
		floor.position = Point::new(0.0, -0.5, 0.0);
		floor
	}

	#[test]
	fn only_a_body_that_rotates_is_turned_by_contacts() {
		let mut sphere = body();
		assert_eq!(sphere.inverse_moments(), [0.0; 3]);
		sphere.rotates = true;
		assert_eq!(sphere.inverse_moments(), [2.5; 3]);

		// A box of 1 by 2 by 3 units and half a unit of mass has moments of 13/24, 10/24, and 5/24.
		let mut cube = body();
		cube.collision_shape = Shapes::cube(Vector::new(0.5, 1.0, 1.5));
		cube.inv_mass = 2.0;
		assert_eq!(cube.inverse_moments(), [0.0; 3]);
		cube.rotates = true;
		let moments = cube.inverse_moments();
		assert!(
			(moments[0] - 24.0 / 13.0).abs() <= 1e-5 && (moments[1] - 2.4).abs() <= 1e-5 && (moments[2] - 4.8).abs() <= 1e-5
		);
	}

	#[test]
	fn the_bounds_of_a_body_that_rotates_follow_its_orientation() {
		let mut cube = body();
		cube.collision_shape = Shapes::cube(Vector::new(1.0, 0.5, 0.25));
		cube.position = Point::new(3.0, 0.0, 0.0);
		cube.orientation = about_z(std::f32::consts::FRAC_PI_2);

		// As it was made, whatever its orientation.
		assert!((cube.bounds().max() - Point::new(4.0, 0.5, 0.25)).length() <= 1e-6);

		// Turned a quarter turn, its long side stands up.
		cube.rotates = true;
		assert!((cube.bounds().min() - Point::new(2.5, -1.0, -0.25)).length() <= 1e-5);
		assert!((cube.bounds().max() - Point::new(3.5, 1.0, 0.25)).length() <= 1e-5);

		// A capsule that lies down reaches along x, around its middle, which turned with it.
		let mut capsule = body();
		capsule.collision_shape = Shapes::capsule(0.3, 1.8, Point::new(0.0, 0.9, 0.0));
		capsule.orientation = about_z(-std::f32::consts::FRAC_PI_2);
		capsule.rotates = true;
		assert!((capsule.bounds().min() - Point::new(0.0, -0.3, -0.3)).length() <= 1e-5);
		assert!((capsule.bounds().max() - Point::new(1.8, 0.3, 0.3)).length() <= 1e-5);
	}

	#[test]
	fn a_box_that_rotates_touches_with_the_edge_that_it_turned_up() {
		// A cube turned an eighth of a turn about z has an edge on top, the square root of two above its middle.
		let mut cube = body();
		cube.collision_shape = Shapes::cube(Vector::new(1.0, 1.0, 1.0));
		cube.orientation = about_z(std::f32::consts::FRAC_PI_4);
		let mut sphere = body();
		sphere.collision_shape = Shapes::sphere(0.5);
		sphere.position = Point::new(0.0, std::f32::consts::SQRT_2 + 0.4, 0.25);

		// As it was made, its flat top is 0.4 below the sphere.
		assert!(touch((&cube, 0), (&sphere, 1), 1.0 / 60.0).is_none());

		cube.rotates = true;
		let contact = touch((&cube, 0), (&sphere, 1), 1.0 / 60.0).expect("the edge is inside the sphere");
		assert!(
			(contact.normal.into_vector() - Vector::new(0.0, 1.0, 0.0)).length() <= 1e-5,
			"{contact:?}"
		);
		assert!((contact.depth - 0.1).abs() <= 1e-5, "{contact:?}");
		assert!(
			(contact.a.point - Point::new(0.0, std::f32::consts::SQRT_2, 0.25)).length() <= 1e-5,
			"{contact:?}"
		);

		let backward = touch((&sphere, 0), (&cube, 1), 1.0 / 60.0).expect("the edge is inside the sphere");
		assert!((backward.normal.into_vector() + contact.normal.into_vector()).length() <= 1e-5);
		assert!((backward.depth - contact.depth).abs() <= 1e-5);
	}

	#[test]
	fn a_box_that_rotates_is_held_at_the_corners_of_the_side_it_lies_on() {
		let floor = floor();
		let mut cube = body();
		cube.collision_shape = Shapes::cube(Vector::new(0.5, 0.5, 0.5));
		cube.rotates = true;
		cube.position = Point::new(1.0, 0.495, -2.0);

		let contacts = touches((&cube, 0), (&floor, 1));

		// One at each corner of the underside, each 0.005 into the floor, and each with its own number.
		assert_eq!(contacts.len(), 4);
		let mut numbers: Vec<u8> = contacts.iter().map(|contact| contact.corner).collect();
		numbers.sort_unstable();
		assert_eq!(numbers, [0, 1, 2, 3]);
		for (x, z) in [(0.5, -2.5), (1.5, -2.5), (1.5, -1.5), (0.5, -1.5)] {
			let contact = contacts
				.iter()
				.find(|contact| (contact.a.point - Point::new(x, -0.005, z)).length() <= 1e-4)
				.unwrap_or_else(|| panic!("no contact is at ({x}, {z}): {contacts:?}"));
			assert!((contact.normal.into_vector() - Vector::new(0.0, -1.0, 0.0)).length() <= 1e-5);
			assert!((contact.depth - 0.005).abs() <= 1e-5);
			assert!((contact.b.point - Point::new(x, 0.0, z)).length() <= 1e-4);
		}

		// A box that does not rotate cannot rock, so one contact holds it.
		cube.rotates = false;
		assert_eq!(touches((&cube, 0), (&floor, 1)).len(), 1);

		// Standing on an edge, the box that rotates is held at the two ends of that edge.
		cube.rotates = true;
		cube.orientation = about_z(std::f32::consts::FRAC_PI_4);
		cube.position = Point::new(0.0, std::f32::consts::FRAC_1_SQRT_2 - 0.01, 0.0);
		let contacts = touches((&cube, 0), (&floor, 1));
		assert_eq!(contacts.len(), 2, "{contacts:?}");
		for z in [-0.5, 0.5] {
			assert!(
				contacts
					.iter()
					.any(|contact| (contact.b.point - Point::new(0.0, 0.0, z)).length() <= 1e-4
						&& (contact.depth - 0.01).abs() <= 1e-4),
				"no contact is at z = {z}: {contacts:?}"
			);
		}
	}

	#[test]
	fn a_capsule_that_rotates_lies_along_the_floor() {
		let floor = floor();
		// Lying on its side along x, 0.01 into the floor. Its segment reaches 0.6 either way.
		let mut capsule = body();
		capsule.collision_shape = Shapes::capsule(0.3, 1.8, Point::origin());
		capsule.orientation = about_z(std::f32::consts::FRAC_PI_2);
		capsule.rotates = true;
		capsule.position = Point::new(0.0, 0.29, 1.0);

		let contacts = touches((&capsule, 0), (&floor, 1));

		assert_eq!(contacts.len(), 2, "{contacts:?}");
		for x in [-0.6, 0.6] {
			assert!(
				contacts
					.iter()
					.any(|contact| (contact.b.point - Point::new(x, 0.0, 1.0)).length() <= 1e-4
						&& (contact.depth - 0.01).abs() <= 1e-4
						&& (contact.normal.into_vector() - Vector::new(0.0, -1.0, 0.0)).length() <= 1e-5),
				"no contact is at x = {x}: {contacts:?}"
			);
		}

		// The same capsule on a body that does not rotate stands up, with its lower cap deep in the floor.
		capsule.rotates = false;
		let contact = touch((&capsule, 0), (&floor, 1), 1.0 / 60.0).expect("the capsule stands in the floor");
		assert!((contact.depth - 0.61).abs() <= 1e-5, "{contact:?}");
	}

	#[test]
	fn a_hull_that_rotates_touches_with_the_edge_that_it_turned_down() {
		let floor = floor();
		// Turned an eighth of a turn about z, two of the diamond's tips are level and lowest, and the edge between
		// them is the square root of a half below its middle.
		let mut hull = body();
		hull.collision_shape = diamond();
		hull.orientation = about_z(std::f32::consts::FRAC_PI_4);
		hull.rotates = true;
		hull.position = Point::new(0.0, std::f32::consts::FRAC_1_SQRT_2 - 0.01, 0.0);

		let contacts = touches((&hull, 0), (&floor, 1));

		assert_eq!(contacts.len(), 2, "{contacts:?}");
		for x in [-std::f32::consts::FRAC_1_SQRT_2, std::f32::consts::FRAC_1_SQRT_2] {
			assert!(
				contacts
					.iter()
					.any(|contact| (contact.b.point - Point::new(x, 0.0, 0.0)).length() <= 1e-4
						&& (contact.depth - 0.01).abs() <= 1e-4),
				"no contact is at x = {x}: {contacts:?}"
			);
		}
		// Its bounds hold it however it is turned.
		assert!(hull.bounds().min().y() <= -0.01 && hull.bounds().max().x() >= std::f32::consts::FRAC_1_SQRT_2);
	}

	#[test]
	fn turned_boxes_are_as_deep_in_each_other_as_gjk_finds_them() {
		// A source of numbers from -1 to 1 that is the same on every run.
		let mut state = 0x2545_f491_4f6c_dd1du64;
		let mut next = move || {
			state = state.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
			(state >> 40) as f32 / (1u64 << 23) as f32 - 1.0
		};
		let mut turned_box = |spread: f32| {
			let mut cube = body();
			cube.collision_shape = Shapes::cube(Vector::new(next().abs() + 0.2, next().abs() + 0.2, next().abs() + 0.2));
			cube.position = Point::new(next() * spread, next() * spread, next() * spread);
			cube.orientation = Orientation::try_from_rotation_vector(Vector::<math::WorldSpace>::new(
				next() * 3.0,
				next() * 3.0,
				next() * 3.0,
			))
			.unwrap();
			cube.rotates = true;
			cube
		};

		let mut overlapping = 0;
		for _ in 0..1000 {
			let (a, b) = (turned_box(0.5), turned_box(1.5));
			let (placed_a, placed_b) = (Placement::of(&a), Placement::of(&b));
			let (Shapes::Cube { size: a_size }, Shapes::Cube { size: b_size }) = (&a.collision_shape, &b.collision_shape)
			else {
				unreachable!()
			};

			let own = obb_vs_obb(&placed_a.oriented_box(*a_size), &placed_b.oriented_box(*b_size));
			let through_gjk = rounded_shapes_contact(
				|direction| placed_a.core_support(direction),
				0.0,
				|direction| placed_b.core_support(direction),
				0.0,
				&mut ContactScratch::default(),
			);

			// Boxes that are apart for both tests, or that all but touch, which may be reported either way.
			let Some(depth) = own.as_ref().or(through_gjk.as_ref()).map(Intersection::depth) else {
				continue;
			};
			if depth <= 1e-3 {
				continue;
			}
			let (Some(own), Some(through_gjk)) = (own, through_gjk) else {
				panic!("only one of the tests finds the boxes {depth} into each other");
			};
			overlapping += 1;
			assert!(
				(own.depth() - through_gjk.depth()).abs() <= 2e-4,
				"{own:?} and {through_gjk:?}"
			);
			// Moved out along the normal by a little more than that depth, the boxes are apart.
			let mut moved = turned_box(0.0);
			moved.collision_shape = b.collision_shape.clone();
			moved.orientation = b.orientation;
			moved.position = b.position + own.normal().into_vector() * (own.depth() + 1e-3);
			assert!(
				obb_vs_obb(&placed_a.oriented_box(*a_size), &Placement::of(&moved).oriented_box(*b_size)).is_none(),
				"{own:?}"
			);
		}
		assert!(overlapping > 200, "{overlapping} of 1000 overlap");
	}
}
