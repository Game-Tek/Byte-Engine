//! UI shader program construction and platform shader sources.

use super::*;

/// Lexes a complete UI shader scope and returns the entry point consumed by render pipeline creation.
pub(super) fn lex_ui_shader(root: ParserNode<'_>, shader_name: &str) -> besl::NodeReference {
	let root = besl::lex(root)
		.unwrap_or_else(|_| panic!("Failed to lex {shader_name}. The most likely cause is invalid BESL syntax."));
	root.get_main().unwrap_or_else(|| {
		panic!("Failed to find {shader_name} entry point. The most likely cause is a missing main function.")
	})
}

/// Builds the portable UI rectangle vertex program shared by VM tests and production backends.
pub(super) fn create_ui_vertex_program() -> besl::NodeReference {
	let member = ParserNode::member_expression;
	let forward = |output: &'static str, input: &'static str| ParserNode::member_assignment(output, member(input));

	// Express the portable vertex plumbing as BESL nodes so the VM and every backend execute the same program.
	let main = ParserNode::main_function(vec![
		ParserNode::member_assignment(
			"position",
			ParserNode::call(
				"vec4f",
				vec![
					ParserNode::accessor(member("in_position"), member("x")),
					ParserNode::accessor(member("in_position"), member("y")),
					ParserNode::literal_expression("0.0"),
					ParserNode::literal_expression("1.0"),
				],
			),
		),
		forward("out_color", "in_color"),
		forward("out_pixel_position", "in_pixel_position"),
		forward("out_local_position", "in_local_position"),
		forward("out_rect_size", "in_rect_size"),
		forward("out_corner_radius", "in_corner_radius"),
		forward("out_corner_exponent", "in_corner_exponent"),
		forward("out_layer_kind", "in_layer_kind"),
		forward("out_stroke_width", "in_stroke_width"),
		forward("out_feather_mask_position", "in_feather_mask_position"),
		forward("out_feather_mask_size", "in_feather_mask_size"),
		forward("out_feather_mask_edges", "in_feather_mask_edges"),
		forward("out_feather_mask_corner", "in_feather_mask_corner"),
		forward("out_blur_resolution_mix", "in_blur_resolution_mix"),
		ParserNode::member_assignment(
			"out_screen_uv",
			ParserNode::call(
				"vec2f",
				vec![
					ParserNode::operator(
						"+",
						ParserNode::operator(
							"*",
							ParserNode::accessor(member("in_position"), member("x")),
							ParserNode::literal_expression("0.5"),
						),
						ParserNode::literal_expression("0.5"),
					),
					ParserNode::operator(
						"-",
						ParserNode::literal_expression("0.5"),
						ParserNode::operator(
							"*",
							ParserNode::accessor(member("in_position"), member("y")),
							ParserNode::literal_expression("0.5"),
						),
					),
				],
			),
		),
	]);

	let shader_scope = ParserNode::scope(
		"Shader",
		vec![
			ParserNode::input("in_position", "vec2f", 0),
			ParserNode::input("in_pixel_position", "vec2f", 1),
			ParserNode::input("in_local_position", "vec2f", 2),
			ParserNode::input("in_rect_size", "vec2f", 3),
			ParserNode::input("in_color", "vec4f", 4),
			ParserNode::input("in_corner_radius", "f32", 5),
			ParserNode::input("in_corner_exponent", "f32", 6),
			ParserNode::input("in_layer_kind", "f32", 7),
			ParserNode::input("in_stroke_width", "f32", 8),
			ParserNode::input("in_feather_mask_position", "vec2f", 9),
			ParserNode::input("in_feather_mask_size", "vec2f", 10),
			ParserNode::input("in_feather_mask_edges", "vec4f", 11),
			ParserNode::input("in_feather_mask_corner", "vec2f", 12),
			ParserNode::input("in_blur_resolution_mix", "f32", 13),
			ParserNode::output("position", "vec4f", 0),
			ParserNode::output("out_color", "vec4f", 0),
			ParserNode::output("out_pixel_position", "vec2f", 1),
			ParserNode::output("out_local_position", "vec2f", 2),
			ParserNode::output("out_rect_size", "vec2f", 3),
			ParserNode::output("out_corner_radius", "f32", 4),
			ParserNode::output("out_corner_exponent", "f32", 5),
			ParserNode::output("out_layer_kind", "f32", 6),
			ParserNode::output("out_stroke_width", "f32", 7),
			ParserNode::output("out_feather_mask_position", "vec2f", 8),
			ParserNode::output("out_feather_mask_size", "vec2f", 9),
			ParserNode::output("out_feather_mask_edges", "vec4f", 10),
			ParserNode::output("out_feather_mask_corner", "vec2f", 11),
			ParserNode::output("out_screen_uv", "vec2f", 12),
			ParserNode::output("out_blur_resolution_mix", "f32", 13),
			main,
		],
	);
	let mut root = ParserNode::root();
	root.add(vec![shader_scope]);
	lex_ui_shader(root, "UI vertex shader")
}

