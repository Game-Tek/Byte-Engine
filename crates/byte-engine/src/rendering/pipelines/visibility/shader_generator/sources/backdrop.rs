//! The helper blended materials read the position of the opaque scene behind their surfaces with.

/// Returns the world-space position of the opaque surface the backdrop holds at the screen position `at`, as `view`, the
/// camera's, saw it.
///
/// The screen view constants turn a pixel of the depth pyramid into its view ray, `pixel * pixel_to_ray_mul +
/// pixel_to_ray_add`, from the same projection the backdrop's linear depth came from. At the screen's top-left corner,
/// half a pixel before the first pixel, that ray is `pixel_to_ray_add - pixel_to_ray_mul / 2`, and it falls linearly to
/// its negation at the bottom-right corner, so the pyramid's resolution cancels out. The sky's depth is the far depth,
/// so it resolves to the far plane.
pub(crate) const BACKDROP_POSITION_SOURCE: &str = r#"
backdrop_position: fn (material_evaluation: MaterialEvaluation, view: View, at: vec2f) -> vec3f {
	let depth: f32 = texture_lod(material_evaluation.backdrop_depth, at, 0.0).x;
	let corner: vec2f = material_evaluation.ssgi_view.pixel_to_ray_add - material_evaluation.ssgi_view.pixel_to_ray_mul * 0.5;
	let ray: vec2f = corner * (vec2f(1.0, 1.0) - at * 2.0) * depth;
	return view.inverse_view * vec4f(ray.x, ray.y, depth * material_evaluation.ssgi_view.view_z_sign, 1.0);
}
"#;
