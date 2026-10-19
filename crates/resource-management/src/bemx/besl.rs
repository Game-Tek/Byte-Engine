//! Lowers the values of a BEMX graph into the BESL statements that compute them.
//!
//! A renderer's [`ProgramGenerator`](crate::asset::handler::implementations::bema::ProgramGenerator) owns what a
//! material program looks like: its entry point, its bindings, and how the surface reaches the BRDF. What every
//! generator shares is how a node computes its value, which is what this module writes. The generator describes
//! the values only it can supply, such as the shaded point's normal or how a texture is sampled, by implementing
//! [`Stage`], then asks a [`Lowering`] for the value of each surface input and writes those values wherever its
//! program expects them.
//!
//! ```
//! use besl::parser::Node;
//! use resource_management::bemx::{self, besl::{Geometry, LowerError, Lowering, Stage}};
//!
//! /// A stage whose shaded point exposes its geometry as plain variables.
//! struct Variables;
//!
//! impl<'a> Stage<'a> for Variables {
//! 	fn geometry(&mut self, geometry: Geometry) -> Node<'a> {
//! 		Node::member_expression(match geometry {
//! 			Geometry::Position => "position",
//! 			Geometry::Normal => "normal",
//! 			Geometry::Tangent => "tangent",
//! 			Geometry::Bitangent => "bitangent",
//! 			Geometry::View => "view",
//! 			Geometry::TexCoord => "uv",
//! 		})
//! 	}
//!
//! 	fn sample(&mut self, texture: &'a str) -> Result<Node<'a>, LowerError> {
//! 		Ok(Node::call("sample", vec![Node::member_expression(texture), Node::member_expression("uv")]))
//! 	}
//!
//! 	fn parameter(&mut self, name: &'a str, _: bemx::Type) -> Result<Node<'a>, LowerError> {
//! 		Ok(Node::member_expression(name))
//! 	}
//! }
//!
//! let graph = bemx::json::parse(r#"{ nodes: [{ name: "surface", node: "surface", roughness: 0.25 }] }"#)?;
//! let mut lowering = Lowering::new(&graph, Variables);
//! let roughness = lowering.value(graph.surface().roughness.unwrap())?;
//! let (mut statements, _) = lowering.finish();
//!
//! statements.push(Node::member_assignment("roughness", roughness.syntax));
//! # Ok::<(), Box<dyn std::error::Error>>(())
//! ```
//!
//! Values are computed in `f32`. Every node that does work lands in its own local, so a value read twice is
//! computed once, and only the nodes a requested value reaches are written.

use std::fmt::{Display, Formatter};

use besl::parser::{Expressions, Node, Nodes};

use super::BEMX_DOCS_PATH;
use super::graph::{BinaryOperator, Channel, Comparison, Graph, Node as GraphNode, NodeId, Type, UnaryOperator, Value};
use crate::online_docs_url;

/// The smallest alpha [`GraphNode::Unpremultiply`] divides by, so a fully transparent texel does not divide by zero.
const MINIMUM_ALPHA: f32 = 1.0e-6;

/// The `Geometry` enum names the values of the shaded point a [`Stage`] supplies.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Geometry {
	/// The world-space position, a `vec3f`.
	Position,
	/// The unit world-space geometric normal, a `vec3f`.
	Normal,
	/// The unit world-space tangent, along increasing texture coordinate `u`, a `vec3f`.
	Tangent,
	/// The unit world-space bitangent, along increasing texture coordinate `v`, a `vec3f`.
	Bitangent,
	/// The unit world-space direction from the shaded point toward the viewer, a `vec3f`.
	View,
	/// Texture coordinate set zero, a `vec2f`.
	TexCoord,
}

/// The `Stage` trait lets a program generator supply the values of a BEMX graph that depend on its renderer.
///
/// Implement it on the generator's per-program state, which can also record what the graph used, such as whether it
/// samples textures, while [`Lowering`] asks for each value.
pub trait Stage<'a> {
	/// Returns an `f32` BESL expression reading `geometry` at the shaded point.
	fn geometry(&mut self, geometry: Geometry) -> Node<'a>;

	/// Returns a `vec4f` BESL expression sampling the texture parameter named `texture` at texture coordinate set zero.
	///
	/// Return [`LowerError::UnknownTexture`] when the material declares no such texture.
	fn sample(&mut self, texture: &'a str) -> Result<Node<'a>, LowerError>;

	/// Returns a BESL expression reading the parameter named `name`, as the `f32` type of the same width as `type`.
	///
	/// Return [`LowerError::UnknownParameter`] when the material declares no such parameter with that type.
	fn parameter(&mut self, name: &'a str, r#type: Type) -> Result<Node<'a>, LowerError>;
}

/// The `Expression` struct carries one lowered value: its BESL syntax and the BEMX type it holds.
///
/// The syntax is a name, a literal, or a stage expression, so reading it again costs nothing.
#[derive(Clone, Debug)]
pub struct Expression<'a> {
	pub syntax: Node<'a>,
	pub r#type: Type,
}