/// Builds the portable UI rectangle fragment program shared by VM tests and production backends.
pub(super) fn create_ui_fragment_program() -> besl::NodeReference {
	let mut root = besl::Node::root();
	let vec4f = root.get_child("vec4f").expect("vec4f type not found in BESL root");
	let vec2f = root.get_child("vec2f").expect("vec2f type not found in BESL root");
	let f32 = root.get_child("f32").expect("f32 type not found in BESL root");

	root.add_child(besl::Node::input("in_color", vec4f.clone(), 0).into());
	root.add_child(besl::Node::input("in_pixel_position", vec2f.clone(), 1).into());
	root.add_child(besl::Node::input("in_local_position", vec2f.clone(), 2).into());
	root.add_child(besl::Node::input("in_rect_size", vec2f.clone(), 3).into());
	root.add_child(besl::Node::input("in_corner_radius", f32.clone(), 4).into());
	root.add_child(besl::Node::input("in_corner_exponent", f32.clone(), 5).into());
	root.add_child(besl::Node::input("in_layer_kind", f32.clone(), 6).into());
	root.add_child(besl::Node::input("in_stroke_width", f32, 7).into());
	root.add_child(besl::Node::input("in_feather_mask_position", vec2f.clone(), 8).into());
	root.add_child(besl::Node::input("in_feather_mask_size", vec2f.clone(), 9).into());
	root.add_child(besl::Node::input("in_feather_mask_edges", vec4f.clone(), 10).into());
	root.add_child(besl::Node::input("in_feather_mask_corner", vec2f, 11).into());
	root.add_child(besl::Node::output("out_color_attachment", vec4f, 0).into());

	let program = besl::compile_to_besl(UI_FRAGMENT_SHADER_BESL, Some(root))
		.expect("Failed to compile UI fragment BESL. The most likely cause is invalid BESL syntax.");
	program
		.get_main()
		.expect("Failed to find UI fragment shader entry point. The most likely cause is a missing main function.")
}

pub(super) const UI_FRAGMENT_SHADER_BESL: &str = r#"
main: fn() -> void {
	let half_size: vec2f = in_rect_size * 0.5;
	let corner_radius: f32 = min(in_corner_radius, min(half_size.x, half_size.y));
	let corner_exponent: f32 = in_corner_exponent;
	let centered_position: vec2f = in_local_position - half_size;
	let rounded_extent: vec2f = half_size - vec2f(corner_radius, corner_radius);
	let corner_delta: vec2f = abs(centered_position) - rounded_extent;
	let abs_corner: vec2f = max(corner_delta, vec2f(0.0, 0.0));
	let corner_sum: f32 = pow(abs_corner.x, corner_exponent) + pow(abs_corner.y, corner_exponent);
	let corner_distance: f32 = pow(corner_sum, 1.0 / corner_exponent);
	let field_distance: f32 = corner_distance + min(max(corner_delta.x, corner_delta.y), 0.0) - corner_radius;
	let edge_width: f32 = max(fwidth(field_distance), 1.0);
	let rounded_shape: f32 = step(0.0001, corner_radius);
	let rounded_fill_coverage: f32 = 1.0 - smoothstep(0.0 - edge_width, edge_width, field_distance);
	let fill_coverage: f32 = mix(1.0, rounded_fill_coverage, rounded_shape);

	let corner_gradient_scale: f32 = pow(max(corner_sum, 0.0001), (1.0 / corner_exponent) - 1.0);
	let corner_gradient: vec2f = vec2f(
		pow(abs_corner.x, corner_exponent - 1.0) * corner_gradient_scale,
		pow(abs_corner.y, corner_exponent - 1.0) * corner_gradient_scale
	);
	let field_gradient_length: f32 = mix(1.0, max(length(vec4f(corner_gradient.x, corner_gradient.y, 0.0, 0.0)), 0.0001), step(0.0001, corner_sum));
	let signed_distance: f32 = field_distance / field_gradient_length;
	let corrected_edge_width: f32 = max(fwidth(signed_distance), 1.0);
	let inner_signed_distance: f32 = signed_distance + in_stroke_width;
	let inner_coverage: f32 = 1.0 - smoothstep(0.0 - corrected_edge_width, corrected_edge_width, inner_signed_distance);
	let stroke_coverage: f32 = max(fill_coverage - inner_coverage, 0.0);
	let coverage: f32 = mix(fill_coverage, stroke_coverage, step(0.5, in_layer_kind));
	let feather_top: f32 = mix(1.0, smoothstep(0.0, max(in_feather_mask_edges.x, 0.0001), in_pixel_position.y - in_feather_mask_position.y), step(0.0001, in_feather_mask_edges.x));
	let feather_right: f32 = mix(1.0, smoothstep(0.0, max(in_feather_mask_edges.y, 0.0001), in_feather_mask_position.x + in_feather_mask_size.x - in_pixel_position.x), step(0.0001, in_feather_mask_edges.y));
	let feather_bottom: f32 = mix(1.0, smoothstep(0.0, max(in_feather_mask_edges.z, 0.0001), in_feather_mask_position.y + in_feather_mask_size.y - in_pixel_position.y), step(0.0001, in_feather_mask_edges.z));
	let feather_left: f32 = mix(1.0, smoothstep(0.0, max(in_feather_mask_edges.w, 0.0001), in_pixel_position.x - in_feather_mask_position.x), step(0.0001, in_feather_mask_edges.w));
	let feather_half_size: vec2f = in_feather_mask_size * 0.5;
	let feather_corner_radius: f32 = min(in_feather_mask_corner.x, min(feather_half_size.x, feather_half_size.y));
	let feather_corner_exponent: f32 = in_feather_mask_corner.y;
	let feather_centered_position: vec2f = in_pixel_position - in_feather_mask_position - feather_half_size;
	let feather_rounded_extent: vec2f = feather_half_size - vec2f(feather_corner_radius, feather_corner_radius);
	let feather_corner_delta: vec2f = abs(feather_centered_position) - feather_rounded_extent;
	let feather_abs_corner: vec2f = max(feather_corner_delta, vec2f(0.0, 0.0));
	let feather_corner_sum: f32 = pow(feather_abs_corner.x, feather_corner_exponent) + pow(feather_abs_corner.y, feather_corner_exponent);
	let feather_corner_distance: f32 = pow(feather_corner_sum, 1.0 / feather_corner_exponent);
	let feather_field_distance: f32 = feather_corner_distance + min(max(feather_corner_delta.x, feather_corner_delta.y), 0.0) - feather_corner_radius;
	let feather_mask_enabled: f32 = step(0.0001, min(in_feather_mask_size.x, in_feather_mask_size.y));
	let feather_rounded_shape: f32 = step(0.0001, feather_corner_radius);
	let feather_shape_coverage: f32 = mix(1.0, 1.0 - smoothstep(0.0 - 1.0, 1.0, feather_field_distance), feather_rounded_shape);
	let feather_coverage: f32 = mix(1.0, feather_top * feather_right * feather_bottom * feather_left * feather_shape_coverage, feather_mask_enabled);
	out_color_attachment = vec4f(in_color.x, in_color.y, in_color.z, in_color.w * coverage * feather_coverage);
}
"#;

