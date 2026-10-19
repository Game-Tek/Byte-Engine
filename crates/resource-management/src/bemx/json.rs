//! Reads the JSON form of a BEMX graph, the `.bemx` file format.
//!
//! A `.bemx` file is a JSON5 object with a `nodes` array. Every node has a `name`, a `node` kind spelled as the
//! MaterialX category it follows, such as `multiply` or `image`, and one field per input, named as MaterialX names
//! the input. An input holds either the name of another node or an inline constant: a number, or an array of two to
//! four numbers. Nodes may appear in any order. The optional `surface` field names the surface node; without it the
//! graph must hold exactly one.
//!
//! ```
//! use resource_management::bemx;
//!
//! let graph = bemx::json::parse(
//! 	r#"{
//! 		nodes: [
//! 			{ name: "albedo", node: "image", texture: "floor_texture", type: "vector3" },
//! 			{ name: "tinted", node: "multiply", in1: "albedo", in2: [1.0, 0.9, 0.8] },
//! 			{ name: "surface", node: "surface", base_color: "tinted", roughness: 0.7 },
//! 		],
//! 	}"#,
//! )?;
//!
//! assert!(graph.surface().base_color.is_some());
//! # Ok::<(), bemx::json::ParseError>(())
//! ```
//!
//! The [BEMX reference](/docs/reference/bemx) lists every node kind with its inputs and defaults.

use std::collections::HashMap;
use std::fmt::{Display, Formatter};

use serde_json::{Map, Value as Json};

use super::graph::{
	BinaryOperator, Channel, Comparison, Graph, GraphBuilder, GraphError, Node, NodeId, Surface, Type, UnaryOperator, Value,
};
use super::{BEMX_DOCS_PATH, COMBINE_INPUTS, DEFAULT_LUMINANCE_WEIGHTS, NUMBERED_INPUTS};
use crate::online_docs_url;

/// The fields every node carries regardless of its kind.
const COMMON_FIELDS: [&str; 2] = ["name", "node"];

/// Reads `.bemx` source text into a graph.
///
/// Next, give the graph to a renderer's
/// [`ProgramGenerator`](crate::asset::handler::implementations::bema::ProgramGenerator).
pub fn parse(source: &str) -> Result<Graph, ParseError> {
	let document = crate::asset::parse_json(source).map_err(|error| ParseError::Syntax {
		message: error.to_string(),
	})?;

	read(&document)
}

/// Reads a graph from an already parsed JSON document, such as one embedded in another asset.
pub fn read(document: &Json) -> Result<Graph, ParseError> {
	let root = document.as_object().ok_or(ParseError::NotAnObject)?;
	let nodes = root.get("nodes").and_then(Json::as_array).ok_or(ParseError::MissingNodes)?;

	if let Some(field) = root.keys().find(|key| !matches!(key.as_str(), "nodes" | "surface")) {
		return Err(ParseError::UnknownField {
			node: None,
			field: field.clone(),
		});
	}

	let mut entries = Vec::with_capacity(nodes.len());
	let mut positions = HashMap::with_capacity(nodes.len());

	for (position, node) in nodes.iter().enumerate() {
		let fields = node.as_object().ok_or(ParseError::UnnamedNode { position })?;
		let name = fields
			.get("name")
			.and_then(Json::as_str)
			.filter(|name| !name.is_empty())
			.ok_or(ParseError::UnnamedNode { position })?;

		if positions.insert(name, position).is_some() {
			return Err(ParseError::DuplicateName { node: name.to_string() });
		}

		entries.push((name, fields));
	}

	let surface = match root.get("surface") {
		Some(Json::String(name)) => *positions.get(name.as_str()).ok_or_else(|| ParseError::UnknownReference {
			node: None,
			field: "surface".to_string(),
			reference: name.clone(),
		})?,
		Some(_) => {
			return Err(ParseError::InvalidField {
				node: None,
				field: "surface".to_string(),
				expected: "the name of a node",
			});
		}
		None => {
			let mut surfaces = entries
				.iter()
				.enumerate()
				.filter(|(_, (_, fields))| fields.get("node").and_then(Json::as_str) == Some("surface"));

			match (surfaces.next(), surfaces.next()) {
				(Some((position, _)), None) => position,
				(None, _) => return Err(ParseError::MissingSurface),
				(Some(_), Some(_)) => return Err(ParseError::AmbiguousSurface),
			}
		}
	};

	let mut reader = Reader {
		entries: &entries,
		positions: &positions,
		ids: vec![State::Unread; entries.len()],
		builder: GraphBuilder::new(),
	};

	// Nodes nothing reads still have to be valid, so every entry is read, not only those the surface reaches.
	for position in 0..entries.len() {
		reader.resolve(position)?;
	}

	let State::Read(surface_id) = reader.ids[surface] else {
		unreachable!("Every entry was resolved above.");
	};

	reader.builder.finish(surface_id).map_err(|error| ParseError::Graph {
		node: entries[surface].0.to_string(),
		error,
	})
}

