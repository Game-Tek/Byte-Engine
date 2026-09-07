//! Pure action value conversion. The caller supplies the control values its layer may read.

use std::f32::consts::PI;

use super::action::TriggerMapping;
use super::events::Record;
use super::{Axis2, Axis3, Function, TriggerHandle, Types, Value};

/// Resolves one action value from the latest trigger record and current trigger state.
pub(super) fn resolve_action_value(
	kind: Types,
	mappings: &[TriggerMapping],
	mapping: &TriggerMapping,
	record: &Record,
	read: &impl Fn(TriggerHandle) -> Option<Record>,
) -> Option<Value> {
	// Snapshot bindings convert the retained source value at the trigger's place in the queue.
	let record = if mapping.trigger.is_some() {
		read(mapping.trigger_handle)?
	} else {
		*record
	};
	match kind {
		Types::Boolean => match record.value {
			Value::Bool(value) => Some(Value::Bool(value)),
			Value::Float(value) => Some(Value::Bool(value != 0.0)),
			_ => unsupported_conversion(),
		},
		Types::Unicode => match record.value {
			Value::Unicode(value) => Some(Value::Unicode(value)),
			_ => unsupported_conversion(),
		},
		Types::Float => resolve_float(mappings, mapping, &record, read).map(Value::Float),
		Types::Vector2 => resolve_vector2(mappings, &record, read).map(Value::Vector2),
		Types::Vector3 => resolve_vector3(mappings, mapping, &record, read).map(Value::Vector3),
		_ => unsupported_conversion(),
	}
}

/// Resolves a scalar, giving the newest active boolean binding priority.
fn resolve_float(
	mappings: &[TriggerMapping],
	mapping: &TriggerMapping,
	record: &Record,
	read: &impl Fn(TriggerHandle) -> Option<Record>,
) -> Option<f32> {
	match record.value {
		Value::Bool(record_value) => {
			// Opposing scalar bindings follow the most recently pressed control.
			if let Some((active_mapping, _)) = active_boolean_mappings(mappings, read).max_by_key(|(_, record)| record.sequence)
			{
				Some(value_as_float(active_mapping.mapping, true))
			} else {
				Some(value_as_float(mapping.mapping, record_value))
			}
		}
		Value::Float(value) => Some(value),
		_ => unsupported_conversion(),
	}
}

/// Applies one boolean binding to a scalar output.
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

/// Combines active directional bindings or converts a recorded vector.
fn resolve_vector2(
	mappings: &[TriggerMapping],
	record: &Record,
	read: &impl Fn(TriggerHandle) -> Option<Record>,
) -> Option<Axis2> {
	match record.value {
		Value::Bool(_) => {
			let value =
				active_boolean_mappings(mappings, read).fold(Axis2::zero(), |sum, (mapping, _)| match mapping.mapping {
					Value::Vector2(value) => sum + value,
					_ => sum,
				});
			Some(value.normalized())
		}
		Value::Vector2(value) => Some(value),
		Value::Vector3(value) => Some(Axis2::new(value.x, value.y)),
		_ => unsupported_conversion(),
	}
}

/// Combines directional bindings or applies the configured spherical projection.
fn resolve_vector3(
	mappings: &[TriggerMapping],
	mapping: &TriggerMapping,
	record: &Record,
	read: &impl Fn(TriggerHandle) -> Option<Record>,
) -> Option<Axis3> {
	match record.value {
		Value::Bool(_) => {
			let value =
				active_boolean_mappings(mappings, read).fold(Axis3::zero(), |sum, (mapping, _)| match mapping.mapping {
					Value::Vector3(value) => sum + value,
					_ => sum,
				});
			Some(value.normalized())
		}
		Value::Vector2(value) => match mapping.function {
			Some(Function::Sphere) => {
				let x_angle = value.x * PI;
				let y_angle = value.y * PI * 0.5;
				let direction = Axis3::new(x_angle.sin() * y_angle.cos(), y_angle.sin(), x_angle.cos() * y_angle.cos());
				let Value::Vector3(transformation) = mapping.mapping else {
					return unsupported_conversion();
				};
				Some(direction * transformation)
			}
			None => Some(Axis3::new(value.x, value.y, 0.0)),
			_ => unsupported_conversion(),
		},
		Value::Vector3(value) => Some(value),
		_ => unsupported_conversion(),
	}
}

/// Borrows active bindings directly; vector sums and scalar priority need no scratch storage.
fn active_boolean_mappings<'a>(
	mappings: &'a [TriggerMapping],
	read: &'a impl Fn(TriggerHandle) -> Option<Record>,
) -> impl Iterator<Item = (&'a TriggerMapping, Record)> + 'a {
	mappings.iter().filter_map(|mapping| {
		let record = read(mapping.trigger_handle)?;
		(record.value == Value::Bool(true)).then_some((mapping, record))
	})
}

/// Reports a binding whose value cannot be converted to the requested output type.
fn unsupported_conversion<T>() -> Option<T> {
	log::error!(
		"Input action conversion is not implemented for this value combination. The most likely cause is that a trigger mapping produces a value that the action output type cannot accept. See {}.",
		crate::online_docs_url("reference/input")
	);
	None
}