pub(super) fn create_curve_vertex_shader(context: &mut ghi::implementation::Context) -> ghi::ShaderHandle {
	crate::rendering::create_shader_from_source(
		context,
		Some("UI Curve Vertex Shader"),
		ghi::shader::ShaderSource::Platform {
			glsl: UI_CURVE_VERTEX_SHADER_GLSL,
			msl: UI_CURVE_VERTEX_SHADER_MSL,
			msl_entry_point: "ui_curve_vertex_main",
		},
		ghi::ShaderTypes::Vertex,
		[],
	)
	.expect("Failed to create the UI curve vertex shader. The most likely cause is an incompatible shader interface.")
}

pub(super) fn create_curve_fragment_shader(context: &mut ghi::implementation::Context) -> ghi::ShaderHandle {
	crate::rendering::create_shader_from_source(
		context,
		Some("UI Curve Fragment Shader"),
		ghi::shader::ShaderSource::Platform {
			glsl: UI_CURVE_FRAGMENT_SHADER_GLSL,
			msl: UI_CURVE_FRAGMENT_SHADER_MSL,
			msl_entry_point: "ui_curve_fragment_main",
		},
		ghi::ShaderTypes::Fragment,
		[],
	)
	.expect("Failed to create the UI curve fragment shader. The most likely cause is an incompatible shader interface.")
}

pub(super) const UI_CURVE_VERTEX_SHADER_GLSL: &str = r#"
#version 450

layout(location = 0) in vec2 in_position;
layout(location = 1) in vec2 in_pixel_position;
layout(location = 2) in vec2 in_segment_from;
layout(location = 3) in vec2 in_segment_to;
layout(location = 4) in vec4 in_color;
layout(location = 5) in float in_half_width;
layout(location = 6) in vec2 in_feather_mask_position;
layout(location = 7) in vec2 in_feather_mask_size;
layout(location = 8) in vec4 in_feather_mask_edges;
layout(location = 9) in vec2 in_feather_mask_corner;

layout(location = 0) out vec2 out_pixel_position;
layout(location = 1) out vec2 out_segment_from;
layout(location = 2) out vec2 out_segment_to;
layout(location = 3) out vec4 out_color;
layout(location = 4) out float out_half_width;
layout(location = 5) out vec2 out_feather_mask_position;
layout(location = 6) out vec2 out_feather_mask_size;
layout(location = 7) out vec4 out_feather_mask_edges;
layout(location = 8) out vec2 out_feather_mask_corner;