/// The `State` enum records how far reading one entry has got, so a cycle is noticed instead of recursing forever.
#[derive(Clone, Copy)]
enum State {
	Unread,
	Reading,
	Read(NodeId),
}

/// The `Reader` struct turns named JSON entries into graph nodes, reading every node an entry connects to first.
struct Reader<'d, 'e> {
	entries: &'e [(&'d str, &'d Map<String, Json>)],
	positions: &'e HashMap<&'d str, usize>,
	ids: Vec<State>,
	builder: GraphBuilder,
}

impl<'d> Reader<'d, '_> {
	/// Returns the identifier of the entry at `position`, adding it and everything it reads on first use.
	fn resolve(&mut self, position: usize) -> Result<NodeId, ParseError> {
		match self.ids[position] {
			State::Read(id) => return Ok(id),
			State::Reading => {
				return Err(ParseError::Cycle {
					node: self.entries[position].0.to_string(),
				});
			}
			State::Unread => {}
		}

		self.ids[position] = State::Reading;

		let (name, fields) = self.entries[position];
		let node = self.node(name, fields)?;
		let id = self.builder.add(node).map_err(|error| ParseError::Graph {
			node: name.to_string(),
			error,
		})?;

		self.ids[position] = State::Read(id);

		Ok(id)
	}

	/// Builds the node one entry describes.
	// One kind per arm keeps the whole format readable in a single place.
	#[allow(clippy::too_many_lines)]
	fn node(&mut self, name: &'d str, fields: &'d Map<String, Json>) -> Result<Node, ParseError> {
		let kind = fields
			.get("node")
			.and_then(Json::as_str)
			.ok_or_else(|| ParseError::MissingField {
				node: name.to_string(),
				field: "node",
			})?;

		let field = Fields { name, fields };

		if let Some(operator) = UnaryOperator::from_category(kind) {
			field.only(&["in"])?;

			return Ok(Node::Unary {
				operator,
				value: self.required(field, "in")?,
			});
		}

		if let Some(operator) = BinaryOperator::from_category(kind) {
			field.only(&["in1", "in2"])?;

			return Ok(Node::Binary {
				operator,
				left: self.required(field, "in1")?,
				right: self.or(field, "in2", operator.default_right())?,
			});
		}

		if let Some(comparison) = Comparison::from_category(kind) {
			field.only(&["value1", "value2", "in1", "in2"])?;

			return Ok(Node::Compare {
				comparison,
				left: self.or(field, "value1", Comparison::DEFAULT_LEFT)?,
				right: self.or(field, "value2", 0.0)?,
				then: self.or(field, "in1", 0.0)?,
				otherwise: self.or(field, "in2", 0.0)?,
			});
		}

		Ok(match kind {
			"constant" => {
				field.only(&["value"])?;

				Node::Constant(field.value("value")?.ok_or_else(|| field.missing("value"))?)
			}
			"parameter" => {
				field.only(&["parameter", "type"])?;

				Node::Parameter {
					name: field
						.string("parameter")?
						.ok_or_else(|| field.missing("parameter"))?
						.to_string(),
					r#type: field.r#type("type")?.ok_or_else(|| field.missing("type"))?,
				}
			}
			"position" | "normal" | "tangent" | "bitangent" | "viewdirection" => {
				field.only(&[])?;

				match kind {
					"position" => Node::Position,
					"normal" => Node::Normal,
					"tangent" => Node::Tangent,
					"bitangent" => Node::Bitangent,
					_ => Node::ViewDirection,
				}
			}
			"texcoord" => {
				field.only(&["index"])?;

				Node::TexCoord {
					index: field.integer("index")?.unwrap_or(0),
				}
			}
			"image" => {
				field.only(&["texture", "type", "texcoord"])?;

				Node::Image {
					texture: field.string("texture")?.ok_or_else(|| field.missing("texture"))?.to_string(),
					r#type: field.r#type("type")?.unwrap_or(Type::Vector4),
					texcoord: self.input(field, "texcoord")?,
				}
			}
			"clamp" | "smoothstep" => {
				field.only(&["in", "low", "high"])?;

				let (value, low, high) = (
					self.required(field, "in")?,
					self.or(field, "low", 0.0)?,
					self.or(field, "high", 1.0)?,
				);

				if kind == "clamp" {
					Node::Clamp { value, low, high }
				} else {
					Node::Smoothstep { value, low, high }
				}
			}
			"mix" => {
				field.only(&["bg", "fg", "mix"])?;

				Node::Mix {
					background: self.required(field, "bg")?,
					foreground: self.required(field, "fg")?,
					factor: self.or(field, "mix", 0.0)?,
				}
			}
			"remap" => {
				field.only(&["in", "inlow", "inhigh", "outlow", "outhigh"])?;

				Node::Remap {
					value: self.required(field, "in")?,
					input_low: self.or(field, "inlow", 0.0)?,
					input_high: self.or(field, "inhigh", 1.0)?,
					output_low: self.or(field, "outlow", 0.0)?,
					output_high: self.or(field, "outhigh", 1.0)?,
				}
			}
			"dotproduct" | "crossproduct" => {
				field.only(&["in1", "in2"])?;

				let (left, right) = (self.required(field, "in1")?, self.required(field, "in2")?);

				if kind == "dotproduct" {
					Node::DotProduct { left, right }
				} else {
					Node::CrossProduct { left, right }
				}
			}
			"magnitude" => {
				field.only(&["in"])?;

				Node::Magnitude {
					value: self.required(field, "in")?,
				}
			}
			"extract" => {
				field.only(&["in", "index"])?;

				let index = field.integer("index")?.unwrap_or(0);

				Node::Extract {
					value: self.required(field, "in")?,
					index: u8::try_from(index).map_err(|_| field.invalid("index", "a channel index from 0 to 3"))?,
				}
			}
			"swizzle" => {
				field.only(&["in", "channels"])?;

				let channels = field.string("channels")?.ok_or_else(|| field.missing("channels"))?;

				Node::Swizzle {
					value: self.required(field, "in")?,
					channels: channels
						.chars()
						.map(Channel::from_letter)
						.collect::<Option<Vec<_>>>()
						.ok_or_else(|| field.invalid("channels", "channel letters from xyzw, rgba, 0 and 1"))?,
				}
			}
			"combine" => {
				let names = &NUMBERED_INPUTS[..COMBINE_INPUTS];
				field.only(names)?;

				Node::Combine {
					values: self.numbered(field, names)?,
				}
			}
			"convert" => {
				field.only(&["in", "type"])?;

				Node::Convert {
					value: self.required(field, "in")?,
					r#type: field.r#type("type")?.ok_or_else(|| field.missing("type"))?,
				}
			}
			"luminance" => {
				field.only(&["in", "lumacoeffs"])?;

				let weights = match field.value("lumacoeffs")? {
					None => DEFAULT_LUMINANCE_WEIGHTS,
					Some(Value::Vector3(weights)) => weights,
					Some(_) => return Err(field.invalid("lumacoeffs", "an array of three numbers")),
				};

				Node::Luminance {
					value: self.required(field, "in")?,
					weights,
				}
			}
			"invert" => {
				field.only(&["in", "amount"])?;

				Node::Invert {
					value: self.required(field, "in")?,
					amount: self.or(field, "amount", 1.0)?,
				}
			}
			"premult" | "unpremult" => {
				field.only(&["in"])?;

				let value = self.required(field, "in")?;

				if kind == "premult" {
					Node::Premultiply { value }
				} else {
					Node::Unpremultiply { value }
				}
			}
			"switch" => {
				field.allow(|key| key == "which" || NUMBERED_INPUTS.contains(&key))?;

				Node::Switch {
					which: self.or(field, "which", 0.0)?,
					values: self.numbered(field, &NUMBERED_INPUTS)?,
				}
			}
			"normalmap" => {
				field.only(&["in", "scale"])?;

				Node::NormalMap {
					value: self.required(field, "in")?,
					scale: self.or(field, "scale", 1.0)?,
				}
			}
			"surface" => {
				field.only(&[
					"base_color",
					"opacity",
					"metalness",
					"roughness",
					"normal",
					"emission",
					"occlusion",
				])?;

				Node::Surface(Surface {
					base_color: self.input(field, "base_color")?,
					opacity: self.input(field, "opacity")?,
					metalness: self.input(field, "metalness")?,
					roughness: self.input(field, "roughness")?,
					normal: self.input(field, "normal")?,
					emission: self.input(field, "emission")?,
					occlusion: self.input(field, "occlusion")?,
				})
			}
			_ => {
				return Err(ParseError::UnknownKind {
					node: name.to_string(),
					kind: kind.to_string(),
				});
			}
		})
	}

	/// Reads one input: the node it names, a constant written inline, or `None` when the entry leaves it out.
	fn input(&mut self, field: Fields<'d>, key: &'static str) -> Result<Option<NodeId>, ParseError> {
		match field.fields.get(key) {
			None => Ok(None),
			Some(Json::String(reference)) => {
				let position = *self
					.positions
					.get(reference.as_str())
					.ok_or_else(|| ParseError::UnknownReference {
						node: Some(field.name.to_string()),
						field: key.to_string(),
						reference: reference.clone(),
					})?;

				self.resolve(position).map(Some)
			}
			Some(_) => {
				let value = field.value(key)?.expect("The key is present.");

				Ok(Some(self.builder.constant(value)))
			}
		}
	}

	/// Reads an input the node cannot do without.
	fn required(&mut self, field: Fields<'d>, key: &'static str) -> Result<NodeId, ParseError> {
		self.input(field, key)?.ok_or_else(|| field.missing(key))
	}

	/// Reads an input, falling back to a float constant when the entry leaves it out.
	fn or(&mut self, field: Fields<'d>, key: &'static str, default: f32) -> Result<NodeId, ParseError> {
		match self.input(field, key)? {
			Some(id) => Ok(id),
			None => Ok(self.builder.constant(Value::Float(default))),
		}
	}

	/// Reads inputs numbered from one, stopping at the first one the entry leaves out.
	fn numbered(&mut self, field: Fields<'d>, names: &[&'static str]) -> Result<Vec<NodeId>, ParseError> {
		let mut values = Vec::with_capacity(names.len());

		for name in names {
			match self.input(field, name)? {
				Some(id) => values.push(id),
				None => break,
			}
		}

		// A later input after a gap would be silently ignored, so it is reported instead.
		if let Some(skipped) = names[values.len()..].iter().find(|name| field.fields.contains_key(**name)) {
			return Err(field.invalid(skipped, "an input that follows every lower-numbered input"));
		}

		Ok(values)
	}
}

