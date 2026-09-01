//! JSON-oriented descriptors for Facet shapes exposed through inspection protocols.

use facet::{Def, Field, ScalarType, Shape, StructKind, StructType, Type, UserType};
use serde_json::{Value, json};

/// Describes the JSON representation accepted by Facet for one reflected shape.
pub(super) fn describe_json_shape(shape: &'static Shape) -> Value {
	describe_shape(shape, &mut Vec::new())
}

/// Recursively translates a Facet shape while stopping self-referential types.
fn describe_shape(shape: &'static Shape, active: &mut Vec<&'static Shape>) -> Value {
	if active.iter().any(|candidate| candidate.id == shape.id) {
		return json!({ "type": "recursive", "name": shape.to_string() });
	}
	active.push(shape);

	// Facet JSON uses format-specific proxies before general proxies. Follow the
	// same path so the descriptor matches the payload accepted by facet-json.
	let descriptor = if let Some(proxy) = shape.effective_proxy(Some("json")) {
		describe_shape(proxy.shape, active)
	} else if let Some(scalar) = shape.scalar_type() {
		describe_scalar(scalar, shape)
	} else {
		describe_composite(shape, active)
	};

	active.pop();
	descriptor
}

/// Describes Facet containers before falling back to their Rust type category.
fn describe_composite(shape: &'static Shape, active: &mut Vec<&'static Shape>) -> Value {
	match shape.def {
		Def::Map(map) => json!({
			"type": "map",
			"keys": describe_shape(map.k, active),
			"values": describe_shape(map.v, active),
		}),
		Def::Set(set) => json!({
			"type": "array",
			"items": describe_shape(set.t, active),
			"unique": true,
		}),
		Def::List(list) => array_shape(list.t, None, active),
		Def::Array(array) => array_shape(array.t, Some(array.n), active),
		Def::NdArray(array) => json!({
			"type": "array",
			"items": describe_shape(array.t, active),
			"dimensions": "dynamic",
		}),
		Def::Slice(slice) => array_shape(slice.t, None, active),
		Def::Option(option) => json!({
			"type": "optional",
			"shape": describe_shape(option.t, active),
		}),
		Def::Result(result) => json!({
			"type": "result",
			"ok": describe_shape(result.t, active),
			"error": describe_shape(result.e, active),
		}),
		Def::Pointer(pointer) => pointer
			.pointee
			.map_or_else(|| unknown_shape(shape), |pointee| describe_shape(pointee, active)),
		Def::DynamicValue(_) => json!({ "type": "any" }),
		Def::Undefined | Def::Scalar => describe_rust_type(shape, active),
		_ => unknown_shape(shape),
	}
}

/// Describes structs, enums, and built-in sequences not identified by `Def`.
fn describe_rust_type(shape: &'static Shape, active: &mut Vec<&'static Shape>) -> Value {
	match shape.ty {
		Type::Sequence(sequence) => match sequence {
			facet::SequenceType::Array(array) => array_shape(array.t, Some(array.n), active),
			facet::SequenceType::Slice(slice) => array_shape(slice.t, None, active),
			_ => unknown_shape(shape),
		},
		Type::User(UserType::Struct(structure)) => describe_struct(
			structure,
			shape.is_transparent(),
			!shape.has_deny_unknown_fields_attr(),
			active,
		),
		Type::User(UserType::Enum(enumeration)) => describe_enum(shape, enumeration, active),
		Type::Pointer(facet::PointerType::Reference(reference) | facet::PointerType::Raw(reference)) => {
			describe_shape(reference.target(), active)
		}
		_ => shape
			.inner
			.map_or_else(|| unknown_shape(shape), |inner| describe_shape(inner, active)),
	}
}

/// Describes the JSON scalar category and retains the Rust width for editor validation.
fn describe_scalar(scalar: ScalarType, shape: &'static Shape) -> Value {
	let descriptor = match scalar {
		ScalarType::Unit => return json!({ "type": "null" }),
		ScalarType::Bool => return json!({ "type": "boolean" }),
		ScalarType::Char => ("string", Some("char")),
		ScalarType::Str | ScalarType::String | ScalarType::CowStr => ("string", None),
		ScalarType::F32 | ScalarType::F64 => ("number", Some(shape.type_identifier)),
		ScalarType::U8
		| ScalarType::U16
		| ScalarType::U32
		| ScalarType::U64
		| ScalarType::U128
		| ScalarType::USize
		| ScalarType::I8
		| ScalarType::I16
		| ScalarType::I32
		| ScalarType::I64
		| ScalarType::I128
		| ScalarType::ISize => ("integer", Some(shape.type_identifier)),
		ScalarType::ConstTypeId => ("string", Some("type-id")),
		_ => return unknown_shape(shape),
	};
	match descriptor {
		(kind, Some(format)) => json!({ "type": kind, "format": format }),
		(kind, None) => json!({ "type": kind }),
	}
}