void main() {
	gl_Position = vec4(in_position, 0.0, 1.0);
	out_pixel_position = in_pixel_position;
	out_segment_from = in_segment_from;
	out_segment_to = in_segment_to;
	out_color = in_color;
	out_half_width = in_half_width;
	out_feather_mask_position = in_feather_mask_position;
	out_feather_mask_size = in_feather_mask_size;
	out_feather_mask_edges = in_feather_mask_edges;
	out_feather_mask_corner = in_feather_mask_corner;
}
"#;

pub(super) const UI_CURVE_FRAGMENT_SHADER_GLSL: &str = r#"
#version 450

layout(location = 0) in vec2 in_pixel_position;
layout(location = 1) in vec2 in_segment_from;
layout(location = 2) in vec2 in_segment_to;
layout(location = 3) in vec4 in_color;
layout(location = 4) in float in_half_width;
layout(location = 5) in vec2 in_feather_mask_position;
layout(location = 6) in vec2 in_feather_mask_size;
layout(location = 7) in vec4 in_feather_mask_edges;
layout(location = 8) in vec2 in_feather_mask_corner;

layout(location = 0) out vec4 out_color_attachment;

void main() {
	vec2 segment = in_segment_to - in_segment_from;
	float length_squared = max(dot(segment, segment), 0.0001);
	float segment_length = sqrt(length_squared);
	vec2 tangent = segment / segment_length;
	vec2 normal = vec2(-tangent.y, tangent.x);
	vec2 center = (in_segment_from + in_segment_to) * 0.5;
	vec2 relative_position = in_pixel_position - center;
	vec2 strip_distance = abs(vec2(dot(relative_position, tangent), dot(relative_position, normal))) - vec2(segment_length * 0.5, in_half_width);
	float outside_distance = length(max(strip_distance, vec2(0.0)));
	float inside_distance = min(max(strip_distance.x, strip_distance.y), 0.0);
	float signed_distance = outside_distance + inside_distance;
	float edge_width = max(fwidth(signed_distance), 1.0);
	float coverage = 1.0 - smoothstep(-edge_width, edge_width, signed_distance);

	float feather_top = mix(1.0, smoothstep(0.0, max(in_feather_mask_edges.x, 0.0001), in_pixel_position.y - in_feather_mask_position.y), step(0.0001, in_feather_mask_edges.x));
	float feather_right = mix(1.0, smoothstep(0.0, max(in_feather_mask_edges.y, 0.0001), in_feather_mask_position.x + in_feather_mask_size.x - in_pixel_position.x), step(0.0001, in_feather_mask_edges.y));
	float feather_bottom = mix(1.0, smoothstep(0.0, max(in_feather_mask_edges.z, 0.0001), in_feather_mask_position.y + in_feather_mask_size.y - in_pixel_position.y), step(0.0001, in_feather_mask_edges.z));
	float feather_left = mix(1.0, smoothstep(0.0, max(in_feather_mask_edges.w, 0.0001), in_pixel_position.x - in_feather_mask_position.x), step(0.0001, in_feather_mask_edges.w));
	vec2 feather_half_size = in_feather_mask_size * 0.5;
	float feather_corner_radius = min(in_feather_mask_corner.x, min(feather_half_size.x, feather_half_size.y));
	float feather_corner_exponent = in_feather_mask_corner.y;
	vec2 feather_centered_position = in_pixel_position - in_feather_mask_position - feather_half_size;
	vec2 feather_rounded_extent = feather_half_size - vec2(feather_corner_radius);
	vec2 feather_corner_delta = abs(feather_centered_position) - feather_rounded_extent;
	vec2 feather_abs_corner = max(feather_corner_delta, vec2(0.0));
	float feather_corner_sum = pow(feather_abs_corner.x, feather_corner_exponent) + pow(feather_abs_corner.y, feather_corner_exponent);
	float feather_corner_distance = pow(feather_corner_sum, 1.0 / feather_corner_exponent);
	float feather_field_distance = feather_corner_distance + min(max(feather_corner_delta.x, feather_corner_delta.y), 0.0) - feather_corner_radius;
	float feather_mask_enabled = step(0.0001, min(in_feather_mask_size.x, in_feather_mask_size.y));
	float feather_rounded_shape = step(0.0001, feather_corner_radius);
	float feather_shape_coverage = mix(1.0, 1.0 - smoothstep(-1.0, 1.0, feather_field_distance), feather_rounded_shape);
	float feather_coverage = mix(1.0, feather_top * feather_right * feather_bottom * feather_left * feather_shape_coverage, feather_mask_enabled);

	out_color_attachment = vec4(in_color.rgb, in_color.a * coverage * feather_coverage);
}
"#;

pub(super) const UI_CURVE_VERTEX_SHADER_MSL: &str = r#"
#include <metal_stdlib>
using namespace metal;

