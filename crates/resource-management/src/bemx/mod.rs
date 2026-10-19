//! BEMX, the Byte Engine material expression: the graph every material is described with before it becomes a shader.
//!
//! A BEMX graph is a typed, acyclic network of nodes that ends in one metallic-roughness [`Surface`]. The node set
//! follows the [MaterialX](https://materialx.org) standard library, so a MaterialX network maps onto it node for node,
//! but a graph carries no MaterialX machinery: no node definitions, no nested graphs, no names to resolve. Every
//! material source becomes one, and every renderer reads one:
//!
//! - Authored materials are `.bemx` files, read with [`json::parse`] and referenced from a `.bema` material.
//! - MaterialX documents convert with [`materialx::bemx::convert`](crate::materialx::bemx::convert).
//! - glTF and FBX importers build graphs from the materials they import.
//! - A renderer's [`ProgramGenerator`](crate::asset::handler::implementations::bema::ProgramGenerator) turns a graph
//!   into its shader program, writing node values with [`besl::Lowering`].
//!
//! # Building a graph
//!
//! [`GraphBuilder`] checks every node as it is added: inputs must exist, come earlier, and carry a type the node
//! accepts. A finished [`Graph`] is therefore always valid, and code that reads one never checks it again.
//!
//! ```
//! use resource_management::bemx::{BinaryOperator, GraphBuilder, Node, Surface, Type, Value};
//!
//! let mut builder = GraphBuilder::new();
//! let albedo = builder.add(Node::Image { texture: "albedo".to_string(), r#type: Type::Vector3, texcoord: None })?;
//! let tint = builder.constant(Value::Vector3([1.0, 0.9, 0.8]));
//! let tinted = builder.add(Node::Binary { operator: BinaryOperator::Multiply, left: albedo, right: tint })?;
//! let surface = builder.add(Node::Surface(Surface { base_color: Some(tinted), ..Surface::default() }))?;
//! let graph = builder.finish(surface)?;
//!
//! assert_eq!(graph.r#type(tinted), Some(Type::Vector3));
//! # Ok::<(), resource_management::bemx::GraphError>(())
//! ```
//!
//! # Parameters
//!
//! A graph names the values a material supplies instead of holding them: a [`Node::Parameter`] reads a value
//! parameter and a [`Node::Image`] samples a texture parameter, both by name. The `.bema` material that uses the
//! graph declares those parameters with their values, so variants share one graph and one shader.
//!
//! The [BEMX reference](/docs/reference/bemx) defines every node and the JSON form.

pub mod besl;
pub(crate) mod gltf;
mod graph;
pub(crate) mod import;
pub mod json;

pub use graph::{
	BinaryOperator, Channel, Comparison, Expected, Graph, GraphBuilder, GraphError, Node, NodeId, Surface, Type, UnaryOperator,
	Value,
};

/// The luminance weights MaterialX's standard library uses when a luminance node writes none.
pub const DEFAULT_LUMINANCE_WEIGHTS: [f32; 3] = [0.272_228_7, 0.674_081_8, 0.053_589_5];

/// The documentation page that defines BEMX nodes and their JSON form.
pub(crate) const BEMX_DOCS_PATH: &str = "reference/bemx";

/// The most values a MaterialX switch node chooses between.
pub const SWITCH_INPUTS: usize = 10;

/// The most values a MaterialX combine node joins.
pub const COMBINE_INPUTS: usize = 4;

/// The names MaterialX gives numbered inputs, `in1` up to `in10`, as switch and combine nodes read them.
pub const NUMBERED_INPUTS: [&str; SWITCH_INPUTS] = ["in1", "in2", "in3", "in4", "in5", "in6", "in7", "in8", "in9", "in10"];

/// The prefix of the texture parameters importers number by slot.
const MATERIAL_TEXTURE_PREFIX: &str = "material_texture_";

/// Names the material texture parameter an importer binds texture slot `slot` to.
///
/// Importers number the textures a generated material samples from zero, in first-use order, so materials that
/// differ only in their images build identical graphs and share one shader. [`material_texture_slot`] reads the slot
/// back.
pub(crate) fn material_texture_parameter_name(slot: usize) -> String {
	format!("{MATERIAL_TEXTURE_PREFIX}{slot}")
}

/// Returns the slot a texture parameter named by [`material_texture_parameter_name`] binds, or `None` for any other
/// name.
pub(crate) fn material_texture_slot(name: &str) -> Option<usize> {
	name.strip_prefix(MATERIAL_TEXTURE_PREFIX)?.parse().ok()
}

#[cfg(test)]
mod tests;
