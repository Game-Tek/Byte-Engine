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
}

impl Body {
	/// Creates a body payload with default material and motion properties.
	pub fn new(body_type: BodyTypes, shape: Shapes) -> Self {
		Self {
			body_type,
			shape,
			velocity: Vector::zero(),
			mass: 1.0,
			center_of_mass: Point::origin(),
			elasticity: 0.1,
			friction: 0.5,
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
