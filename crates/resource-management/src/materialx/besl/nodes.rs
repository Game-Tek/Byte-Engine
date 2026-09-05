//! Turns one MaterialX node category into the BESL expression that computes it.

use besl::parser::Node;

use super::Texture;
use super::error::LowerError;
use super::lowering::{Lowering, geometry};
use super::syntax::{self, Expression};
use crate::materialx::{DataType, Input, Node as Instance, NodeId, PortIndex, Source, Value};

/// The ratio between a base-two and a natural logarithm.
const LOGARITHM_OF_TWO: f32 = std::f32::consts::LN_2;

/// The luminance weights the MaterialX standard library uses when a node writes none.
const LUMINANCE_WEIGHTS: [f32; 3] = [0.272_228_7, 0.674_081_8, 0.053_589_5];

/// The smallest alpha `unpremult` divides by, so a fully transparent texel does not divide by zero.
const MINIMUM_ALPHA: f32 = 1.0e-6;

/// The count of numbered inputs a MaterialX switch node declares.
const MAXIMUM_SWITCH_INPUTS: usize = 10;

/// The geometric properties the material stage carries a texture coordinate under.
const TEXTURE_COORDINATES: [&str; 3] = ["UV0", "st", "uv"];

/// Lowers one output of one node.
///
/// A category this module knows is lowered directly, because that is this renderer's implementation
/// of it. Anything else falls back to the node graph that implements the node's declaration, which is
/// how a document extends the standard library with graphs of its own.
pub(super) fn lower<'a>(
	lowering: &mut Lowering<'a, '_>,
	frame: usize,
	node: NodeId,
	output: PortIndex,
) -> Result<Expression<'a>, LowerError> {
	let instance = lowering.dag.node(node);

	if let Some(value) = builtin(lowering, frame, node, instance, output)? {
		return Ok(value);
	}

	if let Some(graph) = lowering.implementation(instance.declaration) {
		let expansion = lowering.expansion(frame, node, graph)?;

		return lowering.graph_result(expansion, graph, output);
	}

	Err(LowerError::UnsupportedNode {
		node: instance.name.to_string(),
		category: instance.category.to_string(),
	})
}

