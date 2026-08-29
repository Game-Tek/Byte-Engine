//! Protocol-to-world message posting for runtime debugging tools.

use math::{Orientation, Point, Quaternion, Scale};
use serde_json::Value;

use super::Inspector;
use crate::{
	core::{channel::Channel, factory::Handle},
	gameplay::{Transform, TransformationUpdate},
};

/// The stable inspection-protocol name for a transform replacement message.
pub const TRANSFORMATION_UPDATE_MESSAGE_TYPE: &str = "TransformationUpdate";

impl Inspector {
	/// Publishes one targeted world message from its stable protocol name and JSON payload.
	///
	/// `TransformationUpdate` payloads contain `position`, `scale`, and `orientation`
	/// arrays with three, three, and four finite numbers respectively. Quaternion
	/// components use `[x, y, z, w]` order.
	pub fn post_message(&self, message_type: &str, target: Handle, payload: &Value) -> Result<(), String> {
		match message_type {
			TRANSFORMATION_UPDATE_MESSAGE_TYPE => {
				let transform = parse_transform_payload(payload)?;
				self.messages
					.channel::<TransformationUpdate>()
					.send(TransformationUpdate::new(target, transform));
				Ok(())
			}
			_ => Err(format!(
				"Inspector message type is unsupported. The most likely cause is that '{message_type}' is not registered for inspector posting."
			)),
		}
	}
}

/// Converts the protocol transform object into the engine's checked spatial types.
fn parse_transform_payload(payload: &Value) -> Result<Transform, String> {
	let object = payload.as_object().ok_or_else(invalid_transform_payload)?;
	if object.len() != 3
		|| object
			.keys()
			.any(|key| !matches!(key.as_str(), "position" | "scale" | "orientation"))
	{
		return Err(invalid_transform_payload());
	}

	let [position_x, position_y, position_z] = parse_f32_array::<3>(object.get("position"))?;
	let [scale_x, scale_y, scale_z] = parse_f32_array::<3>(object.get("scale"))?;
	let [rotation_x, rotation_y, rotation_z, rotation_w] = parse_f32_array::<4>(object.get("orientation"))?;
	let orientation = Orientation::try_from_maths(Quaternion::new(rotation_x, rotation_y, rotation_z, rotation_w))
		.map_err(|error| {
			format!(
				"TransformationUpdate orientation is invalid. The most likely cause is that the quaternion is zero-length or non-finite: {error}"
			)
		})?;

	Ok(Transform::new(
		Point::new(position_x, position_y, position_z),
		Scale::new(scale_x, scale_y, scale_z),
		orientation,
	))
}

/// Reads one fixed-size array without accepting lossy or non-finite `f32` values.
fn parse_f32_array<const N: usize>(value: Option<&Value>) -> Result<[f32; N], String> {
	let values = value
		.and_then(Value::as_array)
		.filter(|values| values.len() == N)
		.ok_or_else(invalid_transform_payload)?;
	let mut result = [0.0; N];
	for (result, value) in result.iter_mut().zip(values) {
		let Some(value) = value.as_f64() else {
			return Err(invalid_transform_payload());
		};
		let value = value as f32;
		if !value.is_finite() {
			return Err(invalid_transform_payload());
		}
		*result = value;
	}
	Ok(result)
}

fn invalid_transform_payload() -> String {
	"TransformationUpdate payload is invalid. The most likely cause is that it must contain only `position: [x, y, z]`, `scale: [x, y, z]`, and `orientation: [x, y, z, w]` with finite numbers."
		.to_string()
}

#[cfg(test)]
mod tests {
	use super::*;
	use crate::{
		configuration::Configuration,
		core::{channel::DefaultChannel, factory::Factory, listener::Listener, message_bus::MessageBus},
	};

	/// Creates an inspector whose transform route already has a listener.
	fn test_inspector() -> (Inspector, crate::core::listener::DefaultListener<TransformationUpdate>) {
		let messages = MessageBus::default().new_scope("inspector-test-world");
		let transforms = messages.channel();
		let listener = transforms.listener();

		(
			Inspector::new(DefaultChannel::new(), Configuration::new(), messages),
			listener,
		)
	}

	#[test]
	fn transformation_update_posts_the_selected_target_and_complete_payload() {
		let (inspector, mut transforms) = test_inspector();
		let factory = Factory::new();
		let _factory_listener = factory.listener();
		let target = factory.create(());

		inspector
			.post_message(
				TRANSFORMATION_UPDATE_MESSAGE_TYPE,
				target,
				&serde_json::json!({
					"position": [1.0, 2.0, 3.0],
					"scale": [2.0, 3.0, 4.0],
					"orientation": [0.0, 0.0, 0.0, 1.0]
				}),
			)
			.expect("post transform update");

		let update = transforms.read().expect("posted transform update");
		assert_eq!(update.handle(), target);
		assert_eq!(update.transform().get_position(), Point::new(1.0, 2.0, 3.0));
		assert_eq!(update.transform().scale(), Scale::new(2.0, 3.0, 4.0));
		assert_eq!(update.transform().get_orientation(), Orientation::identity());
	}

	#[test]
	fn unsupported_message_types_do_not_publish_to_the_transform_route() {
		let (inspector, mut transforms) = test_inspector();
		let factory = Factory::new();
		let _factory_listener = factory.listener();
		let target = factory.create(());

		let error = inspector
			.post_message("DeleteMessage", target, &Value::Null)
			.expect_err("unsupported inspector message type");

		assert!(error.contains("DeleteMessage"));
		assert!(transforms.read().is_none());
	}

	#[test]
	fn incomplete_transform_payloads_do_not_publish() {
		let (inspector, mut transforms) = test_inspector();
		let factory = Factory::new();
		let _factory_listener = factory.listener();
		let target = factory.create(());

		let error = inspector
			.post_message(
				TRANSFORMATION_UPDATE_MESSAGE_TYPE,
				target,
				&serde_json::json!({
					"position": [1.0, 2.0, 3.0]
				}),
			)
			.expect_err("incomplete transform payload");

		assert!(error.contains("TransformationUpdate payload is invalid"));
		assert!(transforms.read().is_none());
	}
}
