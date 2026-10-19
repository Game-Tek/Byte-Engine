//! The node graph a BEMX material expression is made of, and the rules that keep it well typed.

use std::fmt::{Display, Formatter};

use super::BEMX_DOCS_PATH;
use crate::online_docs_url;

/// The `Type` enum names the value a BEMX node produces, so connections can be checked before anything evaluates them.
///
/// MaterialX separates colours from vectors only to pick node overloads; BEMX folds them together because a colour
/// and a vector of the same width are evaluated the same way.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq, serde::Serialize)]
pub enum Type {
	Float,
	Vector2,
	Vector3,
	Vector4,
}

impl Type {
	/// Returns how many float components a value of this type holds.
	pub const fn width(self) -> usize {
		match self {
			Type::Float => 1,
			Type::Vector2 => 2,
			Type::Vector3 => 3,
			Type::Vector4 => 4,
		}
	}

	/// Returns the type that holds `width` float components, when one does.
	pub const fn from_width(width: usize) -> Option<Type> {
		match width {
			1 => Some(Type::Float),
			2 => Some(Type::Vector2),
			3 => Some(Type::Vector3),
			4 => Some(Type::Vector4),
			_ => None,
		}
	}

	/// Returns the type a name spells: a BEMX type name, or the MaterialX colour name of the same width.
	pub fn from_name(name: &str) -> Option<Type> {
		Some(match name {
			"float" => Type::Float,
			"vector2" => Type::Vector2,
			"vector3" | "color3" => Type::Vector3,
			"vector4" | "color4" => Type::Vector4,
			_ => return None,
		})
	}

	/// Returns the name the JSON form of BEMX spells this type with.
	pub const fn name(self) -> &'static str {
		match self {
			Type::Float => "float",
			Type::Vector2 => "vector2",
			Type::Vector3 => "vector3",
			Type::Vector4 => "vector4",
		}
	}
}

/// The `Value` enum holds a constant a graph computes with, such as a colour factor or a roughness.
#[derive(Clone, Copy, Debug, PartialEq, serde::Serialize)]
pub enum Value {
	Float(f32),
	Vector2([f32; 2]),
	Vector3([f32; 3]),
	Vector4([f32; 4]),
}

impl Value {
	/// Returns the type of this constant.
	pub const fn r#type(&self) -> Type {
		match self {
			Value::Float(_) => Type::Float,
			Value::Vector2(_) => Type::Vector2,
			Value::Vector3(_) => Type::Vector3,
			Value::Vector4(_) => Type::Vector4,
		}
	}

	/// Returns the components of this constant, first to last.
	pub fn components(&self) -> &[f32] {
		match self {
			Value::Float(value) => std::slice::from_ref(value),
			Value::Vector2(value) => value,
			Value::Vector3(value) => value,
			Value::Vector4(value) => value,
		}
	}

	/// Builds the constant whose components are `components`, when a type holds that many.
	pub fn from_components(components: &[f32]) -> Option<Value> {
		Some(match *components {
			[x] => Value::Float(x),
			[x, y] => Value::Vector2([x, y]),
			[x, y, z] => Value::Vector3([x, y, z]),
			[x, y, z, w] => Value::Vector4([x, y, z, w]),
			_ => return None,
		})
	}
}

/// The `NodeId` struct names one node of a [`Graph`] so other nodes can read its value.
///
/// A node can only read nodes added before it, so the order of identifiers is also an order in which the graph can
/// be evaluated.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd, serde::Serialize)]
pub struct NodeId(u32);

impl NodeId {
	/// Returns the position of the node in [`Graph::nodes`].
	pub const fn index(self) -> usize {
		self.0 as usize
	}

	/// Returns the identifier of the node at `index` in [`Graph::nodes`].
	pub(crate) fn from_index(index: usize) -> NodeId {
		NodeId(u32::try_from(index).expect("Graphs number their nodes with u32, so a position inside one always fits."))
	}
}

/// The `UnaryOperator` enum names the functions a [`Node::Unary`] applies to every component of its input.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq, serde::Serialize)]
pub enum UnaryOperator {
	/// MaterialX `absval`.
	Absolute,
	Floor,
	Ceil,
	Round,
	/// One for a positive component, minus one for a negative one, zero for zero.
	Sign,
	/// MaterialX `sqrt`.
	SquareRoot,
	/// MaterialX `exp`.
	Exponential,
	/// The natural logarithm, MaterialX `ln`.
	Logarithm,
	Sine,
	Cosine,
	Tangent,
	/// MaterialX `asin`.
	ArcSine,
	/// MaterialX `acos`.
	ArcCosine,
	/// Scales the input to unit length. A float input becomes its own sign.
	Normalize,
}