/// Lowers the node categories this renderer evaluates itself, or reports that it does not know one.
// One category per arm keeps the whole supported surface readable in a single place.
#[allow(clippy::too_many_lines)]
fn builtin<'a>(
	lowering: &mut Lowering<'a, '_>,
	frame: usize,
	node: NodeId,
	instance: &Instance<'a>,
	output: PortIndex,
) -> Result<Option<Expression<'a>>, LowerError> {
	let name = instance.name;
	let category = instance.category;
	let result = instance
		.outputs
		.get(output.index())
		.map_or(instance.data_type, |port| port.data_type);

	let value = match category {
		// Values and structure.
		"constant" => operand(lowering, frame, node, "value", 0.0, result)?,
		"dot" => operand(lowering, frame, node, "in", 0.0, result)?,
		"convert" => {
			let value = operand(lowering, frame, node, "in", 0.0, result)?;

			convert(lowering, name, value, result)?
		}
		"combine2" | "combine3" | "combine4" => combine(lowering, frame, node, name, result)?,
		"separate2" | "separate3" | "separate4" => {
			let value = operand(lowering, frame, node, "in", 0.0, DataType::Vector4)?;

			separate(lowering, name, value, instance, output, result)?
		}
		"extract" => {
			let value = operand(lowering, frame, node, "in", 0.0, DataType::Vector4)?;
			let index = integer_parameter(lowering, frame, node, "index").unwrap_or(0).max(0) as usize;
			let width = value.width();
			let value = lowering.addressable(name, value)?;

			lowering.bind(name, result, syntax::component(&value.syntax, index.min(width - 1), width))?
		}
		"swizzle" => {
			let value = operand(lowering, frame, node, "in", 0.0, DataType::Vector4)?;
			let channels = string_parameter(lowering, frame, node, "channels").unwrap_or("x");

			swizzle(lowering, name, value, channels, result)?
		}

		// Geometry.
		"position" => geometry("Pworld", name)?,
		"normal" => geometry("Nworld", name)?,
		"tangent" => geometry("Tworld", name)?,
		"bitangent" => geometry("Bworld", name)?,
		"viewdirection" => {
			// MaterialX points a view direction from the viewer at the surface; the stage carries the opposite.
			let view = geometry("Vworld", name)?;

			lowering.bind(
				name,
				DataType::Vector3,
				Node::operator("-", syntax::splat_literal(0.0, 3), view.syntax),
			)?
		}
		"texcoord" => {
			let index = integer_parameter(lowering, frame, node, "index").unwrap_or(0);

			if index != 0 {
				return Err(LowerError::UnknownGeometricProperty {
					node: name.to_string(),
					property: format!("UV{index}"),
				});
			}

			geometry("UV0", name)?
		}
		"geompropvalue" => geometry(string_parameter(lowering, frame, node, "geomprop").unwrap_or(""), name)?,

		// Textures.
		"image" => image(lowering, frame, node, instance, result)?,

		// Arithmetic. BESL applies these across components and broadcasts a scalar operand, so they stay whole.
		"add" | "subtract" | "multiply" | "divide" => {
			// A missing second operand leaves the first one alone, which is a different number per operator.
			let (operator, identity) = match category {
				"add" => ("+", 0.0),
				"subtract" => ("-", 0.0),
				"multiply" => ("*", 1.0),
				_ => ("/", 1.0),
			};
			let left = operand(lowering, frame, node, "in1", 0.0, result)?;
			let right = operand(lowering, frame, node, "in2", identity, result)?;

			lowering.bind(name, result, Node::operator(operator, left.syntax, right.syntax))?
		}
		"modulo" => {
			let left = operand(lowering, frame, node, "in1", 0.0, result)?;
			let right = operand(lowering, frame, node, "in2", 1.0, result)?;

			// MaterialX takes the floored remainder, which keeps the sign of the divisor.
			componentwise(lowering, name, result, vec![left, right], |lanes| {
				let quotient = Node::operator("/", lanes[0].clone(), lanes[1].clone());
				let whole = Node::call("floor", vec![quotient]);

				Node::operator("-", lanes[0].clone(), Node::operator("*", lanes[1].clone(), whole))
			})?
		}
		// The exponent and the second bound leave the first operand alone when nothing writes them.
		"power" | "min" | "max" | "atan2" => {
			let identity = f32::from(category == "power");
			let left = operand(lowering, frame, node, "in1", 0.0, result)?;
			let right = operand(lowering, frame, node, "in2", identity, result)?;
			let intrinsic = if category == "power" { "pow" } else { category };

			componentwise(lowering, name, result, vec![left, right], |lanes| {
				Node::call(intrinsic, lanes.to_vec())
			})?
		}

		// Component-wise functions.
		"absval" | "floor" | "ceil" | "round" | "sign" | "sqrt" | "exp" | "sin" | "cos" | "tan" | "asin" | "acos" => {
			let value = operand(lowering, frame, node, "in", 0.0, result)?;

			componentwise(lowering, name, result, vec![value], |lanes| {
				scalar_function(category, &lanes[0])
			})?
		}
		"ln" => {
			let value = operand(lowering, frame, node, "in", 1.0, result)?;

			logarithm(lowering, name, value, result)?
		}
		"clamp" | "smoothstep" => {
			let value = operand(lowering, frame, node, "in", 0.0, result)?;
			let low = operand(lowering, frame, node, "low", 0.0, DataType::Float)?;
			let high = operand(lowering, frame, node, "high", 1.0, DataType::Float)?;

			componentwise(lowering, name, result, vec![value, low, high], |lanes| {
				// A clamp takes the value first and the bounds after; a smooth step takes them the other way.
				match category {
					"clamp" => Node::call("clamp", lanes.to_vec()),
					_ => Node::call("smoothstep", vec![lanes[1].clone(), lanes[2].clone(), lanes[0].clone()]),
				}
			})?
		}

		// Vector functions.
		"normalize" => {
			let value = operand(lowering, frame, node, "in", 0.0, result)?;

			normalize(lowering, name, value, result)?
		}
		"magnitude" => {
			let value = operand(lowering, frame, node, "in", 0.0, DataType::Vector3)?;

			magnitude(lowering, name, value)?
		}
		"dotproduct" => {
			let left = operand(lowering, frame, node, "in1", 0.0, DataType::Vector3)?;
			let right = operand(lowering, frame, node, "in2", 0.0, DataType::Vector3)?;

			let product = if left.width() == 1 {
				Node::operator("*", left.syntax, right.syntax)
			} else {
				Node::call("dot", vec![left.syntax, right.syntax])
			};

			lowering.bind(name, DataType::Float, product)?
		}
		"crossproduct" => {
			let left = operand(lowering, frame, node, "in1", 0.0, DataType::Vector3)?;
			let right = operand(lowering, frame, node, "in2", 0.0, DataType::Vector3)?;

			lowering.bind(name, result, Node::call("cross", vec![left.syntax, right.syntax]))?
		}

		// Blends and adjustments.
		"mix" => {
			let foreground = operand(lowering, frame, node, "fg", 0.0, result)?;
			let background = operand(lowering, frame, node, "bg", 0.0, result)?;
			let factor = operand(lowering, frame, node, "mix", 0.0, DataType::Float)?;

			lowering.bind(name, result, blend(background.syntax, foreground.syntax, factor.syntax))?
		}
		"remap" => remap(lowering, frame, node, name, result)?,
		"invert" => {
			let value = operand(lowering, frame, node, "in", 0.0, result)?;
			let amount = operand(lowering, frame, node, "amount", 1.0, result)?;

			lowering.bind(name, result, Node::operator("-", amount.syntax, value.syntax))?
		}
		"luminance" => {
			let value = operand(lowering, frame, node, "in", 0.0, result)?;

			luminance(lowering, frame, node, name, value, result)?
		}
		"premult" | "unpremult" => {
			let value = operand(lowering, frame, node, "in", 0.0, result)?;

			premultiply(lowering, name, value, result, category == "premult")?
		}
		"ifgreater" | "ifgreatereq" | "ifequal" => {
			let gate = comparison(lowering, frame, node, name, category)?;
			let matched = operand(lowering, frame, node, "in1", 0.0, result)?;
			let other = operand(lowering, frame, node, "in2", 0.0, result)?;

			lowering.bind(name, result, blend(other.syntax, matched.syntax, gate.syntax))?
		}
		"switch" => switch(lowering, frame, node, name, result)?,

		// Shading.
		"normalmap" => normal_map(lowering, frame, node, name)?,

		_ => return Ok(None),
	};

	Ok(Some(value))
}

