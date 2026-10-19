//! Turns one MaterialX node category into the BEMX node that computes it.

use super::Texture;
use super::conversion::{Converter, bemx_type};
use super::error::ConvertError;
use crate::bemx::{self, BinaryOperator, Channel, Comparison, DEFAULT_LUMINANCE_WEIGHTS, Node, Type, UnaryOperator};
use crate::materialx::{Input, Node as Instance, NodeId, PortIndex, Source, Value};

/// The geometric properties the material stage carries texture coordinate set zero under.
pub(super) const TEXTURE_COORDINATES: [&str; 3] = ["UV0", "st", "uv"];

/// Converts one output of one node.
///
/// A category this module knows maps onto its BEMX node directly. Anything else falls back to the
/// node graph that implements the node's declaration, which is how a document extends the standard
/// library with graphs of its own.
pub(super) fn convert<'a>(
	converter: &mut Converter<'a, '_>,
	frame: usize,
	node: NodeId,
	output: PortIndex,
) -> Result<bemx::NodeId, ConvertError> {
	let instance = converter.dag.node(node);

	if let Some(value) = builtin(converter, frame, node, instance, output)? {
		return Ok(value);
	}

	if let Some(graph) = converter.implementation(instance.declaration) {
		let expansion = converter.expansion(frame, node, graph)?;

		return converter.graph_result(expansion, graph, output);
	}

	Err(ConvertError::UnsupportedNode {
		node: instance.name.to_string(),
		category: instance.category.to_string(),
	})
}

