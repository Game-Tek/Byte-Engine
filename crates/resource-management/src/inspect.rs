//! Turn baked resources into JSON for command-line and editor inspection.
//!
//! Call [`inspect_resource`] for a resource's metadata and model. Call [`inspect_payload`] as well when the reader
//! needs the payload bytes, then serialize both together with [`ResourceDocument`].

use std::{error::Error, fmt};

use exr::prelude::f16;
use serde::{
	Serialize, Serializer,
	ser::{SerializeMap, SerializeSeq},
};
use serde_json::{Map, Value, json};

use crate::{
	QueryableValue, SerializableResource, StreamDescription,
	processors::mesh::MESHLET_STREAM_STRIDE,
	resources::{
		animation::AnimationModel,
		audio::Audio,
		image::{IBL_DIFFUSE_IRRADIANCE_STREAM_NAME, IBL_PREFILTERED_SPECULAR_STREAM_PREFIX, Image},
		lut::Lut,
		material::{MaterialModel, Shader, VariantModel},
		mesh::{MeshModel, Primitive},
		skeleton::SkeletonModel,
	},
	types::{BitDepths, Formats, IndexStreamTypes, Streams},
};

/// The `ResourceInspection` struct provides a JSON view for command-line and editor resource inspection.
#[derive(Debug)]
pub struct ResourceInspection {
	pub json: Value,
	pub unsupported_resource_section: bool,
}

/// The `ResourceInspectError` enum identifies failures while creating a resource inspection view.
#[derive(Debug)]
pub enum ResourceInspectError {
	DeserializeResource {
		class: String,
		error: String,
	},
	SerializeResource {
		class: String,
		error: String,
	},
	/// A stream description points outside the payload, or the stream doesn't hold a whole number of elements.
	MalformedStream {
		id: String,
		stream: String,
	},
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
			ResourceInspectError::MalformedStream { id, stream } => write!(
				f,
				"Failed to decode stream '{}' of resource '{}'. The most likely cause is that the stream metadata doesn't match the stored payload, so the resource should be baked again.",
				stream, id
			),
		}
	}
}

impl Error for ResourceInspectError {}

/// Converts a resource's metadata and deserialized model into the JSON shape BELD and editors print.
///
/// Call [`inspect_payload`] next when the reader also needs the payload bytes.
pub fn inspect_resource(resource: &SerializableResource) -> Result<ResourceInspection, ResourceInspectError> {
	let (resource_json, unsupported_resource_section) = inspect_resource_section(resource)?;

	let json = json!({
		"id": resource.id(),
		"uid": resource.uid(),
		"class": resource.class(),
		"hash": resource.hash(),
		"size": resource.size(),
		"stored_size": resource.stored_size(),
		"encoding": resource.encoding().as_str(),
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
		"Primitive" => deserialize_resource::<Primitive>(resource)?,
		"Shader" => deserialize_resource::<Shader>(resource)?,
		"Skeleton" => deserialize_resource::<SkeletonModel>(resource)?,
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
	let value: T = deserialize_model(resource)?;

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

/// The name of the one stream that covers a payload with no named streams, such as audio samples or LUT entries.
pub const WHOLE_PAYLOAD_STREAM_NAME: &str = "payload";

/// The `PayloadFormat` enum lets the caller of [`inspect_payload`] choose between readable values and exact bytes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PayloadFormat {
	/// Decodes every stream with a known element layout into JSON values, and falls back to base64 for the rest.
	Typed,
	/// Prints every stream as base64, for byte-exact checks or other tools.
	Base64,
}

/// The `PayloadInspection` struct exists so tools can print a resource's payload streams without building them as a
/// JSON tree first.
///
/// It borrows the decoded payload and writes each element as it is serialized, so a large image or mesh costs no
/// more memory than its bytes. Serialize it inside a [`ResourceDocument`], where it replaces the `streams`
/// descriptions of [`inspect_resource`]'s output.
#[derive(Debug)]
pub struct PayloadInspection<'a> {
	streams: Vec<StreamInspection<'a>>,
}

/// One payload stream and, when [`PayloadFormat::Typed`] knows it, the layout of its elements.
#[derive(Debug)]
struct StreamInspection<'a> {
	name: &'a str,
	offset: usize,
	bytes: &'a [u8],
	layout: Option<StreamLayout>,
}

/// The JSON element name and the decoding of one stream.
#[derive(Debug)]
struct StreamLayout {
	name: String,
	element: Element,
}

/// The shape of one element of a decoded stream.
#[derive(Clone, Copy, Debug)]
enum Element {
	Scalar(Scalar),
	/// A fixed number of scalars printed as one array, like a vertex position or a texel.
	Vector(Scalar, usize),
	/// One record of a mesh's `Meshlets` stream.
	Meshlet,
}