/// Reads one input of a node, or a constant of the given type when nothing writes it.
pub(super) fn operand<'a>(
	lowering: &mut Lowering<'a, '_>,
	frame: usize,
	node: NodeId,
	name: &str,
	default: f32,
	data_type: DataType<'a>,
) -> Result<Expression<'a>, LowerError> {
	if let Some(value) = lowering.input(frame, node, name)? {
		return Ok(value);
	}

	let width = syntax::components(data_type).unwrap_or(1);

	Ok(Expression::new(
		syntax::splat_literal(default, width),
		syntax::float_data_type(width),
	))
}

/// Returns one input's constant as an integer.
fn integer_parameter(lowering: &Lowering<'_, '_>, frame: usize, node: NodeId, name: &str) -> Option<i32> {
	match lowering.parameter(frame, node, name)? {
		Value::Integer(value) => Some(*value),
		Value::Float(value) => Some(*value as i32),
		_ => None,
	}
}

/// Returns one input's constant as text.
fn string_parameter<'a>(lowering: &Lowering<'a, '_>, frame: usize, node: NodeId, name: &str) -> Option<&'a str> {
	match lowering.parameter(frame, node, name)? {
		Value::String(value) | Value::Filename(value) | Value::GeomName(value) | Value::Opaque(value) => Some(value),
		_ => None,
	}
}