impl UnaryOperator {
	/// Returns the operator a MaterialX category names, such as `absval`.
	pub fn from_category(category: &str) -> Option<UnaryOperator> {
		Some(match category {
			"absval" => UnaryOperator::Absolute,
			"floor" => UnaryOperator::Floor,
			"ceil" => UnaryOperator::Ceil,
			"round" => UnaryOperator::Round,
			"sign" => UnaryOperator::Sign,
			"sqrt" => UnaryOperator::SquareRoot,
			"exp" => UnaryOperator::Exponential,
			"ln" => UnaryOperator::Logarithm,
			"sin" => UnaryOperator::Sine,
			"cos" => UnaryOperator::Cosine,
			"tan" => UnaryOperator::Tangent,
			"asin" => UnaryOperator::ArcSine,
			"acos" => UnaryOperator::ArcCosine,
			"normalize" => UnaryOperator::Normalize,
			_ => return None,
		})
	}
}

/// The `BinaryOperator` enum names the functions a [`Node::Binary`] applies component by component to two inputs.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq, serde::Serialize)]
pub enum BinaryOperator {
	Add,
	Subtract,
	Multiply,
	Divide,
	/// The floored remainder, which keeps the sign of the divisor.
	Modulo,
	Power,
	Minimum,
	Maximum,
	/// MaterialX `atan2`: the arc tangent of `left` over `right`, with both signs picking the quadrant.
	ArcTangent2,
}

impl BinaryOperator {
	/// Returns the operator a MaterialX category names, such as `multiply`.
	pub fn from_category(category: &str) -> Option<BinaryOperator> {
		Some(match category {
			"add" => BinaryOperator::Add,
			"subtract" => BinaryOperator::Subtract,
			"multiply" => BinaryOperator::Multiply,
			"divide" => BinaryOperator::Divide,
			"modulo" => BinaryOperator::Modulo,
			"power" => BinaryOperator::Power,
			"min" => BinaryOperator::Minimum,
			"max" => BinaryOperator::Maximum,
			"atan2" => BinaryOperator::ArcTangent2,
			_ => return None,
		})
	}

	/// Returns the value MaterialX gives the right operand, `in2`, when nothing writes it.
	///
	/// For every operator but `atan2` it leaves the left operand unchanged.
	pub const fn default_right(self) -> f32 {
		match self {
			BinaryOperator::Add | BinaryOperator::Subtract | BinaryOperator::Minimum | BinaryOperator::Maximum => 0.0,
			BinaryOperator::Multiply
			| BinaryOperator::Divide
			| BinaryOperator::Modulo
			| BinaryOperator::Power
			| BinaryOperator::ArcTangent2 => 1.0,
		}
	}
}

/// The `Comparison` enum names the test a [`Node::Compare`] picks between its two values with.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq, serde::Serialize)]
pub enum Comparison {
	/// MaterialX `ifgreater`.
	Greater,
	/// MaterialX `ifgreatereq`.
	GreaterOrEqual,
	/// MaterialX `ifequal`.
	Equal,
}

impl Comparison {
	/// The value MaterialX gives `value1` when nothing writes it, which passes the test against the default `value2`.
	pub const DEFAULT_LEFT: f32 = 1.0;

	/// Returns the comparison a MaterialX category names, such as `ifgreater`.
	pub fn from_category(category: &str) -> Option<Comparison> {
		Some(match category {
			"ifgreater" => Comparison::Greater,
			"ifgreatereq" => Comparison::GreaterOrEqual,
			"ifequal" => Comparison::Equal,
			_ => return None,
		})
	}
}

/// The `Channel` enum names what one lane of a [`Node::Swizzle`] result reads.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq, serde::Serialize)]
pub enum Channel {
	X,
	Y,
	Z,
	W,
	/// A constant zero lane.
	Zero,
	/// A constant one lane.
	One,
}

impl Channel {
	/// Returns the channel a MaterialX channel letter names: `xyzw`, `rgba`, `0` or `1`.
	pub const fn from_letter(letter: char) -> Option<Channel> {
		Some(match letter {
			'x' | 'r' => Channel::X,
			'y' | 'g' => Channel::Y,
			'z' | 'b' => Channel::Z,
			'w' | 'a' => Channel::W,
			'0' => Channel::Zero,
			'1' => Channel::One,
			_ => return None,
		})
	}

