//! Turns one MaterialX node category into the BESL expression that computes it.

use besl::parser::Node;

use crate::materialx::{DataType, Node as Instance, NodeId, PortIndex, Value};

use super::Texture;
use super::error::LowerError;
use super::lowering::{Lowering, geometry};
use super::syntax::{self, Expression};

/// The ratio between a base-two and a natural logarithm.
const LOGARITHM_OF_TWO: f32 = std::f32::consts::LN_2;

/// The luminance weights the MaterialX standard library uses when a node writes none.
const LUMINANCE_WEIGHTS: [f32; 3] = [0.272_228_7, 0.674_081_8, 0.053_589_5];

/// The smallest alpha `unpremult` divides by, so a fully transparent texel does not divide by zero.
const MINIMUM_ALPHA: f32 = 1.0e-6;

/// The count of numbered inputs a MaterialX switch node declares.
const MAXIMUM_SWITCH_INPUTS: usize = 10;

/// Lowers one output of one node.
///
/// A category this module knows is lowered directly, because that is this renderer's implementation
/// of it. Anything else falls back to the node graph that implements the node's declaration, which
/// is how a document extends the standard library with graphs of its own.
pub(super) fn lower<'a>(
	lowering: &mut Lowering<'a, '_>,
	frame: usize,
	node: NodeId,
	output: PortIndex,
) -> Result<Expression<'a>, LowerError> {
	let dag = lowering.dag;
	let instance = dag.node(node);

	if let Some(value) = builtin(lowering, frame, node, instance, output)? {
		return Ok(value);
	}

	if let Some(graph) = lowering.implementation(instance.declaration) {
		return expand(lowering, frame, node, graph, output);
	}

	Err(LowerError::UnsupportedNode {
		node: instance.name.to_string(),
		category: instance.category.to_string(),
	})
}

