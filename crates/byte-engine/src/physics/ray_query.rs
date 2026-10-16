use math::{NormalizationError, Point, Ray, UnitVector};

use crate::core::factory::Handle;

/// The `RayQuery` struct asks the physics world which body a ray or a segment meets first, for gameplay that needs
/// to know what lies along a line, such as the ground under a character or what a shot hits.
///
/// Create it through the world's factory for `RayQuery`, as [`crate::gameplay::DefaultWorld`] does with
/// [`crate::core::factory::Creator::create`], and keep the handle that returns. The physics world answers every
/// query once, after its next step, by creating a [`RayQueryResponse`] that carries the query's handle. Read the
/// responses from a listener of the world's factory for `RayQueryResponse`, created before the query.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RayQuery {
	ray: Ray,
	max_distance: f32,
}

impl RayQuery {
	/// Creates a query for the first body that `ray` meets within `max_distance` of its origin. Pass infinity for a
	/// ray without an end.
	pub fn new(ray: Ray, max_distance: f32) -> Self {
		Self { ray, max_distance }
	}

	/// Creates a query for the first body that the segment from `start` to `end` meets, counted from `start`.
	///
	/// Returns an error when `start` and `end` are at the same place, which leaves the segment without a
	/// direction.
	pub fn segment(start: Point, end: Point) -> Result<Self, NormalizationError> {
		let (direction, length) = (end - start).normalize_with_length()?;
		Ok(Self::new(Ray::new(start, direction), length))
	}

	/// Returns the ray the query is made along.
	pub fn ray(&self) -> Ray {
		self.ray
	}

	/// Returns how far along the ray the query looks.
	pub fn max_distance(&self) -> f32 {
		self.max_distance
	}
}

/// The `RayQueryResponse` struct answers a [`RayQuery`], so that the gameplay that asked can match the answer to its
/// question by the query's handle.
///
/// The physics world creates one for every query, with a handle of its own. Read the query's handle with
/// [`Self::query`], and what the ray met with [`Self::hit`].
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RayQueryResponse {
	query: Handle,
	hit: Option<RayHit>,
}

impl RayQueryResponse {
	/// Creates the response to the query created with the handle `query`.
	pub(crate) fn new(query: Handle, hit: Option<RayHit>) -> Self {
		Self { query, hit }
	}

	/// Returns the handle the answered [`RayQuery`] was created with.
	pub fn query(&self) -> Handle {
		self.query
	}

	/// Returns where the ray first met a body, or `None` when it met none within its length.
	pub fn hit(&self) -> Option<RayHit> {
		self.hit
	}
}

/// The `RayHit` struct describes where a queried ray first meets a body, so that gameplay can act on that body and
/// place things on its surface.
///
/// A ray that starts inside a body, or on its surface, meets it at a distance of zero, with a normal that leads back
/// along the ray.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RayHit {
	body: Handle,
	distance: f32,
	point: Point,
	normal: UnitVector,
}

impl RayHit {
	/// Creates a hit on the body created with the handle `body`, `distance` along `ray`.
	pub(crate) fn new(body: Handle, ray: &Ray, distance: f32, normal: UnitVector) -> Self {
		Self {
			body,
			distance,
			point: ray.point_at(distance),
			normal,
		}
	}

	/// Returns the handle the body was created with.
	pub fn body(&self) -> Handle {
		self.body
	}

	/// Returns how far along the ray it meets the body.
	pub fn distance(&self) -> f32 {
		self.distance
	}

	/// Returns where the ray meets the body, in world space.
	pub fn point(&self) -> Point {
		self.point
	}

	/// Returns the body's outward surface normal where the ray meets it.
	pub fn normal(&self) -> UnitVector {
		self.normal
	}
}
