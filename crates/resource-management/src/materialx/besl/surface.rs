//! Maps a MaterialX shading model onto the material properties this renderer's BRDF evaluates.

use besl::parser::Node;

use crate::materialx::{DataType, NodeId, Source};

use super::error::LowerError;
use super::lowering::Lowering;
use super::nodes::{convert, operand};
use super::syntax::{self, Expression};

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

/// The `Model` struct maps one MaterialX shading model onto this renderer's material properties.
///
/// The renderer evaluates a metallic-roughness BRDF, so every supported model is described by which
/// of its inputs carries each of that BRDF's parameters. A model whose inputs cannot be named this
/// way is reported as unsupported rather than approximated.
struct Model {
	category: &'static str,
	/// The colour the surface reflects, which an unlit model does not have.
	base_color: Option<Property>,
	/// The scale applied to the base colour, when the model separates the two.
	base_weight: Option<Property>,
	metalness: Property,
	roughness: Property,
	/// The opacity input, whose first channel becomes the surface's alpha.
	opacity: Property,
	emission_color: Option<Property>,
	/// The scale applied to the emission colour, when the model separates the two.
	emission_weight: Option<Property>,
	occlusion: Option<Property>,
	normal: &'static str,
}

/// The shading models this renderer evaluates.
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

/// Reports whether a node category is one of the shading models this renderer evaluates.
pub(super) fn is_shading_model(category: &str) -> bool {
	MODELS.iter().any(|model| model.category == category)
}

/// Lowers one surface shader into the assignments that hand its values to the renderer's BRDF.
pub(super) fn lower<'a>(
	lowering: &mut Lowering<'a, '_>,
	frame: usize,
	shader: NodeId,
	material: &str,
) -> Result<Vec<Node<'a>>, LowerError> {
	let instance = lowering.dag.node(shader);
	let name = instance.name;

	let model = MODELS
		.iter()
		.find(|model| model.category == instance.category)
		.ok_or_else(|| LowerError::UnsupportedShader {
			material: material.to_string(),
			category: instance.category.to_string(),
		})?;

	let mut statements = Vec::with_capacity(6);

	let base = weighted(lowering, frame, shader, model.base_color, model.base_weight)?;
	let opacity = property(lowering, frame, shader, model.opacity, DataType::Float)?;
	let opacity = lowering.addressable(name, opacity)?;
	let alpha = syntax::component(&opacity.syntax, 0, opacity.width());
	let base = lowering.addressable(name, base)?;
	let base_width = base.width();

	let albedo = Node::call(
		"vec4f",
		vec![
			syntax::component(&base.syntax, 0, base_width),
			syntax::component(&base.syntax, 1, base_width),
			syntax::component(&base.syntax, 2, base_width),
			alpha,
		],
	);

	statements.push(Node::member_assignment("albedo", albedo));

	let metalness = property(lowering, frame, shader, model.metalness, DataType::Float)?;
	statements.push(Node::member_assignment("metalness", metalness.syntax));

	let roughness = property(lowering, frame, shader, model.roughness, DataType::Float)?;
	statements.push(Node::member_assignment("roughness", roughness.syntax));

	if model.emission_color.is_some() {
		let emission = weighted(lowering, frame, shader, model.emission_color, model.emission_weight)?;

		statements.push(Node::member_assignment("emission", emission.syntax));
	}

	if let Some(occlusion) = model.occlusion {
		let occlusion = property(lowering, frame, shader, occlusion, DataType::Float)?;

		statements.push(Node::member_assignment("occlusion", occlusion.syntax));
	}

	if let Some(normal) = shading_normal(lowering, frame, shader, model.normal)? {
		statements.push(Node::member_assignment("normal", normal));
	}

	Ok(statements)
}

/// Lowers a colour of a shading model together with the weight that scales it.
///
/// Models differ in whether they separate the two, and an unlit model has no reflected colour at all,
/// so both are optional and a model that names neither contributes black.
fn weighted<'a>(
	lowering: &mut Lowering<'a, '_>,
	frame: usize,
	shader: NodeId,
	color: Option<Property>,
	weight: Option<Property>,
) -> Result<Expression<'a>, LowerError> {
	let Some(color) = color else {
		return Ok(Expression::new(syntax::splat_literal(0.0, 3), DataType::Color3));
	};

	let name = lowering.dag.node(shader).name;
	let color = property(lowering, frame, shader, color, DataType::Color3)?;

	let Some(weight) = weight else {
		return Ok(color);
	};

	let weight = property(lowering, frame, shader, weight, DataType::Float)?;

	lowering.bind(name, DataType::Color3, Node::operator("*", color.syntax, weight.syntax))
}

/// Lowers one input of a shading model, converting it to the type the material property carries.
fn property<'a>(
	lowering: &mut Lowering<'a, '_>,
	frame: usize,
	shader: NodeId,
	property: Property,
	data_type: DataType<'a>,
) -> Result<Expression<'a>, LowerError> {
	let name = lowering.dag.node(shader).name;
	let value = operand(lowering, frame, shader, property.name, property.default, data_type)?;

	convert(lowering, name, value, data_type)
}

/// Lowers a shading model's normal input into the tangent-space normal the renderer expects.
///
/// MaterialX shading models take a world-space normal, while the renderer rebuilds the shaded point's
/// tangent frame and expects a normal expressed in it. Projecting onto that frame is exact, because
/// the frame is orthonormal. An input that nothing drives leaves the renderer's own geometric normal
/// in place.
fn shading_normal<'a>(
	lowering: &mut Lowering<'a, '_>,
	frame: usize,
	shader: NodeId,
	input: &str,
) -> Result<Option<Node<'a>>, LowerError> {
	let instance = lowering.dag.node(shader);
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

	let world = lowering.port(frame, port)?;
	let world = lowering.addressable(name, world)?;

	Ok(Some(Node::call(
		"vec3f",
		vec![
			Node::call("dot", vec![world.syntax.clone(), Node::member_expression("T")]),
			Node::call("dot", vec![world.syntax.clone(), Node::member_expression("B")]),
			Node::call("dot", vec![world.syntax, Node::member_expression("N")]),
		],
	)))
}
