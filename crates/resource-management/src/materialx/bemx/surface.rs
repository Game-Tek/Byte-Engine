//! Maps a MaterialX shading model onto the metallic-roughness surface a BEMX graph ends in.

use super::conversion::Converter;
use super::error::ConvertError;
use super::nodes::{cast, operand};
use crate::bemx::{self, BinaryOperator, Node, Surface, Type};
use crate::materialx::{NodeId, Source};

/// The `Property` struct names one input of a shading model and the value it falls back to.
///
/// The fallback only applies when the document neither writes the input nor includes the node
/// definition that declares its default, which is what happens when a material is baked without its
/// library.
#[derive(Clone, Copy)]
struct Property {
	name: &'static str,
	default: f32,
}

impl Property {
	const fn new(name: &'static str, default: f32) -> Self {
		Property { name, default }
	}
}

/// The `Model` struct maps one MaterialX shading model onto the inputs of a BEMX [`Surface`].
///
/// A BEMX surface is metallic-roughness, so every supported model is described by which of its inputs
/// carries each of the surface's inputs. A model whose inputs cannot be named this way is reported as
/// unsupported rather than approximated.
struct Model {
	category: &'static str,
	/// The colour the surface reflects, which an unlit model does not have.
	base_color: Option<Property>,
	/// The scale applied to the base colour, when the model separates the two.
	base_weight: Option<Property>,
	metalness: Property,
	roughness: Property,
	/// The opacity input, whose first channel becomes the surface's opacity.
	opacity: Property,
	emission_color: Option<Property>,
	/// The scale applied to the emission colour, when the model separates the two.
	emission_weight: Option<Property>,
	occlusion: Option<Property>,
	normal: &'static str,
}

/// The shading models that reduce onto a BEMX surface.
///
/// These cover the models authoring tools write: Autodesk's Standard Surface, the OpenPBR surface
/// that succeeds it, Pixar's USD preview surface, the glTF material model, Disney's principled
/// surface, and the unlit surface that carries emission alone.
const MODELS: [Model; 6] = [
	Model {
		category: "standard_surface",
		base_color: Some(Property::new("base_color", 0.8)),
		base_weight: Some(Property::new("base", 1.0)),
		metalness: Property::new("metalness", 0.0),
		roughness: Property::new("specular_roughness", 0.2),
		opacity: Property::new("opacity", 1.0),
		emission_color: Some(Property::new("emission_color", 1.0)),
		emission_weight: Some(Property::new("emission", 0.0)),
		occlusion: None,
		normal: "normal",
	},
	Model {
		category: "open_pbr_surface",
		base_color: Some(Property::new("base_color", 0.8)),
		base_weight: Some(Property::new("base_weight", 1.0)),
		metalness: Property::new("base_metalness", 0.0),
		roughness: Property::new("specular_roughness", 0.3),
		opacity: Property::new("geometry_opacity", 1.0),
		emission_color: Some(Property::new("emission_color", 1.0)),
		emission_weight: Some(Property::new("emission_luminance", 0.0)),
		occlusion: None,
		normal: "geometry_normal",
	},
	Model {
		category: "UsdPreviewSurface",
		base_color: Some(Property::new("diffuseColor", 0.18)),
		base_weight: None,
		metalness: Property::new("metallic", 0.0),
		roughness: Property::new("roughness", 0.5),
		opacity: Property::new("opacity", 1.0),
		emission_color: Some(Property::new("emissiveColor", 0.0)),
		emission_weight: None,
		occlusion: Some(Property::new("occlusion", 1.0)),
		normal: "normal",
	},
	Model {
		category: "gltf_pbr",
		base_color: Some(Property::new("base_color", 1.0)),
		base_weight: None,
		metalness: Property::new("metallic", 1.0),
		roughness: Property::new("roughness", 1.0),
		opacity: Property::new("alpha", 1.0),
		emission_color: Some(Property::new("emissive", 0.0)),
		emission_weight: Some(Property::new("emissive_strength", 1.0)),
		occlusion: Some(Property::new("occlusion", 1.0)),
		normal: "normal",
	},
	Model {
		category: "disney_principled",
		base_color: Some(Property::new("baseColor", 0.16)),
		base_weight: None,
		metalness: Property::new("metallic", 0.0),
		roughness: Property::new("roughness", 0.5),
		opacity: Property::new("opacity", 1.0),
		emission_color: None,
		emission_weight: None,
		occlusion: None,
		normal: "normal",
	},
	Model {
		// An unlit surface reflects nothing, so its emission carries everything the renderer shows.
		category: "surface_unlit",
		base_color: None,
		base_weight: None,
		metalness: Property::new("metalness", 0.0),
		roughness: Property::new("roughness", 1.0),
		opacity: Property::new("opacity", 1.0),
		emission_color: Some(Property::new("emission_color", 1.0)),
		emission_weight: Some(Property::new("emission", 1.0)),
		occlusion: None,
		normal: "normal",
	},
];