/// Lowers a node by expanding the node graph that implements its declaration.
fn expand<'a>(
	lowering: &mut Lowering<'a, '_>,
	frame: usize,
	node: NodeId,
	graph: crate::materialx::GraphId,
	output: PortIndex,
) -> Result<Expression<'a>, LowerError> {
	// The node's own inputs bind the graph's interface; anything it leaves out keeps the declared default.
	let arguments = lowering.bindings(frame, graph, node);
	let instantiation = lowering.expansion(frame, node, graph, arguments)?;

	lowering.graph_result(instantiation, graph, output)
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
	let result = output_type(instance, output);

	let value = match instance.category {
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
			let width = value.width().unwrap_or(1);
			let value = lowering.addressable(name, value)?;

			lowering.bind(
				name,
				result,
				syntax::component(&value.syntax, index.min(width.saturating_sub(1)), width),
			)?
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
				Node::operator("-", syntax::splat(0.0, 3), view.syntax),
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
		"geompropvalue" => {
			let property = string_parameter(lowering, frame, node, "geomprop").unwrap_or("");

			geometry(property, name)?
		}

		// Textures.
		"image" => image(lowering, frame, node, instance, result)?,

		// Arithmetic. BESL applies these across components and broadcasts a scalar operand, so they stay whole.
		"add" | "subtract" | "multiply" | "divide" => {
			let identity = f32::from(instance.category == "multiply" || instance.category == "divide");
			let left = operand(lowering, frame, node, "in1", 0.0, result)?;
			let right = operand(lowering, frame, node, "in2", identity, result)?;
			let operator = match instance.category {
				"add" => "+",
				"subtract" => "-",
				"multiply" => "*",
				_ => "/",
			};

			lowering.bind(name, result, Node::operator(operator, left.syntax, right.syntax))?
		}
		"modulo" => {
			let left = operand(lowering, frame, node, "in1", 0.0, result)?;
			let right = operand(lowering, frame, node, "in2", 1.0, result)?;

			// MaterialX takes the floored remainder, which keeps the sign of the divisor.
			componentwise(lowering, name, result, vec![left, right], |arguments| {
				let quotient = Node::operator("/", arguments[0].clone(), arguments[1].clone());
				let whole = syntax::call("floor", vec![quotient]);

				Node::operator(
					"-",
					arguments[0].clone(),
					Node::operator("*", arguments[1].clone(), whole),
				)
			})?
		}
		"power" => {
			let left = operand(lowering, frame, node, "in1", 0.0, result)?;
			let right = operand(lowering, frame, node, "in2", 1.0, result)?;

			componentwise(lowering, name, result, vec![left, right], |arguments| {
				syntax::call("pow", arguments.to_vec())
			})?
		}
		"min" | "max" => {
			let intrinsic = if instance.category == "min" { "min" } else { "max" };
			let left = operand(lowering, frame, node, "in1", 0.0, result)?;
			let right = operand(lowering, frame, node, "in2", 0.0, result)?;

			componentwise(lowering, name, result, vec![left, right], move |arguments| {
				syntax::call(intrinsic, arguments.to_vec())
			})?
		}
		"atan2" => {
			let y = operand(lowering, frame, node, "in1", 0.0, result)?;
			let x = operand(lowering, frame, node, "in2", 1.0, result)?;

			componentwise(lowering, name, result, vec![y, x], |arguments| {
				syntax::call("atan2", arguments.to_vec())
			})?
		}

		// Component-wise functions.
		"absval" | "floor" | "ceil" | "round" | "sign" | "sqrt" | "exp" | "sin" | "cos" | "tan" | "asin" | "acos" => {
			let value = operand(lowering, frame, node, "in", 0.0, result)?;
			let category = instance.category;

			componentwise(lowering, name, result, vec![value], move |arguments| {
				scalar_function(category, &arguments[0])
			})?
		}
		"ln" => {
			let value = operand(lowering, frame, node, "in", 1.0, result)?;

			logarithm(lowering, name, value, result)?
		}
		"clamp" => {
			let value = operand(lowering, frame, node, "in", 0.0, result)?;
			let low = operand(lowering, frame, node, "low", 0.0, DataType::Float)?;
			let high = operand(lowering, frame, node, "high", 1.0, DataType::Float)?;

			componentwise(lowering, name, result, vec![value, low, high], |arguments| {
				syntax::call("clamp", arguments.to_vec())
			})?
		}
		"smoothstep" => {
			let value = operand(lowering, frame, node, "in", 0.0, result)?;
			let low = operand(lowering, frame, node, "low", 0.0, DataType::Float)?;
			let high = operand(lowering, frame, node, "high", 1.0, DataType::Float)?;

			componentwise(lowering, name, result, vec![value, low, high], |arguments| {
				syntax::call(
					"smoothstep",
					vec![arguments[1].clone(), arguments[2].clone(), arguments[0].clone()],
				)
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
			let width = left.width().unwrap_or(1);

			let product = if width == 1 {
				Node::operator("*", left.syntax, right.syntax)
			} else {
				syntax::call("dot", vec![left.syntax, right.syntax])
			};

			lowering.bind(name, DataType::Float, product)?
		}
		"crossproduct" => {
			let left = operand(lowering, frame, node, "in1", 0.0, DataType::Vector3)?;
			let right = operand(lowering, frame, node, "in2", 0.0, DataType::Vector3)?;

			lowering.bind(name, result, syntax::call("cross", vec![left.syntax, right.syntax]))?
		}

		// Blends and adjustments.
		"mix" => {
			let foreground = operand(lowering, frame, node, "fg", 0.0, result)?;
			let background = operand(lowering, frame, node, "bg", 0.0, result)?;
			let factor = operand(lowering, frame, node, "mix", 0.0, DataType::Float)?;

			lowering.bind(name, result, blend(&background, &foreground, &factor.syntax))?
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

			premultiply(lowering, name, value, result, instance.category == "premult")?
		}
		"ifgreater" | "ifgreatereq" | "ifequal" => {
			let gate = comparison(lowering, frame, node, name, instance.category)?;
			let matched = operand(lowering, frame, node, "in1", 0.0, result)?;
			let other = operand(lowering, frame, node, "in2", 0.0, result)?;

			lowering.bind(name, result, blend(&other, &matched, &gate.syntax))?
		}
		"switch" => switch(lowering, frame, node, name, result)?,

		// Shading.
		"normalmap" => normal_map(lowering, frame, node, name)?,

		_ => return Ok(None),
	};

	Ok(Some(value))
}

/// Returns the type one output of a node carries.
fn output_type<'a>(instance: &Instance<'a>, output: PortIndex) -> DataType<'a> {
	instance
		.outputs
		.get(output.index())
		.map_or(instance.data_type, |port| port.data_type)
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

	let width = syntax::width(data_type).unwrap_or(1);

	Ok(Expression::new(syntax::splat(default, width), syntax::float_data_type(width)))
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
pub(super) fn componentwise<'a>(
	lowering: &mut Lowering<'a, '_>,
	hint: &str,
	result: DataType<'a>,
	operands: Vec<Expression<'a>>,
	build: impl Fn(&[Node<'a>]) -> Node<'a>,
) -> Result<Expression<'a>, LowerError> {
	let width = syntax::width(result).ok_or_else(|| LowerError::UnsupportedType {
		node: hint.to_string(),
		data_type: result.name().to_string(),
	})?;

	// Component access reads its operand once per lane, so every operand has to be a name first.
	let mut addressable = Vec::with_capacity(operands.len());

	for operand in operands {
		addressable.push(lowering.addressable(hint, operand)?);
	}

	let mut components = Vec::with_capacity(width);

	for index in 0..width {
		let arguments: Vec<Node<'a>> = addressable.iter().map(|operand| component(operand, index)).collect();

		components.push(build(&arguments));
	}

	lowering.bind(hint, result, syntax::construct(components))
}

