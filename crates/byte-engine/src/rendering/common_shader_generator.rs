use std::sync::OnceLock;

use resource_management::asset::{bema_asset_handler::ProgramGenerator, JsonObject};

// Keeping the shared helpers in portable BESL makes their VM tests exercise the
// same implementation that every graphics backend lowers for production use.
const COMMON_SHADER_SOURCE: &str = r#"
	UVDerivatives: struct {
		du: vec3f,
		dv: vec3f,
	}

	BarycentricDeriv: struct {
		lambda: vec3f,
		ddx: vec3f,
		ddy: vec3f,
	}

	vec2f_squared_length: fn (v: vec2f) -> f32 {
		return dot(v, v);
	}

	vec3f_squared_length: fn (v: vec3f) -> f32 {
		return dot(v, v);
	}

	vec4f_squared_length: fn (v: vec4f) -> f32 {
		return dot(v, v);
	}

	source_over: fn (source: vec4f, destination: vec4f) -> vec4f {
		let inverse_alpha: f32 = 1.0 - source.w;
		let source_rgb: vec3f = vec3f(source.x, source.y, source.z);
		let destination_rgb: vec3f = vec3f(destination.x, destination.y, destination.z);
		let color: vec3f = source_rgb + destination_rgb * inverse_alpha;
		let alpha: f32 = source.w + destination.w * inverse_alpha;
		return vec4f(color.x, color.y, color.z, alpha);
	}

	min_diff: fn (p: vec3f, a: vec3f, b: vec3f) -> vec3f {
		let a_to_p: vec3f = a - p;
		let b_to_p: vec3f = p - b;
		if (vec3f_squared_length(a_to_p) < vec3f_squared_length(b_to_p)) {
			return a_to_p;
		}

		return b_to_p;
	}

	interleaved_gradient_noise: fn (pixel_x: u32, pixel_y: u32, frame: u32) -> f32 {
		let wrapped_frame: u32 = frame % 64;
		let frame_offset: f32 = 5.588238 * f32(wrapped_frame);
		let x: f32 = f32(pixel_x) + frame_offset;
		let y: f32 = f32(pixel_y) + frame_offset;
		return fract(52.9829189 * fract(0.06711056 * x + 0.00583715 * y));
	}

	make_world_space_position_from_depth: fn (
		depth: f32,
		uv: vec2f,
		inverse_projection_matrix: mat4f,
		inverse_view_matrix: mat4f
	) -> vec3f {
		let clip_space: vec4f = vec4f(uv.x * 2.0 - 1.0, uv.y * 2.0 - 1.0, depth, 1.0);
		let view_space: vec4f = inverse_projection_matrix * clip_space;
		let normalized_view_space: vec4f = view_space / view_space.w;
		let world_space: vec4f = inverse_view_matrix * normalized_view_space;
		return vec3f(world_space.x, world_space.y, world_space.z);
	}

	get_world_space_position_from_depth: fn (
		depth_map: Texture2D,
		coords: vec2u,
		inverse_projection_matrix: mat4f,
		inverse_view_matrix: mat4f
	) -> vec3f {
		let depth_value: f32 = fetch(depth_map, coords).x;
		let extent: vec2u = texture_size(depth_map);
		let uv: vec2f = vec2f(
			(f32(coords.x) + 0.5) / f32(extent.x),
			(f32(coords.y) + 0.5) / f32(extent.y)
		);
		return make_world_space_position_from_depth(
			depth_value,
			uv,
			inverse_projection_matrix,
			inverse_view_matrix
		);
	}

	get_view_space_position_from_depth: fn (
		depth_map: Texture2D,
		uv: vec2f,
		inverse_projection_matrix: mat4f
	) -> vec3f {
		let depth_value: f32 = sample(depth_map, uv).x;
		let clip_space: vec4f = vec4f(uv.x * 2.0 - 1.0, uv.y * 2.0 - 1.0, depth_value, 1.0);
		let view_space: vec4f = inverse_projection_matrix * clip_space;
		let normalized_view_space: vec4f = view_space / view_space.w;
		return vec3f(normalized_view_space.x, normalized_view_space.y, normalized_view_space.z);
	}

	sin_from_tan: fn (x: f32) -> f32 {
		return x * inversesqrt(x * x + 1.0);
	}

	tangent: fn (p: vec3f, s: vec3f) -> f32 {
		let horizontal_delta: vec2f = vec2f(s.x - p.x, s.y - p.y);
		return (p.z - s.z) * inversesqrt(dot(horizontal_delta, horizontal_delta));
	}

	make_normal_from_positions: fn (
		p: vec3f,
		pr: vec3f,
		pl: vec3f,
		pt: vec3f,
		pb: vec3f
	) -> vec3f {
		let horizontal: vec3f = min_diff(p, pr, pl);
		let vertical: vec3f = min_diff(p, pt, pb);
		return normalize(cross(horizontal, vertical));
	}

	make_perpendicular_vector: fn (v: vec3f) -> vec3f {
		if (abs(v.x) > abs(v.z)) {
			return normalize(vec3f(0.0 - v.y, v.x, 0.0));
		}

		return normalize(vec3f(0.0, 0.0 - v.z, v.y));
	}

	snap_uv: fn (uv: vec2f, extent: vec2u) -> vec2f {
		let extent_f: vec2f = vec2f(f32(extent.x), f32(extent.y));
		return round(uv * extent_f) / extent_f;
	}

	make_cosine_hemisphere_sample: fn (
		rand_1: f32,
		rand_2: f32,
		hit_normal: vec3f
	) -> vec3f {
		let bitangent: vec3f = make_perpendicular_vector(hit_normal);
		let tangent_vector: vec3f = cross(bitangent, hit_normal);
		let radius: f32 = sqrt(rand_1);
		let phi: f32 = 2.0 * 3.14159265359 * rand_2;
		let tangent_component: vec3f = tangent_vector * (radius * cos(phi));
		let bitangent_component: vec3f = bitangent * (radius * sin(phi));
		let normal_component: vec3f = hit_normal * sqrt(max(0.0, 1.0 - rand_1));
		return normalize(tangent_component + bitangent_component + normal_component);
	}

	make_uv: fn (coordinates: vec2i, extent: vec2u) -> vec2f {
		return vec2f(
			(f32(coordinates.x) + 0.5) / f32(extent.x),
			(f32(coordinates.y) + 0.5) / f32(extent.y)
		);
	}

	make_normal_from_depth_map: fn (
		depth_map: Texture2D,
		coord: vec2i,
		extent: vec2u,
		inverse_projection: mat4f,
		inverse_view: mat4f
	) -> vec3f {
		let left_coord: vec2i = coord + vec2i(0 - 1, 0);
		let right_coord: vec2i = coord + vec2i(1, 0);
		let top_coord: vec2i = coord + vec2i(0, 0 - 1);
		let bottom_coord: vec2i = coord + vec2i(0, 1);
		let center_texel: vec2u = vec2u(u32(coord.x), u32(coord.y));
		let left_texel: vec2u = vec2u(u32(left_coord.x), u32(left_coord.y));
		let right_texel: vec2u = vec2u(u32(right_coord.x), u32(right_coord.y));
		let top_texel: vec2u = vec2u(u32(top_coord.x), u32(top_coord.y));
		let bottom_texel: vec2u = vec2u(u32(bottom_coord.x), u32(bottom_coord.y));
		let center_depth: f32 = fetch(depth_map, center_texel).x;
		if (center_depth == 0.0) {
			return vec3f(0.0, 0.0, 0.0);
		}

		let left_depth: f32 = fetch(depth_map, left_texel).x;
		let right_depth: f32 = fetch(depth_map, right_texel).x;
		let top_depth: f32 = fetch(depth_map, top_texel).x;
		let bottom_depth: f32 = fetch(depth_map, bottom_texel).x;
		let center_position: vec3f = make_world_space_position_from_depth(
			center_depth,
			make_uv(coord, extent),
			inverse_projection,
			inverse_view
		);
		let left_position: vec3f = make_world_space_position_from_depth(
			left_depth,
			make_uv(left_coord, extent),
			inverse_projection,
			inverse_view
		);
		let right_position: vec3f = make_world_space_position_from_depth(
			right_depth,
			make_uv(right_coord, extent),
			inverse_projection,
			inverse_view
		);
		let top_position: vec3f = make_world_space_position_from_depth(
			top_depth,
			make_uv(top_coord, extent),
			inverse_projection,
			inverse_view
		);
		let bottom_position: vec3f = make_world_space_position_from_depth(
			bottom_depth,
			make_uv(bottom_coord, extent),
			inverse_projection,
			inverse_view
		);
		return make_normal_from_positions(
			center_position,
			left_position,
			right_position,
			top_position,
			bottom_position
		);
	}

	rotate_directions: fn (direction: vec2f, cos_sin: vec2f) -> vec2f {
		return vec2f(
			direction.x * cos_sin.x - direction.y * cos_sin.y,
			direction.x * cos_sin.y + direction.y * cos_sin.x
		);
	}

	distribution_ggx: fn (n: vec3f, h: vec3f, roughness: f32) -> f32 {
		let alpha: f32 = roughness * roughness;
		let alpha_squared: f32 = alpha * alpha;
		let n_dot_h: f32 = max(dot(n, h), 0.0);
		let denominator_base: f32 = n_dot_h * n_dot_h * (alpha_squared - 1.0) + 1.0;
		let denominator: f32 = 3.14159265359 * denominator_base * denominator_base;
		return alpha_squared / denominator;
	}

	geometry_schlick_ggx: fn (n_dot_v: f32, roughness: f32) -> f32 {
		let adjusted_roughness: f32 = roughness + 1.0;
		let k: f32 = adjusted_roughness * adjusted_roughness / 8.0;
		return n_dot_v / (n_dot_v * (1.0 - k) + k);
	}

	geometry_smith: fn (n: vec3f, v: vec3f, l: vec3f, roughness: f32) -> f32 {
		let n_dot_v: f32 = max(dot(n, v), 0.0);
		let n_dot_l: f32 = max(dot(n, l), 0.0);
		return geometry_schlick_ggx(n_dot_v, roughness) * geometry_schlick_ggx(n_dot_l, roughness);
	}

	fresnel_schlick: fn (cos_theta: f32, f0: vec3f) -> vec3f {
		let factor: f32 = pow(clamp(1.0 - cos_theta, 0.0, 1.0), 5.0);
		return f0 + (1.0 - f0) * factor;
	}

	fresnel_schlick_roughness: fn (cos_theta: f32, f0: vec3f, roughness: f32) -> vec3f {
		let maximum: f32 = 1.0 - roughness;
		let grazing: vec3f = max(vec3f(maximum, maximum, maximum), f0);
		let factor: f32 = pow(clamp(1.0 - cos_theta, 0.0, 1.0), 5.0);
		return f0 + (grazing - f0) * factor;
	}

	calculate_full_bary: fn (
		pt0: vec4f,
		pt1: vec4f,
		pt2: vec4f,
		pixel_ndc: vec2f,
		window_size: vec2f
	) -> BarycentricDeriv {
		let inverse_w: vec3f = vec3f(1.0 / pt0.w, 1.0 / pt1.w, 1.0 / pt2.w);
		let ndc0: vec2f = vec2f(pt0.x * inverse_w.x, pt0.y * inverse_w.x);
		let ndc1: vec2f = vec2f(pt1.x * inverse_w.y, pt1.y * inverse_w.y);
		let ndc2: vec2f = vec2f(pt2.x * inverse_w.z, pt2.y * inverse_w.z);
		let determinant: f32 =
			(ndc2.x - ndc1.x) * (ndc0.y - ndc1.y) -
			(ndc0.x - ndc1.x) * (ndc2.y - ndc1.y);
		let inverse_determinant: f32 = 1.0 / determinant;
		let raw_ddx: vec3f = vec3f(
			ndc1.y - ndc2.y,
			ndc2.y - ndc0.y,
			ndc0.y - ndc1.y
		) * inverse_determinant * inverse_w;
		let raw_ddy: vec3f = vec3f(
			ndc2.x - ndc1.x,
			ndc0.x - ndc2.x,
			ndc1.x - ndc0.x
		) * inverse_determinant * inverse_w;
		let ddx_sum: f32 = dot(raw_ddx, vec3f(1.0, 1.0, 1.0));
		let ddy_sum: f32 = dot(raw_ddy, vec3f(1.0, 1.0, 1.0));
		let delta: vec2f = pixel_ndc - ndc0;
		let interpolated_inverse_w: f32 = inverse_w.x + delta.x * ddx_sum + delta.y * ddy_sum;
		let interpolated_w: f32 = 1.0 / interpolated_inverse_w;
		let lambda: vec3f = vec3f(
			interpolated_w * (inverse_w.x + delta.x * raw_ddx.x + delta.y * raw_ddy.x),
			interpolated_w * (delta.x * raw_ddx.y + delta.y * raw_ddy.y),
			interpolated_w * (delta.x * raw_ddx.z + delta.y * raw_ddy.z)
		);
		let x_scale: f32 = 2.0 / window_size.x;
		let y_scale: f32 = 2.0 / window_size.y;
		let scaled_ddx: vec3f = raw_ddx * x_scale;
		let scaled_ddy: vec3f = raw_ddy * y_scale;
		let scaled_ddx_sum: f32 = ddx_sum * x_scale;
		let scaled_ddy_sum: f32 = ddy_sum * y_scale;
		let interpolated_w_ddx: f32 = 1.0 / (interpolated_inverse_w + scaled_ddx_sum);
		let interpolated_w_ddy: f32 = 1.0 / (interpolated_inverse_w + scaled_ddy_sum);
		let derivative_x: vec3f =
			interpolated_w_ddx * (lambda * interpolated_inverse_w + scaled_ddx) - lambda;
		let derivative_y: vec3f =
			interpolated_w_ddy * (lambda * interpolated_inverse_w + scaled_ddy) - lambda;
		return BarycentricDeriv(lambda, derivative_x, derivative_y);
	}

	calculate_barycentric_from_position: fn (
		position: vec3f,
		v0: vec3f,
		v1: vec3f,
		v2: vec3f
	) -> vec3f {
		let edge0: vec3f = v1 - v0;
		let edge1: vec3f = v2 - v0;
		let point_delta: vec3f = position - v0;
		let d00: f32 = dot(edge0, edge0);
		let d01: f32 = dot(edge0, edge1);
		let d11: f32 = dot(edge1, edge1);
		let d20: f32 = dot(point_delta, edge0);
		let d21: f32 = dot(point_delta, edge1);
		let denominator: f32 = d00 * d11 - d01 * d01;
		if (abs(denominator) <= 0.00000001) {
			return vec3f(1.0, 0.0, 0.0);
		}

		let barycentric_1: f32 = (d11 * d20 - d01 * d21) / denominator;
		let barycentric_2: f32 = (d00 * d21 - d01 * d20) / denominator;
		return vec3f(1.0 - barycentric_1 - barycentric_2, barycentric_1, barycentric_2);
	}

	make_raster_ndc_from_pixel_coordinates: fn (
		pixel_coordinates: vec2i,
		image_extent: vec2i
	) -> vec2f {
		let normalized: vec2f = vec2f(
			(f32(pixel_coordinates.x) + 0.5) / f32(image_extent.x),
			(f32(pixel_coordinates.y) + 0.5) / f32(image_extent.y)
		);
		return vec2f(normalized.x * 2.0 - 1.0, 1.0 - normalized.y * 2.0);
	}

	interpolate_vec3f_with_deriv: fn (
		interpolation: vec3f,
		v0: vec3f,
		v1: vec3f,
		v2: vec3f
	) -> vec3f {
		return v0 * interpolation.x + v1 * interpolation.y + v2 * interpolation.z;
	}

	interpolate_vec2f_with_deriv: fn (
		interpolation: vec3f,
		v0: vec2f,
		v1: vec2f,
		v2: vec2f
	) -> vec2f {
		return v0 * interpolation.x + v1 * interpolation.y + v2 * interpolation.z;
	}

	unit_vector_from_xy: fn (v: vec2f) -> vec3f {
		let transformed: vec2f = v * 2.0 - 1.0;
		let z: f32 = sqrt(max(0.0, 1.0 - transformed.x * transformed.x - transformed.y * transformed.y));
		return normalize(vec3f(transformed.x, transformed.y, z));
	}

	get_debug_color: fn (i: u32) -> vec4f {
		let palette_index: u32 = i % 16;
		if (palette_index == 0) { return vec4f(0.16863, 0.40392, 0.77647, 1.0); }
		if (palette_index == 1) { return vec4f(0.32941, 0.76863, 0.21961, 1.0); }
		if (palette_index == 2) { return vec4f(0.81961, 0.16078, 0.67451, 1.0); }
		if (palette_index == 3) { return vec4f(0.96863, 0.98824, 0.45490, 1.0); }
		if (palette_index == 4) { return vec4f(0.75294, 0.09020, 0.75686, 1.0); }
		if (palette_index == 5) { return vec4f(0.30588, 0.95686, 0.54510, 1.0); }
		if (palette_index == 6) { return vec4f(0.66667, 0.06667, 0.75686, 1.0); }
		if (palette_index == 7) { return vec4f(0.78824, 0.91765, 0.27451, 1.0); }
		if (palette_index == 8) { return vec4f(0.40980, 0.12745, 0.48627, 1.0); }
		if (palette_index == 9) { return vec4f(0.89804, 0.28235, 0.20784, 1.0); }
		if (palette_index == 10) { return vec4f(0.93725, 0.67843, 0.33725, 1.0); }
		if (palette_index == 11) { return vec4f(0.95294, 0.96863, 0.00392, 1.0); }
		if (palette_index == 12) { return vec4f(1.00000, 0.27843, 0.67843, 1.0); }
		if (palette_index == 13) { return vec4f(0.29020, 0.90980, 0.56863, 1.0); }
		if (palette_index == 14) { return vec4f(0.30980, 0.70980, 0.27059, 1.0); }
		return vec4f(0.69804, 0.16078, 0.39216, 1.0);
	}