/// Converts the node categories BEMX has a node for, or returns nothing for any other category.
// One category per arm keeps the whole supported surface readable in a single place.
#[allow(clippy::too_many_lines)]
fn builtin<'a>(
	converter: &mut Converter<'a, '_>,
	frame: usize,
	node: NodeId,
	instance: &Instance<'a>,
	output: PortIndex,
) -> Result<Option<bemx::NodeId>, ConvertError> {
	let name = instance.name;
	let category = instance.category;
	let data_type = instance
		.outputs
		.get(output.index())
		.map_or(instance.data_type, |port| port.data_type);
	// Only categories BEMX knows need a BEMX type; a document's own category may carry any type its graph produces.
	let result = || bemx_type(data_type, name);

	// Arithmetic, component-wise functions and comparisons share their names and defaults with the JSON form.
	if let Some(operator) = BinaryOperator::from_category(category) {
		let result = result()?;
		let left = operand(converter, frame, node, "in1", 0.0, result)?;
		let right = operand(converter, frame, node, "in2", operator.default_right(), result)?;

		return converter.add(name, Node::Binary { operator, left, right }).map(Some);
	}

	if let Some(operator) = UnaryOperator::from_category(category) {
		// A logarithm of one is zero, which keeps an unwritten input well defined.
		let default = f32::from(operator == UnaryOperator::Logarithm);
		let value = operand(converter, frame, node, "in", default, result()?)?;

		return converter.add(name, Node::Unary { operator, value }).map(Some);
	}

	if let Some(comparison) = Comparison::from_category(category) {
		let result = result()?;
		let left = operand(converter, frame, node, "value1", Comparison::DEFAULT_LEFT, Type::Float)?;
		let right = operand(converter, frame, node, "value2", 0.0, Type::Float)?;
		let then = operand(converter, frame, node, "in1", 0.0, result)?;
		let otherwise = operand(converter, frame, node, "in2", 0.0, result)?;

		return converter
			.add(
				name,
				Node::Compare {
					comparison,
					left,
					right,
					then,
					otherwise,
				},
			)
			.map(Some);
	}

	let value = match category {
		// Values and structure.
		"constant" => operand(converter, frame, node, "value", 0.0, result()?)?,
		"dot" => operand(converter, frame, node, "in", 0.0, result()?)?,
		"convert" => {
			let result = result()?;
			let value = operand(converter, frame, node, "in", 0.0, result)?;

			cast(converter, name, value, result)?
		}
		"combine2" | "combine3" | "combine4" => combine(converter, frame, node, name, result()?)?,
		"separate2" | "separate3" | "separate4" => {
			let value = operand(converter, frame, node, "in", 0.0, Type::Vector4)?;

			separate(converter, name, value, instance, output)?
		}
		"extract" => {
			let value = operand(converter, frame, node, "in", 0.0, Type::Vector4)?;
			let width = converter.r#type(value).width();
			let index = integer_parameter(converter, frame, node, "index").unwrap_or(0).max(0) as usize;

			extract(converter, name, value, index.min(width - 1))?
		}
		"swizzle" => {
			let value = operand(converter, frame, node, "in", 0.0, Type::Vector4)?;
			let channels = string_parameter(converter, frame, node, "channels").unwrap_or("x");

			swizzle(converter, name, value, channels, result()?)?
		}

		// Geometry.
		"position" => converter.geometry("Pworld", name)?,
		"normal" => converter.geometry("Nworld", name)?,
		"tangent" => converter.geometry("Tworld", name)?,
		"bitangent" => converter.geometry("Bworld", name)?,
		// MaterialX and BEMX both point a view direction from the viewer at the surface.
		"viewdirection" => converter.view_direction(name)?,
		"texcoord" => {
			let index = integer_parameter(converter, frame, node, "index").unwrap_or(0);

			match u32::try_from(index) {
				Ok(0) => converter.geometry("UV0", name)?,
				// BEMX describes every coordinate set; a renderer that carries set zero alone reports the others.
				Ok(index) => converter.add(name, Node::TexCoord { index })?,
				Err(_) => {
					return Err(ConvertError::UnknownGeometricProperty {
						node: name.to_string(),
						property: format!("UV{index}"),
					});
				}
			}
		}
		"geompropvalue" => {
			let property = string_parameter(converter, frame, node, "geomprop").unwrap_or("");

			converter.geometry(property, name)?
		}

		// Textures.
		"image" => image(converter, frame, node, instance, result()?)?,

		"clamp" | "smoothstep" => {
			let value = operand(converter, frame, node, "in", 0.0, result()?)?;
			let low = operand(converter, frame, node, "low", 0.0, Type::Float)?;
			let high = operand(converter, frame, node, "high", 1.0, Type::Float)?;

			converter.add(
				name,
				match category {
					"clamp" => Node::Clamp { value, low, high },
					_ => Node::Smoothstep { value, low, high },
				},
			)?
		}

		// Vector functions.
		"magnitude" => {
			let value = operand(converter, frame, node, "in", 0.0, Type::Vector3)?;

			converter.add(name, Node::Magnitude { value })?
		}
		"dotproduct" | "crossproduct" => {
			let left = operand(converter, frame, node, "in1", 0.0, Type::Vector3)?;
			let right = operand(converter, frame, node, "in2", 0.0, Type::Vector3)?;

			converter.add(
				name,
				match category {
					"dotproduct" => Node::DotProduct { left, right },
					_ => Node::CrossProduct { left, right },
				},
			)?
		}

		// Blends and adjustments.
		"mix" => {
			let result = result()?;
			let foreground = operand(converter, frame, node, "fg", 0.0, result)?;
			let background = operand(converter, frame, node, "bg", 0.0, result)?;
			let factor = operand(converter, frame, node, "mix", 0.0, Type::Float)?;

			converter.add(
				name,
				Node::Mix {
					background,
					foreground,
					factor,
				},
			)?
		}
		"remap" => {
			let value = operand(converter, frame, node, "in", 0.0, result()?)?;
			let input_low = operand(converter, frame, node, "inlow", 0.0, Type::Float)?;
			let input_high = operand(converter, frame, node, "inhigh", 1.0, Type::Float)?;
			let output_low = operand(converter, frame, node, "outlow", 0.0, Type::Float)?;
			let output_high = operand(converter, frame, node, "outhigh", 1.0, Type::Float)?;

			converter.add(
				name,
				Node::Remap {
					value,
					input_low,
					input_high,
					output_low,
					output_high,
				},
			)?
		}
		"invert" => {
			let result = result()?;
			let value = operand(converter, frame, node, "in", 0.0, result)?;
			let amount = operand(converter, frame, node, "amount", 1.0, result)?;

			converter.add(name, Node::Invert { value, amount })?
		}
		"luminance" => {
			let value = operand(converter, frame, node, "in", 0.0, result()?)?;
			// The weights are a uniform input, so MaterialX only ever writes them as a constant or binds them to one.
			let weights = match converter.parameter(frame, node, "lumacoeffs") {
				Some(Value::Color3(weights) | Value::Vector3(weights)) => *weights,
				_ => DEFAULT_LUMINANCE_WEIGHTS,
			};

			converter.add(name, Node::Luminance { value, weights })?
		}
		"premult" | "unpremult" => {
			let result = result()?;
			let value = operand(converter, frame, node, "in", 0.0, result)?;

			if converter.r#type(value) == Type::Vector4 {
				converter.add(
					name,
					match category {
						"premult" => Node::Premultiply { value },
						_ => Node::Unpremultiply { value },
					},
				)?
			} else {
				// Without an alpha channel there is nothing to premultiply by.
				cast(converter, name, value, result)?
			}
		}
		"switch" => switch(converter, frame, node, name, result()?)?,

		// Shading.
		"normalmap" => {
			let value = operand(converter, frame, node, "in", 0.5, Type::Vector3)?;
			let scale = operand(converter, frame, node, "scale", 1.0, Type::Float)?;

			converter.add(name, Node::NormalMap { value, scale })?
		}

		_ => return Ok(None),
	};

	Ok(Some(value))
}

