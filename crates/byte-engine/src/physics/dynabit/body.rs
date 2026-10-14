use math::{
	AABB, Capsule, Orientation, OrientedBox, Point, Scale, Sphere, UnitVector, Vector,
	collision::{
		Intersection, ObbSeparation, aabb_vs_aabb, capsule_vs_aabb, capsule_vs_capsule, capsule_vs_sphere, obb_overlap,
		sphere_vs_aabb, sphere_vs_sphere, sphere_vs_sphere_dynamic,
	},
};
use smallvec::SmallVec;

use crate::{
	core::factory::Handle,
	physics::{
		LocalSpace,
		body::BodyTypes,
		collider::{Shapes, capsule_reach, sphere_moment},
		contact_patch::{FEATURE_SLACK, Feature, PatchCorner, box_side_patch, patch_corners},
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
	/// Whether the world simulates the body or lets it sleep.
	pub(crate) rest: Rest,
}

/// The `Rest` enum tells whether a physics world simulates a body or lets it sleep, so that a step costs nothing
/// for the bodies that lie still.
///
/// [`crate::physics::dynabit::sleep::Sleep`] puts bodies to sleep and wakes them. A step reads this to leave a
/// sleeping body out, and tells it here when a body moves.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Rest {
	/// The body is simulated.
	Awake {
		/// Whether the body has left where it was when the world last began to watch its bodies for rest, or the
		/// watch did not see where it was. A body only goes to sleep after a whole watch that it stayed there for.
		strayed: bool,
		/// Whether the body woke, or was created, during the current watch, which it then stays awake through.
		woke: bool,
		/// The first round of the current step's search for contacts that the body is awake for: zero when it
		/// began the step awake, and the round after the one that found what woke it otherwise.
		since: u32,
		/// Whether the body began the current step moving, so that it wakes the sleeping bodies it comes near. A
		/// body that woke during the step does not.
		stirs: bool,
	},
	/// The body lies still and no step moves it, until something wakes it. To the bodies that are awake, it is as
	/// fixed as one without mass. Bodies that a constraint holds together sleep and wake as one island.
	Asleep {
		/// The slot of the next body of the island, which leads around the island and back to this body.
		next: u32,
	},
}

impl Default for Rest {
	/// Returns the state a new body starts in: awake, and with a whole watch ahead of it before it may sleep.
	fn default() -> Self {
		Self::Awake {
			strayed: true,
			woke: true,
			since: 0,
			stirs: false,
		}
	}
}

impl PhysicsBody {
	/// The [`Self::active_since`] of a body that a step does not move.
	pub(crate) const NEVER_ACTIVE: u32 = u32::MAX;

	/// Returns whether the body sleeps.
	pub(crate) fn is_asleep(&self) -> bool {
		matches!(self.rest, Rest::Asleep { .. })
	}