"#;

static COMMON_SHADER_SCOPE: OnceLock<besl::parser::Node<'static>> = OnceLock::new();

/// Parses the common module once so repeated shader builds only clone its syntax tree.
fn parse_common_shader_scope() -> besl::parser::Node<'static> {
	let mut root = besl::parse(COMMON_SHADER_SOURCE)
		.expect("Failed to parse the common BESL shader module. The most likely cause is invalid portable BESL syntax.");
	let children = match root.node_mut() {
		besl::parser::Nodes::Scope { children, .. } => std::mem::take(children),
		_ => unreachable!("Invalid common BESL shader root. The most likely cause is a parser contract regression."),
	};
	besl::parser::Node::scope("Common", children)
}

/// The `CommonShaderScope` struct provides the portable helper namespace shared by production shaders.
pub struct CommonShaderScope {}

/// The `CommonShaderGenerator` struct preserves common-module programs while they pass through asset generation.
pub struct CommonShaderGenerator {}

impl Default for CommonShaderGenerator {
	fn default() -> Self {
		Self::new()
	}
}

impl CommonShaderGenerator {
	pub fn new() -> Self {
		Self {}
	}
}

impl ProgramGenerator for CommonShaderGenerator {
	fn transform<'a>(&self, mut root: besl::parser::Node<'a>, _: &JsonObject) -> besl::parser::Node<'a> {
		root.add(vec![CommonShaderScope::new()]);
		root
	}
}