impl<'a> Expression<'a> {
	pub fn new(syntax: Node<'a>, r#type: Type) -> Self {
		Expression { syntax, r#type }
	}

	/// Returns how many float components this value holds.
	pub fn width(&self) -> usize {
		self.r#type.width()
	}

	/// Returns one component of this value. A float is its own only component.
	pub fn component(&self, index: usize) -> Node<'a> {
		component(&self.syntax, index.min(self.width() - 1), self.width())
	}
}

/// The `Lowering` struct writes the BESL statements that compute the values a generator asks for.
///
/// Ask for values with [`Lowering::value`] and [`Lowering::tangent_normal`], then take the statements with
/// [`Lowering::finish`] and place them before the code that reads the returned expressions.
pub struct Lowering<'a, S> {
	graph: &'a Graph,
	stage: S,
	statements: Vec<Node<'a>>,
	/// The lowered value of each node, once it has been asked for.
	values: Vec<Option<Expression<'a>>>,
	/// The tangent-space normal of each normal-map node, once it has been asked for.
	tangent_normals: Vec<(NodeId, Expression<'a>)>,
	/// The raw sample of each texture, shared by every image node that samples it.
	samples: Vec<(&'a str, Expression<'a>)>,
	/// Which nodes the current request reaches, kept between requests so each one reuses the allocation.
	needed: Vec<bool>,
	locals: u32,
}