/// Applies a scalar operation to every component of a value.
///
/// BESL registers most of its math intrinsics for `f32` alone, so a node that works on colours is
/// written out one lane at a time. Operands narrower than the result repeat their only component,
/// which is how MaterialX broadcasts a scalar input across a vector one.
fn componentwise<'a>(
	lowering: &mut Lowering<'a, '_>,
	hint: &str,
	result: DataType<'a>,
	operands: Vec<Expression<'a>>,
	build: impl Fn(&[Node<'a>]) -> Node<'a>,
) -> Result<Expression<'a>, LowerError> {
	let width = syntax::width(result, hint)?;

	// Component access reads its operand once per lane, so every operand has to be a name first.
	let mut addressable = Vec::with_capacity(operands.len());

	for operand in operands {
		addressable.push(lowering.addressable(hint, operand)?);
	}

	let components = (0..width)
		.map(|index| build(&addressable.iter().map(|operand| lane(operand, index)).collect::<Vec<_>>()))
		.collect();

	lowering.bind(hint, result, syntax::construct(components))
}

/// Reads one lane of a value, repeating its only component when it has just one.
fn lane<'a>(value: &Expression<'a>, index: usize) -> Node<'a> {
	let width = value.width();

	syntax::component(&value.syntax, index.min(width - 1), width)
}

/// Writes one of the scalar functions that map straight onto a lane of a value.
///
/// Four MaterialX nodes have no BESL intrinsic and are written out of the ones that do; the rest name
/// their intrinsic outright, apart from the absolute value, which MaterialX spells differently.
fn scalar_function<'a>(category: &'a str, value: &Node<'a>) -> Node<'a> {
	let zero = || syntax::literal(0.0);
	let step = |edge: Node<'a>, value: Node<'a>| Node::call("step", vec![edge, value]);

	match category {
		"absval" => Node::call("abs", vec![value.clone()]),
		// BESL has no ceiling, and flooring the negated value rounds the other way.
		"ceil" => Node::operator(
			"-",
			zero(),
			Node::call("floor", vec![Node::operator("-", zero(), value.clone())]),
		),
		// BESL rounds only half-precision and two-component values, so round through the floor instead.
		"round" => Node::call("floor", vec![Node::operator("+", value.clone(), syntax::literal(0.5))]),
		// A step either side of zero gives 1, -1 and 0 without a comparison.
		"sign" => Node::operator("-", step(zero(), value.clone()), step(value.clone(), zero())),
		// BESL has no inverse cosine, and the two inverse functions are complements.
		"acos" => Node::operator(
			"-",
			syntax::literal(std::f32::consts::FRAC_PI_2),
			Node::call("asin", vec![value.clone()]),
		),
		intrinsic => Node::call(intrinsic, vec![value.clone()]),
	}
}

/// Writes a natural logarithm.
///
/// BESL registers `log2` for three-component vectors alone, so the components travel through it in
/// groups of three and are scaled back to a natural logarithm afterwards.
fn logarithm<'a>(
	lowering: &mut Lowering<'a, '_>,
	hint: &str,
	value: Expression<'a>,
	result: DataType<'a>,
) -> Result<Expression<'a>, LowerError> {
	let width = syntax::width(result, hint)?;
	let value = lowering.addressable(hint, value)?;
	let mut components = Vec::with_capacity(width);

	for first in (0..width).step_by(3) {
		let lanes = (0..3).map(|offset| lane(&value, first + offset)).collect();
		let logarithms = lowering.bind(hint, DataType::Vector3, Node::call("log2", vec![Node::call("vec3f", lanes)]))?;

		for offset in 0..(width - first).min(3) {
			components.push(Node::operator(
				"*",
				syntax::component(&logarithms.syntax, offset, 3),
				syntax::literal(LOGARITHM_OF_TWO),
			));
		}
	}

	lowering.bind(hint, result, syntax::construct(components))
}