pub(super) struct UiCurveVertexIn {
	float2 position [[attribute(0)]];
	float2 pixel_position [[attribute(1)]];
	float2 segment_from [[attribute(2)]];
	float2 segment_to [[attribute(3)]];
	float4 color [[attribute(4)]];
	float half_width [[attribute(5)]];
	float2 feather_mask_position [[attribute(6)]];
	float2 feather_mask_size [[attribute(7)]];
	float4 feather_mask_edges [[attribute(8)]];
	float2 feather_mask_corner [[attribute(9)]];
};

pub(super) struct UiCurveVertexOut {
	float4 position [[position]];
	float2 pixel_position;
	float2 segment_from;
	float2 segment_to;
	float4 color;
	float half_width;
	float2 feather_mask_position;
	float2 feather_mask_size;
	float4 feather_mask_edges;
	float2 feather_mask_corner;
};

vertex UiCurveVertexOut ui_curve_vertex_main(UiCurveVertexIn in [[stage_in]]) {
	UiCurveVertexOut out;
	out.position = float4(in.position, 0.0, 1.0);
	out.pixel_position = in.pixel_position;
	out.segment_from = in.segment_from;
	out.segment_to = in.segment_to;
	out.color = in.color;
	out.half_width = in.half_width;
	out.feather_mask_position = in.feather_mask_position;
	out.feather_mask_size = in.feather_mask_size;
	out.feather_mask_edges = in.feather_mask_edges;
	out.feather_mask_corner = in.feather_mask_corner;
	return out;
}
"#;

pub(super) const UI_CURVE_FRAGMENT_SHADER_MSL: &str = r#"
#include <metal_stdlib>
using namespace metal;

pub(super) struct UiCurveVertexOut {
	float4 position [[position]];
	float2 pixel_position;
	float2 segment_from;
	float2 segment_to;
	float4 color;
	float half_width;
	float2 feather_mask_position;
	float2 feather_mask_size;
	float4 feather_mask_edges;
	float2 feather_mask_corner;
};

fragment float4 ui_curve_fragment_main(UiCurveVertexOut in [[stage_in]]) {
	float2 segment = in.segment_to - in.segment_from;
	float length_squared = max(dot(segment, segment), 0.0001);
	float segment_length = sqrt(length_squared);
	float2 tangent = segment / segment_length;
	float2 normal = float2(-tangent.y, tangent.x);
	float2 center = (in.segment_from + in.segment_to) * 0.5;
	float2 relative_position = in.pixel_position - center;
	float2 strip_distance = abs(float2(dot(relative_position, tangent), dot(relative_position, normal))) - float2(segment_length * 0.5, in.half_width);
	float outside_distance = length(max(strip_distance, float2(0.0)));
	float inside_distance = min(max(strip_distance.x, strip_distance.y), 0.0);
	float signed_distance = outside_distance + inside_distance;
	float edge_width = max(fwidth(signed_distance), 1.0);
	float coverage = 1.0 - smoothstep(-edge_width, edge_width, signed_distance);

	float feather_top = mix(1.0, smoothstep(0.0, max(in.feather_mask_edges.x, 0.0001), in.pixel_position.y - in.feather_mask_position.y), step(0.0001, in.feather_mask_edges.x));
	float feather_right = mix(1.0, smoothstep(0.0, max(in.feather_mask_edges.y, 0.0001), in.feather_mask_position.x + in.feather_mask_size.x - in.pixel_position.x), step(0.0001, in.feather_mask_edges.y));
	float feather_bottom = mix(1.0, smoothstep(0.0, max(in.feather_mask_edges.z, 0.0001), in.feather_mask_position.y + in.feather_mask_size.y - in.pixel_position.y), step(0.0001, in.feather_mask_edges.z));
	float feather_left = mix(1.0, smoothstep(0.0, max(in.feather_mask_edges.w, 0.0001), in.pixel_position.x - in.feather_mask_position.x), step(0.0001, in.feather_mask_edges.w));
	float2 feather_half_size = in.feather_mask_size * 0.5;
	float feather_corner_radius = min(in.feather_mask_corner.x, min(feather_half_size.x, feather_half_size.y));
	float feather_corner_exponent = in.feather_mask_corner.y;
	float2 feather_centered_position = in.pixel_position - in.feather_mask_position - feather_half_size;
	float2 feather_rounded_extent = feather_half_size - float2(feather_corner_radius);
	float2 feather_corner_delta = abs(feather_centered_position) - feather_rounded_extent;
	float2 feather_abs_corner = max(feather_corner_delta, float2(0.0));
	float feather_corner_sum = pow(feather_abs_corner.x, feather_corner_exponent) + pow(feather_abs_corner.y, feather_corner_exponent);
	float feather_corner_distance = pow(feather_corner_sum, 1.0 / feather_corner_exponent);
	float feather_field_distance = feather_corner_distance + min(max(feather_corner_delta.x, feather_corner_delta.y), 0.0) - feather_corner_radius;
	float feather_mask_enabled = step(0.0001, min(in.feather_mask_size.x, in.feather_mask_size.y));
	float feather_rounded_shape = step(0.0001, feather_corner_radius);
	float feather_shape_coverage = mix(1.0, 1.0 - smoothstep(-1.0, 1.0, feather_field_distance), feather_rounded_shape);
	float feather_coverage = mix(1.0, feather_top * feather_right * feather_bottom * feather_left * feather_shape_coverage, feather_mask_enabled);
	return float4(in.color.rgb, in.color.a * coverage * feather_coverage);
}
"#;