/// The `Fields` struct reads the plain fields of one entry and builds the errors that name it.
#[derive(Clone, Copy)]
struct Fields<'d> {
	name: &'d str,
	fields: &'d Map<String, Json>,
}

impl<'d> Fields<'d> {
	/// Reports the first field the entry's kind does not read, so a misspelt input is not silently left at its
	/// default.
	fn only(&self, allowed: &[&str]) -> Result<(), ParseError> {
		self.allow(|key| allowed.contains(&key))
	}

	/// Reports the first field that is not common to every node and that `allowed` rejects.
	fn allow(&self, allowed: impl Fn(&str) -> bool) -> Result<(), ParseError> {
		match self
			.fields
			.keys()
			.find(|key| !COMMON_FIELDS.contains(&key.as_str()) && !allowed(key))
		{
			Some(field) => Err(ParseError::UnknownField {
				node: Some(self.name.to_string()),
				field: field.clone(),
			}),
			None => Ok(()),
		}
	}

	fn string(&self, key: &'static str) -> Result<Option<&'d str>, ParseError> {
		match self.fields.get(key) {
			None => Ok(None),
			Some(Json::String(value)) => Ok(Some(value)),
			Some(_) => Err(self.invalid(key, "a string")),
		}
	}

	fn integer(&self, key: &'static str) -> Result<Option<u32>, ParseError> {
		match self.fields.get(key) {
			None => Ok(None),
			Some(value) => value
				.as_u64()
				.and_then(|value| u32::try_from(value).ok())
				.map(Some)
				.ok_or_else(|| self.invalid(key, "a whole number from zero")),
		}
	}

	fn r#type(&self, key: &'static str) -> Result<Option<Type>, ParseError> {
		let Some(name) = self.string(key)? else {
			return Ok(None);
		};

		Type::from_name(name)
			.map(Some)
			.ok_or_else(|| self.invalid(key, "float, vector2, vector3, vector4, color3, or color4"))
	}

