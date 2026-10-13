use math::{Point, Vector};

use crate::physics::{LocalSpace, collider::Shapes};

/// The `Body` struct describes the non-spatial properties used to create a physics body.
///
/// Send this payload through a physics body factory, then publish a
/// [`crate::gameplay::transform::TransformationUpdate`] with the same handle to place it in the world.
#[derive(Debug, Clone)]
pub struct Body {
	body_type: BodyTypes,
	shape: Shapes,
	velocity: Vector,
	mass: f32,
	center_of_mass: Point<LocalSpace>,
	elasticity: f32,
	friction: f32,
	rotates: bool,
}

impl Body {
	/// Creates a body payload with default material and motion properties.
	///
	/// The center of mass starts in the middle of `shape`: the `center` of a [`Shapes::Capsule`], the
	/// [`crate::physics::ConvexHull::center_of_mass`] of a [`Shapes::ConvexHull`], and the collider origin for the
	/// other shapes. Replace it with [`Self::with_center_of_mass`].
	pub fn new(body_type: BodyTypes, shape: Shapes) -> Self {
		Self {
			body_type,
			velocity: Vector::zero(),
			mass: 1.0,
			center_of_mass: match &shape {
				Shapes::ConvexHull(hull) => hull.center_of_mass(),
				_ => shape.bounds().center(),
			},
			shape,
			elasticity: 0.1,
			friction: 0.5,
			rotates: true,
		}
	}

	/// Returns this payload with a replacement world-space linear velocity.
	pub fn with_velocity(self, velocity: Vector) -> Self {
		Self { velocity, ..self }
	}

	/// Returns this payload with a replacement mass in kilograms.
	pub fn with_mass(self, mass: f32) -> Self {
		Self { mass, ..self }
	}

	/// Returns this payload with a replacement collider-local center of mass.
	pub fn with_center_of_mass(self, center_of_mass: Point<LocalSpace>) -> Self {
		Self { center_of_mass, ..self }
	}

	/// Returns this payload with replacement collision elasticity.
	pub fn with_elasticity(self, elasticity: f32) -> Self {
		Self { elasticity, ..self }
	}

	/// Returns this payload with replacement collision friction.
	pub fn with_friction(self, friction: f32) -> Self {
		Self { friction, ..self }
	}

	/// Returns this payload with the body rotating or not.
	///
	/// A body that rotates is turned by its contacts, and its shape collides turned the way the body is. A body that
	/// does not is only ever moved by its contacts, and its shape collides as it was made whatever the body's
	/// orientation is. Turn rotation off for a character, so that its capsule stays upright when it is pushed and
	/// while an animation turns its transform.
	pub fn with_rotation(self, rotates: bool) -> Self {
		Self { rotates, ..self }
	}

	/// Returns how this body responds to simulation.
	pub fn body_type(&self) -> BodyTypes {
		self.body_type
	}

	/// Returns the collider-local geometry.
	pub fn shape(&self) -> &Shapes {
		&self.shape
	}

	/// Returns the initial world-space linear velocity.
	pub fn velocity(&self) -> Vector {
		self.velocity
	}

	/// Returns the mass in kilograms.
	pub fn mass(&self) -> f32 {
		self.mass
	}

	/// Returns the center of mass relative to the collider origin.
	pub fn center_of_mass(&self) -> Point<LocalSpace> {
		self.center_of_mass
	}

	/// Returns the collision elasticity.
	pub fn elasticity(&self) -> f32 {
		self.elasticity
	}

	/// Returns the collision friction.
	pub fn friction(&self) -> f32 {
		self.friction
	}

	/// Returns whether contacts turn the body and its shape follows its orientation.
	pub fn rotates(&self) -> bool {
		self.rotates
	}

	/// Takes the collider geometry without cloning its owned data.
	pub(crate) fn into_shape(self) -> Shapes {
		self.shape
	}
}

/// The `BodyTypes` enum selects how a physics body responds to forces and collisions.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BodyTypes {
	/// Ignores forces and collision resolution.
	Static,
	/// Ignores forces but participates in collision resolution.
	Kinematic,
	/// Responds to forces and collision resolution.
	Dynamic,
}
