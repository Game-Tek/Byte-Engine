//! Builds the BESL expressions the lowering assembles, without knowing what a MaterialX node means.

use besl::parser::Node;

use crate::materialx::DataType;

/// The `Expression` struct carries one lowered value: its BESL syntax and the MaterialX type it holds.
///
/// The type travels with the syntax because MaterialX nodes are polymorphic over width, so the same
/// category lowers differently for a `float` than for a `color3`. Build one with
/// [`Lowering::bind`](super::lowering::Lowering::bind) so the value lands in a local that component
/// access can read more than once.
#[derive(Clone, Debug)]
pub(super) struct Expression<'a> {
	pub syntax: Node<'a>,
	pub data_type: DataType<'a>,
}

impl<'a> Expression<'a> {
	pub fn new(syntax: Node<'a>, data_type: DataType<'a>) -> Self {
		Expression { syntax, data_type }
	}

	/// Returns how many float components this value holds, or `None` when it is not a float value.
	pub fn width(&self) -> Option<usize> {
		width(self.data_type)
	}
}

/// Returns the BESL type that holds a MaterialX type, or `None` when no BESL type does.
///
/// Colours and vectors of the same width share one BESL type, because BESL draws no distinction
/// between them and MaterialX only uses it to pick node overloads.
pub(super) fn besl_type(data_type: DataType<'_>) -> Option<&'static str> {
	Some(match data_type {
		DataType::Float => "f32",
		DataType::Integer => "i32",
		// A MaterialX boolean is only ever compared, and comparing it as a number needs no conversion.
		DataType::Boolean => "f32",
		DataType::Vector2 => "vec2f",
		DataType::Color3 | DataType::Vector3 => "vec3f",
		DataType::Color4 | DataType::Vector4 => "vec4f",
		DataType::Matrix44 => "mat4f",
		_ => return None,
	})
}

/// Returns how many float components a MaterialX type holds, or `None` when it holds none.
///
/// Only the types a shading network computes with have a width; matrices, strings and closures do
/// not, because no node lowers them component by component. A boolean counts as one component,
/// because it lowers to the number a comparison produces.
pub(super) fn width(data_type: DataType<'_>) -> Option<usize> {
	Some(match data_type {
		DataType::Float | DataType::Boolean => 1,
		DataType::Vector2 => 2,
		DataType::Color3 | DataType::Vector3 => 3,
		DataType::Color4 | DataType::Vector4 => 4,
		_ => return None,
	})
}

/// Returns the float type of the given width.
pub(super) fn float_type(width: usize) -> &'static str {
	match width {
		1 => "f32",
		2 => "vec2f",
		3 => "vec3f",
		_ => "vec4f",
	}
}

/// Returns the MaterialX type a float value of the given width lowers through.
///
/// Widths pick a vector type rather than a colour type; the two share a BESL type, so the choice
/// only matters to the node overloads that read the result back.
pub(super) fn float_data_type<'a>(width: usize) -> DataType<'a> {
	match width {
		1 => DataType::Float,
		2 => DataType::Vector2,
		3 => DataType::Vector3,
		_ => DataType::Vector4,
	}
}

const COMPONENT_NAMES: [&str; 4] = ["x", "y", "z", "w"];

/// Reads one float component out of a value.
///
/// A one-component value is its own only component, so it is returned untouched rather than
/// accessed through `.x`, which BESL does not define for `f32`.
pub(super) fn component<'a>(value: &Node<'a>, index: usize, width: usize) -> Node<'a> {
	if width <= 1 {
		return value.clone();
	}

	Node::accessor(value.clone(), Node::member_expression(COMPONENT_NAMES[index]))
}

/// Assembles float components back into a value of their combined width.
pub(super) fn construct<'a>(mut components: Vec<Node<'a>>) -> Node<'a> {
	if components.len() == 1 {
		return components.remove(0);
	}

	Node::call(float_type(components.len()), components)
}

/// Writes a float as a BESL literal.
///
/// BESL literals hold digits and one optional point, so this expands the exponent that Rust's
/// shortest representation reaches for on very small and very large values. It never writes a sign,
/// because BESL has no unary minus; [`signed_literal`] subtracts from zero instead.
pub(super) fn float_literal(value: f32) -> String {
	// A shading network has no meaningful infinite or undefined value, so fold both into a finite one.
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
pub(super) fn signed_literal<'a>(value: f32) -> Node<'a> {
	let literal = Node::literal_expression(float_literal(value));

	if value.is_sign_negative() && value != 0.0 {
		// BESL has no unary minus, so a negative value is written as a subtraction.
		return Node::operator("-", Node::literal_expression("0.0"), literal);
	}

	literal
}

/// Writes a float value of the given width, repeating one component across every lane.
pub(super) fn splat<'a>(value: f32, width: usize) -> Node<'a> {
	construct((0..width).map(|_| signed_literal(value)).collect())
}

/// Writes a call to a BESL intrinsic or constructor.
pub(super) fn call<'a>(name: &'static str, arguments: Vec<Node<'a>>) -> Node<'a> {
	Node::call(name, arguments)
}

/// Writes a value of the given width, repeating one expression across every lane.
///
/// The expression is repeated verbatim, so it has to be a name or a literal; bind it to a local
/// first when it is anything else.
pub(super) fn splat_of<'a>(value: &Node<'a>, width: usize) -> Node<'a> {
	construct((0..width).map(|_| value.clone()).collect())
}