	/// Returns the input component this channel reads, or `None` for a constant lane.
	pub const fn component(self) -> Option<usize> {
		match self {
			Channel::X => Some(0),
			Channel::Y => Some(1),
			Channel::Z => Some(2),
			Channel::W => Some(3),
			Channel::Zero | Channel::One => None,
		}
	}
}

/// The `Surface` struct describes the metallic-roughness surface a material shades with.
///
/// Every BEMX graph ends in one. An input left as `None` takes the default the field documents, so a graph only
/// spells out what differs from a plain dielectric.
#[derive(Clone, Copy, Debug, Default, Eq, Hash, PartialEq, serde::Serialize)]
pub struct Surface {
	/// The colour the surface reflects, a `vector3`. Defaults to [`Surface::DEFAULT_BASE_COLOR`].
	pub base_color: Option<NodeId>,
	/// How much of the surface covers what is behind it, a `float`. Defaults to one.
	pub opacity: Option<NodeId>,
	/// How metallic the surface is, a `float` from zero to one. Defaults to zero.
	pub metalness: Option<NodeId>,
	/// How rough the surface is, a `float` from zero to one. Defaults to [`Surface::DEFAULT_ROUGHNESS`].
	pub roughness: Option<NodeId>,
	/// The shading normal in world space, a `vector3`. Defaults to the geometric normal.
	pub normal: Option<NodeId>,
	/// The light the surface emits, a `vector3`. Defaults to none.
	pub emission: Option<NodeId>,
	/// How much ambient light reaches the surface, a `float` from zero to one. Defaults to one.
	pub occlusion: Option<NodeId>,
}

impl Surface {
	/// The base colour of a surface that writes none: the light grey MaterialX's standard surface defaults to.
	pub const DEFAULT_BASE_COLOR: Value = Value::Vector3([0.8, 0.8, 0.8]);
	pub const DEFAULT_OPACITY: Value = Value::Float(1.0);
	pub const DEFAULT_METALNESS: Value = Value::Float(0.0);
	/// The roughness of a surface that writes none.
	pub const DEFAULT_ROUGHNESS: Value = Value::Float(0.5);
	pub const DEFAULT_EMISSION: Value = Value::Vector3([0.0, 0.0, 0.0]);
	pub const DEFAULT_OCCLUSION: Value = Value::Float(1.0);

