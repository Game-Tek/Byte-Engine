use std::fmt::{Display, Formatter};

use crate::bemx::GraphError;
use crate::materialx::error::MATERIALX_DOCS_PATH;
use crate::online_docs_url;

/// The `ConvertError` enum explains why a MaterialX material cannot become a BEMX graph.
///
/// Every variant owns its text, so a failure can still be reported after the arena holding the
/// MaterialX graph is gone. Each one names the MaterialX element that caused it, because the fix is
/// always an edit to the `.mtlx` document rather than to the renderer.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ConvertError {
	/// The document declares no `<surfacematerial>`, so there is nothing to shade with.
	NoMaterial,
	/// A material's `surfaceshader` input is not connected to a shader node.
	MissingSurfaceShader { material: String },
	/// The surface shader is not one of the shading models that reduce onto a BEMX surface.
	UnsupportedShader { material: String, category: String },
	/// A node category in the shading network has no BEMX equivalent.
	UnsupportedNode { node: String, category: String },
	/// A node carries a MaterialX type that no BEMX type represents.
	UnsupportedType { node: String, data_type: String },
	/// An `<image>` samples with coordinates other than the mesh's own.
	UnsupportedTextureCoordinates { node: String },
	/// A geometric node reads a property the material stage does not carry.
	UnknownGeometricProperty { node: String, property: String },
	/// An `<image>` names no file to sample.
	MissingTextureFile { node: String },
	/// A node reads an output its declaration does not have.
	UnknownOutput { node: String, output: u32 },
	/// Node graph instantiation nested deeper than the conversion follows.
	InliningLimitExceeded { node: String },
	/// A node converted into a BEMX node whose inputs do not type-check.
	Graph { node: String, error: GraphError },
}

impl Display for ConvertError {
	fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
		match self {
			ConvertError::NoMaterial => write!(
				f,
				"MaterialX document declares no material. The most likely cause is a library of node definitions being baked as if it were a material. See {}.",
				online_docs_url(MATERIALX_DOCS_PATH)
			),
			ConvertError::MissingSurfaceShader { material } => write!(
				f,
				"Material '{material}' has no surface shader. The most likely cause is a 'surfaceshader' input that carries no 'nodename'."
			),
			ConvertError::UnsupportedShader { material, category } => write!(
				f,
				"Unsupported shading model <{category}> on material '{material}'. The most likely cause is a document written for a renderer with a different set of closures; rebuild the material on standard_surface, open_pbr_surface, UsdPreviewSurface or gltf_pbr. See {}.",
				online_docs_url(MATERIALX_DOCS_PATH)
			),
			ConvertError::UnsupportedNode { node, category } => write!(
				f,
				"Unsupported MaterialX node <{category}> named '{node}'. The most likely cause is a shading network that uses a node BEMX has no equivalent for; bake it into a texture or replace it. See {}.",
				online_docs_url(MATERIALX_DOCS_PATH)
			),
			ConvertError::UnsupportedType { node, data_type } => write!(
				f,
				"Unsupported MaterialX type '{data_type}' on node '{node}'. The most likely cause is a shading network carrying matrices, strings or closures where a material expects a colour or a number."
			),
			ConvertError::UnsupportedTextureCoordinates { node } => write!(
				f,
				"Unsupported texture coordinates on image node '{node}'. The most likely cause is a 'texcoord' input driven by a transform; converted images sample with the mesh's own texture coordinates only. See {}.",
				online_docs_url(MATERIALX_DOCS_PATH)
			),
			ConvertError::UnknownGeometricProperty { node, property } => write!(
				f,
				"Unknown geometric property '{property}' read by node '{node}'. The most likely cause is a primvar the mesh importer does not carry; the material stage supplies position, normal, tangent, bitangent, view direction and texture coordinates. See {}.",
				online_docs_url(MATERIALX_DOCS_PATH)
			),
			ConvertError::MissingTextureFile { node } => write!(
				f,
				"Image node '{node}' names no file. The most likely cause is a 'file' input left empty or connected instead of written."
			),
			ConvertError::UnknownOutput { node, output } => write!(
				f,
				"Node '{node}' has no output {output}. The most likely cause is a connection naming an output the node's declaration does not have."
			),
			ConvertError::InliningLimitExceeded { node } => write!(
				f,
				"Node graphs nested too deeply at '{node}'. The most likely cause is a node graph that instantiates itself, which MaterialX does not allow."
			),
			// The graph error already carries its own cause, so it only needs the MaterialX name in place of the
			// BEMX identifier.
			ConvertError::Graph { node, error } => error.describe(f, &format_args!("'{node}'")),
		}
	}
}

impl std::error::Error for ConvertError {}
