//! Runs lowered BEMX graphs in the BESL VM and checks the values they compute.

use besl::parser::Node;
use besl::vm::{Buffer, DescriptorBindings, ExecutableProgram, ResourceSlot, Texture, Value, output_slot};

use super::{Backdrop, Coordinates, Input, LowerError, Lowering, ScreenAxis, Stage};
use crate::bemx::{self, NodeId, Type};

/// The texel the `albedo` test texture holds everywhere.
const ALBEDO: [f32; 4] = [0.2, 0.4, 0.6, 0.8];

/// The `FixedPoint` struct is a stage whose shaded point has fixed geometry, one `albedo` texture, and one `half`
/// parameter.
///
/// The frame is the identity: the tangent is X, the bitangent Y, and the normal and view direction Z. The time is 4,
/// and the shaded point lies at (0.4, 0.6) on the screen, which moves by 0.001 along x and 0.002 along y across a pixel.
/// The backdrop at screen position (u, v) has the colour (u, v, 0.5) and the position (u, 9, v). Across one pixel
/// along x, the position moves by (0.1, 0, 0.2) and the texture coordinates by (0.01, 0); along y, by (0, 0.3, 0.4)
/// and (0, 0.02).
///
/// A sample of `albedo` reads its texel. A sample of `probe` instead reports where it was asked to sample, as
/// (coordinate x, coordinate y, change of x along x, change of y along y), so a test reads back the coordinate and the
/// differences the lowering handed the stage.
struct FixedPoint;

impl<'a> Stage<'a> for FixedPoint {
	fn input(&mut self, input: Input) -> Option<Node<'a>> {
		Some(match input {
			Input::Position => vector(&["1.0", "2.0", "3.0"]),
			Input::Normal | Input::View => vector(&["0.0", "0.0", "1.0"]),
			Input::Tangent => vector(&["1.0", "0.0", "0.0"]),
			Input::Bitangent => vector(&["0.0", "1.0", "0.0"]),
			Input::TexCoord => vector(&["0.25", "0.75"]),
			Input::Time => Node::literal_expression("4.0"),
			Input::ScreenPosition => vector(&["0.4", "0.6"]),
		})
	}

	fn difference(&mut self, input: Input, axis: ScreenAxis) -> Option<Node<'a>> {
		match (input, axis) {
			(Input::Position, ScreenAxis::X) => Some(vector(&["0.1", "0.0", "0.2"])),
			(Input::Position, ScreenAxis::Y) => Some(vector(&["0.0", "0.3", "0.4"])),
			(Input::TexCoord, ScreenAxis::X) => Some(vector(&["0.01", "0.0"])),
			(Input::TexCoord, ScreenAxis::Y) => Some(vector(&["0.0", "0.02"])),
			(Input::ScreenPosition, ScreenAxis::X) => Some(vector(&["0.001", "0.0"])),
			(Input::ScreenPosition, ScreenAxis::Y) => Some(vector(&["0.0", "0.002"])),
			_ => None,
		}
	}

	fn backdrop(&mut self, backdrop: Backdrop, at: Node<'a>) -> Option<Node<'a>> {
		let lane = |lane: &'static str| Node::accessor(at.clone(), Node::member_expression(lane));

		Some(Node::call(
			"vec3f",
			match backdrop {
				Backdrop::Color => vec![lane("x"), lane("y"), Node::literal_expression("0.5")],
				Backdrop::Position => vec![lane("x"), Node::literal_expression("9.0"), lane("y")],
			},
		))
	}

	fn sample(&mut self, texture: &'a str, coordinates: Coordinates<'a>) -> Result<Node<'a>, LowerError> {
		match texture {
			"albedo" => return Ok(Node::call("sample", vec![Node::member_expression("albedo"), coordinates.value])),
			"probe" => {}
			_ => {
				return Err(LowerError::UnknownTexture {
					name: texture.to_string(),
				});
			}
		}

		let Coordinates { value, differences } = coordinates;
		let [along_x, along_y] = differences.expect("The fixed point carries differences.");
		let lane = |value: &Node<'a>, lane: &'static str| Node::accessor(value.clone(), Node::member_expression(lane));

		Ok(Node::call(
			"vec4f",
			vec![lane(&value, "x"), lane(&value, "y"), lane(&along_x, "x"), lane(&along_y, "y")],
		))
	}

	fn parameter(&mut self, name: &'a str, r#type: Type) -> Result<Node<'a>, LowerError> {
		if name != "half" || r#type != Type::Float {
			return Err(LowerError::UnknownParameter {
				name: name.to_string(),
				r#type,
			});
		}

		Ok(Node::literal_expression("0.5"))
	}
}