	/// Lists every input with its name, the type it has to carry, and the value it takes when the graph leaves it
	/// out, in declaration order. The normal has no constant default: it falls back to the geometric normal.
	pub fn inputs(&self) -> [(&'static str, Option<NodeId>, Type, Option<Value>); 7] {
		[
			("base_color", self.base_color, Type::Vector3, Some(Self::DEFAULT_BASE_COLOR)),
			("opacity", self.opacity, Type::Float, Some(Self::DEFAULT_OPACITY)),
			("metalness", self.metalness, Type::Float, Some(Self::DEFAULT_METALNESS)),
			("roughness", self.roughness, Type::Float, Some(Self::DEFAULT_ROUGHNESS)),
			("normal", self.normal, Type::Vector3, None),
			("emission", self.emission, Type::Vector3, Some(Self::DEFAULT_EMISSION)),
			("occlusion", self.occlusion, Type::Float, Some(Self::DEFAULT_OCCLUSION)),
		]
	}
}

/// The `Node` enum lists the operations a BEMX graph is built from.
///
/// The set follows the MaterialX standard library, with one node per operation instead of one per overload: a node
/// whose inputs may be floats or vectors takes its type from them. Geometric values are in world space.
#[derive(Clone, Debug, PartialEq, serde::Serialize)]
pub enum Node {
	/// A constant value.
	Constant(Value),
	/// A value the material supplies by name, such as a tint a variant overrides.
	Parameter { name: String, r#type: Type },
	/// The position of the shaded point.
	Position,
	/// The interpolated geometric normal, unit length.
	Normal,
	/// The tangent of the shaded point, unit length, pointing along increasing texture coordinate `u`.
	Tangent,
	/// The bitangent of the shaded point, unit length, pointing along increasing texture coordinate `v`.
	Bitangent,
	/// The direction from the viewer toward the shaded point, unit length, as MaterialX defines it.
	ViewDirection,
	/// One texture coordinate set of the mesh, a `vector2`.
	TexCoord { index: u32 },
	/// A texture sample. `texture` names the material texture parameter to sample, `type` how many of its channels
	/// to keep, and `texcoord` the coordinates, which default to set zero.
	Image {
		texture: String,
		r#type: Type,
		texcoord: Option<NodeId>,
	},
	/// A function applied to every component of `value`.
	Unary { operator: UnaryOperator, value: NodeId },
	/// A function applied component by component. A `float` operand applies to every component of the other one.
	Binary {
		operator: BinaryOperator,
		left: NodeId,
		right: NodeId,
	},
	/// `value` limited to the range from `low` to `high`.
	Clamp { value: NodeId, low: NodeId, high: NodeId },
	/// A smooth step from zero at `low` to one at `high`, evaluated at `value`.
	Smoothstep { value: NodeId, low: NodeId, high: NodeId },
	/// The linear blend from `background` at a `factor` of zero to `foreground` at one.
	Mix {
		background: NodeId,
		foreground: NodeId,
		factor: NodeId,
	},
	/// `value` moved from the range `input_low` to `input_high` onto the range `output_low` to `output_high`.
	Remap {
		value: NodeId,
		input_low: NodeId,
		input_high: NodeId,
		output_low: NodeId,
		output_high: NodeId,
	},
	/// The dot product of two values of the same type, a `float`.
	DotProduct { left: NodeId, right: NodeId },
	/// The cross product of two `vector3` values.
	CrossProduct { left: NodeId, right: NodeId },
	/// The length of `value`, a `float`.
	Magnitude { value: NodeId },
	/// One component of `value`, a `float`.
	Extract { value: NodeId, index: u8 },
	/// The components of `value` picked, repeated, or replaced by constants, one lane per channel.
	Swizzle { value: NodeId, channels: Vec<Channel> },
	/// The components of every value joined into one, first to last.
	Combine { values: Vec<NodeId> },
	/// `value` converted to another width. A `float` fills every component; a narrower vector gains ones and a wider
	/// one loses its last components.
	Convert { value: NodeId, r#type: Type },
	/// The luminance of a colour, written to each of its colour channels. Alpha passes through.
	Luminance { value: NodeId, weights: [f32; 3] },
	/// `amount` minus `value`.
	Invert { value: NodeId, amount: NodeId },
	/// A `vector4` colour with its alpha multiplied into its colour channels.
	Premultiply { value: NodeId },
	/// A `vector4` colour with its alpha divided out of its colour channels.
	Unpremultiply { value: NodeId },
	/// `then` when `left` passes `comparison` against `right`, otherwise `otherwise`.
	Compare {
		comparison: Comparison,
		left: NodeId,
		right: NodeId,
		then: NodeId,
		otherwise: NodeId,
	},
	/// The value at position `which`, rounded to the nearest whole number, counting from zero.
	Switch { which: NodeId, values: Vec<NodeId> },
	/// The world-space normal a tangent-space normal map encodes.
	///
	/// `value` holds the map's texel in the zero-to-one range. Only its first two channels are read: the third is
	/// rebuilt from them, so maps stored with two channels decode the same as maps stored with three. `scale` scales
	/// the tangent-plane components, as glTF's normal scale does.
	NormalMap { value: NodeId, scale: NodeId },
	/// The surface the graph shades with.
	Surface(Surface),
}

impl Node {
	/// Calls `visit` with every node this node reads, in declaration order.
	pub fn visit_inputs(&self, mut visit: impl FnMut(NodeId)) {
		match self {
			Node::Constant(_)
			| Node::Parameter { .. }
			| Node::Position
			| Node::Normal
			| Node::Tangent
			| Node::Bitangent
			| Node::ViewDirection
			| Node::TexCoord { .. } => {}
			Node::Image { texcoord, .. } => texcoord.iter().copied().for_each(visit),
			Node::Unary { value, .. }
			| Node::Magnitude { value }
			| Node::Extract { value, .. }
			| Node::Swizzle { value, .. }
			| Node::Convert { value, .. }
			| Node::Luminance { value, .. }
			| Node::Premultiply { value }
			| Node::Unpremultiply { value } => visit(*value),
			Node::Binary { left, right, .. } | Node::DotProduct { left, right } | Node::CrossProduct { left, right } => {
				visit(*left);
				visit(*right);
			}
			Node::Invert { value, amount } => {
				visit(*value);
				visit(*amount);
			}
			Node::NormalMap { value, scale } => {
				visit(*value);
				visit(*scale);
			}
			Node::Clamp { value, low, high } | Node::Smoothstep { value, low, high } => {
				visit(*value);
				visit(*low);
				visit(*high);
			}
			Node::Mix {
				background,
				foreground,
				factor,
			} => {
				visit(*background);
				visit(*foreground);
				visit(*factor);
			}
			Node::Remap {
				value,
				input_low,
				input_high,
				output_low,
				output_high,
			} => {
				for input in [value, input_low, input_high, output_low, output_high] {
					visit(*input);
				}
			}
			Node::Combine { values } => values.iter().copied().for_each(visit),
			Node::Compare {
				left,
				right,
				then,
				otherwise,
				..
			} => {
				for input in [left, right, then, otherwise] {
					visit(*input);
				}
			}
			Node::Switch { which, values } => {
				visit(*which);
				values.iter().copied().for_each(visit);
			}
			Node::Surface(surface) => surface
				.inputs()
				.into_iter()
				.filter_map(|(_, input, ..)| input)
				.for_each(visit),
		}
	}
}

/// The `Graph` struct holds one material expression: the nodes that compute its surface and the surface they feed.
///
/// A graph is always well typed: [`GraphBuilder`] checks every node as it is added, so a renderer can evaluate a
/// graph without checking it again. Build one with [`GraphBuilder`], read one from JSON with
/// [`super::json::parse`], or convert one from MaterialX with
/// [`materialx::bemx::convert`](crate::materialx::bemx::convert).
///
/// Next, give the graph to a renderer's
/// [`ProgramGenerator`](crate::asset::handler::implementations::bema::ProgramGenerator), or lower it yourself
/// with [`super::besl::Lowering`].
#[derive(Clone, Debug, PartialEq, serde::Serialize)]
pub struct Graph {
	nodes: Vec<Node>,
	/// The type each node produces, or `None` for the surface. Derived from `nodes`, so it is not serialized.
	#[serde(skip)]
	types: Vec<Option<Type>>,
	surface: NodeId,
}

impl Graph {
	/// Returns every node, in an order in which each node comes after the nodes it reads.
	pub fn nodes(&self) -> &[Node] {
		&self.nodes
	}

	/// Returns one node.
	pub fn node(&self, id: NodeId) -> &Node {
		&self.nodes[id.index()]
	}

	/// Returns the type a node produces, or `None` for the surface node.
	pub fn r#type(&self, id: NodeId) -> Option<Type> {
		self.types[id.index()]
	}

	/// Returns the surface the graph shades with.
	pub fn surface(&self) -> &Surface {
		match self.node(self.surface) {
			Node::Surface(surface) => surface,
			_ => unreachable!("A graph's surface identifier always names a surface node. The builder checks it."),
		}
	}

	/// Moves reads of image channels to the channels a packed texture stores them in.
	///
	/// Importers call this when the image processor bakes a texture with fewer channels than it was authored with.
	/// `stored_channel` receives a texture parameter name and an authored channel, and returns the channel the baked
	/// texture holds it in, `Ok(None)` when the texture is not packed, or an error when the packing drops it. Only
	/// [`Node::Extract`] reads of an [`Node::Image`] move, because those are the reads a packing describes.
	pub(crate) fn pack_texture_channels<E>(
		&mut self,
		mut stored_channel: impl FnMut(&str, u8) -> Result<Option<u8>, E>,
	) -> Result<(), E> {
		for index in 0..self.nodes.len() {
			let Node::Extract { value, index: channel } = self.nodes[index] else {
				continue;
			};

			let Node::Image { texture, .. } = &self.nodes[value.index()] else {
				continue;
			};

			if let Some(stored) = stored_channel(texture, channel)? {
				// An image always produces a `vector4` sample before it is narrowed, so a lower channel stays in range.
				debug_assert!(usize::from(stored) < self.types[value.index()].map_or(4, Type::width));

				self.nodes[index] = Node::Extract { value, index: stored };
			}
		}

		Ok(())
	}
}

/// The `GraphBuilder` struct assembles a [`Graph`] one node at a time, checking each node as it is added.
///
/// Add the nodes a value depends on before the node that reads them, then call [`GraphBuilder::finish`] with the
/// surface node.
#[derive(Debug, Default)]
pub struct GraphBuilder {
	nodes: Vec<Node>,
	types: Vec<Option<Type>>,
}

impl GraphBuilder {
	pub fn new() -> Self {
		Self::default()
	}

