//! The statements wrapped around a material body in a closest-hit shader, which reports the surface a ray hit.
//!
//! The prefix rebuilds the hit point from what the ray tracing pipeline passes, under the same names material
//! evaluation's prefix gives the shaded pixel, so one material body lowers for both. The suffix writes the payload the
//! surfel shade pass lights.

pub(crate) const MATERIAL_HIT_PREFIX_SOURCE: &str = r#"
material_hit_prefix: fn (input: StageInput) -> void {
	// The geometry table leads a hit to the frame's record of the primitive it hit and to the primitive's first
	// triangle in the flat triangle lists.
	let geometry: vec2u = ray_tracing_geometries.geometries[input.hit_custom_index + input.hit_geometry_index];
	let mesh: Mesh = meshes.meshes[geometry.x];
	let material: Material = materials.materials[mesh.material_index];
	let first_index: u32 = (geometry.y + input.hit_primitive_index) * 3;
	let triangle_vertex_indices: u32[3] = u32[3](
		mesh.base_vertex_index + u16_to_u32(flat_triangle_indices[first_index]),
		mesh.base_vertex_index + u16_to_u32(flat_triangle_indices[first_index + 1]),
		mesh.base_vertex_index + u16_to_u32(flat_triangle_indices[first_index + 2])
	);
	let model: mat4x3f = mesh.model;
	let position0: vec3f = vertex_positions[triangle_vertex_indices[0]];
	let position1: vec3f = vertex_positions[triangle_vertex_indices[1]];
	let position2: vec3f = vertex_positions[triangle_vertex_indices[2]];
	let world_vertex_position0: vec3f = model * vec4f(position0.x, position0.y, position0.z, 1.0);
	let world_vertex_position1: vec3f = model * vec4f(position1.x, position1.y, position1.z, 1.0);
	let world_vertex_position2: vec3f = model * vec4f(position2.x, position2.y, position2.z, 1.0);
	let world_edge1: vec3f = world_vertex_position1 - world_vertex_position0;
	let world_edge2: vec3f = world_vertex_position2 - world_vertex_position0;
	// The weights of the second and third vertices at the hit; the first vertex weighs the rest.
	let barycentric: vec2f = input.hit_barycentrics;
	let world_space_vertex_position: vec3f = world_vertex_position0 + barycentric.x * world_edge1
		+ barycentric.y * world_edge2;
	// As in material evaluation, unit vertex normals are interpolated in model space and turned into world space once.
	let model_space_vertex_normal0: vec3f = normalize(decode_octahedral_normal(vertex_normals[triangle_vertex_indices[0]]));
	let model_space_vertex_normal1: vec3f = normalize(decode_octahedral_normal(vertex_normals[triangle_vertex_indices[1]]));
	let model_space_vertex_normal2: vec3f = normalize(decode_octahedral_normal(vertex_normals[triangle_vertex_indices[2]]));
	let model_space_normal: vec3f = model_space_vertex_normal0
		+ barycentric.x * (model_space_vertex_normal1 - model_space_vertex_normal0)
		+ barycentric.y * (model_space_vertex_normal2 - model_space_vertex_normal0);
	let N: vec3f = normalize(model * vec4f(model_space_normal.x, model_space_normal.y, model_space_normal.z, 0.0));
	let V: vec3f = normalize(vec3f(0.0, 0.0, 0.0) - input.ray_direction);
	// The triangle's plane, turned to the side its vertex normals face, tells the front of the surface from its back
	// for any winding. Both normals then turn to the side the ray came from.
	let surface_normal: vec3f = normalize(cross(world_edge1, world_edge2));
	surface_normal = surface_normal * (step(0.0, dot(surface_normal, N)) * 2.0 - 1.0);
	let front_face: f32 = step(0.0, dot(surface_normal, V));
	surface_normal = surface_normal * (front_face * 2.0 - 1.0);
	N = N * (front_face * 2.0 - 1.0);
	// One where the surface reflects and emits light toward the ray: its front, or either side of a double-sided
	// surface, which flag bit 0 marks.
	let facing: f32 = max(front_face, f32(mesh.flags & 1));
}
"#;