/// Writes a float vector of literal components.
fn vector<'a>(components: &[&'static str]) -> Node<'a> {
	let constructor = ["", "", "vec2f", "vec3f"][components.len()];

	Node::call(
		constructor,
		components
			.iter()
			.map(|component| Node::literal_expression(*component))
			.collect(),
	)
}

/// Lowers the base colour and roughness of the surface `source` describes, runs them in the VM, and returns them.
fn evaluate(source: &str) -> ([f32; 3], f32) {
	let graph = bemx::json::parse(source).unwrap_or_else(|error| panic!("Invalid test graph: {error}"));
	let surface = *graph.surface();
	let mut lowering = Lowering::new(&graph, FixedPoint);
	let base_color = lowering
		.value(surface.base_color.expect("Test graphs write a base colour."))
		.unwrap_or_else(|error| panic!("{error}"));
	let roughness = lowering
		.value(surface.roughness.expect("Test graphs write a roughness."))
		.unwrap_or_else(|error| panic!("{error}"));
	let (mut statements, _) = lowering.finish();

	statements.push(Node::member_assignment("base_color", base_color.syntax));
	statements.push(Node::member_assignment("roughness", roughness.syntax));

	let mut program = Node::root_with_children(vec![Node::main_function(statements)]);
	program.add(vec![
		Node::binding("albedo", Node::combined_image_sampler(), 0, true, false),
		Node::output("base_color", "vec3f", 0),
		Node::output("roughness", "f32", 1),
	]);

	let program = besl::lex(program).expect("Failed to link a lowered graph. The most likely cause is invalid lowered BESL.");
	let executable = ExecutableProgram::compile(program)
		.expect("Failed to compile a lowered graph for the VM. The most likely cause is missing VM intrinsic support.");
	let mut outputs: [Buffer; 2] = std::array::from_fn(|location| {
		Buffer::new(
			executable
				.output_layout(location as u8)
				.expect("Every output is assigned.")
				.clone(),
		)
	});
	let mut albedo = Texture::new(1, 1).expect("A one-texel texture is valid.");
	albedo.write([0, 0], ALBEDO).expect("The texel is inside the texture.");

	{
		let mut descriptors = DescriptorBindings::new();
		descriptors.bind_texture(ResourceSlot::new(0), &mut albedo);

		for (location, output) in outputs.iter_mut().enumerate() {
			descriptors.bind_buffer(output_slot(location as u8), output);
		}

		executable
			.run_main(&mut descriptors)
			.expect("Failed to run a lowered graph. The most likely cause is incomplete BESL VM support.");
	}

	let Value::Vec3F(base_color) = outputs[0].read("base_color").expect("The base colour is an output.") else {
		panic!("The base colour output is a vec3f.");
	};

	(
		base_color,
		outputs[1].read_f32("roughness").expect("The roughness is an output."),
	)
}