impl CommonShaderScope {
	/// Builds the common scope from the single portable source used by VM tests and graphics backends.
	pub fn new() -> besl::parser::Node<'static> {
		COMMON_SHADER_SCOPE.get_or_init(parse_common_shader_scope).clone()
	}
}

#[cfg(test)]
mod tests {
	use besl::vm::{Buffer, DescriptorBindings, ExecutableProgram, ResourceSlot, Value};

	use super::CommonShaderScope;
	use crate::rendering::shader_vm_test::{buffer, compile, run_at, texture_2d};

	const RESULT_SLOT: ResourceSlot = ResourceSlot::new(0);
	const DEPTH_SLOT: ResourceSlot = ResourceSlot::new(1);
	const EMPTY_DEPTH_SLOT: ResourceSlot = ResourceSlot::new(2);
	const SAMPLE_DEPTH_SLOT: ResourceSlot = ResourceSlot::new(3);

	/// Compiles a synthetic main that exposes common-function values through one result buffer.
	fn compile_common_main(
		source: &'static str,
		result_members: Vec<besl::ParserNode<'static>>,
		extra_nodes: Vec<besl::ParserNode<'static>>,
	) -> (ExecutableProgram, Buffer) {
		let mut root = besl::parse(source)
			.expect("Failed to parse a common shader VM test. The most likely cause is invalid BESL test syntax.");
		root.add(extra_nodes);
		root.add(vec![
			CommonShaderScope::new(),
			besl::ParserNode::binding(
				"results",
				besl::ParserNode::buffer("CommonShaderTestResults", result_members),
				RESULT_SLOT.slot(),
				false,
				true,
			),
		]);

		let program = besl::lex(root).expect(
			"Failed to lex a common shader VM test. The most likely cause is an unresolved portable helper or test binding.",
		);
		let executable = compile(program);
		let results = buffer(&executable, RESULT_SLOT);
		(executable, results)
	}