/// Writes a vector's length.
fn magnitude<'a>(lowering: &mut Lowering<'a, '_>, hint: &str, value: Expression<'a>) -> Result<Expression<'a>, LowerError> {
	let length = match value.width() {
		1 => Node::call("abs", vec![value.syntax]),
		// BESL registers `length` for three and four components only.
		2 => Node::call("sqrt", vec![Node::call("dot", vec![value.syntax.clone(), value.syntax])]),
		_ => Node::call("length", vec![value.syntax]),
	};

	lowering.bind(hint, DataType::Float, length)
}

/// Writes a vector scaled to unit length.
fn normalize<'a>(
	lowering: &mut Lowering<'a, '_>,
	hint: &str,
	value: Expression<'a>,
	result: DataType<'a>,
) -> Result<Expression<'a>, LowerError> {
	if value.width() >= 3 {
		return lowering.bind(hint, result, Node::call("normalize", vec![value.syntax]));
	}

	let value = lowering.addressable(hint, value)?;
	let length = magnitude(lowering, hint, value.clone())?;

	lowering.bind(hint, result, Node::operator("/", value.syntax, length.syntax))
}

/// Writes a linear blend between two values.
fn blend<'a>(from: Node<'a>, to: Node<'a>, factor: Node<'a>) -> Node<'a> {
	Node::operator("+", from.clone(), Node::operator("*", Node::operator("-", to, from), factor))
}

/// Adds a list of terms together, or writes nothing when the list is empty.
fn sum<'a>(terms: Vec<Node<'a>>) -> Option<Node<'a>> {
	terms.into_iter().reduce(|total, term| Node::operator("+", total, term))
}

/// Writes the zero-or-one gate of one of the comparison nodes.
fn comparison<'a>(
	lowering: &mut Lowering<'a, '_>,
	frame: usize,
	node: NodeId,
	hint: &str,
	category: &str,
) -> Result<Expression<'a>, LowerError> {
	let left = operand(lowering, frame, node, "value1", 0.0, DataType::Float)?;
	let left = as_float(lowering, hint, left)?.syntax;
	let right = operand(lowering, frame, node, "value2", 0.0, DataType::Float)?;
	let right = as_float(lowering, hint, right)?.syntax;

	// A step is one when its second argument reaches its first, which spells every comparison here.
	let not_below = |edge: &Node<'a>, value: &Node<'a>| Node::call("step", vec![edge.clone(), value.clone()]);

	let gate = match category {
		"ifgreatereq" => not_below(&right, &left),
		"ifequal" => Node::operator("*", not_below(&right, &left), not_below(&left, &right)),
		_ => Node::operator("-", syntax::literal(1.0), not_below(&left, &right)),
	};

	lowering.bind(hint, DataType::Float, gate)
}

/// Converts an integer value to a float, so a comparison can be written with float intrinsics.
fn as_float<'a>(lowering: &mut Lowering<'a, '_>, hint: &str, value: Expression<'a>) -> Result<Expression<'a>, LowerError> {
	if value.data_type != DataType::Integer {
		return lowering.addressable(hint, value);
	}

	lowering.bind(hint, DataType::Float, Node::call("f32", vec![value.syntax]))
}

/// Writes the value one of a switch node's numbered inputs carries.
fn switch<'a>(
	lowering: &mut Lowering<'a, '_>,
	frame: usize,
	node: NodeId,
	hint: &str,
	result: DataType<'a>,
) -> Result<Expression<'a>, LowerError> {
	let selector = operand(lowering, frame, node, "which", 0.0, DataType::Float)?;
	let selector = as_float(lowering, hint, selector)?;
	let mut terms = Vec::with_capacity(MAXIMUM_SWITCH_INPUTS);

	// BESL has no conditional expression, so every branch is weighted by whether the selector picks it.
	for index in 0..MAXIMUM_SWITCH_INPUTS {
		let Some(branch) = lowering.input(frame, node, &format!("in{}", index + 1))? else {
			continue;
		};

		// A branch is picked when the selector reaches its number but not the next one.
		let reaches = |edge: f32| Node::call("step", vec![syntax::literal(edge), selector.syntax.clone()]);
		let gate = lowering.bind(
			hint,
			DataType::Float,
			Node::operator("-", reaches(index as f32 - 0.5), reaches(index as f32 + 0.5)),
		)?;

		terms.push(Node::operator("*", branch.syntax, gate.syntax));
	}

	let total = sum(terms).unwrap_or_else(|| syntax::splat_literal(0.0, syntax::components(result).unwrap_or(1)));

	lowering.bind(hint, result, total)
}