	/// Returns the first round of the current step's search for contacts in which the body can move: zero for one
	/// that began the step awake. Returns [`Self::NEVER_ACTIVE`] for a body that the step does not move, because
	/// it sleeps or has no mass.
	///
	/// A contact changes nothing unless one of its bodies is active, so a pair of bodies is only tested in the
	/// round that the first of them is active in.
	#[inline]
	pub(crate) fn active_since(&self) -> u32 {
		match self.rest {
			Rest::Awake { since, .. } if self.inv_mass != 0.0 => since,
			_ => Self::NEVER_ACTIVE,
		}
	}

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
		self.to_world(self.center_of_mass)
	}

	/// Returns where a point of the body is in world coordinates. `local` is relative to the body's origin, and
	/// turned as the body is.
	pub fn to_world(&self, local: Point<LocalSpace>) -> Point {
		let local_offset = local - Point::origin();
		// Rotating a local offset into the world frame crosses a spatial math boundary.
		self.position + Vector::from_maths(self.orientation.into_maths() * local_offset.into_maths())
	}

	/// Returns where the body is, how it is turned, and how it spins once it has moved for `seconds` at its current
	/// linear and angular velocities, in that sequence. [`Self::update`] moves the body there.
	pub(crate) fn moved(&self, seconds: f32) -> (Point, Orientation, Vector) {
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
		(
			center_of_mass + delta_orientation.rotate_vector(center_offset),
			delta_orientation.compose(self.orientation),
			angular_velocity,
		)
	}

	/// Advances the body by `dt` using its current linear and angular velocities.
	pub fn update(&mut self, dt: MediaTime) {
		(self.position, self.orientation, self.angular_velocity) = self.moved(dt.as_seconds_f32());
	}

	/// Returns the current world-space axis-aligned bounds.
	// Every lane works this out for each of its bodies in every step. The world also asks it of bodies that sleep
	// or have no mass when something disturbs them, and with those calls the compiler left it out of line, which
	// made steps of a small pile slower.
	#[inline(always)]
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

		// A turned box reaches along each world axis as far as its three half sizes do together, each along its own
		// turned axis.
		let axes: [UnitVector; 3] = self.orientation.axes();
		let (half_sizes, middle) = (half_sizes.to_array(), middle.to_array());
		let (mut reach, mut center) = (Vector::new(radius, radius, radius), self.position);
		for axis in 0..3 {
			let side = axes[axis] * half_sizes[axis];
			reach += Vector::new(side.x().abs(), side.y().abs(), side.z().abs());
			center += axes[axis] * middle[axis];
		}
		AABB::from_center_and_half_extents(center, reach)
	}

	/// Returns the square of how far from the body's center of mass a point that the body's spin moves can be.
	///
	/// A shape that is turned the way its body is turns about the center of mass, so this is how far the furthest
	/// corner of the box around the shape is from there. Any other shape keeps facing the way it was made, and only
	/// goes around the center of mass with the body's position, so this is how far the position is from there.
	fn swing_squared(&self) -> f32 {
		let center_of_mass = self.center_of_mass.to_array();
		if !self.turns_shape() {
			return center_of_mass.iter().map(|offset| offset * offset).sum();
		}
		self.reach_squared(center_of_mass)
	}

	/// Returns the square of how far the furthest corner of the box around the body's shape is from `pivot`, a
	/// point in the body's own space. However the body is turned about `pivot`, no point of its shape is further.
	pub(crate) fn reach_squared(&self, pivot: [f32; 3]) -> f32 {
		let bounds = self.collision_shape.bounds();
		let (center, half_sizes) = (bounds.center().to_array(), bounds.half_extents().to_array());
		(0..3)
			.map(|axis| {
				let furthest = (center[axis] - pivot[axis]).abs() + half_sizes[axis];
				furthest * furthest
			})
			.sum()
	}

	/// Returns the most that the body's spin adds to how fast a point of its shape moves along `direction`, however
	/// the body is turned. Without a direction, returns the most that it adds along any.
	fn spin_speed(&self, direction: Option<UnitVector>) -> f32 {
		// A point moves along a direction as fast as its offset from the center of mass reaches along the direction
		// crossed with the spin, which is no longer than the spin, and no offset is longer than the swing.
		let across = direction.map_or(self.angular_velocity, |direction| direction.cross(self.angular_velocity));
		(self.swing_squared() * across.length_squared()).sqrt()
	}

	/// Returns whether the body moves far enough in the next `dt` seconds that [`intersect`] also tests it along its
	/// path, where every body is tested where it is now.
	///
	/// A body is tested along its path once a step carries a point of it further than half of what its shape
	/// reaches where it is thinnest, by its linear velocity and its spin together. A slower body cannot pass through
	/// another one between two steps: whatever it reaches, it still overlaps when the step ends. Testing it in place
	/// lets its contacts be found with exact bounds.
	pub fn outruns_its_shape(&self, dt: f32) -> bool {
		self.speed() * dt > self.collision_shape.thinnest_reach() * 0.5
	}

	/// Returns the most that any point of the body's shape moves in a second, by its linear velocity and its spin
	/// together.
	pub(crate) fn speed(&self) -> f32 {
		self.linear_velocity.length() + self.spin_speed(None)
	}

	/// Returns world-space bounds around everywhere [`intersect`] can find this body in contact during the next
	/// `dt` seconds.
	///
	/// A body that [`Self::outruns_its_shape`] is tested along its path, so its bounds cover the path at its
	/// current linear velocity and spin, slightly enlarged so bodies that are about to touch still count as
	/// overlapping. Any other body is only tested where it is now, so its bounds are exact and no pair is tested
	/// that cannot be in contact. Pass the bounds to [`crate::physics::SweepBounds::rebuild`] to find the bodies
	/// this one may collide with.
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
		// Spinning moves no point of the shape further than this from where the body's travel alone takes it.
		let spun = self.spin_speed(None) * dt;
		let reach = Vector::new(MARGIN + spun, MARGIN + spun, MARGIN + spun);
		let travel = self.linear_velocity * dt;
		let min = maths_rs::min(current.min().into_maths(), (current.min() + travel).into_maths());
		let max = maths_rs::max(current.max().into_maths(), (current.max() + travel).into_maths());
		AABB::new(Point::from_maths(min) - reach, Point::from_maths(max) + reach)
	}
}

/// How near each other two bodies that are tested along their paths have to come to count as touching, in meters.
/// GJK tells how far apart two shapes are to about a tenth of this.
const TOUCH_GAP: f32 = 0.001;

/// How many times at most two bodies are moved toward each other to find when they meet. Each time takes one GJK
/// search. Bodies that only move straight meet after one or two, and bodies that spin after a few more.
const ADVANCES: usize = 10;

/// Adds the contacts between two world-space bodies to `contacts`: none when they are apart, one where they touch
/// at a point, and up to four where they touch along a stretch or over a flat side and contacts turn one of them.
///
/// Bodies that are apart now but meet within the next `dt` seconds also get contacts when one of them
/// [`PhysicsBody::outruns_its_shape`], so that it does not pass through the other between two steps. Such a contact
/// tells how far into the step the bodies meet.
///
/// `scratch` is working storage for the pairs that go through GJK. Pass the same one for every pair that one thread
/// tests, so that testing them does not allocate.
// The search for contacts calls this for every pair of bodies whose bounds overlap and that it does not leave out.
// Made part of that search, it kept the rest of the search out of line, as a call for every pair of overlapping
// bounds, and steps of a small pile took longer.
#[inline(never)]
pub fn intersect(
	(a, i): (&PhysicsBody, usize),
	(b, j): (&PhysicsBody, usize),
	dt: f32,
	contacts: &mut Vec<Contact>,
	scratch: &mut ContactScratch,
) {
	let before = contacts.len();
	match own_contact((a, i), (b, j)) {
		Some(contact) => contacts.extend(contact),
		None => convex_contacts((a, i), (b, j), contacts, scratch),
	}
	if contacts.len() == before && (a.outruns_its_shape(dt) || b.outruns_its_shape(dt)) {
		meeting_contacts((a, i), (b, j), dt, contacts, scratch);
	}
}

