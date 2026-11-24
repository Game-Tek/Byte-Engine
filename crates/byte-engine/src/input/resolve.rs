//! Pure conversion of control values into an action's value type.
//!
//! The caller supplies `read`, which returns the control values its sink may
//! see. Nothing here touches the queue or the registry.

use std::f32::consts::PI;

use super::gesture::TriggerMapping;
use super::queue::Record;
use super::{AnalogDefaults, Axis2, Axis3, DeadZone, Function, Invert, Types, Value};

/// Resolves one action value from the record driving `mapping` and the visible control state.
pub(super) fn resolve_value<'records>(
	kind: Types,
	mappings: &[TriggerMapping],
	mapping: &TriggerMapping,
	record: &Record,
	analog: &[AnalogDefaults],
	read: &impl Fn(u32) -> Option<&'records Record>,
) -> Option<Value> {
	// A gated binding converts the source value visible at the button's place in the queue.
	// A source event already supplies the visible sample; button events still need their snapshot lookup.
	let record = if mapping.gated() && mapping.source != record.trigger {
		read(mapping.source)?
	} else {
		record
	};
	// An analog value is shaped by its binding's dead zone, curve, and inversion, or its device class's, before it
	// converts. A value nothing shapes skips the copy.
	let shaped;
	let record = match record.value {
		Value::Float(_) | Value::Vector2(_) => {
			let class = analog.get(mapping.class as usize).copied().unwrap_or_default();
			let own = &mapping.mapping;
			let dead_zone = own.dead_zone.or(class.dead_zone);
			let curve = own.curve.or(class.curve).filter(|exponent| *exponent != 1.0);
			if dead_zone.is_none() && curve.is_none() && own.invert == Invert::default() {
				record
			} else {
				let mut copy = *record;
				copy.value = shape(record.value, dead_zone, curve, own.invert);
				shaped = copy;
				&shaped
			}
		}
		_ => record,
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
		Types::Float => resolve_float(mappings, mapping, record, read).map(Value::Float),
		Types::Vector2 => resolve_vector2(mappings, record, read).map(Value::Vector2),
		Types::Vector3 => resolve_vector3(mappings, mapping, record, read).map(Value::Vector3),
		_ => unsupported_conversion(),
	}
}

/// Shapes a Float or Vector2 `value`: its magnitude passes `dead_zone`, then becomes its `curve` power, keeping the
/// value's direction or sign, and then the axes `invert` names change sign. Other values pass through.
fn shape(value: Value, dead_zone: Option<DeadZone>, curve: Option<f32>, invert: Invert) -> Value {
	let (x, y) = match value {
		Value::Float(x) => (x, 0.0),
		Value::Vector2(value) => (value.x, value.y),
		value => return value,
	};
	let magnitude = x.hypot(y);
	let shaped = dead_zone.map_or(magnitude, |dead_zone| dead_zone.magnitude(magnitude));
	let shaped = curve.map_or(shaped, |exponent| shaped.powf(exponent));
	// A value at rest has no direction to keep.
	let scale = if magnitude > 0.0 { shaped / magnitude } else { 0.0 };
	let sign = |inverted: bool| if inverted { -scale } else { scale };
	match value {
		Value::Float(_) => Value::Float(x * sign(invert.x)),
		_ => Value::Vector2(Axis2::new(x * sign(invert.x), y * sign(invert.y))),
	}
}

/// Resolves a scalar, giving the most recently pressed boolean binding priority.
fn resolve_float<'records>(
	mappings: &[TriggerMapping],
	mapping: &TriggerMapping,
	record: &Record,
	read: &impl Fn(u32) -> Option<&'records Record>,
) -> Option<f32> {
	match record.value {
		Value::Bool(record_value) => {
			// Opposing scalar bindings follow the most recently pressed control.
			if let Some((active_mapping, _)) = active_boolean_mappings(mappings, read).max_by_key(|(_, record)| record.sequence)
			{
				Some(value_as_float(active_mapping.mapping.value, true))
			} else {
				Some(value_as_float(mapping.mapping.value, record_value))
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
fn resolve_vector2<'records>(
	mappings: &[TriggerMapping],
	record: &Record,
	read: &impl Fn(u32) -> Option<&'records Record>,
) -> Option<Axis2> {
	match record.value {
		Value::Bool(_) => {
			let value =
				active_boolean_mappings(mappings, read).fold(Axis2::zero(), |sum, (mapping, _)| match mapping.mapping.value {
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
fn resolve_vector3<'records>(
	mappings: &[TriggerMapping],
	mapping: &TriggerMapping,
	record: &Record,
	read: &impl Fn(u32) -> Option<&'records Record>,
) -> Option<Axis3> {
	match record.value {
		Value::Bool(_) => {
			let value =
				active_boolean_mappings(mappings, read).fold(Axis3::zero(), |sum, (mapping, _)| match mapping.mapping.value {
					Value::Vector3(value) => sum + value,
					_ => sum,
				});
			Some(value.normalized())
		}
		Value::Vector2(value) => match mapping.mapping.function {
			Function::Sphere => {
				let x_angle = value.x * PI;
				let y_angle = value.y * PI * 0.5;
				let direction = Axis3::new(x_angle.sin() * y_angle.cos(), y_angle.sin(), x_angle.cos() * y_angle.cos());
				let Value::Vector3(transformation) = mapping.mapping.value else {
					return unsupported_conversion();
				};
				Some(direction * transformation)
			}
			_ => unsupported_conversion(),
		},
		Value::Vector3(value) => Some(value),
		_ => unsupported_conversion(),
	}
}

/// Borrows active bindings directly; vector sums and scalar priority need no scratch storage.
fn active_boolean_mappings<'a, 'records: 'a>(
	mappings: &'a [TriggerMapping],
	read: &'a impl Fn(u32) -> Option<&'records Record>,
) -> impl Iterator<Item = (&'a TriggerMapping, &'records Record)> + 'a {
	mappings.iter().filter_map(|mapping| {
		let record = read(mapping.source)?;
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