	/// Adds a node after checking that it reads existing nodes of the types it accepts.
	///
	/// The error names the node by the identifier it would have had, which is the number of nodes added before it.
	pub fn add(&mut self, node: Node) -> Result<NodeId, GraphError> {
		let id = NodeId(u32::try_from(self.nodes.len()).map_err(|_| GraphError::TooManyNodes)?);
		let r#type = infer(id, &node, &self.types)?;

		self.nodes.push(node);
		self.types.push(r#type);

		Ok(id)
	}

	/// Adds a constant, which reads nothing and so cannot fail to check.
	pub fn constant(&mut self, value: Value) -> NodeId {
		self.add(Node::Constant(value))
			.expect("A constant reads no other node, so it always checks.")
	}

	/// Returns the type a node added earlier produces, or `None` for a surface node.
	pub fn r#type(&self, id: NodeId) -> Option<Type> {
		self.types.get(id.index()).copied().flatten()
	}

	/// Finishes the graph with `surface` as the surface it shades with.
	pub fn finish(self, surface: NodeId) -> Result<Graph, GraphError> {
		if !matches!(self.nodes.get(surface.index()), Some(Node::Surface(_))) {
			return Err(GraphError::NotASurface { node: surface });
		}

		Ok(Graph {
			nodes: self.nodes,
			types: self.types,
			surface,
		})
	}
}

/// Returns the type `node` produces, after checking every input it reads against the types it accepts.
///
/// `types` holds the type of every node added before `node`, whose identifier is `id`.
fn infer(id: NodeId, node: &Node, types: &[Option<Type>]) -> Result<Option<Type>, GraphError> {
	let input = |input: NodeId, name: &'static str| -> Result<Type, GraphError> {
		match types.get(input.index()) {
			// A node can only read nodes added before it, which also keeps the graph free of cycles.
			None => Err(GraphError::UnknownInput { node: id, input: name }),
			Some(None) => Err(GraphError::ReadsSurface { node: id, input: name }),
			Some(Some(r#type)) => Ok(*r#type),
		}
	};
	let mismatch = |name: &'static str, expected: Expected, found: Type| GraphError::TypeMismatch {
		node: id,
		input: name,
		expected,
		found,
	};
	// Checks an input that must have type `expected`.
	let exactly = |value: NodeId, name: &'static str, expected: Type| -> Result<Type, GraphError> {
		let found = input(value, name)?;

		if found == expected {
			Ok(found)
		} else {
			Err(mismatch(name, Expected::Type(expected), found))
		}
	};
	// Checks an input that must have type `like` or be a float, which applies to every component.
	let like_or_float = |value: NodeId, name: &'static str, like: Type| -> Result<(), GraphError> {
		let found = input(value, name)?;