	/// Executes a common-function test that only writes its result buffer.
	fn run_common(program: &ExecutableProgram, results: &mut Buffer) {
		let mut descriptors = DescriptorBindings::new();
		descriptors.bind_buffer(RESULT_SLOT, results);
		run_at(program, &mut descriptors, [0, 0]);
	}

	/// Reads one scalar result while keeping type failures local to the named assertion.
	fn read_f32(results: &Buffer, name: &str) -> f32 {
		match results
			.read(name)
			.expect("Missing common shader scalar result. The most likely cause is a mismatched test buffer member.")
		{
			Value::F32(value) => value,
			value => panic!(
				"Invalid common shader scalar result `{value:?}`. The most likely cause is an incorrect test member type."
			),
		}
	}

	/// Reads one two-component vector result while preserving an allocation-free assertion path.
	fn read_vec2f(results: &Buffer, name: &str) -> [f32; 2] {
		match results
			.read(name)
			.expect("Missing common shader vec2 result. The most likely cause is a mismatched test buffer member.")
		{
			Value::Vec2F(value) => value,
			value => {
				panic!("Invalid common shader vec2 result `{value:?}`. The most likely cause is an incorrect test member type.")
			}
		}
	}

	/// Reads one three-component vector result while preserving an allocation-free assertion path.
	fn read_vec3f(results: &Buffer, name: &str) -> [f32; 3] {
		match results
			.read(name)
			.expect("Missing common shader vec3 result. The most likely cause is a mismatched test buffer member.")
		{
			Value::Vec3F(value) => value,
			value => {
				panic!("Invalid common shader vec3 result `{value:?}`. The most likely cause is an incorrect test member type.")
			}
		}
	}

	/// Reads one four-component vector result while preserving an allocation-free assertion path.
	fn read_vec4f(results: &Buffer, name: &str) -> [f32; 4] {
		match results
			.read(name)
			.expect("Missing common shader vec4 result. The most likely cause is a mismatched test buffer member.")
		{
			Value::Vec4F(value) => value,
			value => {
				panic!("Invalid common shader vec4 result `{value:?}`. The most likely cause is an incorrect test member type.")
			}
		}
	}