/// Adds to `contacts` the contacts between two bodies that are apart now and meet within the next `dt` seconds, as
/// they move at their current velocities. A contact is between the points of the bodies that meet, placed where
/// they are on the bodies now, and has the time they meet at.
///
/// Two spheres meet at a time that follows from their velocities. Every other pair is moved together by conservative
/// advancement: GJK tells how far apart the shapes are and along which direction, the bodies' velocities bound how
/// fast they can close that gap, and so the bodies can be moved on for the time that takes at least without passing
/// through each other. That repeats from where it leaves them until they touch or the step ends.
// Few pairs get here, and every pair goes through `intersect`. When this was made part of `intersect`, steps of a
// small pile in which no body is fast took 2% to 3% longer on an Apple M4.
#[inline(never)]
fn meeting_contacts(
	(a, i): (&PhysicsBody, usize),
	(b, j): (&PhysicsBody, usize),
	dt: f32,
	contacts: &mut Vec<Contact>,
	scratch: &mut ContactScratch,
) {
	if let (Shapes::Sphere { radius: a_radius }, Shapes::Sphere { radius: b_radius }) = (&a.collision_shape, &b.collision_shape)
	{
		let (a_sphere, b_sphere) = (Sphere::new(a.position, *a_radius), Sphere::new(b.position, *b_radius));
		let impact = sphere_vs_sphere_dynamic(&a_sphere, &b_sphere, a.linear_velocity, b.linear_velocity, dt);
		contacts.extend(impact.map(|impact| {
			let mut contact = Contact::from_dynamic(i, j, impact);
			// The points are where the spheres will touch. Contacts are solved from where the bodies are now, so the
			// points move back to where they are on the spheres now.
			contact.a.point = contact.a.point - a.linear_velocity * contact.toi;
			contact.b.point = contact.b.point - b.linear_velocity * contact.toi;
			contact
		}));
		return;
	}

	let held_at_corners = a.is_turned_by_contacts() || b.is_turned_by_contacts();
	// However the bodies are turned, and along whatever direction, no point of one approaches a point of the other
	// faster than this.
	let top_speed = (a.linear_velocity - b.linear_velocity).length() + a.spin_speed(None) + b.spin_speed(None);
	let (mut placed_a, mut placed_b) = (Placement::of(a), Placement::of(b));
	let mut time = 0.0;
	for advance in 0..=ADVANCES {
		// Shapes that are further apart than the bodies can come in what is left of the step do not meet in it,
		// which GJK tells sooner than how far apart they are. Shapes whose cores share a place without any volume
		// between them have no direction to meet along.
		let Some(nearest) = placed_a.contact_with(&placed_b, top_speed * (dt - time), scratch) else {
			return;
		};
		let gap = -nearest.depth();
		// Bodies that are still apart after the last advance may meet in what is left of the step. Their contact lets
		// them come as near as they are known to get, which may hold them a little short of each other until the
		// next step, and never lets them pass.
		if gap <= TOUCH_GAP || advance == ADVANCES {
			let first = contacts.len();
			hold_at_features(contacts, (i, j), (&placed_a, &placed_b), nearest, held_at_corners);
			// Contacts are solved from where the bodies are now, so each point moves back with its body.
			let (now_a, now_b) = (Placement::of(a), Placement::of(b));
			for contact in &mut contacts[first..] {
				contact.toi = time;
				contact.a.point = now_a.to_world(placed_a.to_shape_point(contact.a.point));
				contact.b.point = now_b.to_world(placed_b.to_shape_point(contact.b.point));
			}
			return;
		}

		// The gap closes no faster than the bodies approach along it, added to the most their spins can move a point
		// of each that way. The direction stays as it is while the bodies turn, so this holds for as long as it
		// takes to close the gap.
		let toward_b = nearest.normal();
		let closing_speed =
			toward_b.dot(a.linear_velocity - b.linear_velocity) + a.spin_speed(Some(toward_b)) + b.spin_speed(Some(toward_b));
		// Bodies that do not approach along the gap get no nearer, and neither do bodies with a velocity that is not
		// a number.
		if closing_speed.is_nan() || closing_speed <= 0.0 {
			return;
		}
		time += gap / closing_speed;
		if time.is_nan() || time > dt {
			return;
		}
		(placed_a, placed_b) = (Placement::after(a, time), Placement::after(b, time));
	}
}

/// Returns the square of the sine of half the angle between two orientations, which is about the square of half
/// that angle when it is small.
pub(crate) fn half_turn_squared(from: Orientation, to: Orientation) -> f32 {
	// Two orientations an angle apart have the cosine of half of it as their product, with either sign.
	let alike = from.dot(to);
	(1.0 - alike * alike).max(0.0)
}

/// Returns whether [`intersect`] finds the contacts of two bodies where they are with a search over their shapes,
/// as it does for a pair with a hull or with a shape that is turned the way its body is. The shapes of any other
/// pair have a test of their own for each other, which costs less.
pub(crate) fn takes_search(a: &PhysicsBody, b: &PhysicsBody) -> bool {
	let hull = |body: &PhysicsBody| matches!(body.collision_shape, Shapes::ConvexHull(_));
	hull(a) || hull(b) || a.turns_shape() || b.turns_shape()
}