/// Reads one lane of a value, repeating its only component when it has just one.
fn component<'a>(value: &Expression<'a>, index: usize) -> Node<'a> {
	let width = value.width().unwrap_or(1);

	syntax::component(&value.syntax, index.min(width.saturating_sub(1)), width)
}

/// Writes one of the scalar functions that map straight onto a lane of a value.
fn scalar_function<'a>(category: &str, value: &Node<'a>) -> Node<'a> {
	match category {
		"absval" => syntax::call("abs", vec![value.clone()]),
		"floor" => syntax::call("floor", vec![value.clone()]),
		// BESL has no ceiling, and flooring the negated value rounds the other way.
		"ceil" => Node::operator(
			"-",
			syntax::signed_literal(0.0),
			syntax::call(
				"floor",
				vec![Node::operator("-", syntax::signed_literal(0.0), value.clone())],
			),
		),
		// BESL rounds only half-precision and two-component values, so round through the floor instead.
		"round" => syntax::call(
			"floor",
			vec![Node::operator("+", value.clone(), syntax::signed_literal(0.5))],
		),
		// A step either side of zero gives 1, -1 and 0 without a comparison.
		"sign" => Node::operator(
			"-",
			syntax::call("step", vec![syntax::signed_literal(0.0), value.clone()]),
			syntax::call("step", vec![value.clone(), syntax::signed_literal(0.0)]),
		),
		"sqrt" => syntax::call("sqrt", vec![value.clone()]),
		"exp" => syntax::call("exp", vec![value.clone()]),
		"sin" => syntax::call("sin", vec![value.clone()]),
		"cos" => syntax::call("cos", vec![value.clone()]),
		"tan" => syntax::call("tan", vec![value.clone()]),
		"asin" => syntax::call("asin", vec![value.clone()]),
		// BESL has no inverse cosine, and the two inverse functions are complements.
		_ => Node::operator(
			"-",
			syntax::signed_literal(std::f32::consts::FRAC_PI_2),
			syntax::call("asin", vec![value.clone()]),
		),
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
	let width = syntax::width(result).ok_or_else(|| LowerError::UnsupportedType {
		node: hint.to_string(),
		data_type: result.name().to_string(),
	})?;

	let value = lowering.addressable(hint, value)?;
	let mut components = Vec::with_capacity(width);
	let mut first = 0;

	while first < width {
		let lanes = (0..3).map(|lane| component(&value, first + lane)).collect();
		let logarithms = lowering.bind(
			hint,
			DataType::Vector3,
			syntax::call("log2", vec![Node::call("vec3f", lanes)]),
		)?;

		for lane in 0..(width - first).min(3) {
			components.push(Node::operator(
				"*",
				syntax::component(&logarithms.syntax, lane, 3),
				syntax::signed_literal(LOGARITHM_OF_TWO),
			));
		}

		first += 3;
	}

	lowering.bind(hint, result, syntax::construct(components))
}

