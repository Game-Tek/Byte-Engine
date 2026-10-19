//! Runs lowered BEMX graphs in the BESL VM and checks the values they compute.

use besl::parser::Node;
use besl::vm::{Buffer, DescriptorBindings, ExecutableProgram, ResourceSlot, Texture, Value, output_slot};

use super::{Geometry, LowerError, Lowering, Stage};
use crate::bemx::{self, NodeId, Type};

/// The texel the `albedo` test texture holds everywhere.
const ALBEDO: [f32; 4] = [0.2, 0.4, 0.6, 0.8];

/// The `FixedPoint` struct is a stage whose shaded point has fixed geometry, one `albedo` texture, and one `half`
/// parameter.
///
/// The frame is the identity: the tangent is X, the bitangent Y, and the normal and view direction Z.
struct FixedPoint;

impl<'a> Stage<'a> for FixedPoint {
	fn geometry(&mut self, geometry: Geometry) -> Node<'a> {
		match geometry {
			Geometry::Position => vector(&["1.0", "2.0", "3.0"]),
			Geometry::Normal | Geometry::View => vector(&["0.0", "0.0", "1.0"]),
			Geometry::Tangent => vector(&["1.0", "0.0", "0.0"]),
			Geometry::Bitangent => vector(&["0.0", "1.0", "0.0"]),
			Geometry::TexCoord => vector(&["0.25", "0.75"]),
		}
	}

	fn sample(&mut self, texture: &'a str) -> Result<Node<'a>, LowerError> {
		if texture != "albedo" {
			return Err(LowerError::UnknownTexture {
				name: texture.to_string(),
			});
		}

		Ok(Node::call(
			"sample",
			vec![Node::member_expression("albedo"), vector(&["0.5", "0.5"])],
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
		Node::binding("albedo", Node::combined_image_sampler(), 1, true, false),
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
		descriptors.bind_texture(ResourceSlot::new(1), &mut albedo);

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
	assert!(matches!(
		failure(
			&surface_with(
				r#"{ name: "uv", node: "texcoord" }, { name: "tiled", node: "multiply", in1: "uv", in2: 4.0 },
				{ name: "image", node: "image", texture: "albedo", texcoord: "tiled" }, { name: "rough", node: "extract", in: "image" },"#
			),
			roughness
		),
		LowerError::UnsupportedTextureCoordinates { .. }
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