	/// Compares finite float arrays with tolerance scaled to each expected component.
	fn assert_floats_close<const N: usize>(actual: [f32; N], expected: [f32; N], epsilon: f32) {
		for (index, (actual, expected)) in actual.into_iter().zip(expected).enumerate() {
			let tolerance = epsilon * expected.abs().max(1.0);
			assert!(
				actual.is_finite() && (actual - expected).abs() <= tolerance,
				"Common shader result {index} changed: expected {expected}, found {actual}. The most likely cause is a common helper or VM arithmetic regression."
			);
		}
	}

	fn assert_f32_close(actual: f32, expected: f32, epsilon: f32) {
		assert_floats_close([actual], [expected], epsilon);
	}

	/// Verifies premultiplied source-over composition through the shared production helper.
	#[test]
	fn common_source_over_composites_premultiplied_colors() {
		let source = r#"
			main: fn () -> void {
				results.composited = source_over(
					vec4f(0.2, 0.05, 0.025, 0.25),
					vec4f(0.2, 0.4, 0.6, 0.5)
				);
			}
		"#;
		let members = vec![besl::ParserNode::member("composited", "vec4f")];
		let (program, mut results) = compile_common_main(source, members, Vec::new());
		run_common(&program, &mut results);

		assert_floats_close(read_vec4f(&results, "composited"), [0.35, 0.35, 0.475, 0.625], 0.000001);
	}

	/// Verifies the reusable scalar, vector, sampling-direction, and coordinate helpers together.
	#[test]
	fn common_geometry_helpers_execute_with_expected_vm_results() {
		let source = r#"
			main: fn () -> void {
				results.squared_vec2 = vec2f_squared_length(vec2f(3.0, 4.0));
				results.squared_vec3 = vec3f_squared_length(vec3f(1.0, 2.0, 2.0));
				results.squared_vec4 = vec4f_squared_length(vec4f(1.0, 2.0, 2.0, 4.0));
				results.min_a = min_diff(vec3f(0.0, 0.0, 0.0), vec3f(1.0, 0.0, 0.0), vec3f(3.0, 0.0, 0.0));
				results.min_b = min_diff(vec3f(0.0, 0.0, 0.0), vec3f(4.0, 0.0, 0.0), vec3f(2.0, 0.0, 0.0));
				results.min_tie = min_diff(vec3f(0.0, 0.0, 0.0), vec3f(1.0, 0.0, 0.0), vec3f(0.0, 1.0, 0.0));
				results.noise = interleaved_gradient_noise(10, 20, 1);
				results.periodic_noise = interleaved_gradient_noise(10, 20, 65);
				results.sine = sin_from_tan(1.73205080757);
				results.tangent_value = tangent(vec3f(0.0, 0.0, 1.0), vec3f(3.0, 4.0, 6.0));
				results.normal = make_normal_from_positions(
					vec3f(0.0, 0.0, 0.0),
					vec3f(1.0, 0.0, 0.0),
					vec3f(0.0 - 2.0, 0.0, 0.0),
					vec3f(0.0, 1.0, 0.0),
					vec3f(0.0, 0.0 - 2.0, 0.0)
				);
				results.perpendicular_x = make_perpendicular_vector(vec3f(1.0, 2.0, 0.1));
				results.perpendicular_z = make_perpendicular_vector(vec3f(0.1, 2.0, 1.0));
				results.snapped_uv = snap_uv(vec2f(0.26, 0.74), vec2u(4, 8));
				results.hemisphere = make_cosine_hemisphere_sample(0.25, 0.0, vec3f(0.0, 0.0, 1.0));
				results.hemisphere_normal = make_cosine_hemisphere_sample(0.0, 0.75, vec3f(0.0, 0.0, 1.0));
				results.pixel_uv = make_uv(vec2i(1, 1), vec2u(4, 2));
				results.rotated = rotate_directions(vec2f(1.0, 0.0), vec2f(0.0, 1.0));
			}
		"#;
		let members = vec![
			besl::ParserNode::member("squared_vec2", "f32"),
			besl::ParserNode::member("squared_vec3", "f32"),
			besl::ParserNode::member("squared_vec4", "f32"),
			besl::ParserNode::member("min_a", "vec3f"),
			besl::ParserNode::member("min_b", "vec3f"),
			besl::ParserNode::member("min_tie", "vec3f"),
			besl::ParserNode::member("noise", "f32"),
			besl::ParserNode::member("periodic_noise", "f32"),
			besl::ParserNode::member("sine", "f32"),
			besl::ParserNode::member("tangent_value", "f32"),
			besl::ParserNode::member("normal", "vec3f"),
			besl::ParserNode::member("perpendicular_x", "vec3f"),
			besl::ParserNode::member("perpendicular_z", "vec3f"),
			besl::ParserNode::member("snapped_uv", "vec2f"),
			besl::ParserNode::member("hemisphere", "vec3f"),
			besl::ParserNode::member("hemisphere_normal", "vec3f"),
			besl::ParserNode::member("pixel_uv", "vec2f"),
			besl::ParserNode::member("rotated", "vec2f"),
		];
		let (program, mut results) = compile_common_main(source, members, Vec::new());
		run_common(&program, &mut results);

		assert_eq!(read_f32(&results, "squared_vec2"), 25.0);
		assert_eq!(read_f32(&results, "squared_vec3"), 9.0);
		assert_eq!(read_f32(&results, "squared_vec4"), 25.0);
		assert_eq!(read_vec3f(&results, "min_a"), [1.0, 0.0, 0.0]);
		assert_eq!(read_vec3f(&results, "min_b"), [-2.0, 0.0, 0.0]);
		assert_eq!(read_vec3f(&results, "min_tie"), [0.0, -1.0, 0.0]);

		let wrapped_frame = 1.0_f32;
		let x = 10.0 + 5.588238 * wrapped_frame;
		let y = 20.0 + 5.588238 * wrapped_frame;
		let expected_noise = (52.9829189 * (0.06711056 * x + 0.00583715 * y).fract()).fract();
		assert_f32_close(read_f32(&results, "noise"), expected_noise, 0.00001);
		assert_eq!(read_f32(&results, "noise"), read_f32(&results, "periodic_noise"));
		assert_f32_close(read_f32(&results, "sine"), 0.8660254, 0.00001);
		assert_f32_close(read_f32(&results, "tangent_value"), -1.0, 0.00001);
		assert_floats_close(read_vec3f(&results, "normal"), [0.0, 0.0, 1.0], 0.00001);
		assert_floats_close(read_vec3f(&results, "perpendicular_x"), [-0.8944272, 0.4472136, 0.0], 0.00001);
		assert_floats_close(read_vec3f(&results, "perpendicular_z"), [0.0, -0.4472136, 0.8944272], 0.00001);
		assert_floats_close(read_vec2f(&results, "snapped_uv"), [0.25, 0.75], 0.00001);
		assert_floats_close(read_vec3f(&results, "hemisphere"), [-0.5, 0.0, 0.8660254], 0.00001);
		assert_floats_close(read_vec3f(&results, "hemisphere_normal"), [0.0, 0.0, 1.0], 0.00001);
		assert_floats_close(read_vec2f(&results, "pixel_uv"), [0.375, 0.75], 0.00001);
		assert_floats_close(read_vec2f(&results, "rotated"), [0.0, 1.0], 0.00001);
	}