/// The little-endian binary form of one number in a payload.
#[derive(Clone, Copy, Debug)]
enum Scalar {
	U8,
	U16,
	I16,
	I24,
	U32,
	I32,
	F16,
	F32,
}

impl Scalar {
	const fn size(self) -> usize {
		match self {
			Scalar::U8 => 1,
			Scalar::U16 | Scalar::I16 | Scalar::F16 => 2,
			Scalar::I24 => 3,
			Scalar::U32 | Scalar::I32 | Scalar::F32 => 4,
		}
	}
}

impl Element {
	const fn size(self) -> usize {
		match self {
			Element::Scalar(scalar) => scalar.size(),
			Element::Vector(scalar, count) => scalar.size() * count,
			Element::Meshlet => MESHLET_STREAM_STRIDE,
		}
	}
}

/// Splits `payload`, the decoded bytes of `resource`, into its streams and attaches their element layouts.
///
/// A payload without named streams becomes one stream named [`WHOLE_PAYLOAD_STREAM_NAME`]; an empty one has no
/// streams. Fails when a stream lies outside the payload or doesn't hold a whole number of its elements.
pub fn inspect_payload<'a>(
	resource: &'a SerializableResource,
	payload: &'a [u8],
	format: PayloadFormat,
) -> Result<PayloadInspection<'a>, ResourceInspectError> {
	let descriptions = resource.streams().unwrap_or_default();
	let malformed = |stream: &str| ResourceInspectError::MalformedStream {
		id: resource.id().to_string(),
		stream: stream.to_string(),
	};

	let mut streams = Vec::with_capacity(descriptions.len().max(1));
	if descriptions.is_empty() {
		if !payload.is_empty() {
			streams.push(StreamInspection {
				name: WHOLE_PAYLOAD_STREAM_NAME,
				offset: 0,
				bytes: payload,
				layout: None,
			});
		}
	} else {
		for description in descriptions {
			let bytes = description
				.offset()
				.checked_add(description.size())
				.and_then(|end| payload.get(description.offset()..end))
				.ok_or_else(|| malformed(description.name()))?;
			streams.push(StreamInspection {
				name: description.name(),
				offset: description.offset(),
				bytes,
				layout: None,
			});
		}
	}

	if format == PayloadFormat::Typed {
		attach_layouts(resource, descriptions, &mut streams)?;
		for stream in &streams {
			if let Some(layout) = &stream.layout
				&& !stream.bytes.len().is_multiple_of(layout.element.size())
			{
				return Err(malformed(stream.name));
			}
		}
	}

	Ok(PayloadInspection { streams })
}