/// Writes a value remapped from one range onto another.
fn remap<'a>(
	lowering: &mut Lowering<'a, '_>,
	frame: usize,
	node: NodeId,
	hint: &str,
	result: DataType<'a>,
) -> Result<Expression<'a>, LowerError> {
	let value = operand(lowering, frame, node, "in", 0.0, result)?;
	let input_low = operand(lowering, frame, node, "inlow", 0.0, DataType::Float)?;
	let input_high = operand(lowering, frame, node, "inhigh", 1.0, DataType::Float)?;
	let output_low = operand(lowering, frame, node, "outlow", 0.0, DataType::Float)?;
	let output_high = operand(lowering, frame, node, "outhigh", 1.0, DataType::Float)?;

	let position = Node::operator(
		"/",
		Node::operator("-", value.syntax, input_low.syntax.clone()),
		Node::operator("-", input_high.syntax, input_low.syntax),
	);
	let span = Node::operator("-", output_high.syntax, output_low.syntax.clone());

	lowering.bind(
		hint,
		result,
		Node::operator("+", output_low.syntax, Node::operator("*", position, span)),
	)
}

/// Writes a colour's luminance across every one of its colour channels.
fn luminance<'a>(
	lowering: &mut Lowering<'a, '_>,
	frame: usize,
	node: NodeId,
	hint: &str,
	value: Expression<'a>,
	result: DataType<'a>,
) -> Result<Expression<'a>, LowerError> {
	let width = value.width();
	let value = lowering.addressable(hint, value)?;
	let weights = match lowering.input(frame, node, "lumacoeffs")? {
		Some(weights) => lowering.addressable(hint, weights)?,
		None => Expression::new(
			syntax::construct(LUMINANCE_WEIGHTS.iter().copied().map(syntax::literal).collect()),
			DataType::Color3,
		),
	};

	let channels = width.min(3);
	let weighted = (0..channels)
		.map(|index| {
			Node::operator(
				"*",
				syntax::component(&value.syntax, index, width),
				syntax::component(&weights.syntax, index, 3),
			)
		})
		.collect();

	let total = lowering.bind(hint, DataType::Float, sum(weighted).unwrap_or_else(|| syntax::literal(0.0)))?;
	let mut components: Vec<Node<'a>> = (0..channels).map(|_| total.syntax.clone()).collect();

	// A four-channel colour keeps its alpha, because luminance only describes the colour channels.
	if width == 4 {
		components.push(syntax::component(&value.syntax, 3, 4));
	}

	lowering.bind(hint, result, syntax::construct(components))
}

/// Writes a colour with its alpha multiplied into, or divided out of, its colour channels.
fn premultiply<'a>(
	lowering: &mut Lowering<'a, '_>,
	hint: &str,
	value: Expression<'a>,
	result: DataType<'a>,
	multiply: bool,
) -> Result<Expression<'a>, LowerError> {
	if value.width() < 4 {
		// Without an alpha channel there is nothing to premultiply by.
		return lowering.bind(hint, result, value.syntax);
	}

	let value = lowering.addressable(hint, value)?;
	let alpha = syntax::component(&value.syntax, 3, 4);

	let mut components: Vec<Node<'a>> = (0..3)
		.map(|index| {
			let channel = syntax::component(&value.syntax, index, 4);

			if multiply {
				Node::operator("*", channel, alpha.clone())
			} else {
				// A transparent texel carries no colour, so keep the divisor away from zero.
				Node::operator(
					"/",
					channel,
					Node::call("max", vec![alpha.clone(), syntax::literal(MINIMUM_ALPHA)]),
				)
			}
		})
		.collect();

	components.push(alpha);

	lowering.bind(hint, result, syntax::construct(components))
}