/// Reads one input of a node, or a constant of type `type` holding `default` when nothing writes it.
pub(super) fn operand<'a>(
	converter: &mut Converter<'a, '_>,
	frame: usize,
	node: NodeId,
	name: &str,
	default: f32,
	r#type: Type,
) -> Result<bemx::NodeId, ConvertError> {
	if let Some(value) = converter.input(frame, node, name)? {
		return Ok(value);
	}

	Ok(converter.splat(default, r#type))
}

/// Returns one input's constant as an integer.
fn integer_parameter(converter: &Converter<'_, '_>, frame: usize, node: NodeId, name: &str) -> Option<i32> {
	match converter.parameter(frame, node, name)? {
		Value::Integer(value) => Some(*value),
		Value::Float(value) => Some(*value as i32),
		_ => None,
	}
}

/// Returns one input's constant as text.
fn string_parameter<'a>(converter: &Converter<'a, '_>, frame: usize, node: NodeId, name: &str) -> Option<&'a str> {
	match converter.parameter(frame, node, name)? {
		Value::String(value) | Value::Filename(value) | Value::GeomName(value) | Value::Opaque(value) => Some(value),
		_ => None,
	}
}

/// Adds the value one of a switch node's numbered inputs carries.
///
/// BEMX switches read their values without gaps, so a numbered input the document leaves out before
/// the last written one becomes a zero, which is what MaterialX picks for an unwritten input.
fn switch<'a>(
	converter: &mut Converter<'a, '_>,
	frame: usize,
	node: NodeId,
	hint: &str,
	result: Type,
) -> Result<bemx::NodeId, ConvertError> {
	let which = operand(converter, frame, node, "which", 0.0, Type::Float)?;
	let mut values = Vec::with_capacity(bemx::SWITCH_INPUTS);
	let mut written = 0;

	for input in bemx::NUMBERED_INPUTS {
		match converter.input(frame, node, input)? {
			Some(value) => {
				values.push(value);
				written = values.len();
			}
			None => values.push(converter.splat(0.0, result)),
		}
	}

	// Past the last written input, BEMX's own out-of-range rule picks zero just as the gap fillers would.
	values.truncate(written.max(1));

	converter.add(hint, Node::Switch { which, values })
}

/// Adds a texture sample, recording the image so the renderer can bind it.
fn image<'a>(
	converter: &mut Converter<'a, '_>,
	frame: usize,
	node: NodeId,
	instance: &Instance<'a>,
	result: Type,
) -> Result<bemx::NodeId, ConvertError> {
	let name = instance.name;

	// Converted images sample with the mesh's own texture coordinates, so a transformed set cannot be honoured.
	if let Some(coordinates) = instance.input("texcoord")
		&& !samples_default_coordinates(converter, frame, coordinates)
	{
		return Err(ConvertError::UnsupportedTextureCoordinates { node: name.to_string() });
	}

	let file = string_parameter(converter, frame, node, "file").unwrap_or("");

	if file.is_empty() {
		return Err(ConvertError::MissingTextureFile { node: name.to_string() });
	}

	let written = instance.input("file");
	let texture = converter.texture(Texture {
		file,
		file_prefix: written.and_then(|input| input.file_prefix).or(instance.file_prefix),
		colorspace: written.and_then(|input| input.colorspace).or(instance.colorspace),
	});

	converter.add(
		name,
		Node::Image {
			texture,
			r#type: result,
			texcoord: None,
		},
	)
}

/// Reports whether an image's texture coordinates are the ones the material stage already carries.
fn samples_default_coordinates<'a>(converter: &Converter<'a, '_>, frame: usize, coordinates: &Input<'a>) -> bool {
	match &coordinates.source {
		Source::Unconnected => true,
		Source::GeomProp(property) => TEXTURE_COORDINATES.contains(property),
		Source::Node { node, .. } => match converter.dag.node(*node).category {
			"texcoord" => integer_parameter(converter, frame, *node, "index").unwrap_or(0) == 0,
			"geompropvalue" => string_parameter(converter, frame, *node, "geomprop")
				.is_some_and(|property| TEXTURE_COORDINATES.contains(&property)),
			_ => false,
		},
		_ => false,
	}
}