/// Returns the contact between two bodies whose shapes have a test of their own for each other, which may find
/// them apart. Returns `None` for a pair that has no such test: one with a hull, or with a shape that is turned the
/// way its body is.
// This hands its contact back, where it could add it to the list of contacts that its caller fills. Adding it in
// here made steps of every kind of pile 2% to 3% slower on an Apple M4.
fn own_contact((a, i): (&PhysicsBody, usize), (b, j): (&PhysicsBody, usize)) -> Option<Option<Contact>> {
	let touching = match (&a.collision_shape, &b.collision_shape) {
		(Shapes::Sphere { radius: a_radius }, Shapes::Sphere { radius: b_radius }) => {
			sphere_vs_sphere(&Sphere::new(a.position, *a_radius), &Sphere::new(b.position, *b_radius))
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
	// A body that cannot be turned is held as well at one point as at several. One that can be would rock on a single
	// point of the stretch or the patch that it touches the other over, so it is held at the patch's corners.
	let held_at_corners = a.is_turned_by_contacts() || b.is_turned_by_contacts();
	if let (Shapes::Cube { size: a_size }, Shapes::Cube { size: b_size }) = (&a.collision_shape, &b.collision_shape) {
		// Boxes that are further apart than their corners reach from their middles cannot touch, however they are
		// turned, which is told without working out how they are turned.
		let reach = a_size.length() + b_size.length();
		if (b.position - a.position).length_squared() > reach * reach {
			return;
		}
		let (placed_a, placed_b) = (Placement::of(a), Placement::of(b));
		let (box_a, box_b) = (placed_a.oriented_box(*a_size), placed_b.oriented_box(*b_size));
		let Some(overlap) = obb_overlap(&box_a, &box_b) else {
			return;
		};
		// Boxes touch over a patch when a side of one faces the other. Edges that cross touch at one point.
		let corners = match overlap.separation() {
			_ if !held_at_corners => SmallVec::new(),
			ObbSeparation::SideOfA(axis) => box_side_patch(&box_a, axis, overlap.normal(), &box_b, true),
			ObbSeparation::SideOfB(axis) => box_side_patch(&box_b, axis, -overlap.normal(), &box_a, false),
			ObbSeparation::Edges(..) => SmallVec::new(),
		};
		if corners.is_empty() {
			contacts.push(Contact::from_static(i, j, overlap.contact(&box_a, &box_b)));
		}
		return hold(contacts, (i, j), overlap.normal(), corners);
	}

	let (placed_a, placed_b) = (Placement::of(a), Placement::of(b));
	if let Some(deepest) = placed_a.contact_with(&placed_b, 0.0, scratch) {
		hold_at_features(contacts, (i, j), (&placed_a, &placed_b), deepest, held_at_corners);
	}
}

/// Adds to `contacts` the contacts between the bodies in slots `i` and `j`, whose shapes as they are `placed` touch
/// at `nearest`, which [`Placement::contact_with`] found: one for each corner of the patch they touch over when
/// they are `held_at_corners` and touch over one, and otherwise the one that `nearest` is.
#[inline]
fn hold_at_features(
	contacts: &mut Vec<Contact>,
	(i, j): (usize, usize),
	(placed_a, placed_b): (&Placement, &Placement),
	nearest: Intersection,
	held_at_corners: bool,
) {
	// GJK's two points are one pair that is as deep in each other as the shapes are. Where one of the shapes touches
	// with a single corner, that corner is the pair.
	let mut corners = SmallVec::new();
	if held_at_corners {
		let reach = nearest.depth() + FEATURE_SLACK;
		let feature_a = placed_a.core_feature(nearest.normal().into(), reach);
		let feature_b = placed_b.core_feature(-nearest.normal().into_vector(), reach);
		if feature_a.len() > 1 && feature_b.len() > 1 {
			let (radius_a, radius_b) = (core_radius(placed_a.shape), core_radius(placed_b.shape));
			corners = patch_corners(&feature_a, radius_a, &feature_b, radius_b, nearest.normal());
		}
	}
	let normal = nearest.normal();
	if corners.is_empty() {
		contacts.push(Contact::from_static(i, j, nearest));
	}
	hold(contacts, (i, j), normal, corners);
}

/// Adds to `contacts` one contact between the bodies in slots `i` and `j` for each corner of the patch they touch
/// over, along `normal`.
fn hold(
	contacts: &mut Vec<Contact>,
	(i, j): (usize, usize),
	normal: UnitVector,
	corners: SmallVec<[PatchCorner<math::WorldSpace>; 4]>,
) {
	contacts.extend(corners.into_iter().map(|held| Contact {
		corner: held.number,
		..Contact::from_static(i, j, Intersection::new(normal, held.depth, held.on_a, held.on_b))
	}));
}

/// The `Placement` struct answers questions about the core of a body's shape in world space: the shape itself for a
/// box and a hull, the center of a sphere, and the segment between the centers of a capsule's caps. [`core_radius`]
/// is how far the shape reaches beyond its core.
///
/// Place a body's shape where the body is with [`Self::of`], or where it will be with [`Self::after`].
struct Placement<'a> {
	shape: &'a Shapes,
	/// Where the body is, which the shape is placed at.
	position: Point,
	/// For a shape that is turned the way its body is: where the shape's own x, y, and z axes point in world space.
	/// Going between the two frames with these takes a few products, where turning by the body's orientation would
	/// work through its quaternion for every point asked about.
	axes: Option<[UnitVector; 3]>,
}

impl<'a> Placement<'a> {
	/// Places the shape of `body` where the body is now.
	fn of(body: &'a PhysicsBody) -> Self {
		Self {
			shape: &body.collision_shape,
			position: body.position,
			axes: body.turns_shape().then(|| body.orientation.axes()),
		}
	}

	/// Places the shape of `body` where the body is once it has moved for `seconds` at its current velocities.
	fn after(body: &'a PhysicsBody, seconds: f32) -> Self {
		let (position, orientation, _) = body.moved(seconds);
		Self {
			shape: &body.collision_shape,
			position,
			axes: body.turns_shape().then(|| orientation.axes()),
		}
	}

	/// Returns where this shape touches the shape `other` places, with this one as the first shape of the
	/// intersection. Shapes that are apart by no more than `within` get an intersection with a negative depth, as
	/// [`rounded_shapes_contact`] describes. Pass the result to [`hold_at_features`].
	#[inline]
	fn contact_with(&self, other: &Self, within: f32, scratch: &mut ContactScratch) -> Option<Intersection> {
		rounded_shapes_contact(
			|direction| self.core_support(direction),
			core_radius(self.shape),
			|direction| other.core_support(direction),
			core_radius(other.shape),
			Some(other.position - self.position),
			Some(self.extent_with(other)),
			within,
			scratch,
		)
	}

	/// Returns the body's box, for a collider that is a [`Shapes::Cube`] with these half sizes.
	fn oriented_box(&self, half_sizes: Vector<LocalSpace>) -> OrientedBox {
		let axes = self
			.axes
			.unwrap_or([UnitVector::x_axis(), UnitVector::y_axis(), UnitVector::z_axis()]);
		OrientedBox::from_axes(self.position, half_sizes.to_array(), axes)
	}

	/// Returns a bound on how far apart a point of this body's shape and a point of `other`'s can be along any one
	/// coordinate axis: no further than the bodies are apart, added to how far each shape reaches from its body
	/// however it is turned.
	fn extent_with(&self, other: &Self) -> f32 {
		let reach = |shape: &Shapes| {
			let bounds = shape.bounds();
			(bounds.center() - Point::origin()).length() + bounds.half_extents().length()
		};
		let apart = (other.position - self.position).to_array();
		apart.iter().fold(0.0, |most: f32, part| most.max(part.abs())) + reach(self.shape) + reach(other.shape)
	}

	/// Returns a world-space direction in the shape's own frame.
	fn to_shape(&self, direction: Vector) -> Vector<LocalSpace> {
		match self.axes {
			Some([x, y, z]) => Vector::new(x.dot(direction), y.dot(direction), z.dot(direction)),
			// Crossing from world simulation to authored local geometry requires an explicit raw boundary conversion.
			None => Vector::from_maths(direction.into_maths()),
		}
	}

	/// Returns where a world-space point is in the shape's own frame, which [`Self::to_world`] undoes.
	fn to_shape_point(&self, world: Point) -> Point<LocalSpace> {
		Point::origin() + self.to_shape(world - self.position)
	}

	/// Returns the world-space position of a point of the shape.
	fn to_world(&self, local: Point<LocalSpace>) -> Point {
		self.position
			+ match self.axes {
				Some([x, y, z]) => x * local.x() + y * local.y() + z * local.z(),
				// Crossing from authored local geometry to world simulation requires an explicit raw boundary conversion.
				None => Vector::from_maths(local.into_maths()),
			}
	}

	/// Returns the point of the core that is furthest along `direction`.
	fn core_support(&self, direction: Vector) -> Point {
		let direction = self.to_shape(direction);
		self.to_world(match self.shape {
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
			Shapes::ConvexHull(hull) => hull.core_support(direction),
		})
	}

	/// Returns the corners of the core that are within `reach` of as far along `direction` as the core gets: the
	/// part of the shape that touches another shape in that direction.
	fn core_feature(&self, direction: Vector, reach: f32) -> Feature<math::WorldSpace> {
		let direction = self.to_shape(direction);
		let along = |corner: Point<LocalSpace>| direction.dot(corner - Point::origin());
		let mut feature = Feature::new();
		match self.shape {
			Shapes::Sphere { .. } => feature.push(self.to_world(Point::origin())),
			Shapes::Cube { size } => {
				// A corner is short of the furthest one by what each axis on which it is on the far side adds up to.
				let half = size.to_array().map(f32::abs);
				let lead = [direction.x(), direction.y(), direction.z()];
				for corner in 0..8 {
					let mut short = 0.0;
					let mut place = [0.0; 3];
					for axis in 0..3 {
						let near = (corner >> axis & 1 == 1) == (lead[axis] >= 0.0);
						place[axis] = if corner >> axis & 1 == 1 { half[axis] } else { -half[axis] };
						if !near {
							short += 2.0 * half[axis] * lead[axis].abs();
						}
					}
					if short <= reach {
						feature.push(self.to_world(Point::from_array(place)));
					}
				}
			}
			Shapes::Capsule { radius, height, center } => {
				let reach_along = Vector::new(0.0, capsule_reach(*radius, *height), 0.0);
				let ends = [*center - reach_along, *center + reach_along];
				let furthest = along(ends[0]).max(along(ends[1]));
				feature.extend(
					ends.into_iter()
						.filter(|&end| along(end) >= furthest - reach)
						.map(|end| self.to_world(end)),
				);
			}
			Shapes::ConvexHull(hull) => {
				let furthest = along(hull.core_support(direction));
				feature.extend(
					hull.core_vertices()
						.iter()
						.filter(|&&corner| along(corner) >= furthest - reach)
						.map(|&corner| self.to_world(corner)),
				);
			}
		}
		feature
	}
}

/// Returns how far `shape` reaches beyond its core in every direction, which is the radius of a sphere and of a
/// capsule, the margin of a hull, and nothing for a box.
fn core_radius(shape: &Shapes) -> f32 {
	match shape {
		Shapes::Sphere { radius } | Shapes::Capsule { radius, .. } => *radius,
		Shapes::ConvexHull(hull) => hull.margin(),
		Shapes::Cube { .. } => 0.0,
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
	use math::collision::obb_vs_obb;

	use super::*;
	use crate::physics::Shapes;

	/// Returns the contacts between two bodies over a step of `dt` seconds.
	fn touches(a: (&PhysicsBody, usize), b: (&PhysicsBody, usize), dt: f32) -> Vec<Contact> {
		let mut contacts = Vec::new();
		intersect(a, b, dt, &mut contacts, &mut ContactScratch::default());
		contacts
	}

	/// Returns the contact between two bodies that touch at one place, or none when they are apart.
	fn touch(a: (&PhysicsBody, usize), b: (&PhysicsBody, usize), dt: f32) -> Option<Contact> {
		let mut contacts = touches(a, b, dt);
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
			rest: Rest::default(),
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
	fn swept_bounds_of_a_slow_box_are_its_bounds() {
		let mut body = body();
		body.collision_shape = Shapes::cube(Vector::new(0.5, 1.0, 1.5));
		body.position = Point::new(2.0, 0.0, 0.0);
		// The box is half a unit thick where it is thinnest and moves less than half of that in the step.
		body.linear_velocity = Vector::new(0.2, 0.0, 0.0);

		let swept = body.swept_bounds(1.0);

		assert_eq!(swept.min(), body.bounds().min());
		assert_eq!(swept.max(), body.bounds().max());
	}

	#[test]
	fn swept_bounds_of_a_fast_box_hold_it_all_along_its_path_as_it_turns() {
		let fast_box = || {
			let mut body = body();
			body.collision_shape = Shapes::cube(Vector::new(0.5, 1.0, 1.5));
			body.rotates = true;
			body.position = Point::new(2.0, 0.0, 0.0);
			body.linear_velocity = Vector::new(10.0, -4.0, 0.0);
			body.angular_velocity = Vector::new(0.0, 0.0, 3.0);
			body
		};
		let swept = fast_box().swept_bounds(1.0);

		for tenth in 0..=10 {
			let mut on_the_way = fast_box();
			(on_the_way.position, on_the_way.orientation, _) = fast_box().moved(tenth as f32 * 0.1);
			let bounds = on_the_way.bounds();

			for axis in 0..3 {
				assert!(
					swept.min().to_array()[axis] <= bounds.min().to_array()[axis],
					"after {tenth} tenths"
				);
				assert!(
					swept.max().to_array()[axis] >= bounds.max().to_array()[axis],
					"after {tenth} tenths"
				);
			}
		}
	}

	/// Returns a wall that nothing moves, a tenth of a unit thick, whose side that faces down the x-axis is at
	/// `face`.
	fn thin_wall(face: f32) -> PhysicsBody {
		let mut wall = body();
		wall.body_type = BodyTypes::Static;
		wall.collision_shape = Shapes::cube(Vector::new(0.05, 4.0, 4.0));
		wall.position = Point::new(face + 0.05, 0.0, 0.0);
		wall.angular_velocity = Vector::zero();
		wall.inv_mass = 0.0;
		wall
	}

	#[test]
	fn fast_bodies_touch_a_thin_wall_where_their_paths_meet_it() {
		let mut sphere = body();
		sphere.collision_shape = Shapes::sphere(0.5);
		let mut cube = body();
		cube.collision_shape = Shapes::cube(Vector::new(0.5, 0.5, 0.5));
		let mut capsule = body();
		capsule.collision_shape = Shapes::capsule(0.5, 2.0, Point::origin());
		let mut hull = body();
		hull.collision_shape = diamond();
		// Each body reaches this far toward the wall.
		let reaches = [0.5, 0.5, 0.5, 1.0 - diamond_tip_rounding()];
		let wall = thin_wall(5.0);

		for (mut fast, reach) in [sphere, cube, capsule, hull].into_iter().zip(reaches) {
			// The step would carry the body ten units, to the far side of the wall.
			fast.linear_velocity = Vector::new(20.0, 0.0, 0.0);
			let shape = fast.collision_shape.clone();

			let contact = touch((&fast, 0), (&wall, 1), 0.5).expect("the path reaches the wall");

			assert!((contact.toi - (5.0 - reach) / 20.0).abs() <= 1e-4, "{shape:?}: {contact:?}");
			assert!(
				(contact.normal.into_vector() - Vector::new(1.0, 0.0, 0.0)).length() <= 1e-4,
				"{shape:?}: {contact:?}"
			);
			// The contact points are where the facing points of the bodies are now.
			assert!((contact.a.point.x() - reach).abs() <= 2e-3, "{shape:?}: {contact:?}");
			assert!((contact.b.point.x() - 5.0).abs() <= 2e-3, "{shape:?}: {contact:?}");
			// The bodies are a hair short of touching at most.
			assert!(contact.depth.abs() <= 2e-3, "{shape:?}: {contact:?}");

			// The same contact is seen from the wall's side.
			let backward = touch((&wall, 0), (&fast, 1), 0.5).expect("the path reaches the wall");
			assert!((backward.toi - contact.toi).abs() <= 1e-4, "{shape:?}: {backward:?}");
			assert!((backward.a.point.x() - 5.0).abs() <= 2e-3, "{shape:?}: {backward:?}");
			assert!((backward.b.point.x() - reach).abs() <= 2e-3, "{shape:?}: {backward:?}");
		}
	}

	#[test]
	fn fast_bodies_that_do_not_reach_each_other_in_the_step_do_not_touch() {
		let fast_box = |velocity: Vector| {
			let mut cube = body();
			cube.collision_shape = Shapes::cube(Vector::new(0.5, 0.5, 0.5));
			cube.linear_velocity = velocity;
			cube
		};
		let wall = thin_wall(5.0);

		// The wall is 4.5 units from the box: further than the box gets, behind it, and beside its path.
		assert!(touches((&fast_box(Vector::new(20.0, 0.0, 0.0)), 0), (&wall, 1), 0.2).is_empty());
		assert!(touches((&fast_box(Vector::new(-20.0, 0.0, 0.0)), 0), (&wall, 1), 0.5).is_empty());
		assert!(touches((&fast_box(Vector::new(0.0, 20.0, 0.0)), 0), (&wall, 1), 0.5).is_empty());
		// A box that passes the edge of the wall, which reaches four units up, misses it.
		let mut above = fast_box(Vector::new(20.0, 0.0, 0.0));
		above.position = Point::new(0.0, 4.6, 0.0);
		assert!(touches((&above, 0), (&wall, 1), 0.5).is_empty());
	}

	#[test]
	fn a_fast_box_that_contacts_turn_meets_a_wall_at_the_corners_of_its_side() {
		let mut cube = body();
		cube.collision_shape = Shapes::cube(Vector::new(0.5, 0.5, 0.5));
		cube.rotates = true;
		cube.angular_velocity = Vector::zero();
		cube.linear_velocity = Vector::new(20.0, 0.0, 0.0);

		let contacts = touches((&cube, 0), (&thin_wall(5.0), 1), 0.5);

		// Held at one point of its side, the box would turn about it as it is stopped.
		assert_eq!(contacts.len(), 4);
		let mut corners: Vec<_> = contacts.iter().map(|contact| contact.corner).collect();
		corners.sort_unstable();
		assert_eq!(corners, [0, 1, 2, 3]);
		for contact in &contacts {
			assert!((contact.toi - 0.225).abs() <= 1e-4, "{contact:?}");
			let on_box = contact.a.point.to_array();
			assert!((on_box[0] - 0.5).abs() <= 2e-3, "{contact:?}");
			assert!(
				(on_box[1].abs() - 0.5).abs() <= 1e-3 && (on_box[2].abs() - 0.5).abs() <= 1e-3,
				"{contact:?}"
			);
		}
	}

	#[test]
	fn a_fast_spinning_bar_meets_a_wall_when_its_end_swings_into_it() {
		// A bar along the x-axis that spins about the z-axis, so that its end at x = 1 swings up.
		let mut bar = body();
		bar.collision_shape = Shapes::cube(Vector::new(1.0, 0.05, 0.05));
		bar.rotates = true;
		bar.angular_velocity = Vector::new(0.0, 0.0, 20.0);
		// A wall whose underside is half a unit above the bar's middle.
		let mut wall = thin_wall(0.0);
		wall.collision_shape = Shapes::cube(Vector::new(4.0, 0.5, 4.0));
		wall.position = Point::new(0.0, 1.0, 0.0);

		let contacts = touches((&bar, 0), (&wall, 1), 0.1);

		// The upper corner of the bar's end is 1 along the bar and 0.05 across it, so once the bar has turned by an
		// angle it is at a height of sin(angle) + 0.05 cos(angle), which is 0.5 at an angle of 0.47292.
		let meets = 0.47292 / 20.0;
		assert!(!contacts.is_empty());
		for contact in &contacts {
			assert!(contact.toi <= meets + 1e-5 && contact.toi >= meets - 2e-4, "{contact:?}");
			assert!(
				(contact.normal.into_vector() - Vector::new(0.0, 1.0, 0.0)).length() <= 1e-3,
				"{contact:?}"
			);
			// The point on the bar is on the upper edge of its end, where that edge is now.
			let on_bar = contact.a.point.to_array();
			assert!(
				(on_bar[0] - 1.0).abs() <= 2e-3 && (on_bar[1] - 0.05).abs() <= 2e-3,
				"{contact:?}"
			);
			assert!(on_bar[2].abs() <= 0.05 + 1e-3, "{contact:?}");
		}

		// A bar that spins a hundredth as fast moves less than it is thick in the step, so it is tested where it is.
		bar.angular_velocity = Vector::new(0.0, 0.0, 0.2);
		assert!(!bar.outruns_its_shape(0.1));
		assert!(touches((&bar, 0), (&wall, 1), 0.1).is_empty());
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

	/// Returns how far the corners and edges of [`diamond`] are rounded when it collides.
	fn diamond_margin() -> f32 {
		let Shapes::ConvexHull(hull) = diamond() else {
			unreachable!("the diamond is a hull");
		};
		hull.margin()
	}

	/// Returns how far short of where its corner is a tip of [`diamond`] ends once it is rounded. The four sides at
	/// a tip each move in by the margin, which moves where they meet in by the square root of three times as much,
	/// and the rounding reaches the margin back out from there.
	fn diamond_tip_rounding() -> f32 {
		diamond_margin() * (3f32.sqrt() - 1.0)
	}

	#[test]
	fn a_hull_touches_a_sphere_a_box_and_a_capsule_with_its_tip() {
		let mut hull = body();
		hull.collision_shape = diamond();
		// The hull's tip at x = 1 is rounded, so it ends a little short of there. Each shape's near side is 0.25
		// inside x = 1, and lower than the tip, so the tip touches a flat or upright part of each.
		let tip = 1.0 - diamond_tip_rounding();
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
			assert!((contact.depth - (tip - 0.75)).abs() <= 1e-5, "{contact:?}");
			assert!((contact.a.point - Point::new(tip, 0.0, 0.0)).length() <= 1e-5, "{contact:?}");
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

		// The tip is rounded, so it is a little less deep in the floor than the hull's corner is.
		assert!((upright.depth - (0.1 - diamond_tip_rounding())).abs() <= 1e-5);
		assert_eq!(
			(turned.depth, turned.a.point, turned.b.point),
			(upright.depth, upright.a.point, upright.b.point)
		);
		assert_eq!(hull.bounds().min(), Point::new(1.0, 0.0, -1.0));
		assert_eq!(hull.bounds().max(), Point::new(3.0, 2.0, 1.0));
		assert_eq!(hull.swept_bounds(1.0).min(), hull.bounds().min());
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

		let contacts = touches((&cube, 0), (&floor, 1), 1.0 / 60.0);

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
		assert_eq!(touches((&cube, 0), (&floor, 1), 1.0 / 60.0).len(), 1);

		// Standing on an edge, the box that rotates is held at the two ends of that edge.
		cube.rotates = true;
		cube.orientation = about_z(std::f32::consts::FRAC_PI_4);
		cube.position = Point::new(0.0, std::f32::consts::FRAC_1_SQRT_2 - 0.01, 0.0);
		let contacts = touches((&cube, 0), (&floor, 1), 1.0 / 60.0);
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

		let contacts = touches((&capsule, 0), (&floor, 1), 1.0 / 60.0);

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
	fn a_hull_rests_on_a_flat_side_as_deep_as_the_side_is() {
		let floor = floor();
		// A hull with the corners of a cube, a hundredth into the floor. That is less than the hull's margin, so
		// the hull's core is clear of the floor, as it is for every hull at rest.
		let mut corners = Vec::new();
		for x in [-0.5, 0.5] {
			for y in [-0.5, 0.5] {
				for z in [-0.5, 0.5] {
					corners.push(Point::new(x, y, z));
				}
			}
		}
		let cube = crate::physics::build_convex_hull(&corners).expect("a cube encloses a volume");
		let margin = cube.margin();
		let mut hull = body();
		hull.collision_shape = Shapes::convex_hull(cube);
		hull.rotates = true;
		hull.position = Point::new(0.0, 0.49, 0.0);

		let contacts = touches((&hull, 0), (&floor, 1), 1.0 / 60.0);

		// The side is as deep in the floor as the cube's is, and it is held where its rounded edges begin.
		assert_eq!(contacts.len(), 4, "{contacts:?}");
		for contact in &contacts {
			assert!((contact.depth - 0.01).abs() <= 1e-4, "{contact:?}");
			assert!(
				(contact.normal.into_vector() - Vector::new(0.0, -1.0, 0.0)).length() <= 1e-5,
				"{contact:?}"
			);
			let on_floor = contact.b.point.to_array();
			assert!(on_floor[1].abs() <= 1e-4, "{contact:?}");
			assert!(
				(on_floor[0].abs() - (0.5 - margin)).abs() <= 1e-4 && (on_floor[2].abs() - (0.5 - margin)).abs() <= 1e-4,
				"{contact:?}"
			);
		}
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

		let contacts = touches((&hull, 0), (&floor, 1), 1.0 / 60.0);

		// The edge is rounded. The two sides that meet at it each move in by the margin, which moves the edge in by
		// the square root of one and a half times as much, and the rounding reaches the margin back out from there.
		// The edge ends where the rounding of the two tips begins.
		let margin = diamond_margin();
		let depth = 0.01 - margin * (1.5f32.sqrt() - 1.0);
		let end = (1.0 - margin * 3f32.sqrt()) * std::f32::consts::FRAC_1_SQRT_2;
		assert_eq!(contacts.len(), 2, "{contacts:?}");
		for x in [-end, end] {
			assert!(
				contacts
					.iter()
					.any(|contact| (contact.b.point - Point::new(x, 0.0, 0.0)).length() <= 1e-4
						&& (contact.depth - depth).abs() <= 1e-4),
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
				None,
				Some(placed_a.extent_with(&placed_b)),
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