	/// Verifies the PBR distribution, geometry, and Fresnel contracts at analytically known inputs.
	#[test]
	fn common_pbr_helpers_execute_with_expected_vm_results() {
		let source = r#"
			main: fn () -> void {
				let normal: vec3f = vec3f(0.0, 0.0, 1.0);
				let half_vector: vec3f = vec3f(0.0, 0.0, 1.0);
				let view: vec3f = vec3f(0.8660254, 0.0, 0.5);
				let light: vec3f = vec3f(0.0, 0.8660254, 0.5);
				let f0: vec3f = vec3f(0.04, 0.04, 0.04);
				results.distribution = distribution_ggx(normal, half_vector, 0.5);
				results.distribution_roughness_one = distribution_ggx(normal, view, 1.0);
				results.geometry = geometry_schlick_ggx(0.5, 0.5);
				results.geometry_zero = geometry_schlick_ggx(0.0, 0.5);
				results.geometry_one = geometry_schlick_ggx(1.0, 0.5);
				results.geometry_smith = geometry_smith(normal, view, light, 0.5);
				results.fresnel = fresnel_schlick(0.5, f0);
				results.fresnel_normal = fresnel_schlick(1.0, f0);
				results.fresnel_grazing = fresnel_schlick(0.0, f0);
				results.rough_fresnel = fresnel_schlick_roughness(0.5, f0, 0.5);
				results.rough_fresnel_grazing = fresnel_schlick_roughness(0.0, f0, 0.5);
			}
		"#;
		let members = vec![
			besl::ParserNode::member("distribution", "f32"),
			besl::ParserNode::member("distribution_roughness_one", "f32"),
			besl::ParserNode::member("geometry", "f32"),
			besl::ParserNode::member("geometry_zero", "f32"),
			besl::ParserNode::member("geometry_one", "f32"),
			besl::ParserNode::member("geometry_smith", "f32"),
			besl::ParserNode::member("fresnel", "vec3f"),
			besl::ParserNode::member("fresnel_normal", "vec3f"),
			besl::ParserNode::member("fresnel_grazing", "vec3f"),
			besl::ParserNode::member("rough_fresnel", "vec3f"),
			besl::ParserNode::member("rough_fresnel_grazing", "vec3f"),
		];
		let (program, mut results) = compile_common_main(source, members, Vec::new());
		run_common(&program, &mut results);

		let expected_geometry = 0.5 / (0.5 * (1.0 - 0.28125) + 0.28125);
		assert_f32_close(read_f32(&results, "distribution"), 16.0 / std::f32::consts::PI, 0.00001);
		assert_f32_close(
			read_f32(&results, "distribution_roughness_one"),
			1.0 / std::f32::consts::PI,
			0.00001,
		);
		assert_f32_close(read_f32(&results, "geometry"), expected_geometry, 0.00001);
		assert_eq!(read_f32(&results, "geometry_zero"), 0.0);
		assert_eq!(read_f32(&results, "geometry_one"), 1.0);
		assert_f32_close(
			read_f32(&results, "geometry_smith"),
			expected_geometry * expected_geometry,
			0.00001,
		);
		assert_floats_close(read_vec3f(&results, "fresnel"), [0.07; 3], 0.00001);
		assert_floats_close(read_vec3f(&results, "fresnel_normal"), [0.04; 3], 0.00001);
		assert_floats_close(read_vec3f(&results, "fresnel_grazing"), [1.0; 3], 0.00001);
		assert_floats_close(read_vec3f(&results, "rough_fresnel"), [0.054375; 3], 0.00001);
		assert_floats_close(read_vec3f(&results, "rough_fresnel_grazing"), [0.5; 3], 0.00001);
	}