/// Reads the resource's model to find the element layout of each stream it knows. Streams it doesn't know keep no
/// layout and print as base64.
fn attach_layouts(
	resource: &SerializableResource,
	descriptions: &[StreamDescription],
	streams: &mut [StreamInspection<'_>],
) -> Result<(), ResourceInspectError> {
	match resource.class() {
		"Mesh" => {
			let mesh: MeshModel = deserialize_model(resource)?;
			for (stream, description) in streams.iter_mut().zip(descriptions) {
				stream.layout = mesh_stream_layout(&mesh, description);
			}
		}
		"Image" => {
			let image: Image = deserialize_model(resource)?;
			for stream in streams.iter_mut() {
				stream.layout = texel_layout(image_stream_format(&image, stream.name));
			}
		}
		"Audio" => {
			let audio: Audio = deserialize_model(resource)?;
			let scalar = match audio.bit_depth {
				BitDepths::Eight => Scalar::U8,
				BitDepths::Sixteen => Scalar::I16,
				BitDepths::TwentyFour => Scalar::I24,
				BitDepths::ThirtyTwo => Scalar::I32,
			};
			for stream in streams.iter_mut() {
				stream.layout = Some(StreamLayout {
					name: "frame".to_string(),
					element: Element::Vector(scalar, usize::from(audio.channel_count.max(1))),
				});
			}
		}
		"Lut" => {
			for stream in streams.iter_mut() {
				stream.layout = Some(StreamLayout {
					name: "rgb".to_string(),
					element: Element::Vector(Scalar::F32, 3),
				});
			}
		}
		_ => {}
	}

	Ok(())
}

/// Finds the mesh stream that `description` names and returns how its elements are stored.
fn mesh_stream_layout(mesh: &MeshModel, description: &StreamDescription) -> Option<StreamLayout> {
	let stream = mesh
		.streams
		.iter()
		.find(|stream| stream.offset == description.offset() && stream.size == description.size())?;

	match stream.stream_type {
		Streams::Vertices(semantic) => {
			let component = mesh
				.vertex_components
				.iter()
				.find(|component| component.semantic == semantic)?;
			let element = vertex_format_element(&component.format)?;
			(element.size() == stream.stride).then(|| StreamLayout {
				name: component.format.clone(),
				element,
			})
		}
		Streams::Indices(IndexStreamTypes::Vertices | IndexStreamTypes::Triangles | IndexStreamTypes::Meshlets) => {
			let (name, scalar) = match stream.stride {
				1 => ("u8", Scalar::U8),
				2 => ("u16", Scalar::U16),
				4 => ("u32", Scalar::U32),
				_ => return None,
			};
			Some(StreamLayout {
				name: name.to_string(),
				element: Element::Scalar(scalar),
			})
		}
		Streams::Meshlets => (stream.stride == MESHLET_STREAM_STRIDE).then(|| StreamLayout {
			name: "meshlet".to_string(),
			element: Element::Meshlet,
		}),
	}
}

/// Parses a vertex component format such as `vec3f` or `vec4u16`, the forms
/// [`VertexComponent::size`](crate::types::VertexComponent::size) accepts.
fn vertex_format_element(format: &str) -> Option<Element> {
	let (components, scalar) = format.strip_prefix("vec")?.split_at_checked(1)?;
	let components: usize = components.parse().ok().filter(|count| (2..=4).contains(count))?;
	let scalar = match scalar {
		"f" => Scalar::F32,
		"u" => Scalar::U32,
		"i" => Scalar::I32,
		"f16" => Scalar::F16,
		"u16" => Scalar::U16,
		"i16" => Scalar::I16,
		_ => return None,
	};
	Some(Element::Vector(scalar, components))
}

/// Returns the texel format of one image stream: its image-based lighting level's format, or the image's own.
fn image_stream_format(image: &Image, stream: &str) -> Formats {
	let Some(ibl) = &image.ibl else {
		return image.format;
	};
	if stream == IBL_DIFFUSE_IRRADIANCE_STREAM_NAME {
		ibl.diffuse_irradiance.format
	} else if stream.starts_with(IBL_PREFILTERED_SPECULAR_STREAM_PREFIX) {
		ibl.prefiltered_specular.format
	} else {
		image.format
	}
}

/// Returns how one texel of `format` is stored, or `None` for block-compressed formats, which print as base64.
fn texel_layout(format: Formats) -> Option<StreamLayout> {
	let element = match format {
		Formats::RG8 => Element::Vector(Scalar::U8, 2),
		Formats::RGB8 => Element::Vector(Scalar::U8, 3),
		Formats::RGBA8 | Formats::RGBA8SRGB => Element::Vector(Scalar::U8, 4),
		Formats::RG16 => Element::Vector(Scalar::U16, 2),
		Formats::RGB16 => Element::Vector(Scalar::U16, 3),
		Formats::RGBA16 => Element::Vector(Scalar::U16, 4),
		Formats::R16F => Element::Vector(Scalar::F16, 1),
		Formats::RGBA16F => Element::Vector(Scalar::F16, 4),
		Formats::BC5 | Formats::BC5SNORM | Formats::BC7 | Formats::BC7SRGB => return None,
	};
	debug_assert_eq!(format.texel_bytes(), Some(element.size()));
	Some(StreamLayout {
		name: format!("{format:?}"),
		element,
	})
}

fn deserialize_model<T>(resource: &SerializableResource) -> Result<T, ResourceInspectError>
where
	T: crate::ResourceArchive,
	<T as rkyv::Archive>::Archived: for<'a> rkyv::bytecheck::CheckBytes<crate::ResourceHighValidator<'a>>
		+ rkyv::Deserialize<T, crate::ResourceHighDeserializer>,
{
	crate::from_slice(resource.resource()).map_err(|error| ResourceInspectError::DeserializeResource {
		class: resource.class().to_string(),
		error: error.to_string(),
	})
}

impl Serialize for PayloadInspection<'_> {
	fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
		serializer.collect_seq(&self.streams)
	}
}

impl Serialize for StreamInspection<'_> {
	fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
		let mut map = serializer.serialize_map(Some(if self.layout.is_some() { 6 } else { 5 }))?;
		map.serialize_entry("name", self.name)?;
		map.serialize_entry("size", &self.bytes.len())?;
		map.serialize_entry("offset", &self.offset)?;
		match &self.layout {
			Some(layout) => {
				map.serialize_entry("encoding", "json")?;
				map.serialize_entry("element", &layout.name)?;
				map.serialize_entry(
					"data",
					&ElementsView {
						bytes: self.bytes,
						element: layout.element,
					},
				)?;
			}
			None => {
				map.serialize_entry("encoding", "base64")?;
				map.serialize_entry("data", &Base64View(self.bytes))?;
			}
		}
		map.end()
	}
}