pub(super) fn create_text_overlay_vertex_shader(context: &mut ghi::implementation::Context) -> ghi::ShaderHandle {
	crate::rendering::create_shader_from_source(
		context,
		Some("UI Text Overlay Vertex Shader"),
		ghi::shader::ShaderSource::Platform {
			glsl: TEXT_OVERLAY_VERTEX_SHADER_GLSL,
			msl: TEXT_OVERLAY_VERTEX_SHADER_MSL,
			msl_entry_point: "ui_text_overlay_vertex",
		},
		ghi::ShaderTypes::Vertex,
		[],
	)
	.expect("Failed to create the UI text overlay vertex shader. The most likely cause is an incompatible shader interface.")
}

pub(super) fn create_text_overlay_fragment_shader(context: &mut ghi::implementation::Context) -> ghi::ShaderHandle {
	crate::rendering::create_shader_from_source(
		context,
		Some("UI Text Overlay Fragment Shader"),
		ghi::shader::ShaderSource::Platform {
			glsl: TEXT_OVERLAY_FRAGMENT_SHADER_GLSL,
			msl: TEXT_OVERLAY_FRAGMENT_SHADER_MSL,
			msl_entry_point: "ui_text_overlay_fragment",
		},
		ghi::ShaderTypes::Fragment,
		[TEXT_OVERLAY_BINDING],
	)
	.expect("Failed to create the UI text overlay fragment shader. The most likely cause is an incompatible shader interface.")
}

pub(super) fn create_image_vertex_shader(context: &mut ghi::implementation::Context) -> ghi::ShaderHandle {
	crate::rendering::create_shader_from_source(
		context,
		Some("UI Image Vertex Shader"),
		ghi::shader::ShaderSource::Platform {
			glsl: IMAGE_VERTEX_SHADER_GLSL,
			msl: IMAGE_VERTEX_SHADER_MSL,
			msl_entry_point: "ui_image_vertex",
		},
		ghi::ShaderTypes::Vertex,
		[],
	)
	.expect("Failed to create the UI image vertex shader. The most likely cause is an incompatible shader interface.")
}

pub(super) fn create_image_fragment_shader(context: &mut ghi::implementation::Context) -> ghi::ShaderHandle {
	crate::rendering::create_shader_from_source(
		context,
		Some("UI Image Fragment Shader"),
		ghi::shader::ShaderSource::Platform {
			glsl: IMAGE_FRAGMENT_SHADER_GLSL,
			msl: IMAGE_FRAGMENT_SHADER_MSL,
			msl_entry_point: "ui_image_fragment",
		},
		ghi::ShaderTypes::Fragment,
		[UI_IMAGE_BINDING],
	)
	.expect("Failed to create the UI image fragment shader. The most likely cause is an incompatible shader interface.")
}

pub(super) const IMAGE_VERTEX_SHADER_GLSL: &str = r#"
#version 460
#pragma shader_stage(vertex)

layout(location = 0) in vec2 in_position;
layout(location = 1) in vec2 in_uv;
layout(location = 2) in float in_opacity;
layout(location = 3) in vec2 in_feather_mask_position;
layout(location = 4) in vec2 in_feather_mask_size;
layout(location = 5) in vec4 in_feather_mask_edges;
layout(location = 6) in vec2 in_feather_mask_corner;

layout(location = 0) out vec2 out_uv;
layout(location = 1) out float out_opacity;
layout(location = 2) out vec2 out_feather_mask_position;
layout(location = 3) out vec2 out_feather_mask_size;
layout(location = 4) out vec4 out_feather_mask_edges;
layout(location = 5) out vec2 out_feather_mask_corner;

void main() {
	gl_Position = vec4(in_position, 0.0, 1.0);
	out_uv = in_uv;
	out_opacity = in_opacity;
	out_feather_mask_position = in_feather_mask_position;
	out_feather_mask_size = in_feather_mask_size;
	out_feather_mask_edges = in_feather_mask_edges;
	out_feather_mask_corner = in_feather_mask_corner;
}
"#;

pub(super) const IMAGE_VERTEX_SHADER_MSL: &str = r#"
#include <metal_stdlib>
using namespace metal;

pub(super) struct ImageVertexIn {
	float2 position [[attribute(0)]];
	float2 uv [[attribute(1)]];
	float opacity [[attribute(2)]];
	float2 feather_mask_position [[attribute(3)]];
	float2 feather_mask_size [[attribute(4)]];
	float4 feather_mask_edges [[attribute(5)]];
	float2 feather_mask_corner [[attribute(6)]];
};