impl<'a, S: Stage<'a>> Lowering<'a, S> {
	pub fn new(graph: &'a Graph, stage: S) -> Self {
		Lowering {
			graph,
			stage,
			statements: Vec::with_capacity(graph.nodes().len()),
			values: vec![None; graph.nodes().len()],
			tangent_normals: Vec::new(),
			samples: Vec::new(),
			needed: vec![false; graph.nodes().len()],
			locals: 0,
		}
	}

	/// Returns the value of `node`, writing the statements that compute it and everything it reads.
	///
	/// The surface node produces no value; asking for it reports [`LowerError::NotAValue`].
	pub fn value(&mut self, node: NodeId) -> Result<Expression<'a>, LowerError> {
		if self.graph.r#type(node).is_none() {
			return Err(LowerError::NotAValue { node });
		}

		self.lower_reaching([node])?;

		Ok(self.values[node.index()].clone().expect("The node was lowered above."))
	}

	/// Returns the tangent-space normal a normal-map node decodes, or `None` for any other node.
	///
	/// A renderer that shades in the tangent frame reads this instead of [`Lowering::value`], which turns the same
	/// normal into world space only to have the renderer turn it back. The result is not normalized.
	pub fn tangent_normal(&mut self, node: NodeId) -> Result<Option<Expression<'a>>, LowerError> {
		let GraphNode::NormalMap { value, scale } = *self.graph.node(node) else {
			return Ok(None);
		};

		self.lower_reaching([value, scale])?;

		self.tangent(node, value, scale).map(Some)
	}

	/// Returns the stage, so a generator can read what it recorded or ask it for a value.
	pub fn stage(&mut self) -> &mut S {
		&mut self.stage
	}

	/// Returns a value whose components can be read more than once, binding it to a local first when it is anything
	/// but a name.
	pub fn addressable(&mut self, value: Expression<'a>) -> Expression<'a> {
		let readable = matches!(value.syntax.node(), Nodes::Expression(Expressions::Member { .. }));

		// A one-component value is never read through a component, so repeating it costs nothing either.
		if readable || value.width() == 1 {
			return value;
		}

		self.bind(value.r#type, value.syntax)
	}

	/// Moves the statements written since the last call to the end of `statements`, in the order they run.
	///
	/// A generator that places each value's statements right before the code that reads it, so locals live no longer
	/// than needed, calls this after each value it asks for.
	pub fn drain_statements_into(&mut self, statements: &mut Vec<Node<'a>>) {
		statements.append(&mut self.statements);
	}

	/// Returns the statements not taken yet, in the order they run, and the stage.
	pub fn finish(self) -> (Vec<Node<'a>>, S) {
		(self.statements, self.stage)
	}

	/// Lowers every node that `roots` reach, in graph order, skipping nodes already lowered.
	///
	/// Graph order puts each node after the nodes it reads, so a single pass in that order never meets an input that
	/// is not lowered yet, and no recursion is needed however deep the graph is.
	fn lower_reaching(&mut self, roots: impl IntoIterator<Item = NodeId>) -> Result<(), LowerError> {
		// Taken out of `self` so lowering can borrow `self` while the marks are read.
		let mut needed = std::mem::take(&mut self.needed);
		let mut last = 0;

		for root in roots {
			needed[root.index()] = true;
			last = last.max(root.index());
		}

		// Inputs always come earlier, so walking backward marks every node a root reaches.
		for index in (0..=last).rev() {
			if needed[index] && self.values[index].is_none() {
				self.graph.nodes()[index].visit_inputs(|input| needed[input.index()] = true);
			}
		}

		let mut result = Ok(());

		for index in 0..=last {
			if needed[index] && self.values[index].is_none() {
				match self.lower(NodeId::from_index(index)) {
					Ok(value) => self.values[index] = Some(value),
					Err(error) => {
						result = Err(error);
						break;
					}
				}
			}
		}

		// Clear the marks for the next request; nothing past `last` was marked.
		needed[..=last].fill(false);
		self.needed = needed;

		result
	}

	/// Returns the already lowered value of an input.
	fn input(&self, node: NodeId) -> Expression<'a> {
		self.values[node.index()]
			.clone()
			.expect("Inputs are lowered before the nodes that read them.")
	}

	/// Lowers one node whose inputs are all lowered.
	// One kind per arm keeps the whole node set readable in a single place.
	#[allow(clippy::too_many_lines)]
	fn lower(&mut self, id: NodeId) -> Result<Expression<'a>, LowerError> {
		let graph = self.graph;
		let result = graph
			.r#type(id)
			.expect("Only value nodes are lowered: `value` rejects the surface, and no node reads it.");

		Ok(match graph.node(id) {
			GraphNode::Constant(value) => Expression::new(constant(value), result),
			GraphNode::Parameter { name, r#type } => Expression::new(self.stage.parameter(name, *r#type)?, result),
			GraphNode::Position => Expression::new(self.stage.geometry(Geometry::Position), result),
			GraphNode::Normal => Expression::new(self.stage.geometry(Geometry::Normal), result),
			GraphNode::Tangent => Expression::new(self.stage.geometry(Geometry::Tangent), result),
			GraphNode::Bitangent => Expression::new(self.stage.geometry(Geometry::Bitangent), result),
			GraphNode::ViewDirection => {
				// BEMX points the view direction from the viewer at the surface, as MaterialX does; stages carry the
				// opposite.
				let view = self.stage.geometry(Geometry::View);

				self.bind(result, Node::operator("-", splat_literal(0.0, 3), view))
			}
			GraphNode::TexCoord { index } => {
				if *index != 0 {
					return Err(LowerError::UnsupportedTexCoord { node: id, index: *index });
				}

				Expression::new(self.stage.geometry(Geometry::TexCoord), result)
			}
			GraphNode::Image { texture, texcoord, .. } => {
				// Stages sample with the mesh's own texture coordinates, so a transformed set cannot be honoured.
				if let Some(texcoord) = texcoord
					&& !matches!(graph.node(*texcoord), GraphNode::TexCoord { index: 0 })
				{
					return Err(LowerError::UnsupportedTextureCoordinates { node: id });
				}

				let sample = self.sample(texture)?;

				self.convert(sample, result)
			}
			GraphNode::Unary { operator, value } => {
				let value = self.input(*value);

				match operator {
					UnaryOperator::Normalize => self.normalize(value)?,
					UnaryOperator::Logarithm => self.logarithm(value),
					operator => self.componentwise(result, vec![value], |lanes| scalar_function(*operator, &lanes[0])),
				}
			}
			GraphNode::Binary { operator, left, right } => {
				let (left, right) = (self.input(*left), self.input(*right));

				match operator {
					// BESL applies these across components and broadcasts a float operand, so they stay whole.
					BinaryOperator::Add | BinaryOperator::Subtract | BinaryOperator::Multiply | BinaryOperator::Divide => {
						let token = match operator {
							BinaryOperator::Add => "+",
							BinaryOperator::Subtract => "-",
							BinaryOperator::Multiply => "*",
							_ => "/",
						};

						self.bind(result, Node::operator(token, left.syntax, right.syntax))
					}
					// The floored remainder keeps the sign of the divisor.
					BinaryOperator::Modulo => self.componentwise(result, vec![left, right], |lanes| {
						let [left, right]: [Node<'a>; 2] = lanes.try_into().expect("Modulo has two operands.");
						let whole = Node::call("floor", vec![Node::operator("/", left.clone(), right.clone())]);

						Node::operator("-", left, Node::operator("*", right, whole))
					}),
					operator => {
						let intrinsic = match operator {
							BinaryOperator::Power => "pow",
							BinaryOperator::Minimum => "min",
							BinaryOperator::Maximum => "max",
							_ => "atan2",
						};

						self.componentwise(result, vec![left, right], |lanes| Node::call(intrinsic, lanes))
					}
				}
			}
			GraphNode::Clamp { value, low, high } => {
				let operands = vec![self.input(*value), self.input(*low), self.input(*high)];

				self.componentwise(result, operands, |lanes| Node::call("clamp", lanes))
			}
			GraphNode::Smoothstep { value, low, high } => {
				let operands = vec![self.input(*value), self.input(*low), self.input(*high)];

				// A smooth step takes its edges first and the value last.
				self.componentwise(result, operands, |mut lanes| {
					lanes.rotate_left(1);
					Node::call("smoothstep", lanes)
				})
			}
			GraphNode::Mix {
				background,
				foreground,
				factor,
			} => {
				let (background, foreground, factor) = (self.input(*background), self.input(*foreground), self.input(*factor));

				self.bind(result, blend(background.syntax, foreground.syntax, factor.syntax))
			}
			GraphNode::Remap {
				value,
				input_low,
				input_high,
				output_low,
				output_high,
			} => {
				let value = self.input(*value);
				let (input_low, input_high) = (self.input(*input_low), self.input(*input_high));
				let (output_low, output_high) = (self.input(*output_low), self.input(*output_high));

				let position = Node::operator(
					"/",
					Node::operator("-", value.syntax, input_low.syntax.clone()),
					Node::operator("-", input_high.syntax, input_low.syntax),
				);
				let span = Node::operator("-", output_high.syntax, output_low.syntax.clone());

				self.bind(
					result,
					Node::operator("+", output_low.syntax, Node::operator("*", position, span)),
				)
			}
			GraphNode::DotProduct { left, right } => {
				let (left, right) = (self.input(*left), self.input(*right));

				let product = if left.width() == 1 {
					Node::operator("*", left.syntax, right.syntax)
				} else {
					Node::call("dot", vec![left.syntax, right.syntax])
				};

				self.bind(result, product)
			}
			GraphNode::CrossProduct { left, right } => {
				let (left, right) = (self.input(*left), self.input(*right));

				self.bind(result, Node::call("cross", vec![left.syntax, right.syntax]))
			}
			GraphNode::Magnitude { value } => {
				let value = self.input(*value);

				self.magnitude(value)
			}
			GraphNode::Extract { value, index } => {
				let value = self.addressable(self.input(*value));

				self.bind(result, value.component(usize::from(*index)))
			}
			GraphNode::Swizzle { value, channels } => {
				let value = self.addressable(self.input(*value));
				let components = channels
					.iter()
					.map(|channel| match channel.component() {
						Some(index) => value.component(index),
						None => literal(f32::from(*channel == Channel::One)),
					})
					.collect();

				self.bind(result, construct(components))
			}
			GraphNode::Combine { values } => {
				let mut components = Vec::with_capacity(result.width());

				for value in values {
					let part = self.addressable(self.input(*value));

					components.extend((0..part.width()).map(|lane| part.component(lane)));
				}

				self.bind(result, construct(components))
			}
			GraphNode::Convert { value, .. } => {
				let value = self.input(*value);

				self.convert(value, result)
			}
			GraphNode::Luminance { value, weights } => {
				let value = self.input(*value);

				self.luminance(value, *weights)
			}
			GraphNode::Invert { value, amount } => {
				let (value, amount) = (self.input(*value), self.input(*amount));

				self.bind(result, Node::operator("-", amount.syntax, value.syntax))
			}
			GraphNode::Premultiply { value } => {
				let value = self.input(*value);

				self.premultiply(value, true)
			}
			GraphNode::Unpremultiply { value } => {
				let value = self.input(*value);

				self.premultiply(value, false)
			}
			GraphNode::Compare {
				comparison,
				left,
				right,
				then,
				otherwise,
			} => {
				let gate = self.comparison(*comparison, self.input(*left), self.input(*right));
				let (then, otherwise) = (self.input(*then), self.input(*otherwise));

				self.bind(result, blend(otherwise.syntax, then.syntax, gate.syntax))
			}
			GraphNode::Switch { which, values } => {
				let which = self.addressable(self.input(*which));
				let mut terms = Vec::with_capacity(values.len());

				// Every value is weighted by whether the selector picks it, so no branch is needed.
				for (index, value) in values.iter().enumerate() {
					let value = self.input(*value);
					// A value is picked when the selector reaches its position but not the next one.
					let reaches = |edge: f32| Node::call("step", vec![literal(edge), which.syntax.clone()]);
					let gate = self.bind(
						Type::Float,
						Node::operator("-", reaches(index as f32 - 0.5), reaches(index as f32 + 0.5)),
					);

					terms.push(Node::operator("*", value.syntax, gate.syntax));
				}

				let total = terms
					.into_iter()
					.reduce(|total, term| Node::operator("+", total, term))
					.expect("A switch always has a value. The builder checks it.");

				self.bind(result, total)
			}
			GraphNode::NormalMap { value, scale } => {
				let tangent = self.tangent(id, *value, *scale)?;
				let axes = [Geometry::Tangent, Geometry::Bitangent, Geometry::Normal];
				let world = axes
					.into_iter()
					.enumerate()
					.map(|(index, axis)| Node::operator("*", self.stage.geometry(axis), tangent.component(index)))
					.reduce(|total, term| Node::operator("+", total, term))
					.expect("A frame has three axes.");

				self.bind(result, Node::call("normalize", vec![world]))
			}
			GraphNode::Surface(_) => unreachable!("The surface produces no value, so it is never lowered."),
		})
	}

	/// Declares a local holding `syntax` and returns an expression that reads it back.
	fn bind(&mut self, r#type: Type, syntax: Node<'a>) -> Expression<'a> {
		let name = format!("bemx_{}", self.locals);

		self.locals += 1;
		self.statements
			.push(Node::let_assignment(name.clone(), type_name(r#type), syntax));

		Expression::new(Node::member_expression(name), r#type)
	}

	/// Returns the raw `vec4f` sample of a texture, sampling it on first use.
	fn sample(&mut self, texture: &'a str) -> Result<Expression<'a>, LowerError> {
		if let Some((_, sample)) = self.samples.iter().find(|(sampled, _)| *sampled == texture) {
			return Ok(sample.clone());
		}

		let syntax = self.stage.sample(texture)?;
		let sample = self.bind(Type::Vector4, syntax);

		self.samples.push((texture, sample.clone()));

		Ok(sample)
	}

	/// Returns the tangent-space normal a normal-map node decodes, decoding it on first use.
	fn tangent(&mut self, node: NodeId, value: NodeId, scale: NodeId) -> Result<Expression<'a>, LowerError> {
		if let Some((_, tangent)) = self.tangent_normals.iter().find(|(decoded, _)| *decoded == node) {
			return Ok(tangent.clone());
		}

		let encoded = self.addressable(self.input(value));
		let scale = self.input(scale);

		// The map stores a unit vector shifted into the zero-to-one range a colour texture holds. The third component
		// is rebuilt from the first two, so two-channel maps decode the same as three-channel ones.
		let decode =
			|index: usize| Node::operator("-", Node::operator("*", encoded.component(index), literal(2.0)), literal(1.0));
		let plane = self.bind(Type::Vector2, construct(vec![decode(0), decode(1)]));
		let height = Node::call(
			"sqrt",
			vec![Node::call(
				"max",
				vec![
					literal(0.0),
					Node::operator(
						"-",
						literal(1.0),
						Node::call("dot", vec![plane.syntax.clone(), plane.syntax.clone()]),
					),
				],
			)],
		);

		// Scaling the tangent-plane components alone tilts the normal without changing which way it faces.
		let tangent = self.bind(
			Type::Vector3,
			construct(vec![
				Node::operator("*", plane.component(0), scale.syntax.clone()),
				Node::operator("*", plane.component(1), scale.syntax),
				height,
			]),
		);

		self.tangent_normals.push((node, tangent.clone()));

		Ok(tangent)
	}

	/// Applies a scalar operation to every component of a value.
	///
	/// BESL registers most math intrinsics for `f32` alone, so a node that works on vectors is written one lane at a
	/// time. Operands narrower than the result repeat their only component.
	fn componentwise(
		&mut self,
		result: Type,
		operands: Vec<Expression<'a>>,
		build: impl Fn(Vec<Node<'a>>) -> Node<'a>,
	) -> Expression<'a> {
		// Component access reads its operand once per lane, so every operand has to be a name first.
		let operands = operands
			.into_iter()
			.map(|operand| self.addressable(operand))
			.collect::<Vec<_>>();

		let components = (0..result.width())
			.map(|index| build(operands.iter().map(|operand| operand.component(index)).collect()))
			.collect();

		self.bind(result, construct(components))
	}

	/// Writes a natural logarithm.
	///
	/// BESL registers `log2` for three-component vectors, so components travel through it in groups of three and are
	/// scaled to a natural logarithm afterwards.
	fn logarithm(&mut self, value: Expression<'a>) -> Expression<'a> {
		let width = value.width();
		let value = self.addressable(value);
		let mut components = Vec::with_capacity(width);

		for first in (0..width).step_by(3) {
			let lanes = (0..3).map(|offset| value.component(first + offset)).collect();
			let logarithms = self.bind(Type::Vector3, Node::call("log2", vec![Node::call("vec3f", lanes)]));

			for offset in 0..(width - first).min(3) {
				components.push(Node::operator(
					"*",
					logarithms.component(offset),
					literal(std::f32::consts::LN_2),
				));
			}
		}

		self.bind(value.r#type, construct(components))
	}

	/// Writes a value's length.
	fn magnitude(&mut self, value: Expression<'a>) -> Expression<'a> {
		let length = match value.width() {
			1 => Node::call("abs", vec![value.syntax]),
			// BESL registers `length` for three and four components only.
			2 => Node::call("sqrt", vec![Node::call("dot", vec![value.syntax.clone(), value.syntax])]),
			_ => Node::call("length", vec![value.syntax]),
		};

		self.bind(Type::Float, length)
	}

	/// Writes a value scaled to unit length.
	fn normalize(&mut self, value: Expression<'a>) -> Result<Expression<'a>, LowerError> {
		// BESL registers `normalize` for vectors only, so a float divides by its own magnitude instead.
		if value.width() >= 2 {
			return Ok(self.bind(value.r#type, Node::call("normalize", vec![value.syntax])));
		}

		let value = self.addressable(value);
		let length = self.magnitude(value.clone());

		Ok(self.bind(value.r#type, Node::operator("/", value.syntax, length.syntax)))
	}

	/// Writes the zero-or-one gate of a comparison.
	fn comparison(&mut self, comparison: Comparison, left: Expression<'a>, right: Expression<'a>) -> Expression<'a> {
		let (left, right) = (left.syntax, right.syntax);
		// A step is one when its second argument reaches its first, which spells every comparison here.
		let not_below = |edge: &Node<'a>, value: &Node<'a>| Node::call("step", vec![edge.clone(), value.clone()]);

		let gate = match comparison {
			Comparison::GreaterOrEqual => not_below(&right, &left),
			Comparison::Equal => Node::operator("*", not_below(&right, &left), not_below(&left, &right)),
			Comparison::Greater => Node::operator("-", literal(1.0), not_below(&left, &right)),
		};

		self.bind(Type::Float, gate)
	}

	/// Writes a colour's luminance across every one of its colour channels, keeping its alpha.
	fn luminance(&mut self, value: Expression<'a>, weights: [f32; 3]) -> Expression<'a> {
		let value = self.addressable(value);
		let total = (0..3)
			.map(|index| Node::operator("*", value.component(index), literal(weights[index])))
			.reduce(|total, term| Node::operator("+", total, term))
			.expect("A colour has three channels.");
		let total = self.bind(Type::Float, total);

		let mut components = vec![total.syntax.clone(), total.syntax.clone(), total.syntax];

		if value.width() == 4 {
			components.push(value.component(3));
		}

		self.bind(value.r#type, construct(components))
	}

	/// Writes a colour with its alpha multiplied into, or divided out of, its colour channels.
	fn premultiply(&mut self, value: Expression<'a>, multiply: bool) -> Expression<'a> {
		let value = self.addressable(value);
		let alpha = value.component(3);

		let mut components: Vec<Node<'a>> = (0..3)
			.map(|index| {
				let channel = value.component(index);

				if multiply {
					Node::operator("*", channel, alpha.clone())
				} else {
					// A transparent texel carries no colour, so keep the divisor away from zero.
					Node::operator("/", channel, Node::call("max", vec![alpha.clone(), literal(MINIMUM_ALPHA)]))
				}
			})
			.collect();

		components.push(alpha);

		self.bind(Type::Vector4, construct(components))
	}

	/// Writes a value converted to another width.
	///
	/// A float fills every component, a narrower vector gains ones, which is how MaterialX opens a colour to full
	/// opacity, and a wider one drops the components the target has no room for.
	fn convert(&mut self, value: Expression<'a>, result: Type) -> Expression<'a> {
		let (source, target) = (value.width(), result.width());

		if source == target {
			return Expression::new(value.syntax, result);
		}

		let value = self.addressable(value);
		let components = (0..target)
			.map(|index| match index {
				_ if source == 1 => value.syntax.clone(),
				index if index < source => value.component(index),
				_ => literal(1.0),
			})
			.collect();

		self.bind(result, construct(components))
	}
}

/// Writes one of the scalar functions that map straight onto a lane of a value.
fn scalar_function<'a>(operator: UnaryOperator, value: &Node<'a>) -> Node<'a> {
	let zero = || literal(0.0);
	let step = |edge: Node<'a>, value: Node<'a>| Node::call("step", vec![edge, value]);

	match operator {
		UnaryOperator::Absolute => Node::call("abs", vec![value.clone()]),
		UnaryOperator::Floor => Node::call("floor", vec![value.clone()]),
		// BESL has no ceiling, and flooring the negated value rounds the other way.
		UnaryOperator::Ceil => Node::operator(
			"-",
			zero(),
			Node::call("floor", vec![Node::operator("-", zero(), value.clone())]),
		),
		// BESL rounds only some types, so round through the floor instead.
		UnaryOperator::Round => Node::call("floor", vec![Node::operator("+", value.clone(), literal(0.5))]),
		// A step either side of zero gives 1, -1 and 0 without a comparison.
		UnaryOperator::Sign => Node::operator("-", step(zero(), value.clone()), step(value.clone(), zero())),
		UnaryOperator::SquareRoot => Node::call("sqrt", vec![value.clone()]),
		UnaryOperator::Exponential => Node::call("exp", vec![value.clone()]),
		UnaryOperator::Sine => Node::call("sin", vec![value.clone()]),
		UnaryOperator::Cosine => Node::call("cos", vec![value.clone()]),
		UnaryOperator::Tangent => Node::call("tan", vec![value.clone()]),
		UnaryOperator::ArcSine => Node::call("asin", vec![value.clone()]),
		// BESL has no inverse cosine, and the two inverse functions are complements.
		UnaryOperator::ArcCosine => Node::operator(
			"-",
			literal(std::f32::consts::FRAC_PI_2),
			Node::call("asin", vec![value.clone()]),
		),
		UnaryOperator::Logarithm | UnaryOperator::Normalize => {
			unreachable!("Logarithms and normalization are not lane functions; `lower` writes them whole.")
		}
	}
}

/// Writes a linear blend between two values.
fn blend<'a>(from: Node<'a>, to: Node<'a>, factor: Node<'a>) -> Node<'a> {
	Node::operator("+", from.clone(), Node::operator("*", Node::operator("-", to, from), factor))
}

/// Writes a constant as a BESL expression.
pub fn constant<'a>(value: &Value) -> Node<'a> {
	match value {
		// The common case needs no component list.
		Value::Float(value) => literal(*value),
		value => construct(value.components().iter().copied().map(literal).collect()),
	}
}

/// Returns the BESL `f32` type that holds a value of `type`, as lowered values and generator declarations spell it.
pub fn type_name(r#type: Type) -> &'static str {
	match r#type {
		Type::Float => "f32",
		Type::Vector2 => "vec2f",
		Type::Vector3 => "vec3f",
		Type::Vector4 => "vec4f",
	}
}

const COMPONENT_NAMES: [&str; 4] = ["x", "y", "z", "w"];

/// Reads one float component out of a value of the given width.
///
/// A one-component value is its own only component, so it is returned untouched rather than accessed through `.x`,
/// which BESL does not define for `f32`.
fn component<'a>(value: &Node<'a>, index: usize, width: usize) -> Node<'a> {
	if width <= 1 {
		return value.clone();
	}

	Node::accessor(value.clone(), Node::member_expression(COMPONENT_NAMES[index]))
}

/// Assembles float components back into a value of their combined width.
fn construct<'a>(mut components: Vec<Node<'a>>) -> Node<'a> {
	if components.len() == 1 {
		return components.remove(0);
	}

	let r#type = Type::from_width(components.len()).expect("Lowered values hold one to four components.");

	Node::call(type_name(r#type), components)
}

/// Writes the digits of a float as a BESL literal.
///
/// BESL literals hold digits and one optional point, so this expands the exponent that Rust's shortest representation
/// reaches for on very small and very large values. It never writes a sign; [`literal`] subtracts from zero instead.
fn digits(value: f32) -> String {
	// A material has no meaningful infinite or undefined value, so fold both into a finite one.
	let value = if value.is_finite() { value.abs() } else { 0.0 };

	let mut text = format!("{value}");

	if text.contains(['e', 'E']) {
		// Nine decimals cover the whole f32 subnormal range without reaching for an exponent.
		text = format!("{value:.9}");
	}

	if !text.contains('.') {
		text.push_str(".0");
	}

	text
}

/// Writes a float as a BESL expression, subtracting from zero when it is negative.
pub fn literal<'a>(value: f32) -> Node<'a> {
	let digits = Node::literal_expression(digits(value));

	if value.is_sign_negative() && value != 0.0 {
		return Node::operator("-", Node::literal_expression("0.0"), digits);
	}

	digits
}

/// Writes a constant of the given width, repeating one number across every lane.
fn splat_literal<'a>(value: f32, width: usize) -> Node<'a> {
	construct((0..width).map(|_| literal(value)).collect())
}

/// The `LowerError` enum explains why a graph's value cannot be written for a stage.
///
/// A graph is always well typed, so these only describe what a stage cannot supply.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum LowerError {
	/// The surface node was asked for a value, which it does not produce.
	NotAValue { node: NodeId },
	/// A texture coordinate set other than zero was read.
	UnsupportedTexCoord { node: NodeId, index: u32 },
	/// An image samples with coordinates other than the mesh's own.
	UnsupportedTextureCoordinates { node: NodeId },
	/// A parameter node names a parameter the material does not declare with that type.
	UnknownParameter { name: String, r#type: Type },
	/// An image node names a texture the material does not declare.
	UnknownTexture { name: String },
}

impl Display for LowerError {
	fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
		let docs = online_docs_url(BEMX_DOCS_PATH);

		match self {
			LowerError::NotAValue { node } => write!(
				f,
				"BEMX node #{} produces no value. The most likely cause is a generator asking for the surface node's value instead of its inputs'.",
				node.index()
			),
			LowerError::UnsupportedTexCoord { node, index } => write!(
				f,
				"BEMX node #{} reads texture coordinate set {index}. The most likely cause is a material written for meshes with several coordinate sets; the material stage carries set zero only. See {docs}.",
				node.index()
			),
			LowerError::UnsupportedTextureCoordinates { node } => write!(
				f,
				"BEMX image node #{} samples with transformed coordinates. The most likely cause is a 'texcoord' input driven by anything but a 'texcoord' node; the material stage samples with the mesh's own coordinates only. See {docs}.",
				node.index()
			),
			LowerError::UnknownParameter { name, r#type } => write!(
				f,
				"Material parameter '{name}' of type {} is not declared. The most likely cause is a BEMX parameter node whose name or type differs from the material's variables. See {docs}.",
				r#type.name()
			),
			LowerError::UnknownTexture { name } => write!(
				f,
				"Material texture '{name}' is not declared. The most likely cause is a BEMX image node whose texture is missing from the material's Texture2D variables. See {docs}."
			),
		}
	}
}

impl std::error::Error for LowerError {}

#[cfg(test)]
mod tests;