	/// Reads a constant: a number, or an array of two to four numbers.
	fn value(&self, key: &'static str) -> Result<Option<Value>, ParseError> {
		let invalid = || self.invalid(key, "a node name, a number, or an array of two to four numbers");

		match self.fields.get(key) {
			None => Ok(None),
			Some(Json::Number(number)) => Ok(Some(Value::Float(number.as_f64().ok_or_else(invalid)? as f32))),
			Some(Json::Array(components)) => {
				let components = components
					.iter()
					.map(|component| component.as_f64().map(|component| component as f32))
					.collect::<Option<Vec<_>>>()
					.ok_or_else(invalid)?;

				match components.len() {
					2..=4 => Ok(Value::from_components(&components)),
					_ => Err(invalid()),
				}
			}
			Some(_) => Err(invalid()),
		}
	}

	fn missing(&self, field: &'static str) -> ParseError {
		ParseError::MissingField {
			node: self.name.to_string(),
			field,
		}
	}

	fn invalid(&self, field: &str, expected: &'static str) -> ParseError {
		ParseError::InvalidField {
			node: Some(self.name.to_string()),
			field: field.to_string(),
			expected,
		}
	}
}

/// The `ParseError` enum explains why `.bemx` source text does not describe a graph.
///
/// Every variant owns its text and names the node it is about, so the fix can be found in the file.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ParseError {
	/// The text is not valid JSON5.
	Syntax { message: String },
	/// The document is not an object.
	NotAnObject,
	/// The document has no `nodes` array.
	MissingNodes,
	/// A node is not an object with a non-empty `name`.
	UnnamedNode { position: usize },
	/// Two nodes share a name.
	DuplicateName { node: String },
	/// A node's kind is not a BEMX node kind.
	UnknownKind { node: String, kind: String },
	/// A field is not one the node's kind reads. `node` is `None` for a field of the document itself.
	UnknownField { node: Option<String>, field: String },
	/// A node leaves out a field its kind needs.
	MissingField { node: String, field: &'static str },
	/// A field holds a value of the wrong shape.
	InvalidField {
		node: Option<String>,
		field: String,
		expected: &'static str,
	},
	/// A field names a node the document does not have.
	UnknownReference {
		node: Option<String>,
		field: String,
		reference: String,
	},
	/// A node reads itself through its inputs.
	Cycle { node: String },
	/// The document has no surface node and names none.
	MissingSurface,
	/// The document has several surface nodes and does not name the one to use.
	AmbiguousSurface,
	/// A node does not fit the graph, such as an input of the wrong type.
	Graph { node: String, error: GraphError },
}