/// Interpolates the hit's texture coordinates and sizes their derivatives with a ray cone, after "Improved Shader and
/// Texture Level of Detail Using Ray Cones" by Tomas Akenine-Möller et al. (2021).
///
/// The cone's width at the hit, divided by the cosine it meets the surface at, is the footprint's width in world
/// units, and the triangle's ratio of texture area to world area turns it into texture units. Equal derivatives along
/// both axes make the sampler pick the level of detail that width asks for.
pub(crate) const MATERIAL_HIT_UV_SOURCE: &str = r#"
material_hit_uv: fn (input: StageInput) -> void {
	let vertex_uv0: vec2f = decode_f16_vec2(vertex_uvs[triangle_vertex_indices[0]]);
	let uv_edge1: vec2f = decode_f16_vec2(vertex_uvs[triangle_vertex_indices[1]]) - vertex_uv0;
	let uv_edge2: vec2f = decode_f16_vec2(vertex_uvs[triangle_vertex_indices[2]]) - vertex_uv0;
	let vertex_uv: vec2f = vertex_uv0 + barycentric.x * uv_edge1 + barycentric.y * uv_edge2;
	let world_area: f32 = length(cross(world_edge1, world_edge2));
	let uv_area: f32 = abs(uv_edge1.x * uv_edge2.y - uv_edge2.x * uv_edge1.y);
	let cone_width: f32 = HIT_CONE_SPREAD * input.hit_distance * length(input.ray_direction);
	let footprint: f32 = cone_width / max(dot(surface_normal, V), 0.01)
		* sqrt(uv_area / max(world_area, 0.000000000001));
	let uv_derivative_x: vec2f = vec2f(footprint, 0.0);
	let uv_derivative_y: vec2f = vec2f(0.0, footprint);
}
"#;

/// Builds the triangle's tangent frame from its edges in world space and in texture space, the way material evaluation
/// builds it from a pixel's derivatives.
pub(crate) const MATERIAL_HIT_TANGENT_SOURCE: &str = r#"
material_hit_tangent: fn () -> void {
	// T and B are normalized, so only the sign of the UV mapping's determinant matters, and a mapping without area
	// leaves the frame at its length floor instead of turning it into NaN.
	let uv_determinant: f32 = uv_edge1.x * uv_edge2.y - uv_edge2.x * uv_edge1.y;
	let tangent_sign: f32 = step(0.0, uv_determinant) * 2.0 - 1.0;
	let tangent_direction: vec3f = tangent_sign * (uv_edge2.y * world_edge1 - uv_edge1.y * world_edge2);
	let bitangent_direction: vec3f = tangent_sign * ((0.0 - uv_edge2.x) * world_edge1 + uv_edge1.x * world_edge2);
	let length_squared_floor: f32 = 0.000000000000000000000000000001;
	let T: vec3f16 = vec3f16(
		tangent_direction * inversesqrt(max(dot(tangent_direction, tangent_direction), length_squared_floor))
	);
	let B: vec3f16 = vec3f16(
		bitangent_direction * inversesqrt(max(dot(bitangent_direction, bitangent_direction), length_squared_floor))
	);
}
"#;

/// Declares the outputs a hit's material body writes, at their compact types.
pub(crate) const MATERIAL_HIT_DEFAULTS_SOURCE: &str = r#"
material_hit_defaults: fn () -> void {
	let albedo: vec3f16 = vec3f16(1.0, 0.0, 0.0);
	let metalness: f16 = 0.0;
	let emission: vec3f16 = vec3f16(0.0, 0.0, 0.0);
}
"#;

/// Writes the payload. Rays carry only diffuse light into surfels, so the hit reports the base color times one minus
/// the metalness. The back of a single-sided surface reports neither albedo nor emission, so light doesn't leak
/// through walls seen from behind.
pub(crate) const MATERIAL_HIT_SUFFIX_SOURCE: &str = r#"
material_hit_suffix: fn (payload: HitSurface) -> HitSurface {
	let hit_surface: HitSurface = payload;
	hit_surface.albedo = vec3f16(vec3f(albedo) * ((1.0 - f32(metalness)) * facing));
	hit_surface.front_face = u32(facing);
	hit_surface.emission = vec3f(emission) * facing;
	hit_surface.normal = vec3f16(N);
	return hit_surface;
}
"#;
