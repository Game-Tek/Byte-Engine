use std::{error::Error, fmt};

use serde::Serialize;
use serde_json::{json, Map, Value};

use crate::{
	resources::{
		animation::AnimationModel,
		audio::Audio,
		image::Image,
		lut::Lut,
		material::{MaterialModel, Shader, VariantModel},
		mesh::{MeshModel, PrimitiveModel},
	},
	QueryableValue, SerializableResource,
};

/// The `ResourceInspection` struct carries a JSON view of a resource and whether the resource section was supported.
#[derive(Debug)]
pub struct ResourceInspection {
	pub json: Value,
	pub unsupported_resource_section: bool,
}

/// The `ResourceInspectError` enum represents failures while creating a resource inspection view.
#[derive(Debug)]
pub enum ResourceInspectError {
	DeserializeResource { class: String, error: String },
	SerializeResource { class: String, error: String },
}

impl fmt::Display for ResourceInspectError {
	fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
		match self {
			ResourceInspectError::DeserializeResource { class, error } => write!(
				f,
				"Failed to deserialize '{}' resource section. The most likely cause is that the baked resource was produced by an incompatible resource-management version. Error: {}",
				class, error
			),
			ResourceInspectError::SerializeResource { class, error } => write!(
				f,
				"Failed to serialize '{}' resource section to JSON. The most likely cause is that the resource contains a value that cannot be represented as JSON. Error: {}",
				class, error
			),
		}
	}
}

impl Error for ResourceInspectError {}

pub fn inspect_resource(resource: &SerializableResource) -> Result<ResourceInspection, ResourceInspectError> {
	let (resource_json, unsupported_resource_section) = inspect_resource_section(resource)?;

	let json = json!({
		"id": resource.id(),
		"uid": resource.uid(),
		"class": resource.class(),
		"hash": resource.hash(),
		"size": resource.size(),
		"serialized_resource_size": resource.resource().len(),
		"properties": queryable_properties_to_json(resource),
		"streams": streams_to_json(resource),
		"resource": resource_json,
	});

	Ok(ResourceInspection {
		json,
		unsupported_resource_section,
	})
}

fn inspect_resource_section(resource: &SerializableResource) -> Result<(Value, bool), ResourceInspectError> {
	let value = match resource.class() {
		"Animation" => deserialize_resource::<AnimationModel>(resource)?,
		"Audio" => deserialize_resource::<Audio>(resource)?,
		"Image" => deserialize_resource::<Image>(resource)?,
		"Lut" => deserialize_resource::<Lut>(resource)?,
		"Material" => deserialize_resource::<MaterialModel>(resource)?,
		"Mesh" => deserialize_resource::<MeshModel>(resource)?,
		"Primitive" => deserialize_resource::<PrimitiveModel>(resource)?,
		"Shader" => deserialize_resource::<Shader>(resource)?,
		"Variant" => deserialize_resource::<VariantModel>(resource)?,
		class => {
			return Ok((
				json!({
					"unsupported": true,
					"message": format!("Unsupported resource class '{}'", class),
				}),
				true,
			));
		}
	};

	Ok((value, false))
}

fn deserialize_resource<T>(resource: &SerializableResource) -> Result<Value, ResourceInspectError>
where
	T: crate::ResourceArchive + Serialize,
	<T as rkyv::Archive>::Archived: for<'a> rkyv::bytecheck::CheckBytes<crate::ResourceHighValidator<'a>>
		+ rkyv::Deserialize<T, crate::ResourceHighDeserializer>,
{
	let value: T = crate::from_slice(resource.resource()).map_err(|error| ResourceInspectError::DeserializeResource {
		class: resource.class().to_string(),
		error: error.to_string(),
	})?;

	serde_json::to_value(value).map_err(|error| ResourceInspectError::SerializeResource {
		class: resource.class().to_string(),
		error: error.to_string(),
	})
}

fn queryable_properties_to_json(resource: &SerializableResource) -> Value {
	let mut properties = Map::new();

	for property in resource.queryable_properties() {
		properties.insert(property.name.clone(), queryable_value_to_json(&property.value));
	}

	Value::Object(properties)
}

fn queryable_value_to_json(value: &QueryableValue) -> Value {
	match value {
		QueryableValue::String(value) => Value::String(value.clone()),
	}
}

fn streams_to_json(resource: &SerializableResource) -> Value {
	resource
		.streams()
		.unwrap_or_default()
		.iter()
		.map(|stream| {
			json!({
				"name": stream.name(),
				"size": stream.size(),
				"offset": stream.offset(),
			})
		})
		.collect()
}