/// Writes bytes as one padded base64 string without building the string first.
struct Base64View<'a>(&'a [u8]);

impl Serialize for Base64View<'_> {
	fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
		serializer.collect_str(&base64::display::Base64Display::with_config(self.0, base64::STANDARD))
	}
}

/// Writes every element of a stream as it is read, so a stream never exists as a JSON tree.
struct ElementsView<'a> {
	bytes: &'a [u8],
	element: Element,
}

impl Serialize for ElementsView<'_> {
	fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
		let elements = self.bytes.chunks_exact(self.element.size());
		let mut sequence = serializer.serialize_seq(Some(elements.len()))?;
		for bytes in elements {
			sequence.serialize_element(&ElementView {
				bytes,
				element: self.element,
			})?;
		}
		sequence.end()
	}
}

/// One element's bytes, exactly [`Element::size`] long.
struct ElementView<'a> {
	bytes: &'a [u8],
	element: Element,
}

impl Serialize for ElementView<'_> {
	fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
		match self.element {
			Element::Scalar(scalar) => ScalarView(self.bytes, scalar).serialize(serializer),
			Element::Vector(scalar, count) => {
				let mut sequence = serializer.serialize_seq(Some(count))?;
				for bytes in self.bytes.chunks_exact(scalar.size()) {
					sequence.serialize_element(&ScalarView(bytes, scalar))?;
				}
				sequence.end()
			}
			Element::Meshlet => {
				// The record layout is written by the mesh processor's `write_meshlet_record`.
				let float = |offset: usize| ScalarView(&self.bytes[offset..offset + 4], Scalar::F32);
				let vector = |offset: usize| [float(offset), float(offset + 4), float(offset + 8)];
				let mut map = serializer.serialize_map(Some(7))?;
				map.serialize_entry("vertex_count", &self.bytes[0])?;
				map.serialize_entry("triangle_count", &self.bytes[1])?;
				map.serialize_entry("center", &vector(4))?;
				map.serialize_entry("radius", &float(16))?;
				map.serialize_entry("cone_apex", &vector(20))?;
				map.serialize_entry("cone_cutoff", &float(32))?;
				map.serialize_entry("cone_axis", &vector(36))?;
				map.end()
			}
		}
	}
}

/// One little-endian number's bytes, exactly [`Scalar::size`] long.
struct ScalarView<'a>(&'a [u8], Scalar);

impl Serialize for ScalarView<'_> {
	fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
		let bytes = self.0;
		match self.1 {
			Scalar::U8 => serializer.serialize_u8(bytes[0]),
			Scalar::U16 => serializer.serialize_u16(u16::from_le_bytes([bytes[0], bytes[1]])),
			Scalar::I16 => serializer.serialize_i16(i16::from_le_bytes([bytes[0], bytes[1]])),
			Scalar::I24 => {
				// Sign-extend the three stored bytes into the high byte.
				let sign = if bytes[2] & 0x80 != 0 { 0xff } else { 0 };
				serializer.serialize_i32(i32::from_le_bytes([bytes[0], bytes[1], bytes[2], sign]))
			}
			Scalar::U32 => serializer.serialize_u32(u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]])),
			Scalar::I32 => serializer.serialize_i32(i32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]])),
			Scalar::F16 => serializer.serialize_f32(f16::from_le_bytes([bytes[0], bytes[1]]).to_f32()),
			Scalar::F32 => serializer.serialize_f32(f32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]])),
		}
	}
}

/// The `ResourceDocument` struct lets a tool print one resource's inspection with or without its payload as a single
/// JSON object.
///
/// Build it from [`ResourceInspection::json`] and, when the reader asked for payload bytes, the [`inspect_payload`]
/// result, which then takes the place of the `streams` descriptions. Serialize it with `serde_json` straight to the
/// output.
pub struct ResourceDocument<'a> {
	pub json: &'a Value,
	pub payload: Option<&'a PayloadInspection<'a>>,
}

impl Serialize for ResourceDocument<'_> {
	fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
		let (Some(payload), Value::Object(object)) = (self.payload, self.json) else {
			return self.json.serialize(serializer);
		};

		let mut map = serializer.serialize_map(Some(object.len()))?;
		for (key, value) in object {
			if key == "streams" {
				map.serialize_entry(key, payload)?;
			} else {
				map.serialize_entry(key, value)?;
			}
		}
		map.end()
	}
}