/// Writes the world-space normal a tangent-space normal map encodes.
fn normal_map<'a>(
	lowering: &mut Lowering<'a, '_>,
	frame: usize,
	node: NodeId,
	hint: &str,
) -> Result<Expression<'a>, LowerError> {
	let encoded = operand(lowering, frame, node, "in", 0.5, DataType::Vector3)?;
	let scale = operand(lowering, frame, node, "scale", 1.0, DataType::Float)?;
	let encoded = lowering.addressable(hint, encoded)?;

	// The map stores a unit vector shifted into the zero-to-one range that a colour texture holds.
	let decode = |index: usize| {
		Node::operator(
			"-",
			Node::operator("*", syntax::component(&encoded.syntax, index, 3), syntax::literal(2.0)),
			syntax::literal(1.0),
		)
	};

	// Scaling the tangent-plane components alone tilts the normal without changing which way it faces.
	let decoded = lowering.bind(
		hint,
		DataType::Vector3,
		Node::call(
			"vec3f",
			vec![
				Node::operator("*", decode(0), scale.syntax.clone()),
				Node::operator("*", decode(1), scale.syntax),
				decode(2),
			],
		),
	)?;

	let frame_axes = ["T", "B", "N"];
	let world = sum(frame_axes
		.iter()
		.enumerate()
		.map(|(index, axis)| {
			Node::operator(
				"*",
				Node::member_expression(*axis),
				syntax::component(&decoded.syntax, index, 3),
			)
		})
		.collect())
	.unwrap_or_else(|| syntax::splat_literal(0.0, 3));

	lowering.bind(hint, DataType::Vector3, Node::call("normalize", vec![world]))
}

/// Writes a texture sample, recording the image so the renderer can bind it.
fn image<'a>(
	lowering: &mut Lowering<'a, '_>,
	frame: usize,
	node: NodeId,
	instance: &Instance<'a>,
	result: DataType<'a>,
) -> Result<Expression<'a>, LowerError> {
	let name = instance.name;

	// The material stage samples with the mesh's own texture coordinates, so a transformed set cannot be honoured.
	if let Some(coordinates) = instance.input("texcoord")
		&& !samples_default_coordinates(lowering, frame, coordinates)
	{
		return Err(LowerError::UnsupportedTextureCoordinates { node: name.to_string() });
	}

	let file = string_parameter(lowering, frame, node, "file").unwrap_or("");

	if file.is_empty() {
		return Err(LowerError::MissingTextureFile { node: name.to_string() });
	}

	let written = instance.input("file");
	let variable = lowering.texture(Texture {
		file,
		file_prefix: written.and_then(|input| input.file_prefix).or(instance.file_prefix),
		colorspace: written.and_then(|input| input.colorspace).or(instance.colorspace),
	});

	let sample = lowering.bind(name, DataType::Vector4, Node::call("sample_material", vec![variable]))?;

	convert(lowering, name, sample, result)
}

/// Reports whether an image's texture coordinates are the ones the material stage already carries.
fn samples_default_coordinates<'a>(lowering: &Lowering<'a, '_>, frame: usize, coordinates: &Input<'a>) -> bool {
	match &coordinates.source {
		Source::Unconnected => true,
		Source::GeomProp(property) => TEXTURE_COORDINATES.contains(property),
		Source::Node { node, .. } => match lowering.dag.node(*node).category {
			"texcoord" => integer_parameter(lowering, frame, *node, "index").unwrap_or(0) == 0,
			"geompropvalue" => string_parameter(lowering, frame, *node, "geomprop")
				.is_some_and(|property| TEXTURE_COORDINATES.contains(&property)),
			_ => false,
		},
		_ => false,
	}
}