pub(super) struct ImageVertexOut {
	float4 position [[position]];
	float2 uv;
	float opacity;
	float2 feather_mask_position;
	float2 feather_mask_size;
	float4 feather_mask_edges;
	float2 feather_mask_corner;
};

vertex ImageVertexOut ui_image_vertex(ImageVertexIn in [[stage_in]]) {
	ImageVertexOut out;
	out.position = float4(in.position, 0.0, 1.0);
	out.uv = in.uv;
	out.opacity = in.opacity;
	out.feather_mask_position = in.feather_mask_position;
	out.feather_mask_size = in.feather_mask_size;
	out.feather_mask_edges = in.feather_mask_edges;
	out.feather_mask_corner = in.feather_mask_corner;
	return out;
}
"#;

pub(super) const IMAGE_FRAGMENT_SHADER_GLSL: &str = r#"
#version 460
#pragma shader_stage(fragment)

layout(set = 0, binding = 0) uniform sampler2D image_texture;

layout(location = 0) in vec2 in_uv;
layout(location = 1) in float in_opacity;
layout(location = 2) in vec2 in_feather_mask_position;
layout(location = 3) in vec2 in_feather_mask_size;
layout(location = 4) in vec4 in_feather_mask_edges;
layout(location = 5) in vec2 in_feather_mask_corner;
layout(location = 0) out vec4 out_color_attachment;

void main() {
	vec2 pixel_position = gl_FragCoord.xy;
	float feather_top = mix(1.0, smoothstep(0.0, max(in_feather_mask_edges.x, 0.0001), pixel_position.y - in_feather_mask_position.y), step(0.0001, in_feather_mask_edges.x));
	float feather_right = mix(1.0, smoothstep(0.0, max(in_feather_mask_edges.y, 0.0001), in_feather_mask_position.x + in_feather_mask_size.x - pixel_position.x), step(0.0001, in_feather_mask_edges.y));
	float feather_bottom = mix(1.0, smoothstep(0.0, max(in_feather_mask_edges.z, 0.0001), in_feather_mask_position.y + in_feather_mask_size.y - pixel_position.y), step(0.0001, in_feather_mask_edges.z));
	float feather_left = mix(1.0, smoothstep(0.0, max(in_feather_mask_edges.w, 0.0001), pixel_position.x - in_feather_mask_position.x), step(0.0001, in_feather_mask_edges.w));
	vec2 feather_half_size = in_feather_mask_size * 0.5;
	float feather_corner_radius = min(in_feather_mask_corner.x, min(feather_half_size.x, feather_half_size.y));
	float feather_corner_exponent = in_feather_mask_corner.y;
	vec2 feather_centered_position = pixel_position - in_feather_mask_position - feather_half_size;
	vec2 feather_rounded_extent = feather_half_size - vec2(feather_corner_radius);
	vec2 feather_corner_delta = abs(feather_centered_position) - feather_rounded_extent;
	vec2 feather_abs_corner = max(feather_corner_delta, vec2(0.0));
	float feather_corner_sum = pow(feather_abs_corner.x, feather_corner_exponent) + pow(feather_abs_corner.y, feather_corner_exponent);
	float feather_corner_distance = pow(feather_corner_sum, 1.0 / feather_corner_exponent);
	float feather_field_distance = feather_corner_distance + min(max(feather_corner_delta.x, feather_corner_delta.y), 0.0) - feather_corner_radius;
	float feather_mask_enabled = step(0.0001, min(in_feather_mask_size.x, in_feather_mask_size.y));
	float feather_rounded_shape = step(0.0001, feather_corner_radius);
	float feather_shape_coverage = mix(1.0, 1.0 - smoothstep(-1.0, 1.0, feather_field_distance), feather_rounded_shape);
	float feather_coverage = mix(1.0, feather_top * feather_right * feather_bottom * feather_left * feather_shape_coverage, feather_mask_enabled);
	vec4 color = texture(image_texture, in_uv);
	out_color_attachment = vec4(color.rgb, color.a * in_opacity * feather_coverage);
}
"#;

pub(super) const IMAGE_FRAGMENT_SHADER_MSL: &str = r#"
#include <metal_stdlib>
using namespace metal;

pub(super) struct ImageVertexOut {
	float4 position [[position]];
	float2 uv;
	float opacity;
	float2 feather_mask_position;
	float2 feather_mask_size;
	float4 feather_mask_edges;
	float2 feather_mask_corner;
};

pub(super) struct ImageSet0 {
	texture2d<float> image_texture [[id(0)]];
	sampler image_sampler [[id(1)]];
};