/// Returns a value converted to type `result`, adding a node only when the type changes.
///
/// A narrower value gains ones, which is how MaterialX opens a colour to full opacity and a vector to
/// a homogeneous coordinate, and a wider value drops the components the target has no room for. A
/// float target keeps the first component through an extraction, which is the same value and lets a
/// packed texture move the read.
pub(super) fn cast(
	converter: &mut Converter<'_, '_>,
	hint: &str,
	value: bemx::NodeId,
	result: Type,
) -> Result<bemx::NodeId, ConvertError> {
	if converter.r#type(value) == result {
		return Ok(value);
	}

	if result == Type::Float {
		return extract(converter, hint, value, 0);
	}

	converter.add(hint, Node::Convert { value, r#type: result })
}

/// Returns one component of a value, which is the value itself when it holds just one.
fn extract(
	converter: &mut Converter<'_, '_>,
	hint: &str,
	value: bemx::NodeId,
	index: usize,
) -> Result<bemx::NodeId, ConvertError> {
	if converter.r#type(value) == Type::Float {
		return Ok(value);
	}

	// A MaterialX value holds at most four components, so the index always fits.
	converter.add(
		hint,
		Node::Extract {
			value,
			index: index as u8,
		},
	)
}

/// Adds the components of several inputs joined into one value.
fn combine<'a>(
	converter: &mut Converter<'a, '_>,
	frame: usize,
	node: NodeId,
	hint: &str,
	result: Type,
) -> Result<bemx::NodeId, ConvertError> {
	let mut values = Vec::with_capacity(4);
	let mut width = 0;

	for input in &bemx::NUMBERED_INPUTS[..bemx::COMBINE_INPUTS] {
		let Some(part) = converter.input(frame, node, input)? else {
			continue;
		};

		width += converter.r#type(part).width();
		values.push(part);
	}

	// Components nothing writes are zero, as MaterialX's own inputs default to.
	while width < result.width() {
		values.push(converter.splat(0.0, Type::Float));
		width += 1;
	}

	let combined = converter.add(hint, Node::Combine { values })?;

	// A value written wider than the result loses its trailing components.
	cast(converter, hint, combined, result)
}

/// Returns the lane a MaterialX channel letter names.
fn lane(letter: char) -> Option<usize> {
	Channel::from_letter(letter).and_then(Channel::component)
}

/// Adds one channel of a value, chosen by which of a separate node's outputs is read.
fn separate<'a>(
	converter: &mut Converter<'a, '_>,
	hint: &str,
	value: bemx::NodeId,
	instance: &Instance<'a>,
	output: PortIndex,
) -> Result<bemx::NodeId, ConvertError> {
	let width = converter.r#type(value).width();
	let port = instance.outputs.get(output.index()).map_or("outx", |port| port.name);

	// Separate nodes name one output per channel, so the last letter of the name picks the lane.
	let index = port.chars().next_back().and_then(lane).unwrap_or(0);

	if index >= width {
		return Err(ConvertError::UnknownOutput {
			node: instance.name.to_string(),
			output: output.index() as u32,
		});
	}

	extract(converter, hint, value, index)
}

/// Adds a value whose components are picked, repeated or replaced by a channel string.
///
/// A letter past the value's last component reads its last one, and anything that names no channel
/// reads zero, as MaterialX's channel strings allow.
pub(super) fn swizzle(
	converter: &mut Converter<'_, '_>,
	hint: &str,
	value: bemx::NodeId,
	channels: &str,
	result: Type,
) -> Result<bemx::NodeId, ConvertError> {
	const LANES: [Channel; 4] = [Channel::X, Channel::Y, Channel::Z, Channel::W];

	let width = converter.r#type(value).width();

	let mut picked: Vec<Channel> = channels
		.chars()
		.map(|letter| match Channel::from_letter(letter) {
			Some(channel) => channel.component().map_or(channel, |lane| LANES[lane.min(width - 1)]),
			None => Channel::Zero,
		})
		.collect();

	if picked.is_empty() {
		picked.push(Channel::X);
	}

	let selected = match picked.as_slice() {
		// Reading one real channel is an extraction, which a packed texture can move.
		[channel] if channel.component().is_some() => extract(converter, hint, value, channel.component().unwrap_or(0))?,
		// Reading every channel in order is the value itself.
		lanes if lanes.len() == width && lanes.iter().enumerate().all(|(index, lane)| lane.component() == Some(index)) => value,
		_ => converter.add(hint, Node::Swizzle { value, channels: picked })?,
	};

	cast(converter, hint, selected, result)
}
