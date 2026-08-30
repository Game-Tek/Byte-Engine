/// The `Transform` struct stores an entity's world location, scale, and orientation.
///
/// Use this type for gameplay entities that implement [`crate::space::Transformable`].
/// Facet serializes it as `position: [x, y, z]`, `scale: [x, y, z]`, and
/// `orientation: [x, y, z, w]`. Deserialization rejects unknown fields,
/// non-finite components, and invalid orientations.
#[derive(Debug, Clone, facet::Facet)]
#[facet(opaque, proxy = SerializableTransform)]
pub struct Transform {
	position: Point,
	scale: Scale,
	orientation: Orientation,
}

/// The `SerializableTransform` struct defines the checked reflection boundary for engine transforms.
#[derive(facet::Facet)]
#[facet(deny_unknown_fields)]
struct SerializableTransform {
	position: [f32; 3],
	scale: [f32; 3],
	orientation: [f32; 4],
}

impl TryFrom<SerializableTransform> for Transform {
	type Error = String;

	fn try_from(value: SerializableTransform) -> Result<Self, Self::Error> {
		if !value.position.into_iter().all(f32::is_finite)
			|| !value.scale.into_iter().all(f32::is_finite)
			|| !value.orientation.into_iter().all(f32::is_finite)
		{
			return Err(
				"Transform payload is invalid. The most likely cause is that a position, scale, or orientation component is not finite."
					.to_string(),
			);
		}

		let [x, y, z, w] = value.orientation;
		let orientation = Orientation::try_from_maths(math::Quaternion::new(x, y, z, w)).map_err(|error| {
			format!(
				"Transform orientation is invalid. The most likely cause is that the quaternion is zero-length or non-finite: {error}"
			)
		})?;
		let [x, y, z] = value.position;
		let position = Point::new(x, y, z);
		let [x, y, z] = value.scale;

		Ok(Self::new(position, Scale::new(x, y, z), orientation))
	}
}

impl From<&Transform> for SerializableTransform {
	fn from(value: &Transform) -> Self {
		let orientation = value.orientation.into_maths();
		Self {
			position: [value.position.x(), value.position.y(), value.position.z()],
			scale: [value.scale.x(), value.scale.y(), value.scale.z()],
			orientation: [orientation.x, orientation.y, orientation.z, orientation.w],
		}
	}
}

impl Default for Transform {
	fn default() -> Self {
		Self::identity()
	}
}

impl Transform {
	/// Creates an identity transform at the world origin.
	pub fn identity() -> Self {
		Self::new(Point::origin(), Scale::identity(), Orientation::identity())
	}

	/// Creates a transform from a world position, scale, and orientation.
	pub fn new(position: Point, scale: Scale, orientation: Orientation) -> Self {
		Self {
			position,
			scale,
			orientation,
		}
	}

	/// Returns this transform with a replacement world position.
	pub fn position(self, position: Point) -> Self {
		Self { position, ..self }
	}

	/// Returns this transform with a replacement orientation.
	pub fn rotation(self, orientation: Orientation) -> Self {
		Self { orientation, ..self }
	}

	/// Creates an identity-oriented transform at `position`.
	pub fn from_position(position: Point) -> Self {
		Self::new(position, Scale::identity(), Orientation::identity())
	}

	/// Creates a transform that changes only the scale.
	pub fn from_scale(scale: Scale) -> Self {
		Self::new(Point::origin(), scale, Orientation::identity())
	}

	/// Creates a transform that changes only the orientation.
	pub fn from_rotation(orientation: Orientation) -> Self {
		Self::new(Point::origin(), Scale::identity(), orientation)
	}

	/// Builds the renderer-facing affine matrix with scale applied before rotation and translation.
	pub fn get_matrix(&self) -> Matrix {
		Matrix::from_translation(self.position.into_maths())
			* self.orientation.into_matrix()
			* Matrix::from_scale(self.scale.into_maths())
	}

	/// Replaces the world position.
	pub fn set_position(&mut self, position: Point) {
		self.position = position;
	}

	/// Returns the world position.
	pub fn get_position(&self) -> Point {
		self.position
	}

	/// Replaces the scale.
	pub fn set_scale(&mut self, scale: Scale) {
		self.scale = scale;
	}

	/// Returns the scale.
	pub fn scale(&self) -> Scale {
		self.scale
	}

	/// Replaces the orientation.
	pub fn set_orientation(&mut self, orientation: Orientation) {
		self.orientation = orientation;
	}