/// Writes a vector's length.
fn magnitude<'a>(
	lowering: &mut Lowering<'a, '_>,
	hint: &str,
	value: Expression<'a>,
) -> Result<Expression<'a>, LowerError> {
	let width = value.width().unwrap_or(1);

	let length = match width {
		1 => syntax::call("abs", vec![value.syntax]),
		// BESL registers `length` for three and four components only.
		2 => syntax::call(
			"sqrt",
			vec![syntax::call("dot", vec![value.syntax.clone(), value.syntax])],
		),
		_ => syntax::call("length", vec![value.syntax]),
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
	let width = value.width().unwrap_or(1);

	if width >= 3 {
		return lowering.bind(hint, result, syntax::call("normalize", vec![value.syntax]));
	}

	let value = lowering.addressable(hint, value)?;
	let length = magnitude(lowering, hint, value.clone())?;

	lowering.bind(hint, result, Node::operator("/", value.syntax, length.syntax))
}

/// Writes a linear blend between two values.
fn blend<'a>(from: &Expression<'a>, to: &Expression<'a>, factor: &Node<'a>) -> Node<'a> {
	Node::operator(
		"+",
		from.syntax.clone(),
		Node::operator(
			"*",
			Node::operator("-", to.syntax.clone(), from.syntax.clone()),
			factor.clone(),
		),
	)
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
	let left = as_float(lowering, hint, left)?;
	let right = operand(lowering, frame, node, "value2", 0.0, DataType::Float)?;
	let right = as_float(lowering, hint, right)?;

	// A step is one when its second argument reaches its first, which spells every comparison here.
	let not_below = |edge: &Node<'a>, value: &Node<'a>| syntax::call("step", vec![edge.clone(), value.clone()]);

	let gate = match category {
		"ifgreatereq" => not_below(&right.syntax, &left.syntax),
		"ifequal" => Node::operator(
			"*",
			not_below(&right.syntax, &left.syntax),
			not_below(&left.syntax, &right.syntax),
		),
		_ => Node::operator(
			"-",
			syntax::signed_literal(1.0),
			not_below(&left.syntax, &right.syntax),
		),
	};

	lowering.bind(hint, DataType::Float, gate)
}

