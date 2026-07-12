use math::{
	mat::{MatScale as _, MatTranslate as _},
	Matrix4, Quaternion, Vector3, Vector4,
};

use crate::core::{
	channel::{Channel as _, DefaultChannel},
	factory::Handle,
	message::Message,
};

#[derive(Debug, Clone)]
pub struct Transform {
	position: Vector3,
	scale: Vector3,
	rotation: Quaternion,
}

impl Default for Transform {
	fn default() -> Self {
		Self {
			position: Vector3::new(0.0, 0.0, 0.0),
			scale: Vector3::new(1.0, 1.0, 1.0),
			rotation: Quaternion::identity(),
		}
	}
}

impl Transform {
	pub fn identity() -> Self {
		Self {
			position: Vector3::new(0.0, 0.0, 0.0),
			scale: Vector3::new(1.0, 1.0, 1.0),
			rotation: Quaternion::identity(),
		}
	}

	pub fn new(position: Vector3, scale: Vector3, rotation: Quaternion) -> Self {
		Self {
			position,
			scale,
			rotation,
		}
	}

	pub fn position(self, position: Vector3) -> Self {
		Self { position, ..self }
	}

	pub fn rotation(self, rotation: Quaternion) -> Self {
		Self { rotation, ..self }
	}

	pub fn from_position(position: Vector3) -> Self {
		Self {
			position,
			scale: Vector3::new(1.0, 1.0, 1.0),
			rotation: Quaternion::identity(),
		}
	}

	fn from_translation(position: Vector3) -> Self {
		Self {
			position,
			scale: Vector3::new(1.0, 1.0, 1.0),
			rotation: Quaternion::identity(),
		}
	}

	fn from_scale(scale: Vector3) -> Self {
		Self {
			position: Vector3::new(0.0, 0.0, 0.0),
			scale,
			rotation: Quaternion::identity(),
		}
	}

	fn from_rotation(rotation: Quaternion) -> Self {
		Self {
			position: Vector3::new(0.0, 0.0, 0.0),
			scale: Vector3::new(1.0, 1.0, 1.0),
			rotation,
		}
	}

	pub fn get_matrix(&self) -> Matrix4 {
		let rotation = self.rotation.get_matrix();
		let x = Vector4::from((rotation.get_row(0), 0.0));
		let y = Vector4::from((rotation.get_row(1), 0.0));
		let z = Vector4::from((rotation.get_row(2), 0.0));
		Matrix4::from_translation(self.position)
			* Matrix4::from((x, y, z, Vector4::new(0.0, 0.0, 0.0, 1.0)))
			* Matrix4::from_scale(self.scale)
	}

	pub fn set_position(&mut self, position: Vector3) {
		self.position = position;
	}

	pub fn get_position(&self) -> Vector3 {
		self.position
	}

	pub fn set_scale(&mut self, scale: Vector3) {
		self.scale = scale;
	}

	pub fn scale(&self) -> Vector3 {
		self.scale
	}

	pub fn set_orientation(&mut self, orientation: Quaternion) {
		self.rotation = orientation;
	}

	pub fn get_orientation(&self) -> Quaternion {
		self.rotation
	}
}

impl From<&Transform> for Matrix4 {
	fn from(transform: &Transform) -> Self {
		transform.get_matrix()
	}
}

#[derive(Clone, Debug)]
pub struct TransformationUpdate {
	handle: Handle,
	transform: Transform,
}

impl TransformationUpdate {
	pub fn new(handle: Handle, transform: Transform) -> Self {
		Self { handle, transform }
	}

	pub fn apply(channel: &mut DefaultChannel<Self>, handle: Handle, transform: Transform) {
		channel.send(TransformationUpdate::new(handle, transform));
	}

	pub fn transform(&self) -> &Transform {
		&self.transform
	}

	pub fn handle(&self) -> &Handle {
		&self.handle
	}
}

impl Message for TransformationUpdate {}

pub trait Applicator {
	type Type;
	fn apply(&mut self, value: Self::Type);
}

#[cfg(test)]
mod tests {
	use math::{Quaternion, Vector3, Vector4};

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
	fn identity_and_default_have_the_same_spatial_effect() {
		let point = Vector4::new(2.0, -3.0, 4.0, 1.0);
		assert_eq!(Transform::default().get_matrix() * point, point);
		assert_eq!(Transform::identity().get_matrix() * point, point);
	}

	#[test]
	fn matrix_applies_scale_before_translation() {
		let transform = Transform::new(
			Vector3::new(10.0, 20.0, 30.0),
			Vector3::new(2.0, 3.0, 4.0),
			Quaternion::identity(),
		);

		let transformed = transform.get_matrix() * Vector4::new(1.0, 1.0, 1.0, 1.0);
		assert_eq!(transformed, Vector4::new(12.0, 23.0, 34.0, 1.0));
		assert_eq!(
			transform.get_matrix() * Vector4::new(1.0, 1.0, 1.0, 0.0),
			Vector4::new(2.0, 3.0, 4.0, 0.0)
		);
	}

	#[test]
	fn constructors_and_builders_preserve_unmodified_components() {
		let rotation = Quaternion::from_axis_angle(Vector3::new(0.0, 1.0, 0.0), 0.5);
		let transform = Transform::from_position(Vector3::new(1.0, 2.0, 3.0)).rotation(rotation);
		assert_eq!(transform.get_position(), Vector3::new(1.0, 2.0, 3.0));
		assert_eq!(transform.scale(), Vector3::new(1.0, 1.0, 1.0));
		assert_eq!(transform.get_orientation(), rotation);

		assert_eq!(
			Transform::from_translation(Vector3::new(4.0, 5.0, 6.0)).get_position(),
			Vector3::new(4.0, 5.0, 6.0)
		);
		assert_eq!(
			Transform::from_scale(Vector3::new(2.0, 3.0, 4.0)).scale(),
			Vector3::new(2.0, 3.0, 4.0)
		);
		assert_eq!(Transform::from_rotation(rotation).get_orientation(), rotation);
	}

	#[test]
	fn transformable_blanket_traits_mutate_one_shared_transform() {
		let orientation = Quaternion::from_axis_angle(Vector3::new(1.0, 0.0, 0.0), 0.25);
		let mut entity = SpatialEntity {
			transform: Transform::default(),
		};

		entity.set_position(Vector3::new(3.0, 4.0, 5.0));
		entity.set_scale(Vector3::new(2.0, 2.0, 2.0));
		entity.set_orientation(orientation);

		assert_eq!(entity.position(), Vector3::new(3.0, 4.0, 5.0));
		assert_eq!(entity.scale(), Vector3::new(2.0, 2.0, 2.0));
		assert_eq!(entity.orientation(), orientation);
		assert_eq!(entity.transform.get_position(), entity.position());
	}

	#[test]
	fn transformation_update_preserves_handle_and_payload_through_channel() {
		let mut factory = Factory::new();
		let handle = factory.create("entity");
		let transform = Transform::from_position(Vector3::new(7.0, 8.0, 9.0));
		let mut channel = DefaultChannel::new();
		let mut listener = channel.listener();

		TransformationUpdate::apply(&mut channel, handle, transform);
		let update = listener.read().expect("transformation update");
		assert_eq!(update.handle(), &handle);
		assert_eq!(update.transform().get_position(), Vector3::new(7.0, 8.0, 9.0));
	}
}