/// Reports whether a node category is one of the shading models that reduce onto a BEMX surface.
pub(super) fn is_shading_model(category: &str) -> bool {
	MODELS.iter().any(|model| model.category == category)
}

/// Converts one surface shader into the BEMX surface that carries its values.
pub(super) fn convert(
	converter: &mut Converter<'_, '_>,
	frame: usize,
	shader: NodeId,
	material: &str,
) -> Result<Surface, ConvertError> {
	let instance = converter.dag.node(shader);

	let model = MODELS
		.iter()
		.find(|model| model.category == instance.category)
		.ok_or_else(|| ConvertError::UnsupportedShader {
			material: material.to_string(),
			category: instance.category.to_string(),
		})?;

	let base_color = match model.base_color {
		Some(color) => weighted(converter, frame, shader, color, model.base_weight)?,
		None => converter.splat(0.0, Type::Vector3),
	};
	let opacity = property(converter, frame, shader, model.opacity, Type::Float)?;
	let metalness = property(converter, frame, shader, model.metalness, Type::Float)?;
	let roughness = property(converter, frame, shader, model.roughness, Type::Float)?;
	let emission = model
		.emission_color
		.map(|color| weighted(converter, frame, shader, color, model.emission_weight))
		.transpose()?;
	let occlusion = model
		.occlusion
		.map(|occlusion| property(converter, frame, shader, occlusion, Type::Float))
		.transpose()?;
	let normal = shading_normal(converter, frame, shader, model.normal)?;

	Ok(Surface {
		base_color: Some(base_color),
		opacity: Some(opacity),
		metalness: Some(metalness),
		roughness: Some(roughness),
		normal,
		emission,
		occlusion,
	})
}

/// Converts a colour of a shading model together with the weight that scales it.
fn weighted(
	converter: &mut Converter<'_, '_>,
	frame: usize,
	shader: NodeId,
	color: Property,
	weight: Option<Property>,
) -> Result<bemx::NodeId, ConvertError> {
	let color = property(converter, frame, shader, color, Type::Vector3)?;

	let Some(weight) = weight else {
		return Ok(color);
	};

	let weight = property(converter, frame, shader, weight, Type::Float)?;
	let name = converter.dag.node(shader).name;

	converter.add(
		name,
		Node::Binary {
			operator: BinaryOperator::Multiply,
			left: color,
			right: weight,
		},
	)
}

/// Converts one input of a shading model to the type the surface input carries.
fn property(
	converter: &mut Converter<'_, '_>,
	frame: usize,
	shader: NodeId,
	property: Property,
	r#type: Type,
) -> Result<bemx::NodeId, ConvertError> {
	let name = converter.dag.node(shader).name;
	let value = operand(converter, frame, shader, property.name, property.default, r#type)?;

	cast(converter, name, value, r#type)
}

/// Converts a shading model's normal input into the surface's world-space normal.
///
/// MaterialX shading models and BEMX surfaces both take a world-space normal, so the value passes
/// straight through. An input that no shading network drives leaves the geometric normal in place.
fn shading_normal(
	converter: &mut Converter<'_, '_>,
	frame: usize,
	shader: NodeId,
	input: &str,
) -> Result<Option<bemx::NodeId>, ConvertError> {
	let instance = converter.dag.node(shader);
	let name = instance.name;

	let Some(port) = instance.input(input) else {
		return Ok(None);
	};

	if !matches!(
		port.source,
		Source::Node { .. } | Source::Graph { .. } | Source::Interface { .. }
	) {
		// Only a shading network can change the normal; a constant or the geometric normal leaves it alone.
		return Ok(None);
	}

	let normal = converter.port(frame, port)?;

	cast(converter, name, normal, Type::Vector3).map(Some)
}