	/// Verifies barycentric coordinates, derivatives, interpolation, raster mapping, and normal decoding.
	#[test]
	fn common_interpolation_helpers_execute_with_expected_vm_results() {
		let source = r#"
			main: fn () -> void {
				let weights: vec3f = calculate_barycentric_from_position(
					vec3f(0.5, 0.5, 0.0),
					vec3f(0.0, 0.0, 0.0),
					vec3f(2.0, 0.0, 0.0),
					vec3f(0.0, 2.0, 0.0)
				);
				let full: BarycentricDeriv = calculate_full_bary(
					vec4f(0.0 - 1.0, 0.0 - 1.0, 0.0, 1.0),
					vec4f(1.0, 0.0 - 1.0, 0.0, 1.0),
					vec4f(0.0 - 1.0, 1.0, 0.0, 1.0),
					vec2f(0.0 - 0.5, 0.0 - 0.5),
					vec2f(4.0, 4.0)
				);
				results.barycentric = weights;
				results.degenerate = calculate_barycentric_from_position(
					vec3f(1.0, 0.0, 0.0),
					vec3f(0.0, 0.0, 0.0),
					vec3f(1.0, 0.0, 0.0),
					vec3f(2.0, 0.0, 0.0)
				);
				results.full_lambda = full.lambda;
				results.full_ddx = full.ddx;
				results.full_ddy = full.ddy;
				results.interpolated_vec3 = interpolate_vec3f_with_deriv(
					weights,
					vec3f(0.0, 0.0, 0.0),
					vec3f(1.0, 0.0, 0.0),
					vec3f(0.0, 1.0, 0.0)
				);
				results.interpolated_vec2 = interpolate_vec2f_with_deriv(
					weights,
					vec2f(0.0, 0.0),
					vec2f(1.0, 0.0),
					vec2f(0.0, 1.0)
				);
				results.raster_ndc = make_raster_ndc_from_pixel_coordinates(vec2i(0, 0), vec2i(4, 2));
				results.raster_ndc_last = make_raster_ndc_from_pixel_coordinates(vec2i(3, 1), vec2i(4, 2));
				results.unit_vector = unit_vector_from_xy(vec2f(0.5, 0.5));
				results.unit_vector_edge = unit_vector_from_xy(vec2f(1.0, 0.5));
				results.unit_vector_outside = unit_vector_from_xy(vec2f(1.0, 1.0));
			}
		"#;
		let members = vec![
			besl::ParserNode::member("barycentric", "vec3f"),
			besl::ParserNode::member("degenerate", "vec3f"),
			besl::ParserNode::member("full_lambda", "vec3f"),
			besl::ParserNode::member("full_ddx", "vec3f"),
			besl::ParserNode::member("full_ddy", "vec3f"),
			besl::ParserNode::member("interpolated_vec3", "vec3f"),
			besl::ParserNode::member("interpolated_vec2", "vec2f"),
			besl::ParserNode::member("raster_ndc", "vec2f"),
			besl::ParserNode::member("raster_ndc_last", "vec2f"),
			besl::ParserNode::member("unit_vector", "vec3f"),
			besl::ParserNode::member("unit_vector_edge", "vec3f"),
			besl::ParserNode::member("unit_vector_outside", "vec3f"),
		];
		let (program, mut results) = compile_common_main(source, members, Vec::new());
		run_common(&program, &mut results);

		assert_floats_close(read_vec3f(&results, "barycentric"), [0.5, 0.25, 0.25], 0.00001);
		assert_eq!(read_vec3f(&results, "degenerate"), [1.0, 0.0, 0.0]);
		assert_floats_close(read_vec3f(&results, "full_lambda"), [0.5, 0.25, 0.25], 0.00001);
		assert_floats_close(read_vec3f(&results, "full_ddx"), [-0.25, 0.25, 0.0], 0.00001);
		assert_floats_close(read_vec3f(&results, "full_ddy"), [-0.25, 0.0, 0.25], 0.00001);
		assert_floats_close(read_vec3f(&results, "interpolated_vec3"), [0.25, 0.25, 0.0], 0.00001);
		assert_floats_close(read_vec2f(&results, "interpolated_vec2"), [0.25, 0.25], 0.00001);
		assert_floats_close(read_vec2f(&results, "raster_ndc"), [-0.75, 0.5], 0.00001);
		assert_floats_close(read_vec2f(&results, "raster_ndc_last"), [0.75, -0.5], 0.00001);
		assert_floats_close(read_vec3f(&results, "unit_vector"), [0.0, 0.0, 1.0], 0.00001);
		assert_floats_close(read_vec3f(&results, "unit_vector_edge"), [1.0, 0.0, 0.0], 0.00001);
		assert_floats_close(
			read_vec3f(&results, "unit_vector_outside"),
			[std::f32::consts::FRAC_1_SQRT_2, std::f32::consts::FRAC_1_SQRT_2, 0.0],
			0.00001,
		);
	}