		if found == like || found == Type::Float {
			Ok(())
		} else {
			Err(mismatch(name, Expected::TypeOrFloat(like), found))
		}
	};
	// Checks two inputs that must agree on their type, unless one of them is a float that applies to every
	// component of the other, and returns the wider one.
	let agree = |first: (NodeId, &'static str), second: (NodeId, &'static str)| -> Result<Type, GraphError> {
		let left = input(first.0, first.1)?;
		let right = input(second.0, second.1)?;

		match (left, right) {
			_ if left == right => Ok(left),
			(Type::Float, _) => Ok(right),
			(_, Type::Float) => Ok(left),
			_ => Err(mismatch(second.1, Expected::TypeOrFloat(left), right)),
		}
	};

	Ok(Some(match node {
		Node::Constant(value) => value.r#type(),
		Node::Parameter { name, r#type } => {
			if name.is_empty() {
				return Err(GraphError::UnnamedParameter { node: id });
			}

			*r#type
		}
		Node::Position | Node::Normal | Node::Tangent | Node::Bitangent | Node::ViewDirection => Type::Vector3,
		Node::TexCoord { .. } => Type::Vector2,
		Node::Image {
			texture,
			r#type,
			texcoord,
		} => {
			if texture.is_empty() {
				return Err(GraphError::UnnamedParameter { node: id });
			}

			if let Some(texcoord) = texcoord {
				exactly(*texcoord, "texcoord", Type::Vector2)?;
			}

			*r#type
		}
		Node::Unary { value, .. } => input(*value, "in")?,
		Node::Binary { left, right, .. } => agree((*left, "in1"), (*right, "in2"))?,
		Node::Clamp { value, low, high } | Node::Smoothstep { value, low, high } => {
			let r#type = input(*value, "in")?;

			like_or_float(*low, "low", r#type)?;
			like_or_float(*high, "high", r#type)?;

			r#type
		}
		Node::Mix {
			background,
			foreground,
			factor,
		} => {
			let r#type = agree((*background, "bg"), (*foreground, "fg"))?;

			like_or_float(*factor, "mix", r#type)?;

			r#type
		}
		Node::Remap {
			value,
			input_low,
			input_high,
			output_low,
			output_high,
		} => {
			let r#type = input(*value, "in")?;

			for (bound, name) in [
				(input_low, "inlow"),
				(input_high, "inhigh"),
				(output_low, "outlow"),
				(output_high, "outhigh"),
			] {
				like_or_float(*bound, name, r#type)?;
			}

			r#type
		}
		Node::DotProduct { left, right } => {
			let r#type = input(*left, "in1")?;

			exactly(*right, "in2", r#type)?;

			Type::Float
		}
		Node::CrossProduct { left, right } => {
			exactly(*left, "in1", Type::Vector3)?;
			exactly(*right, "in2", Type::Vector3)?
		}
		Node::Magnitude { value } => {
			input(*value, "in")?;

			Type::Float
		}
		Node::Extract { value, index } => {
			let width = input(*value, "in")?.width();

			if usize::from(*index) >= width {
				return Err(GraphError::ChannelOutOfRange {
					node: id,
					channel: usize::from(*index),
					width,
				});
			}

			Type::Float
		}
		Node::Swizzle { value, channels } => {
			let width = input(*value, "in")?.width();

			if let Some(channel) = channels
				.iter()
				.filter_map(|channel| channel.component())
				.find(|component| *component >= width)
			{
				return Err(GraphError::ChannelOutOfRange {
					node: id,
					channel,
					width,
				});
			}

			Type::from_width(channels.len()).ok_or(GraphError::InvalidWidth {
				node: id,
				width: channels.len(),
			})?
		}
		Node::Combine { values } => {
			let mut width = 0;

			for value in values {
				width += input(*value, "in")?.width();
			}

			// Joining a single value would only copy it, so a combination needs at least two components.
			match Type::from_width(width) {
				Some(r#type) if width >= 2 => r#type,
				_ => return Err(GraphError::InvalidWidth { node: id, width }),
			}
		}
		Node::Convert { value, r#type } => {
			input(*value, "in")?;

			*r#type
		}
		Node::Luminance { value, .. } => match input(*value, "in")? {
			r#type @ (Type::Vector3 | Type::Vector4) => r#type,
			found => return Err(mismatch("in", Expected::Colour, found)),
		},
		Node::Invert { value, amount } => {
			let r#type = input(*value, "in")?;

			like_or_float(*amount, "amount", r#type)?;

			r#type
		}
		Node::Premultiply { value } | Node::Unpremultiply { value } => exactly(*value, "in", Type::Vector4)?,
		Node::Compare {
			left,
			right,
			then,
			otherwise,
			..
		} => {
			exactly(*left, "value1", Type::Float)?;
			exactly(*right, "value2", Type::Float)?;

			agree((*then, "in1"), (*otherwise, "in2"))?
		}
		Node::Switch { which, values } => {
			exactly(*which, "which", Type::Float)?;

			let (first, rest) = values.split_first().ok_or(GraphError::EmptySwitch { node: id })?;
			let mut r#type = input(*first, "in1")?;

			// Every value has to fit the widest one so far, with a float applying to every component.
			for value in rest {
				r#type = match (r#type, input(*value, "in")?) {
					(current, found) if current == found => current,
					(Type::Float, found) => found,
					(current, Type::Float) => current,
					(current, found) => return Err(mismatch("in", Expected::TypeOrFloat(current), found)),
				};
			}

			r#type
		}
		Node::NormalMap { value, scale } => {
			exactly(*value, "in", Type::Vector3)?;
			exactly(*scale, "scale", Type::Float)?;

			Type::Vector3
		}
		Node::Surface(surface) => {
			for (name, value, r#type, _) in surface.inputs() {
				if let Some(value) = value {
					exactly(value, name, r#type)?;
				}
			}

			return Ok(None);
		}
	}))
}

/// The `Expected` enum describes what an input accepts, so a type mismatch can say what would have fit.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Expected {
	/// Exactly this type.
	Type(Type),
	/// This type, or a `float` that applies to every component.
	TypeOrFloat(Type),
	/// A `vector3` or `vector4` colour.
	Colour,
}

impl Display for Expected {
	fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
		match self {
			Expected::Type(r#type) => write!(f, "{}", r#type.name()),
			Expected::TypeOrFloat(r#type) => write!(f, "{} or float", r#type.name()),
			Expected::Colour => write!(f, "vector3 or vector4"),
		}
	}
}

/// The `GraphError` enum explains why a node cannot join a BEMX graph.
///
/// Each variant names the node by its [`NodeId`]; the JSON reader reports the node's name instead.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum GraphError {
	/// An input names a node that was not added before the node reading it.
	UnknownInput { node: NodeId, input: &'static str },
	/// An input reads the surface, which produces no value.
	ReadsSurface { node: NodeId, input: &'static str },
	/// An input reads a value of a type it does not accept.
	TypeMismatch {
		node: NodeId,
		input: &'static str,
		expected: Expected,
		found: Type,
	},
	/// A channel reads past the last component of its input.
	ChannelOutOfRange { node: NodeId, channel: usize, width: usize },
	/// A node would produce a value no type holds.
	InvalidWidth { node: NodeId, width: usize },
	/// A switch has nothing to choose from.
	EmptySwitch { node: NodeId },
	/// A parameter or image names no material parameter.
	UnnamedParameter { node: NodeId },
	/// The node a graph finishes with is not a surface.
	NotASurface { node: NodeId },
	/// The graph has more nodes than an identifier can number.
	TooManyNodes,
}

impl GraphError {
	/// Returns the node the error is about, when it is about one.
	fn node(&self) -> Option<NodeId> {
		match *self {
			GraphError::UnknownInput { node, .. }
			| GraphError::ReadsSurface { node, .. }
			| GraphError::TypeMismatch { node, .. }
			| GraphError::ChannelOutOfRange { node, .. }
			| GraphError::InvalidWidth { node, .. }
			| GraphError::EmptySwitch { node }
			| GraphError::UnnamedParameter { node }
			| GraphError::NotASurface { node } => Some(node),
			GraphError::TooManyNodes => None,
		}
	}

	/// Writes the error, naming the node it is about with `node`.
	pub(crate) fn describe(&self, f: &mut Formatter<'_>, node: &dyn Display) -> std::fmt::Result {
		let docs = online_docs_url(BEMX_DOCS_PATH);

		match self {
			GraphError::UnknownInput { input, .. } => write!(
				f,
				"Node {node} reads a node that does not exist on input '{input}'. The most likely cause is a connection to a node that was not added before it."
			),
			GraphError::ReadsSurface { input, .. } => write!(
				f,
				"Node {node} reads the surface on input '{input}'. The most likely cause is a connection to the surface node, which produces no value. See {docs}."
			),
			GraphError::TypeMismatch {
				input, expected, found, ..
			} => write!(
				f,
				"Node {node} reads a {} on input '{input}', which accepts a {expected}. The most likely cause is a connection to the wrong node, or a missing convert or swizzle node. See {docs}.",
				found.name()
			),
			GraphError::ChannelOutOfRange { channel, width, .. } => write!(
				f,
				"Node {node} reads channel {channel} of a value with {width} components. The most likely cause is a channel index or letter that counts past the input's last component."
			),
			GraphError::InvalidWidth { width, .. } => write!(
				f,
				"Node {node} would produce {width} components. The most likely cause is a combine or swizzle node that produces fewer than one or more than four components."
			),
			GraphError::EmptySwitch { .. } => write!(
				f,
				"Switch node {node} has no values. The most likely cause is a switch node written without its 'in1' input."
			),
			GraphError::UnnamedParameter { .. } => write!(
				f,
				"Node {node} names no material parameter. The most likely cause is an empty 'parameter' or 'texture' field."
			),
			GraphError::NotASurface { .. } => write!(
				f,
				"Node {node} is not a surface. The most likely cause is a graph whose 'surface' names a node of another kind. See {docs}."
			),
			GraphError::TooManyNodes => write!(
				f,
				"The graph has more nodes than BEMX can number. The most likely cause is a generator that adds nodes without bound."
			),
		}
	}
}

impl Display for GraphError {
	fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
		match self.node() {
			Some(node) => self.describe(f, &format_args!("#{}", node.index())),
			None => self.describe(f, &""),
		}
	}
}

impl std::error::Error for GraphError {}