/// Writes a value converted to another MaterialX type.
///
/// A narrower value gains ones, which is how MaterialX opens a colour to full opacity and a vector to
/// a homogeneous coordinate, and a wider value simply drops the components the target has no room for.
pub(super) fn convert<'a>(
	lowering: &mut Lowering<'a, '_>,
	hint: &str,
	value: Expression<'a>,
	result: DataType<'a>,
) -> Result<Expression<'a>, LowerError> {
	if value.data_type == result {
		return Ok(value);
	}

	let target = syntax::width(result, hint)?;

	if value.data_type == DataType::Integer {
		let float = lowering.bind(hint, DataType::Float, Node::call("f32", vec![value.syntax]))?;

		return Ok(Expression::new(syntax::splat(&float.syntax, target), result));
	}

	let source = syntax::width(value.data_type, hint)?;

	if source == target {
		return Ok(Expression::new(value.syntax, result));
	}

	let value = lowering.addressable(hint, value)?;
	let components = (0..target)
		.map(|index| match index {
			// A single component fills the whole value, which is how MaterialX broadcasts a float.
			_ if source == 1 => value.syntax.clone(),
			index if index < source => syntax::component(&value.syntax, index, source),
			_ => syntax::literal(1.0),
		})
		.collect();

	lowering.bind(hint, result, syntax::construct(components))
}

/// Writes the components of several inputs joined into one value.
fn combine<'a>(
	lowering: &mut Lowering<'a, '_>,
	frame: usize,
	node: NodeId,
	hint: &str,
	result: DataType<'a>,
) -> Result<Expression<'a>, LowerError> {
	let target = syntax::width(result, hint)?;
	let mut components = Vec::with_capacity(target);

	for index in 1..=4 {
		let Some(part) = lowering.input(frame, node, &format!("in{index}"))? else {
			continue;
		};

		let width = part.width();
		let part = lowering.addressable(hint, part)?;

		components.extend((0..width).map(|lane| syntax::component(&part.syntax, lane, width)));
	}

	components.resize(target, syntax::literal(0.0));

	lowering.bind(hint, result, syntax::construct(components))
}

/// Returns the lane a MaterialX channel letter names.
fn channel(letter: char) -> Option<usize> {
	Some(match letter {
		'x' | 'r' => 0,
		'y' | 'g' => 1,
		'z' | 'b' => 2,
		'w' | 'a' => 3,
		_ => return None,
	})
}

/// Writes one channel of a value, chosen by which of a separate node's outputs is read.
fn separate<'a>(
	lowering: &mut Lowering<'a, '_>,
	hint: &str,
	value: Expression<'a>,
	instance: &Instance<'a>,
	output: PortIndex,
	result: DataType<'a>,
) -> Result<Expression<'a>, LowerError> {
	let width = value.width();
	let port = instance.outputs.get(output.index()).map_or("outx", |port| port.name);

	// Separate nodes name one output per channel, so the last letter of the name picks the lane.
	let index = port.chars().next_back().and_then(channel).unwrap_or(0);

	if index >= width {
		return Err(LowerError::UnknownOutput {
			node: instance.name.to_string(),
			output: output.index() as u32,
		});
	}

	let value = lowering.addressable(hint, value)?;

	lowering.bind(hint, result, syntax::component(&value.syntax, index, width))
}

/// Writes a value whose components are picked, repeated or replaced by a channel string.
pub(super) fn swizzle<'a>(
	lowering: &mut Lowering<'a, '_>,
	hint: &str,
	value: Expression<'a>,
	channels: &str,
	result: DataType<'a>,
) -> Result<Expression<'a>, LowerError> {
	let width = syntax::width(value.data_type, hint)?;
	let value = lowering.addressable(hint, value)?;

	let mut components: Vec<Node<'a>> = channels
		.chars()
		.map(|letter| match channel(letter) {
			Some(lane) => syntax::component(&value.syntax, lane.min(width - 1), width),
			// MaterialX lets a channel string write a constant lane directly; anything else reads as zero.
			None => syntax::literal(f32::from(letter == '1')),
		})
		.collect();

	if components.is_empty() {
		components.push(syntax::component(&value.syntax, 0, width));
	}

	let width = components.len();
	let selected = Expression::new(syntax::construct(components), syntax::float_data_type(width));

	convert(lowering, hint, selected, result)
}