/// Describes a homogeneous JSON array with an optional fixed length.
fn array_shape(items: &'static Shape, length: Option<usize>, active: &mut Vec<&'static Shape>) -> Value {
	match length {
		Some(length) => json!({
			"type": "array",
			"items": describe_shape(items, active),
			"length": length,
		}),
		None => json!({
			"type": "array",
			"items": describe_shape(items, active),
		}),
	}
}

/// Describes one struct or enum-variant payload using its JSON field names.
fn describe_struct(
	structure: StructType,
	transparent_single_field: bool,
	additional_fields: bool,
	active: &mut Vec<&'static Shape>,
) -> Value {
	match structure.kind {
		StructKind::Unit => json!({ "type": "null" }),
		StructKind::Tuple | StructKind::TupleStruct if transparent_single_field && structure.fields.len() == 1 => {
			describe_field_shape(&structure.fields[0], active)
		}
		StructKind::Tuple | StructKind::TupleStruct => {
			let items = structure
				.fields
				.iter()
				.filter(|field| !field.should_skip_deserializing())
				.map(|field| describe_field_shape(field, active))
				.collect::<Vec<_>>();
			json!({ "type": "tuple", "items": items })
		}
		StructKind::Struct => {
			let fields = structure
				.fields
				.iter()
				.filter(|field| !field.should_skip_deserializing())
				.map(|field| {
					json!({
						"name": field.effective_name(),
						"required": !field.has_default() && !matches!(field.shape().def, Def::Option(_)),
						"flattened": field.is_flattened(),
						"shape": describe_field_shape(field, active),
					})
				})
				.collect::<Vec<_>>();
			json!({
				"type": "object",
				"fields": fields,
				"additional_fields": additional_fields,
			})
		}
	}
}

/// Follows a field-level JSON proxy before describing the field's Rust shape.
fn describe_field_shape(field: &Field, active: &mut Vec<&'static Shape>) -> Value {
	let shape = field
		.effective_proxy(Some("json"))
		.map_or_else(|| field.shape(), |proxy| proxy.shape);
	describe_shape(shape, active)
}

/// Describes enum variants and the tagging representation used by Facet JSON.
fn describe_enum(shape: &'static Shape, enumeration: facet::EnumType, active: &mut Vec<&'static Shape>) -> Value {
	let representation = match (shape.is_untagged(), shape.tag, shape.content) {
		(true, ..) => "untagged",
		(false, Some(_), Some(_)) => "adjacent",
		(false, Some(_), None) => "internal",
		(false, None, _) => "external",
	};
	let variants = enumeration
		.variants
		.iter()
		.map(|variant| {
			json!({
				"name": variant.effective_name(),
				"shape": describe_struct(variant.data, variant.data.fields.len() == 1, false, active),
			})
		})
		.collect::<Vec<_>>();
	json!({
		"type": "enum",
		"representation": representation,
		"tag": shape.tag,
		"content": shape.content,
		"variants": variants,
	})
}

fn unknown_shape(shape: &Shape) -> Value {
	json!({ "type": "unknown", "name": shape.to_string() })
}

#[cfg(test)]
mod tests {
	use std::collections::HashMap;

	use facet::Facet;
	use serde_json::json;

	use super::describe_json_shape;

	#[derive(Facet)]
	#[facet(deny_unknown_fields)]
	struct Settings {
		name: String,
		retries: Option<u32>,
	}

	#[derive(Facet)]
	#[repr(u8)]
	enum Command {
		Stop,
		Move { x: f32, y: f32 },
		Rename(String),
	}

	#[test]
	fn struct_description_distinguishes_required_and_optional_fields() {
		assert_eq!(
			describe_json_shape(Settings::SHAPE),
			json!({
				"type": "object",
				"fields": [
					{
						"name": "name",
						"required": true,
						"flattened": false,
						"shape": { "type": "string" }
					},
					{
						"name": "retries",
						"required": false,
						"flattened": false,
						"shape": {
							"type": "optional",
							"shape": { "type": "integer", "format": "u32" }
						}
					}
				],
				"additional_fields": false
			})
		);
	}

	#[test]
	fn enum_description_preserves_variant_payload_shapes() {
		let descriptor = describe_json_shape(Command::SHAPE);

		assert_eq!(descriptor["type"], "enum");
		assert_eq!(descriptor["representation"], "external");
		assert_eq!(
			descriptor["variants"][0],
			json!({ "name": "Stop", "shape": { "type": "null" } })
		);
		assert_eq!(descriptor["variants"][1]["name"], "Move");
		assert_eq!(descriptor["variants"][1]["shape"]["type"], "object");
		assert_eq!(
			descriptor["variants"][2],
			json!({
				"name": "Rename",
				"shape": { "type": "string" }
			})
		);
	}

	#[test]
	fn collection_descriptions_retain_key_value_and_fixed_length_contracts() {
		assert_eq!(
			describe_json_shape(<HashMap<String, bool>>::SHAPE),
			json!({
				"type": "map",
				"keys": { "type": "string" },
				"values": { "type": "boolean" }
			})
		);
		assert_eq!(
			describe_json_shape(<[u16; 3]>::SHAPE),
			json!({
				"type": "array",
				"items": { "type": "integer", "format": "u16" },
				"length": 3
			})
		);
	}
}