fragment float4 ui_image_fragment(
	ImageVertexOut in [[stage_in]],
	constant ImageSet0& set0 [[buffer(16)]]
) {
	float2 pixel_position = in.position.xy;
	float feather_top = mix(1.0, smoothstep(0.0, max(in.feather_mask_edges.x, 0.0001), pixel_position.y - in.feather_mask_position.y), step(0.0001, in.feather_mask_edges.x));
	float feather_right = mix(1.0, smoothstep(0.0, max(in.feather_mask_edges.y, 0.0001), in.feather_mask_position.x + in.feather_mask_size.x - pixel_position.x), step(0.0001, in.feather_mask_edges.y));
	float feather_bottom = mix(1.0, smoothstep(0.0, max(in.feather_mask_edges.z, 0.0001), in.feather_mask_position.y + in.feather_mask_size.y - pixel_position.y), step(0.0001, in.feather_mask_edges.z));
	float feather_left = mix(1.0, smoothstep(0.0, max(in.feather_mask_edges.w, 0.0001), pixel_position.x - in.feather_mask_position.x), step(0.0001, in.feather_mask_edges.w));
	float2 feather_half_size = in.feather_mask_size * 0.5;
	float feather_corner_radius = min(in.feather_mask_corner.x, min(feather_half_size.x, feather_half_size.y));
	float feather_corner_exponent = in.feather_mask_corner.y;
	float2 feather_centered_position = pixel_position - in.feather_mask_position - feather_half_size;
	float2 feather_rounded_extent = feather_half_size - float2(feather_corner_radius);
	float2 feather_corner_delta = abs(feather_centered_position) - feather_rounded_extent;
	float2 feather_abs_corner = max(feather_corner_delta, float2(0.0));
	float feather_corner_sum = pow(feather_abs_corner.x, feather_corner_exponent) + pow(feather_abs_corner.y, feather_corner_exponent);
	float feather_corner_distance = pow(feather_corner_sum, 1.0 / feather_corner_exponent);
	float feather_field_distance = feather_corner_distance + min(max(feather_corner_delta.x, feather_corner_delta.y), 0.0) - feather_corner_radius;
	float feather_mask_enabled = step(0.0001, min(in.feather_mask_size.x, in.feather_mask_size.y));
	float feather_rounded_shape = step(0.0001, feather_corner_radius);
	float feather_shape_coverage = mix(1.0, 1.0 - smoothstep(-1.0, 1.0, feather_field_distance), feather_rounded_shape);
	float feather_coverage = mix(1.0, feather_top * feather_right * feather_bottom * feather_left * feather_shape_coverage, feather_mask_enabled);
	float4 color = set0.image_texture.sample(set0.image_sampler, in.uv);
	return float4(color.rgb, color.a * in.opacity * feather_coverage);
}
"#;

pub(super) const TEXT_OVERLAY_VERTEX_SHADER_GLSL: &str = r#"
#version 460
#pragma shader_stage(vertex)

layout(location = 0) out vec2 out_uv;

void main() {
	vec2 positions[3] = vec2[](
		vec2(-1.0, -1.0),
		vec2(-1.0, 3.0),
		vec2(3.0, -1.0)
	);
	vec2 position = positions[gl_VertexIndex];
	gl_Position = vec4(position, 0.0, 1.0);
	out_uv = vec2(position.x * 0.5 + 0.5, 0.5 - position.y * 0.5);
}
"#;

pub(super) const TEXT_OVERLAY_VERTEX_SHADER_MSL: &str = r#"
#include <metal_stdlib>
using namespace metal;

pub(super) struct TextOverlayVertexOut {
	float4 position [[position]];
	float2 uv;
};

vertex TextOverlayVertexOut ui_text_overlay_vertex(uint vertex_id [[vertex_id]]) {
	float2 positions[3] = {
		float2(-1.0, -1.0),
		float2(-1.0, 3.0),
		float2(3.0, -1.0)
	};
	float2 position = positions[vertex_id];
	TextOverlayVertexOut out;
	out.position = float4(position, 0.0, 1.0);
	out.uv = float2(position.x * 0.5 + 0.5, 0.5 - position.y * 0.5);
	return out;
}
"#;

pub(super) const TEXT_OVERLAY_FRAGMENT_SHADER_GLSL: &str = r#"
#version 460
#pragma shader_stage(fragment)

layout(set = 0, binding = 0) uniform sampler2D text_overlay;

layout(location = 0) in vec2 in_uv;
layout(location = 0) out vec4 out_color_attachment;

void main() {
	out_color_attachment = texture(text_overlay, in_uv);
}
"#;

pub(super) const TEXT_OVERLAY_FRAGMENT_SHADER_MSL: &str = r#"
#include <metal_stdlib>
using namespace metal;

pub(super) struct TextOverlayVertexOut {
	float4 position [[position]];
	float2 uv;
};

pub(super) struct TextOverlaySet0 {
	texture2d<float> text_overlay [[id(0)]];
	sampler text_overlay_sampler [[id(1)]];
};

fragment float4 ui_text_overlay_fragment(
	TextOverlayVertexOut in [[stage_in]],
	constant TextOverlaySet0& set0 [[buffer(16)]]
) {
	return set0.text_overlay.sample(set0.text_overlay_sampler, in.uv);
}
"#;