/// Converts an integer value to a float, so a comparison can be written with float intrinsics.
fn as_float<'a>(
	lowering: &mut Lowering<'a, '_>,
	hint: &str,
	value: Expression<'a>,
) -> Result<Expression<'a>, LowerError> {
	if value.data_type != DataType::Integer {
		return lowering.addressable(hint, value);
	}

	lowering.bind(hint, DataType::Float, syntax::call("f32", vec![value.syntax]))
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
	let width = syntax::width(result).unwrap_or(1);
	let mut total: Option<Node<'a>> = None;

	// BESL has no conditional expression, so every branch is weighted by whether the selector picks it.
	for index in 0..MAXIMUM_SWITCH_INPUTS {
		let name = format!("in{}", index + 1);

		let Some(branch) = lowering.input(frame, node, &name)? else {
			continue;
		};

		let selected = syntax::call(
			"step",
			vec![
				syntax::signed_literal(index as f32 - 0.5),
				selector.syntax.clone(),
			],
		);
		let unselected = syntax::call(
			"step",
			vec![
				syntax::signed_literal(index as f32 + 0.5),
				selector.syntax.clone(),
			],
		);
		let gate = lowering.bind(
			hint,
			DataType::Float,
			Node::operator("-", selected, unselected),
		)?;
		let term = Node::operator("*", branch.syntax, gate.syntax);

		total = Some(match total {
			Some(sum) => Node::operator("+", sum, term),
			None => term,
		});
	}

	let total = total.unwrap_or_else(|| syntax::splat(0.0, width));

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
	let width = value.width().unwrap_or(3);
	let value = lowering.addressable(hint, value)?;
	let weights = match lowering.input(frame, node, "lumacoeffs")? {
		Some(weights) => lowering.addressable(hint, weights)?,
		None => Expression::new(
			syntax::construct(LUMINANCE_WEIGHTS.iter().copied().map(syntax::signed_literal).collect()),
			DataType::Color3,
		),
	};

	let mut sum: Option<Node<'a>> = None;

	for index in 0..width.min(3) {
		let term = Node::operator(
			"*",
			syntax::component(&value.syntax, index, width),
			syntax::component(&weights.syntax, index, 3),
		);

		sum = Some(match sum {
			Some(total) => Node::operator("+", total, term),
			None => term,
		});
	}

	let sum = lowering.bind(hint, DataType::Float, sum.unwrap_or_else(|| syntax::splat(0.0, 1)))?;
	let mut components: Vec<Node<'a>> = (0..width.min(3)).map(|_| sum.syntax.clone()).collect();

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
	let width = value.width().unwrap_or(4);

	if width < 4 {
		// Without an alpha channel there is nothing to premultiply by.
		return lowering.bind(hint, result, value.syntax);
	}

	let value = lowering.addressable(hint, value)?;
	let alpha = syntax::component(&value.syntax, 3, 4);
	let mut components = Vec::with_capacity(4);

	for index in 0..3 {
		let channel = syntax::component(&value.syntax, index, 4);

		components.push(if multiply {
			Node::operator("*", channel, alpha.clone())
		} else {
			// A transparent texel carries no colour, so keep the divisor away from zero.
			Node::operator(
				"/",
				channel,
				syntax::call("max", vec![alpha.clone(), syntax::signed_literal(MINIMUM_ALPHA)]),
			)
		});
	}

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
			Node::operator(
				"*",
				syntax::component(&encoded.syntax, index, 3),
				syntax::signed_literal(2.0),
			),
			syntax::signed_literal(1.0),
		)
	};

	let tangent = lowering.bind(
		hint,
		DataType::Float,
		Node::operator("*", decode(0), scale.syntax.clone()),
	)?;
	let bitangent = lowering.bind(hint, DataType::Float, Node::operator("*", decode(1), scale.syntax))?;
	let normal = lowering.bind(hint, DataType::Float, decode(2))?;

	let world = Node::operator(
		"+",
		Node::operator(
			"+",
			Node::operator("*", Node::member_expression("T"), tangent.syntax),
			Node::operator("*", Node::member_expression("B"), bitangent.syntax),
		),
		Node::operator("*", Node::member_expression("N"), normal.syntax),
	);

	lowering.bind(
		hint,
		DataType::Vector3,
		syntax::call("normalize", vec![world]),
	)
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
		&& !samples_default_coordinates(lowering, frame, coordinates)?
	{
		return Err(LowerError::UnsupportedTextureCoordinates { node: name.to_string() });
	}

	let file = string_parameter(lowering, frame, node, "file").unwrap_or("");

	if file.is_empty() {
		return Err(LowerError::MissingTextureFile { node: name.to_string() });
	}

	let source = instance.input("file");
	let texture = Texture {
		file,
		file_prefix: source.and_then(|input| input.file_prefix).or(instance.file_prefix),
		colorspace: source.and_then(|input| input.colorspace).or(instance.colorspace),
	};

	let variable = lowering.texture(texture);
	let sample = lowering.bind(name, DataType::Vector4, syntax::call("sample_material", vec![variable]))?;

	convert(lowering, name, sample, result)
}

/// Reports whether an image's texture coordinates are the ones the material stage already carries.
fn samples_default_coordinates<'a>(
	lowering: &Lowering<'a, '_>,
	frame: usize,
	coordinates: &crate::materialx::Input<'a>,
) -> Result<bool, LowerError> {
	Ok(match &coordinates.source {
		crate::materialx::Source::Unconnected => true,
		crate::materialx::Source::GeomProp(property) => matches!(*property, "UV0" | "st" | "uv"),
		crate::materialx::Source::Node { node, .. } => {
			let instance = lowering.dag.node(*node);

			match instance.category {
				"texcoord" => integer_parameter(lowering, frame, *node, "index").unwrap_or(0) == 0,
				"geompropvalue" => matches!(string_parameter(lowering, frame, *node, "geomprop"), Some("UV0" | "st" | "uv")),
				_ => false,
			}
		}
		_ => false,
	})
}

