use std::collections::HashMap;
use std::f32::consts::PI;

use math::{normalize, Base, Vector2, Vector3};

use super::action::TriggerMapping;
use super::records::Record;
use super::{ActionHandle, DeviceHandle, Function, SeatHandle, TickPolicy, TriggerHandle, Types, Value};
use crate::core::factory::Handle;

/// The `InputAction` struct stores resolved trigger mappings and emission policy for one action.
pub(super) struct InputAction {
	pub(super) name: String,
	pub(super) r#type: Types,
	pub(super) trigger_mappings: Vec<TriggerMapping>,
	pub(super) handle: Option<Handle>,
	pub(super) tick_policy: TickPolicy,
}

/// The `InputEventState` struct represents the latest resolved value of an action for one device.
pub struct InputEventState {
	pub(super) seat_handle: SeatHandle,
	pub(super) device_handle: DeviceHandle,
	pub(super) input_event_handle: ActionHandle,
	pub(super) value: Value,
}

/// Resolves one action value from the latest trigger record and current trigger state.
pub(super) fn resolve_action_value(
	action: &InputAction,
	record: &Record,
	values: &HashMap<(SeatHandle, DeviceHandle, TriggerHandle), Record>,
	frame_allocator: &bumpalo::Bump,
) -> Option<Value> {
	let mapping = action
		.trigger_mappings
		.iter()
		.find(|mapping| mapping.trigger_handle == record.trigger_handle)?;

	match action.r#type {
		Types::Boolean => match record.value {
			Value::Bool(value) => Some(Value::Bool(value)),
			Value::Float(value) => Some(Value::Bool(value != 0.0)),
			_ => unsupported_conversion(),
		},
		Types::Float => resolve_float(action, mapping, record, values, frame_allocator).map(Value::Float),
		Types::Vector2 => resolve_vector2(action, record, values, frame_allocator).map(Value::Vector2),
		Types::Vector3 => resolve_vector3(action, mapping, record, values, frame_allocator).map(Value::Vector3),
		_ => unsupported_conversion(),
	}
}

fn resolve_float(
	action: &InputAction,
	mapping: &TriggerMapping,
	record: &Record,
	values: &HashMap<(SeatHandle, DeviceHandle, TriggerHandle), Record>,
	frame_allocator: &bumpalo::Bump,
) -> Option<f32> {
	match record.value {
		Value::Bool(record_value) => {
			let stack = active_boolean_mappings(action, record, values, frame_allocator);
			if let Some((active_mapping, _)) = stack.last() {
				Some(value_as_float(active_mapping.mapping, true))
			} else {
				Some(value_as_float(mapping.mapping, record_value))
			}
		}
		Value::Float(value) => Some(value),
		_ => unsupported_conversion(),
	}
}

fn value_as_float(value: Value, record_value: bool) -> f32 {
	match value {
		Value::Bool(value) => u32::from(value) as f32,
		Value::Unicode(_) => 0.0,
		Value::Float(value) => value * u32::from(record_value) as f32,
		Value::Int(value) => value as f32,
		Value::Rgba(value) => value.r,
		Value::Vector2(value) => value.x,
		Value::Vector3(value) => value.x,
		Value::Quaternion(value) => value[0],
	}
}

fn resolve_vector2(
	action: &InputAction,
	record: &Record,
	values: &HashMap<(SeatHandle, DeviceHandle, TriggerHandle), Record>,
	frame_allocator: &bumpalo::Bump,
) -> Option<Vector2> {
	match record.value {
		Value::Bool(_) => {
			let value = active_boolean_mappings(action, record, values, frame_allocator).iter().fold(
				Vector2::zero(),
				|sum, (mapping, _)| match mapping.mapping {
					Value::Vector2(value) => sum + value,
					_ => sum,
				},
			);
			Some(if value == Vector2::zero() { value } else { normalize(value) })
		}
		Value::Vector2(value) => Some(value),
		Value::Vector3(value) => Some(Vector2 { x: value.x, y: value.y }),
		_ => unsupported_conversion(),
	}
}

fn resolve_vector3(
	action: &InputAction,
	mapping: &TriggerMapping,
	record: &Record,
	values: &HashMap<(SeatHandle, DeviceHandle, TriggerHandle), Record>,
	frame_allocator: &bumpalo::Bump,
) -> Option<Vector3> {
	match record.value {
		Value::Bool(_) => {
			let value = active_boolean_mappings(action, record, values, frame_allocator).iter().fold(
				Vector3::zero(),
				|sum, (mapping, _)| match mapping.mapping {
					Value::Vector3(value) => sum + value,
					_ => sum,
				},
			);
			Some(if value == Vector3::zero() { value } else { normalize(value) })
		}
		Value::Vector2(value) => match mapping.function {
			Some(Function::Sphere) => {
				let x_angle = value.x * PI;
				let y_angle = value.y * PI * 0.5;
				let direction = Vector3 {
					x: x_angle.sin() * y_angle.cos(),
					y: y_angle.sin(),
					z: x_angle.cos() * y_angle.cos(),
				};
				let Value::Vector3(transformation) = mapping.mapping else {
					return unsupported_conversion();
				};
				Some(direction * transformation)
			}
			None => Some(Vector3 {
				x: value.x,
				y: value.y,
				z: 0.0,
			}),
			_ => unsupported_conversion(),
		},
		Value::Vector3(value) => Some(value),
		_ => unsupported_conversion(),
	}
}

/// Builds the active boolean mapping stack in chronological order using frame scratch.
fn active_boolean_mappings<'a>(
	action: &InputAction,
	record: &Record,
	values: &HashMap<(SeatHandle, DeviceHandle, TriggerHandle), Record>,
	frame_allocator: &'a bumpalo::Bump,
) -> &'a mut [(TriggerMapping, Record)] {
	let active_count = action
		.trigger_mappings
		.iter()
		.filter(|mapping| {
			values
				.get(&(record.seat_handle, record.device_handle, mapping.trigger_handle))
				.is_some_and(|candidate| matches!(candidate.value, Value::Bool(true)))
		})
		.count();

	let mut mappings = action.trigger_mappings.iter();
	let stack = frame_allocator.alloc_slice_fill_with(active_count, |_| loop {
		let mapping = mappings
			.next()
			.expect("active boolean record count must match the action mapping scan");
		let Some(candidate) = values
			.get(&(record.seat_handle, record.device_handle, mapping.trigger_handle))
			.copied()
		else {
			continue;
		};

		if matches!(candidate.value, Value::Bool(true)) {
			break (*mapping, candidate);
		}
	});

	stack.sort_by_key(|(_, record)| record.time);
	stack
}

fn unsupported_conversion<T>() -> Option<T> {
	log::error!("Input action conversion is not implemented for this value combination");
	None
}