impl Display for ParseError {
	fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
		let docs = online_docs_url(BEMX_DOCS_PATH);
		let owner = |node: &Option<String>| {
			node.as_ref()
				.map_or_else(|| "the document".to_string(), |node| format!("node '{node}'"))
		};

		match self {
			ParseError::Syntax { message } => write!(
				f,
				"Invalid BEMX JSON: {message}. The most likely cause is a missing comma, quote, or bracket."
			),
			ParseError::NotAnObject => write!(
				f,
				"BEMX document is not an object. The most likely cause is a file that holds the nodes array alone; wrap it as `{{ nodes: [...] }}`. See {docs}."
			),
			ParseError::MissingNodes => write!(
				f,
				"BEMX document has no 'nodes' array. The most likely cause is a misspelt or missing 'nodes' field. See {docs}."
			),
			ParseError::UnnamedNode { position } => write!(
				f,
				"BEMX node {position} has no name. The most likely cause is a node written without a 'name' field, or one that is not an object."
			),
			ParseError::DuplicateName { node } => write!(
				f,
				"Two BEMX nodes are named '{node}'. The most likely cause is a copied node whose name was not changed."
			),
			ParseError::UnknownKind { node, kind } => write!(
				f,
				"Node '{node}' has the unknown kind '{kind}'. The most likely cause is a misspelt kind or a MaterialX node BEMX does not include. See {docs}."
			),
			ParseError::UnknownField { node, field } => write!(
				f,
				"{} has the unknown field '{field}'. The most likely cause is a misspelt input name. See {docs}.",
				capitalize(&owner(node))
			),
			ParseError::MissingField { node, field } => write!(
				f,
				"Node '{node}' has no '{field}' field. The most likely cause is a node written without an input its kind needs. See {docs}."
			),
			ParseError::InvalidField { node, field, expected } => write!(
				f,
				"Field '{field}' of {} does not hold {expected}. The most likely cause is a value written with the wrong type.",
				owner(node)
			),
			ParseError::UnknownReference { node, field, reference } => write!(
				f,
				"Field '{field}' of {} names the node '{reference}', which does not exist. The most likely cause is a misspelt node name.",
				owner(node)
			),
			ParseError::Cycle { node } => write!(
				f,
				"Node '{node}' reads its own value. The most likely cause is a connection loop; BEMX graphs cannot feed a value back into itself."
			),
			ParseError::MissingSurface => write!(
				f,
				"BEMX document has no surface node. The most likely cause is a graph without a node of kind 'surface'. See {docs}."
			),
			ParseError::AmbiguousSurface => write!(
				f,
				"BEMX document has several surface nodes. The most likely cause is a graph that lists alternatives; name the one to use in the 'surface' field."
			),
			ParseError::Graph { node, error } => error.describe(f, &format_args!("'{node}'")),
		}
	}
}

impl std::error::Error for ParseError {}

/// Returns `text` with its first letter in upper case.
fn capitalize(text: &str) -> String {
	let mut characters = text.chars();

	characters
		.next()
		.map(|first| first.to_uppercase().chain(characters).collect())
		.unwrap_or_default()
}