/// Wraps nodes, followed by a surface that reads `base` as its base colour and `rough` as its roughness.
fn surface(nodes: &str) -> String {
	format!(r#"{{ nodes: [{nodes} {{ name: "surface", node: "surface", base_color: "base", roughness: "rough" }}] }}"#)
}

#[track_caller]
fn assert_close(actual: ([f32; 3], f32), expected: ([f32; 3], f32)) {
	let close = |left: f32, right: f32| (left - right).abs() < 1.0e-4;

	assert!(
		actual.0.iter().zip(expected.0).all(|(left, right)| close(*left, right)) && close(actual.1, expected.1),
		"evaluated {actual:?}, expected {expected:?}"
	);
}

#[test]
fn arithmetic_applies_a_float_to_every_component() {
	let values = evaluate(&surface(
		r#"{ name: "base", node: "multiply", in1: [0.5, 1.0, 2.0], in2: 2.0 },
		{ name: "rough", node: "subtract", in1: 1.0, in2: 0.25 },"#,
	));

	assert_close(values, ([1.0, 2.0, 4.0], 0.75));
}

#[test]
fn modulo_keeps_the_sign_of_the_divisor() {
	let values = evaluate(&surface(
		r#"{ name: "base", node: "modulo", in1: [5.5, -0.5, 2.0], in2: 2.0 },
		{ name: "rough", node: "modulo", in1: -1.5, in2: 1.0 },"#,
	));

	assert_close(values, ([1.5, 1.5, 0.0], 0.5));
}

#[test]
fn images_sample_their_texture_once_and_keep_their_channels() {
	let values = evaluate(&surface(
		r#"{ name: "base", node: "image", texture: "albedo", type: "color3" },
		{ name: "rgba", node: "image", texture: "albedo" },
		{ name: "rough", node: "extract", in: "rgba", index: 3 },"#,
	));

	assert_close(values, ([ALBEDO[0], ALBEDO[1], ALBEDO[2]], ALBEDO[3]));
}

#[test]
fn channel_nodes_pick_join_and_replace_components() {
	let values = evaluate(&surface(
		r#"{ name: "base", node: "swizzle", in: [1.0, 2.0, 3.0], channels: "z1x" },
		{ name: "joined", node: "combine", in1: [0.1, 0.2], in2: 0.3 },
		{ name: "rough", node: "extract", in: "joined", index: 2 },"#,
	));

	assert_close(values, ([3.0, 1.0, 1.0], 0.3));
}

#[test]
fn conversions_fill_and_drop_components() {
	let values = evaluate(&surface(
		r#"{ name: "base", node: "convert", in: 0.5, type: "vector3" },
		{ name: "opened", node: "convert", in: [0.1, 0.2], type: "vector4" },
		{ name: "rough", node: "extract", in: "opened", index: 3 },"#,
	));

	assert_close(values, ([0.5, 0.5, 0.5], 1.0));
}

#[test]
fn comparisons_and_switches_pick_one_value() {
	let values = evaluate(&surface(
		r#"{ name: "base", node: "switch", which: 1.0, in1: [1.0, 0.0, 0.0], in2: [0.0, 1.0, 0.0], in3: [0.0, 0.0, 1.0] },
		{ name: "rough", node: "ifgreater", value1: 0.6, value2: 0.5, in1: 0.9, in2: 0.1 },"#,
	));

	assert_close(values, ([0.0, 1.0, 0.0], 0.9));

	let values = evaluate(&surface(
		r#"{ name: "base", node: "ifequal", value1: 0.5, value2: 0.5, in1: [1.0, 1.0, 1.0], in2: [0.0, 0.0, 0.0] },
		{ name: "rough", node: "ifgreatereq", value1: 0.4, value2: 0.5, in1: 0.9, in2: 0.1 },"#,
	));

	assert_close(values, ([1.0, 1.0, 1.0], 0.1));
}

#[test]
fn blends_and_ranges_follow_materialx() {
	let values = evaluate(&surface(
		r#"{ name: "base", node: "mix", bg: [0.0, 0.0, 0.0], fg: [1.0, 1.0, 1.0], mix: 0.25 },
		{ name: "rough", node: "remap", in: 0.5, outlow: 0.2, outhigh: 0.4 },"#,
	));

	assert_close(values, ([0.25, 0.25, 0.25], 0.3));

	let values = evaluate(&surface(
		r#"{ name: "base", node: "clamp", in: [-1.0, 0.5, 2.0] },
		{ name: "rough", node: "smoothstep", in: 0.5 },"#,
	));

	assert_close(values, ([0.0, 0.5, 1.0], 0.5));
}

#[test]
fn scalar_functions_apply_to_every_component() {
	let values = evaluate(&surface(
		r#"{ name: "floor", node: "floor", in: 1.5 },
		{ name: "ceil", node: "ceil", in: 1.2 },
		{ name: "sign", node: "sign", in: -3.0 },
		{ name: "base", node: "combine", in1: "floor", in2: "ceil", in3: "sign" },
		{ name: "rough", node: "acos", in: 0.0 },"#,
	));

	assert_close(values, ([1.0, 2.0, -1.0], std::f32::consts::FRAC_PI_2));
}

#[test]
fn vector_functions_measure_and_scale() {
	let values = evaluate(&surface(
		r#"{ name: "base", node: "normalize", in: [3.0, 0.0, 4.0] },
		{ name: "rough", node: "magnitude", in: [3.0, 4.0] },"#,
	));

	assert_close(values, ([0.6, 0.0, 0.8], 5.0));

	let values = evaluate(&surface(
		r#"{ name: "base", node: "crossproduct", in1: [1.0, 0.0, 0.0], in2: [0.0, 1.0, 0.0] },
		{ name: "rough", node: "dotproduct", in1: [1.0, 2.0, 3.0], in2: [1.0, 1.0, 1.0] },"#,
	));

	assert_close(values, ([0.0, 0.0, 1.0], 6.0));
}

#[test]
fn colour_adjustments_keep_alpha_apart() {
	let values = evaluate(&surface(
		r#"{ name: "premultiplied", node: "premult", in: [0.5, 1.0, 1.0, 0.5] },
		{ name: "base", node: "swizzle", in: "premultiplied", channels: "rgb" },
		{ name: "grey", node: "luminance", in: [1.0, 1.0, 1.0] },
		{ name: "rough", node: "extract", in: "grey", index: 0 },"#,
	));

	assert_close(values, ([0.25, 0.5, 0.5], bemx::DEFAULT_LUMINANCE_WEIGHTS.iter().sum()));
}

/// A normal map rebuilds its height from the first two channels and turns the result out of the tangent frame.
#[test]
fn normal_maps_decode_into_world_space() {
	let values = evaluate(&surface(
		r#"{ name: "base", node: "normalmap", in: [0.75, 0.5, 0.0] },
		{ name: "rough", node: "magnitude", in: "base" },"#,
	));

	// The tangent-plane components decode to (0.5, 0); the height is rebuilt as sqrt(1 - 0.25).
	assert_close(values, ([0.5, 0.0, 0.75f32.sqrt()], 1.0));
}

#[test]
fn normal_maps_turn_out_of_the_frame_they_are_given() {
	let values = evaluate(&surface(
		r#"{ name: "base", node: "normalmap", in: [0.5, 1.0, 1.0], tangent: [1.0, 0.0, 0.0], bitangent: [0.0, 0.0, -1.0], normal: [0.0, 1.0, 0.0] },
		{ name: "rough", node: "extract", in: "base", index: 2 },"#,
	));

	// The texel points straight along the bitangent, which this frame lays along negative z.
	assert_close(values, ([0.0, 0.0, -1.0], -1.0));
}

#[test]
fn images_sample_at_computed_coordinates_with_how_they_change_across_the_pixel() {
	let values = evaluate(&surface(
		r#"{ name: "position", node: "position" },
		{ name: "ground", node: "swizzle", in: "position", channels: "xz" },
		{ name: "tiled", node: "multiply", in1: "ground", in2: 2.0 },
		{ name: "image", node: "image", texture: "probe", texcoord: "tiled" },
		{ name: "base", node: "swizzle", in: "image", channels: "xyz" },
		{ name: "rough", node: "extract", in: "image", index: 3 },"#,
	));

	// The position (1, 2, 3) maps to (2, 6). Along x the position moves by (0.1, 0, 0.2), so x changes by 0.2; along y it
	// moves by (0, 0.3, 0.4), so y changes by 0.8.
	assert_close(values, ([2.0, 6.0, 0.2], 0.8));
}

#[test]
fn texels_and_parameters_hold_still_across_the_pixel() {
	let flowed = evaluate(&surface(
		r#"{ name: "flow", node: "image", texture: "albedo", type: "vector2" },
		{ name: "uv", node: "texcoord" },
		{ name: "half", node: "parameter", parameter: "half", type: "float" },
		{ name: "scaled", node: "multiply", in1: "uv", in2: 4.0 },
		{ name: "offset", node: "add", in1: "scaled", in2: "half" },
		{ name: "flowed", node: "add", in1: "offset", in2: "flow" },
		{ name: "image", node: "image", texture: "probe", texcoord: "flowed" },
		{ name: "base", node: "swizzle", in: "image", channels: "xyz" },
		{ name: "rough", node: "extract", in: "image", index: 3 },"#,
	));

	// (0.25, 0.75) × 4 + 0.5 + the texel's (0.2, 0.4). The texel and the parameter hold still, so only the texture
	// coordinates' change of (0.01, 0.02) counts, four times over.
	assert_close(flowed, ([1.7, 3.9, 0.04], 0.08));

	let still = evaluate(&surface(
		r#"{ name: "half", node: "parameter", parameter: "half", type: "float" },
		{ name: "at", node: "combine", in1: "half", in2: 0.25 },
		{ name: "image", node: "image", texture: "probe", texcoord: "at" },
		{ name: "base", node: "swizzle", in: "image", channels: "xyz" },
		{ name: "rough", node: "extract", in: "image", index: 3 },"#,
	));

	assert_close(still, ([0.5, 0.25, 0.0], 0.0));
}

#[test]
fn time_comes_from_the_stage_and_holds_still_across_the_pixel() {
	let values = evaluate(&surface(
		r#"{ name: "time", node: "time" },
		{ name: "uv", node: "texcoord" },
		{ name: "scrolled", node: "add", in1: "uv", in2: "time" },
		{ name: "image", node: "image", texture: "probe", texcoord: "scrolled" },
		{ name: "base", node: "swizzle", in: "image", channels: "xyz" },
		{ name: "rough", node: "extract", in: "image", index: 3 },"#,
	));

	// (0.25, 0.75) scrolled by 4 seconds, and only the texture coordinates change across the pixel.
	assert_close(values, ([4.25, 4.75, 0.01], 0.02));
}

#[test]
fn the_backdrop_reads_at_the_shaded_point_or_where_the_graph_asks() {
	let values = evaluate(&surface(
		r#"{ name: "behind", node: "scenecolor" },
		{ name: "base", node: "swizzle", in: "behind", channels: "xyz" },
		{ name: "screen", node: "screenposition" },
		{ name: "shifted", node: "add", in1: "screen", in2: [0.1, 0.0] },
		{ name: "floor", node: "sceneposition", texcoord: "shifted" },
		{ name: "rough", node: "extract", in: "floor", index: 0 },"#,
	));

	// The colour reads at the shaded point's (0.4, 0.6); the position at (0.5, 0.6), whose x it keeps.
	assert_close(values, ([0.4, 0.6, 0.5], 0.5));
}

#[test]
fn screen_positions_change_by_a_pixel_across_the_pixel() {
	let values = evaluate(&surface(
		r#"{ name: "screen", node: "screenposition" },
		{ name: "tiled", node: "multiply", in1: "screen", in2: 3.0 },
		{ name: "image", node: "image", texture: "probe", texcoord: "tiled" },
		{ name: "base", node: "swizzle", in: "image", channels: "xyz" },
		{ name: "rough", node: "extract", in: "image", index: 3 },"#,
	));

	// (0.4, 0.6) × 3, changing by three pixels' 0.001 along x and 0.002 along y.
	assert_close(values, ([1.2, 1.8, 0.003], 0.006));
}

#[test]
fn geometry_and_parameters_come_from_the_stage() {
	let values = evaluate(&surface(
		r#"{ name: "base", node: "viewdirection" },
		{ name: "half", node: "parameter", parameter: "half", type: "float" },
		{ name: "uv", node: "texcoord" },
		{ name: "uv_sum", node: "dotproduct", in1: "uv", in2: [1.0, 1.0] },
		{ name: "rough", node: "add", in1: "half", in2: "uv_sum" },"#,
	));

	// MaterialX's view direction points from the viewer toward the surface, against the stage's view vector.
	assert_close(values, ([0.0, 0.0, -1.0], 1.5));
}

/// Lowers `node` of the surface the graph `source` describes and returns the failure.
fn failure(source: &str, input: impl Fn(&bemx::Surface) -> Option<NodeId>) -> LowerError {
	let graph = bemx::json::parse(source).unwrap_or_else(|error| panic!("Invalid test graph: {error}"));
	let node = input(graph.surface()).expect("The test graph writes the input.");

	Lowering::new(&graph, FixedPoint)
		.value(node)
		.expect_err("The value should not lower.")
}

#[test]
fn what_the_stage_cannot_supply_is_reported() {
	let surface_with =
		|nodes: &str| format!(r#"{{ nodes: [{nodes} {{ name: "surface", node: "surface", roughness: "rough" }}] }}"#);
	let roughness = |surface: &bemx::Surface| surface.roughness;

	assert!(matches!(
		failure(
			&surface_with(r#"{ name: "uv", node: "texcoord", index: 1 }, { name: "rough", node: "extract", in: "uv" },"#),
			roughness
		),
		LowerError::UnsupportedTexCoord { index: 1, .. }
	));
	assert_eq!(
		failure(
			&surface_with(r#"{ name: "rough", node: "parameter", parameter: "gloss", type: "float" },"#),
			roughness
		),
		LowerError::UnknownParameter {
			name: "gloss".to_string(),
			r#type: Type::Float,
		}
	);
	assert_eq!(
		failure(
			&surface_with(
				r#"{ name: "image", node: "image", texture: "detail" }, { name: "rough", node: "extract", in: "image" },"#
			),
			roughness
		),
		LowerError::UnknownTexture {
			name: "detail".to_string(),
		}
	);
}