	/// Returns the orientation.
	pub fn get_orientation(&self) -> Orientation {
		self.orientation
	}
}

impl From<&Transform> for Matrix {
	fn from(transform: &Transform) -> Self {
		transform.get_matrix()
	}
}

impl Positionable for Transform {
	fn position(&self) -> Point {
		self.position
	}

	fn set_position(&mut self, position: Point) {
		self.position = position;
	}
}

impl Orientable for Transform {
	fn orientation(&self) -> Orientation {
		self.orientation
	}

	fn set_orientation(&mut self, orientation: Orientation) {
		self.orientation = orientation;
	}
}

/// The `TransformationUpdate` type keeps transform creation and replacement on one typed route with a reflected payload.
pub type TransformationUpdate = CreateMessage<Transform>;

impl CreateMessage<Transform> {
	/// Publishes a transform update to `channel`.
	pub fn apply(channel: &DefaultChannel<Self>, handle: Handle, transform: Transform) {
		channel.send(Self::new(handle, transform));
	}

	/// Returns the transform payload.
	pub fn transform(&self) -> &Transform {
		self.data()
	}
}

#[cfg(test)]
mod tests {
	use math::{Orientation, Point, Scale, UnitVector, WorldSpace};
	use maths_rs::Vec4f;

	use super::{Transform, TransformationUpdate};
	use crate::{
		core::{
			channel::{Channel, DefaultChannel},
			factory::Factory,
			listener::Listener,
		},
		space::{Orientable, Positionable, Scalable, Transformable},
	};

	struct SpatialEntity {
		transform: Transform,
	}

	impl Transformable for SpatialEntity {
		fn transform(&self) -> &Transform {
			&self.transform
		}

		fn transform_mut(&mut self) -> &mut Transform {
			&mut self.transform
		}
	}

	#[test]
	fn matrix_applies_scale_before_translation() {
		let transform = Transform::new(
			Point::new(10.0, 20.0, 30.0),
			Scale::new(2.0, 3.0, 4.0),
			Orientation::identity(),
		);

		assert_eq!(
			transform.get_matrix() * Vec4f::new(1.0, 1.0, 1.0, 1.0),
			Vec4f::new(12.0, 23.0, 34.0, 1.0)
		);
	}

	#[test]
	fn transformable_traits_share_one_transform() {
		let orientation = Orientation::try_from_axis_angle(UnitVector::<WorldSpace>::x_axis(), math::Radians::new(0.25))
			.expect("finite axis-angle orientation");
		let mut entity = SpatialEntity {
			transform: Transform::default(),
		};

		entity.set_position(Point::new(3.0, 4.0, 5.0));
		entity.set_scale(Scale::new(2.0, 2.0, 2.0));
		entity.set_orientation(orientation);

		assert_eq!(entity.position(), Point::new(3.0, 4.0, 5.0));
		assert_eq!(entity.scale(), Scale::new(2.0, 2.0, 2.0));
		assert_eq!(entity.orientation(), orientation);
	}

	#[test]
	fn transformation_update_preserves_handle_and_payload() {
		let mut factory = Factory::new();
		let handle = factory.create("entity");
		let transform = Transform::from_position(Point::new(7.0, 8.0, 9.0));
		let channel = DefaultChannel::new();
		let mut listener = channel.listener();

		TransformationUpdate::apply(&channel, handle, transform);
		let update = listener.read().expect("transformation update");

		assert_eq!(update.handle(), handle);
		assert_eq!(update.transform().get_position(), Point::new(7.0, 8.0, 9.0));
	}

	#[test]
	fn transformation_update_payload_has_a_reflected_json_shape() {
		let mut factory = Factory::new();
		let handle = factory.create("entity");
		let update = TransformationUpdate::new(
			handle,
			Transform::new(Point::new(1.0, 2.0, 3.0), Scale::new(4.0, 5.0, 6.0), Orientation::identity()),
		);

		let json = facet_json::to_string(update.transform()).expect("serialize reflected transform payload");
		let value: serde_json::Value = serde_json::from_str(&json).expect("parse reflected transform payload");

		assert_eq!(
			value,
			serde_json::json!({
				"position": [1, 2, 3],
				"scale": [4, 5, 6],
				"orientation": [0, 0, 0, 1]
			})
		);
	}
}

use math::{Matrix, Orientation, Point, Scale};
use maths_rs::mat::{MatScale as _, MatTranslate as _};

use crate::{
	core::{
		channel::{Channel as _, DefaultChannel},
		factory::{CreateMessage, Handle},
	},
	space::{Orientable, Positionable},
};