	/// Verifies direct, fetched, sampled, and neighborhood depth reconstruction through VM textures.
	#[test]
	fn common_depth_helpers_execute_with_expected_vm_results() {
		let source = r#"
			main: fn () -> void {
				let identity: mat4f = mat4f(
					vec4f(1.0, 0.0, 0.0, 0.0),
					vec4f(0.0, 1.0, 0.0, 0.0),
					vec4f(0.0, 0.0, 1.0, 0.0),
					vec4f(0.0, 0.0, 0.0, 1.0)
				);
				let perspective_w: mat4f = mat4f(
					vec4f(1.0, 0.0, 0.0, 0.0),
					vec4f(0.0, 1.0, 0.0, 0.0),
					vec4f(0.0, 0.0, 1.0, 0.0),
					vec4f(0.0, 0.0, 0.0, 2.0)
				);
				let translated_view: mat4f = mat4f(
					vec4f(1.0, 0.0, 0.0, 0.0),
					vec4f(0.0, 1.0, 0.0, 0.0),
					vec4f(0.0, 0.0, 1.0, 0.0),
					vec4f(10.0, 20.0, 30.0, 1.0)
				);
				results.direct_world = make_world_space_position_from_depth(0.25, vec2f(0.25, 0.75), identity, identity);
				results.perspective_world = make_world_space_position_from_depth(
					0.25,
					vec2f(0.25, 0.75),
					perspective_w,
					translated_view
				);
				results.fetched_world = get_world_space_position_from_depth(depth_map, vec2u(1, 1), identity, identity);
				results.sampled_view = get_view_space_position_from_depth(sample_depth_map, vec2f(0.5, 0.5), identity);
				results.normal = make_normal_from_depth_map(depth_map, vec2i(1, 1), vec2u(3, 3), identity, identity);
				results.empty_normal = make_normal_from_depth_map(empty_depth_map, vec2i(1, 1), vec2u(3, 3), identity, identity);
			}
		"#;
		let members = vec![
			besl::ParserNode::member("direct_world", "vec3f"),
			besl::ParserNode::member("perspective_world", "vec3f"),
			besl::ParserNode::member("fetched_world", "vec3f"),
			besl::ParserNode::member("sampled_view", "vec3f"),
			besl::ParserNode::member("normal", "vec3f"),
			besl::ParserNode::member("empty_normal", "vec3f"),
		];
		let bindings = vec![
			besl::ParserNode::binding(
				"depth_map",
				besl::ParserNode::combined_image_sampler(),
				DEPTH_SLOT.slot(),
				true,
				false,
			),
			besl::ParserNode::binding(
				"empty_depth_map",
				besl::ParserNode::combined_image_sampler(),
				EMPTY_DEPTH_SLOT.slot(),
				true,
				false,
			),
			besl::ParserNode::binding(
				"sample_depth_map",
				besl::ParserNode::combined_image_sampler(),
				SAMPLE_DEPTH_SLOT.slot(),
				true,
				false,
			),
		];
		let (program, mut results) = compile_common_main(source, members, bindings);
		let planar_texels = [[0.25, 0.0, 0.0, 1.0]; 9];
		let mut depth_map = texture_2d(3, 3, &planar_texels);
		let mut empty_texels = planar_texels;
		empty_texels[4][0] = 0.0;
		let mut empty_depth_map = texture_2d(3, 3, &empty_texels);
		let mut sample_depth_map = texture_2d(
			2,
			2,
			&[
				[0.0, 0.0, 0.0, 1.0],
				[0.2, 0.0, 0.0, 1.0],
				[0.4, 0.0, 0.0, 1.0],
				[0.8, 0.0, 0.0, 1.0],
			],
		);
		{
			let mut descriptors = DescriptorBindings::new();
			descriptors.bind_buffer(RESULT_SLOT, &mut results);
			descriptors.bind_texture(DEPTH_SLOT, &mut depth_map);
			descriptors.bind_texture(EMPTY_DEPTH_SLOT, &mut empty_depth_map);
			descriptors.bind_texture(SAMPLE_DEPTH_SLOT, &mut sample_depth_map);
			run_at(&program, &mut descriptors, [0, 0]);
		}

		assert_floats_close(read_vec3f(&results, "direct_world"), [-0.5, 0.5, 0.25], 0.00001);
		assert_floats_close(read_vec3f(&results, "perspective_world"), [9.75, 20.25, 30.125], 0.00001);
		assert_floats_close(read_vec3f(&results, "fetched_world"), [0.0, 0.0, 0.25], 0.00001);
		assert_floats_close(read_vec3f(&results, "sampled_view"), [0.0, 0.0, 0.35], 0.00001);
		assert_floats_close(read_vec3f(&results, "normal"), [0.0, 0.0, 1.0], 0.00001);
		assert_eq!(read_vec3f(&results, "empty_normal"), [0.0, 0.0, 0.0]);
	}

	/// Verifies every debug palette entry and the modulo wrap contract through the production helper.
	#[test]
	fn common_debug_colors_execute_with_expected_vm_results() {
		let source = r#"
			main: fn () -> void {
				results.colors[0] = get_debug_color(0);
				results.colors[1] = get_debug_color(1);
				results.colors[2] = get_debug_color(2);
				results.colors[3] = get_debug_color(3);
				results.colors[4] = get_debug_color(4);
				results.colors[5] = get_debug_color(5);
				results.colors[6] = get_debug_color(6);
				results.colors[7] = get_debug_color(7);
				results.colors[8] = get_debug_color(8);
				results.colors[9] = get_debug_color(9);
				results.colors[10] = get_debug_color(10);
				results.colors[11] = get_debug_color(11);
				results.colors[12] = get_debug_color(12);
				results.colors[13] = get_debug_color(13);
				results.colors[14] = get_debug_color(14);
				results.colors[15] = get_debug_color(15);
				results.colors[16] = get_debug_color(16);
			}
		"#;
		let members = vec![besl::ParserNode::member("colors", "vec4f[17]")];
		let (program, mut results) = compile_common_main(source, members, Vec::new());
		run_common(&program, &mut results);

		let expected = [
			[0.16863, 0.40392, 0.77647, 1.0],
			[0.32941, 0.76863, 0.21961, 1.0],
			[0.81961, 0.16078, 0.67451, 1.0],
			[0.96863, 0.98824, 0.45490, 1.0],
			[0.75294, 0.09020, 0.75686, 1.0],
			[0.30588, 0.95686, 0.54510, 1.0],
			[0.66667, 0.06667, 0.75686, 1.0],
			[0.78824, 0.91765, 0.27451, 1.0],
			[0.40980, 0.12745, 0.48627, 1.0],
			[0.89804, 0.28235, 0.20784, 1.0],
			[0.93725, 0.67843, 0.33725, 1.0],
			[0.95294, 0.96863, 0.00392, 1.0],
			[1.00000, 0.27843, 0.67843, 1.0],
			[0.29020, 0.90980, 0.56863, 1.0],
			[0.30980, 0.70980, 0.27059, 1.0],
			[0.69804, 0.16078, 0.39216, 1.0],
		];
		for (index, expected) in expected.into_iter().enumerate() {
			let value = results
				.read_indexed("colors", index)
				.expect("Missing debug color result. The most likely cause is an incorrect test array layout.");
			let Value::Vec4F(actual) = value else {
				panic!(
					"Invalid debug color result `{value:?}`. The most likely cause is an incorrect test array element type."
				);
			};
			assert_floats_close(actual, expected, 0.00001);
		}

		let wrapped = results
			.read_indexed("colors", 16)
			.expect("Missing wrapped debug color. The most likely cause is an incorrect test array layout.");
		assert_eq!(wrapped, Value::Vec4F(expected[0]));
	}
}