/// Writes a value converted to another MaterialX type.
///
/// A narrower value gains ones, which is how MaterialX opens a colour to full opacity and a vector
/// to a homogeneous coordinate, and a wider value simply drops the components the target has no room
/// for.
pub(super) fn convert<'a>(
	lowering: &mut Lowering<'a, '_>,
	hint: &str,
	value: Expression<'a>,
	result: DataType<'a>,
) -> Result<Expression<'a>, LowerError> {
	if value.data_type == result {
		return Ok(value);
	}

	let target = syntax::width(result).ok_or_else(|| LowerError::UnsupportedType {
		node: hint.to_string(),
		data_type: result.name().to_string(),
	})?;

	if value.data_type == DataType::Integer {
		let float = lowering.bind(hint, DataType::Float, syntax::call("f32", vec![value.syntax]))?;

		return Ok(Expression::new(syntax::splat_of(&float.syntax, target), result));
	}

	let source = value.width().ok_or_else(|| LowerError::UnsupportedType {
		node: hint.to_string(),
		data_type: value.data_type.name().to_string(),
	})?;

	if source == target {
		return Ok(Expression::new(value.syntax, result));
	}

	let value = lowering.addressable(hint, value)?;
	let components = (0..target)
		.map(|index| {
			if index < source {
				syntax::component(&value.syntax, index.min(source - 1), source)
			} else if source == 1 {
				// A single component fills the whole value, which is how MaterialX broadcasts a float.
				value.syntax.clone()
			} else {
				syntax::signed_literal(1.0)
			}
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
	let target = syntax::width(result).ok_or_else(|| LowerError::UnsupportedType {
		node: hint.to_string(),
		data_type: result.name().to_string(),
	})?;

	let mut components = Vec::with_capacity(target);

	for index in 1..=4 {
		let Some(part) = lowering.input(frame, node, &format!("in{index}"))? else {
			continue;
		};

		let width = part.width().unwrap_or(1);
		let part = lowering.addressable(hint, part)?;

		for lane in 0..width {
			components.push(syntax::component(&part.syntax, lane, width));
		}
	}

	components.resize(target, syntax::signed_literal(0.0));

	lowering.bind(hint, result, syntax::construct(components))
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
	let width = value.width().unwrap_or(1);
	let port = instance.outputs.get(output.index()).map_or("outx", |port| port.name);

	// Separate nodes name one output per channel, so the last letter of the name picks the lane.
	let index = match port.chars().next_back() {
		Some('x' | 'r') => 0,
		Some('y' | 'g') => 1,
		Some('z' | 'b') => 2,
		Some('w' | 'a') => 3,
		_ => 0,
	};

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
	let width = value.width().ok_or_else(|| LowerError::UnsupportedType {
		node: hint.to_string(),
		data_type: value.data_type.name().to_string(),
	})?;

	let value = lowering.addressable(hint, value)?;
	let mut components = Vec::with_capacity(channels.len());

	for channel in channels.chars() {
		components.push(match channel {
			'x' | 'r' => syntax::component(&value.syntax, 0, width),
			'y' | 'g' => syntax::component(&value.syntax, 1.min(width - 1), width),
			'z' | 'b' => syntax::component(&value.syntax, 2.min(width - 1), width),
			'w' | 'a' => syntax::component(&value.syntax, 3.min(width - 1), width),
			// MaterialX lets a channel string write a constant lane directly.
			'1' => syntax::signed_literal(1.0),
			_ => syntax::signed_literal(0.0),
		});
	}

	if components.is_empty() {
		components.push(syntax::component(&value.syntax, 0, width));
	}

	let selected = Expression::new(syntax::construct(components.clone()), syntax::float_data_type(components.len()));

	convert(lowering, hint, selected, result)
}
