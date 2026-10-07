pub(crate) const DECODE_F16_VEC2_SOURCE: &str = r#"
decode_f16_vec2: fn (encoded: vec2f16) -> vec2f {
	return vec2f(encoded);
}
"#;

pub(crate) const DECODE_OCTAHEDRAL_NORMAL_SOURCE: &str = r#"
decode_octahedral_normal: fn (encoded: vec2u16) -> vec3f {
	// Combine UNORM expansion and the [-1, 1] remap so each component needs one scale.
	let octahedral: vec2f = vec2f(f32(u32(encoded.x)), f32(u32(encoded.y))) * 0.00003051804379339284
		- vec2f(1.0, 1.0);
	let normal_z: f32 = 1.0 - abs(octahedral.x) - abs(octahedral.y);
	// The lower hemisphere is folded over the diagonals. The fold is zero on the upper one, so no branch is needed.
	let fold: f32 = max(0.0 - normal_z, 0.0);
	return vec3f(
		octahedral.x - (step(0.0, octahedral.x) * 2.0 - 1.0) * fold,
		octahedral.y - (step(0.0, octahedral.y) * 2.0 - 1.0) * fold,
		normal_z
	);
}
"#;

pub(crate) const MATERIAL_EVALUATION_PREFIX_SOURCE: &str = r#"
material_evaluation_prefix: fn (input: StageInput) -> void {
	let invocation: vec2u = input.thread_id;
	// One dispatch shades the pixel list of an evaluation slot, which holds the pixels of every material that compiled to
	// this pipeline. Each pixel reads its own material below.
	if (invocation.x >= material_count.material_count[push_constant.evaluation_index]) {
		return;
	}

	let offset: u32 = material_offset.material_offset[push_constant.evaluation_index];
	let packed_pixel_coordinates: vec2u16 = pixel_mapping.pixel_mapping[offset + invocation.x];
	let raw_pixel_coordinates: vec2u = vec2u(
		u32(packed_pixel_coordinates.x),
		u32(packed_pixel_coordinates.y)
	);
	if (raw_pixel_coordinates.x == 0 || raw_pixel_coordinates.y == 0) {
		return;
	}
	let pixel_coordinates: vec2u = raw_pixel_coordinates - vec2u(1, 1);
	let image_extent: vec2u = image_size(triangle_index);
	if (pixel_coordinates.x >= image_extent.x || pixel_coordinates.y >= image_extent.y) {
		return;
	}

	let triangle_meshlet_indices: u32 = image_load_u32(triangle_index, pixel_coordinates);
	let instance_index: u32 = image_load_u32(instance_index_render_target, pixel_coordinates);
	let meshlet_triangle_index: u32 = triangle_meshlet_indices & 255;
	let meshlet_index: u32 = triangle_meshlet_indices >> 8;
	let meshlet: Meshlet = meshlets[meshlet_index];
	let mesh: Mesh = meshes.meshes[instance_index];
	// Materials of one slot run the same program and differ only in the textures they bind, so the texture slots come
	// from the pixel's own material. Pixel mapping groups pixels by tile, so neighboring lanes usually share it.
	let material: Material = materials.materials[mesh.material_index];

	let primitive_index_base: u32 = (mesh.base_triangle_index + meshlet.triangle_offset + meshlet_triangle_index) * 3;
	let triangle_vertex_indices: u32[3] = compute_vertex_indices(mesh, meshlet, primitive_index_base);
	// Every lane sets up its own triangle. Lanes of one SIMD group often share a triangle, but a region that only a
	// leader lane runs issues the same instructions as one every lane runs, so sharing the setup through ballots and
	// broadcasts only added their own cost: material evaluation ran 3 % faster without it (2026-10-05).
	let model_space_vertex_positions: vec4f[3] = vec4f[3](
		vec4f(0.0, 0.0, 0.0, 1.0),
		vec4f(0.0, 0.0, 0.0, 1.0),
		vec4f(0.0, 0.0, 0.0, 1.0)
	);
	let model_space_vertex_normals: vec3f[3] = vec3f[3](
		vec3f(0.0, 0.0, 1.0),
		vec3f(0.0, 0.0, 1.0),
		vec3f(0.0, 0.0, 1.0)
	);

	if (mesh.skinned_base_vertex_index != 4294967295) {
		let skinned_vertex_indices: u32[3] = u32[3](
			mesh.skinned_base_vertex_index + (triangle_vertex_indices[0] - mesh.base_vertex_index),
			mesh.skinned_base_vertex_index + (triangle_vertex_indices[1] - mesh.base_vertex_index),
			mesh.skinned_base_vertex_index + (triangle_vertex_indices[2] - mesh.base_vertex_index)
		);
		let skinned_vertices_for_triangle: SkinnedVertex[3] = SkinnedVertex[3](
			skinned_vertices.vertices[skinned_vertex_indices[0]],
			skinned_vertices.vertices[skinned_vertex_indices[1]],
			skinned_vertices.vertices[skinned_vertex_indices[2]]
		);
		let skinned_position0: vec3f = skinned_vertices_for_triangle[0].position;
		let skinned_position1: vec3f = skinned_vertices_for_triangle[1].position;
		let skinned_position2: vec3f = skinned_vertices_for_triangle[2].position;
		model_space_vertex_positions[0] = vec4f(skinned_position0.x, skinned_position0.y, skinned_position0.z, 1.0);
		model_space_vertex_positions[1] = vec4f(skinned_position1.x, skinned_position1.y, skinned_position1.z, 1.0);
		model_space_vertex_positions[2] = vec4f(skinned_position2.x, skinned_position2.y, skinned_position2.z, 1.0);
		model_space_vertex_normals[0] = skinned_vertices_for_triangle[0].normal;
		model_space_vertex_normals[1] = skinned_vertices_for_triangle[1].normal;
		model_space_vertex_normals[2] = skinned_vertices_for_triangle[2].normal;
	} else {
		let position0: vec3f = vertex_positions[triangle_vertex_indices[0]];
		let position1: vec3f = vertex_positions[triangle_vertex_indices[1]];
		let position2: vec3f = vertex_positions[triangle_vertex_indices[2]];
		// Octahedral decoding leaves a normal shorter than one. Interpolation below weights the normals as they are,
		// so they are made unit length first, as skinning already writes them.
		let normal0: vec3f = normalize(decode_octahedral_normal(vertex_normals[triangle_vertex_indices[0]]));
		let normal1: vec3f = normalize(decode_octahedral_normal(vertex_normals[triangle_vertex_indices[1]]));
		let normal2: vec3f = normalize(decode_octahedral_normal(vertex_normals[triangle_vertex_indices[2]]));
		model_space_vertex_positions[0] = vec4f(position0.x, position0.y, position0.z, 1.0);
		model_space_vertex_positions[1] = vec4f(position1.x, position1.y, position1.z, 1.0);
		model_space_vertex_positions[2] = vec4f(position2.x, position2.y, position2.z, 1.0);
		model_space_vertex_normals[0] = normal0;
		model_space_vertex_normals[1] = normal1;
		model_space_vertex_normals[2] = normal2;
	}
	let nc: vec2f = make_raster_ndc_from_pixel_coordinates(pixel_coordinates, image_extent);
	let model: mat4x3f = mesh.model;
	let view_projection: mat4f = views.views[0].view_projection;
	let world_vertex_position0: vec3f = model * model_space_vertex_positions[0];
	let world_vertex_position1: vec3f = model * model_space_vertex_positions[1];
	let world_vertex_position2: vec3f = model * model_space_vertex_positions[2];
	let clip_vertex_position0: vec4f = view_projection * vec4f(world_vertex_position0.x, world_vertex_position0.y, world_vertex_position0.z, 1.0);
	let clip_vertex_position1: vec4f = view_projection * vec4f(world_vertex_position1.x, world_vertex_position1.y, world_vertex_position1.z, 1.0);
	let clip_vertex_position2: vec4f = view_projection * vec4f(world_vertex_position2.x, world_vertex_position2.y, world_vertex_position2.z, 1.0);
	// Perspective-correct barycentrics. A vertex's screen-space weight divided by its w is affine across the screen, and
	// so is the sum of the three, 1 / w, so each pixel's weights are the ratios of those planes at the pixel. They are
	// evaluated once and every attribute below is interpolated from the triangle's edges.
	let triangle_interpolation: TriangleInterpolation = compute_triangle_interpolation(
		clip_vertex_position0,
		clip_vertex_position1,
		clip_vertex_position2
	);
	let triangle_raw_ddx: vec3f = triangle_interpolation.raw_ddx;
	let triangle_raw_ddy: vec3f = triangle_interpolation.raw_ddy;
	let inverse_w_dx: f32 = dot(triangle_raw_ddx, vec3f(1.0, 1.0, 1.0));
	let inverse_w_dy: f32 = dot(triangle_raw_ddy, vec3f(1.0, 1.0, 1.0));
	let interpolation_delta: vec2f = nc - triangle_interpolation.origin;
	let inverse_w_at_pixel: f32 = triangle_interpolation.inverse_w.x + interpolation_delta.x * inverse_w_dx
		+ interpolation_delta.y * inverse_w_dy;
	let perspective_w: f32 = 1.0 / inverse_w_at_pixel;
	// The planes of vertices 1 and 2 start at zero at the planes' origin, vertex 0, whose weight is the rest.
	let weighted_barycentric: vec2f = vec2f(
		interpolation_delta.x * triangle_raw_ddx.y + interpolation_delta.y * triangle_raw_ddy.y,
		interpolation_delta.x * triangle_raw_ddx.z + interpolation_delta.y * triangle_raw_ddy.z
	);
	let barycentric: vec2f = weighted_barycentric * perspective_w;
	// How the weights change to the next pixel to the right and to the next one up in normalized device coordinates.
	// Like a raster pass's quad derivatives, these are differences between two pixels, not the planes' slope at this
	// one, so texture gradients stay what they were.
	let ndc_step_x: f32 = 2.0 / f32(image_extent.x);
	let ndc_step_y: f32 = 2.0 / f32(image_extent.y);
	let barycentric_dx: vec2f = (weighted_barycentric + vec2f(triangle_raw_ddx.y, triangle_raw_ddx.z) * ndc_step_x)
		/ (inverse_w_at_pixel + inverse_w_dx * ndc_step_x) - barycentric;
	let barycentric_dy: vec2f = (weighted_barycentric + vec2f(triangle_raw_ddy.y, triangle_raw_ddy.z) * ndc_step_y)
		/ (inverse_w_at_pixel + inverse_w_dy * ndc_step_y) - barycentric;

	let world_edge1: vec3f = world_vertex_position1 - world_vertex_position0;
	let world_edge2: vec3f = world_vertex_position2 - world_vertex_position0;
	let world_space_vertex_position: vec3f = world_vertex_position0 + barycentric.x * world_edge1
		+ barycentric.y * world_edge2;
	let position_derivative_x: vec3f = barycentric_dx.x * world_edge1 + barycentric_dx.y * world_edge2;
	let position_derivative_y: vec3f = barycentric_dy.x * world_edge1 + barycentric_dy.y * world_edge2;
	// The normal is interpolated in model space and turned into world space once, instead of turning all three. For a
	// model matrix with uniform scale, the direction is the same as turning each unit vertex normal first.
	let model_space_normal: vec3f = model_space_vertex_normals[0]
		+ barycentric.x * (model_space_vertex_normals[1] - model_space_vertex_normals[0])
		+ barycentric.y * (model_space_vertex_normals[2] - model_space_vertex_normals[0]);
	let world_space_normal: vec3f = model * vec4f(model_space_normal.x, model_space_normal.y, model_space_normal.z, 0.0);
	let world_space_vertex_normal: vec3f = normalize(world_space_normal);
	let N: vec3f = world_space_vertex_normal;
	let camera_position: vec3f = views.views[0].inverse_view * vec4f(0.0, 0.0, 0.0, 1.0);
	let V: vec3f = normalize(camera_position - world_space_vertex_position);
	// Flag bit 0 marks a double-sided material. Only those rasterize back faces, so only they need the check.
	if ((mesh.flags & 1) != 0) {
		N = facing_normal(N, V, world_edge1, world_edge2);
	}
	// Everything after the prefix reads the view vector and the interpolated normal at half precision: the BRDF, the
	// tangent frame, SSGI, and the reflection ray. Making the copies here ends their f32 originals with the prefix, so
	// they do not stay live through the material body, the screen-space reads, and the light loop.
	let V_material: vec3f16 = vec3f16(V);
	let geometric_normal: vec3f16 = vec3f16(N);
}
"#;

/// Reverses `normal` when the camera sees the back of the surface, as glTF requires for double-sided materials.
///
/// A compute pass has no front-facing signal. The pixel sees the back face when the camera and the interpolated
/// normal lie on opposite sides of the surface plane, which any two vectors along it give, for any triangle winding.
/// The material prefix passes the triangle's world-space edges, which decide the same way for every pixel of a
/// triangle, even one seen almost edge-on.
pub(crate) const FACING_NORMAL_SOURCE: &str = r#"
facing_normal: fn (normal: vec3f, to_camera: vec3f, surface_vector0: vec3f, surface_vector1: vec3f) -> vec3f {
	let surface_normal: vec3f = cross(surface_vector0, surface_vector1);
	if (dot(surface_normal, to_camera) * dot(surface_normal, normal) < 0.0) {
		return normal * (0.0 - 1.0);
	}
	return normal;
}
"#;

pub(crate) const MATERIAL_EVALUATION_UV_SOURCE: &str = r#"
material_evaluation_uv: fn () -> void {
	// Runtime UVs use half-float storage and are expanded only for materials that sample them. They are interpolated,
	// with their one-pixel differences, from the barycentrics of the prefix.
	let vertex_uv0: vec2f = decode_f16_vec2(vertex_uvs[triangle_vertex_indices[0]]);
	let uv_edge1: vec2f = decode_f16_vec2(vertex_uvs[triangle_vertex_indices[1]]) - vertex_uv0;
	let uv_edge2: vec2f = decode_f16_vec2(vertex_uvs[triangle_vertex_indices[2]]) - vertex_uv0;
	let vertex_uv: vec2f = vertex_uv0 + barycentric.x * uv_edge1 + barycentric.y * uv_edge2;
	let uv_derivative_x: vec2f = barycentric_dx.x * uv_edge1 + barycentric_dx.y * uv_edge2;
	let uv_derivative_y: vec2f = barycentric_dy.x * uv_edge1 + barycentric_dy.y * uv_edge2;
}
"#;

pub(crate) const MATERIAL_EVALUATION_TANGENT_SOURCE: &str = r#"
material_evaluation_tangent: fn () -> void {
	// T and B are normalized, so only the sign of the UV mapping's determinant matters. Taking the sign instead of
	// dividing by it keeps a mapping without area, such as a strip whose UVs lie on a line, from turning the frame into
	// NaN; the floor under the lengths does the same for a mapping whose UVs do not change at all.
	let uv_determinant: f32 = uv_derivative_x.x * uv_derivative_y.y - uv_derivative_y.x * uv_derivative_x.y;
	let tangent_sign: f32 = step(0.0, uv_determinant) * 2.0 - 1.0;
	let tangent_direction: vec3f = tangent_sign
		* (uv_derivative_y.y * position_derivative_x - uv_derivative_x.y * position_derivative_y);
	let bitangent_direction: vec3f = tangent_sign
		* ((0.0 - uv_derivative_y.x) * position_derivative_x + uv_derivative_x.x * position_derivative_y);
	let length_squared_floor: f32 = 0.000000000000000000000000000001;
	// The frame only rotates the half-precision material normal, so it is kept at half precision too.
	let T: vec3f16 = vec3f16(
		tangent_direction * inversesqrt(max(dot(tangent_direction, tangent_direction), length_squared_floor))
	);
	let B: vec3f16 = vec3f16(
		bitangent_direction * inversesqrt(max(dot(bitangent_direction, bitangent_direction), length_squared_floor))
	);
}
"#;

pub(crate) const MATERIAL_EVALUATION_DEFAULTS_SOURCE: &str = r#"
material_evaluation_defaults: fn () -> void {
	// Material inputs are normalized or artist-bounded values. Keep them compact until lighting needs f32 range.
	let albedo: vec4f16 = vec4f16(1.0, 0.0, 0.0, 1.0);
	let normal: vec3f16 = vec3f16(0.0, 0.0, 1.0);
	let metalness: f16 = 0.0;
	let roughness: f16 = 0.5;
	let occlusion: f16 = 1.0;
	let emission: vec3f16 = vec3f16(0.0, 0.0, 0.0);
}
"#;

pub(crate) const MATERIAL_EVALUATION_TANGENT_NORMAL_SOURCE: &str = r#"
material_evaluation_normal: fn () -> void {
	normal = vec3f16(normalize(f32(normal.x) * vec3f(T) + f32(normal.y) * vec3f(B) + f32(normal.z) * vec3f(geometric_normal)));
}
"#;

pub(crate) const MATERIAL_EVALUATION_GEOMETRY_NORMAL_SOURCE: &str = r#"
material_evaluation_normal: fn () -> void {
	normal = geometric_normal;
}
"#;

pub(crate) const IES_PROFILE_UV_SOURCE: &str = r#"
// Converts a light-to-surface ray into the full Type C IES texture domain.
// The orientation-packed C0 tangent defines the horizontal zero plane without a world-axis singularity.
ies_profile_uv: fn (
	emission_direction: vec3f,
	axis: vec3f,
	encoded_c0_tangent: vec2u16
) -> vec2f {
	let axial: f32 = clamp(dot(axis, emission_direction), 0.0 - 1.0, 1.0);
	let polar_radians: f32 = atan2(sqrt(max(1.0 - axial * axial, 0.0)), axial);
	let decoded_c0_tangent: vec3f = decode_octahedral_normal(encoded_c0_tangent);
	// Packing can introduce a small axial component, so restore the orthonormal IES frame before sampling.
	let c0_tangent: vec3f = normalize(decoded_c0_tangent - axis * dot(axis, decoded_c0_tangent));
	let c90_tangent: vec3f = cross(axis, c0_tangent);
	let horizontal_radians: f32 = atan2(
		dot(emission_direction, c90_tangent),
		dot(emission_direction, c0_tangent)
	);
	return vec2f(
		fract(horizontal_radians * 0.15915494309189535 + 1.0),
		polar_radians * 0.3183098861837907
	);
}
"#;

pub(crate) const IES_PROFILE_SAMPLE_SOURCE: &str = r#"
sample_ies_profile: fn (
	texture_index: u32,
	emission_direction: vec3f,
	axis: vec3f,
	encoded_c0_tangent: vec2u16
) -> f32 {
	let uv: vec2f = ies_profile_uv(emission_direction, axis, encoded_c0_tangent);
	return max(
		sample_texture_2d_array_grad(textures, texture_index, uv, vec2f(0.0, 0.0), vec2f(0.0, 0.0)).x,
		0.0
	);
}
"#;

pub(crate) const MATERIAL_EVALUATION_SUFFIX_SOURCE: &str = r#"
material_evaluation_suffix: fn () -> void {
	// The fraction of the hemisphere around the normal that nearby geometry leaves open to the environment. GTAO and
	// SSGI each estimate it from the opaque depth buffer, so a transparent surface has neither, and either may be off.
	let environment_visibility: f32 = 1.0;
	// Light from SSGI rays that hit on-screen geometry, already weighted by the fraction of rays that hit.
	let screen_space_irradiance: vec3f = vec3f(0.0, 0.0, 0.0);
	if (push_constant.blend == 0) {
		if (push_constant.ssgi != 0) {
			// SSGI works in view space at half resolution. The pixel's own surface picks the history texels that lie
			// on it. Alpha is the fraction of rays that hit, so the rest reach the environment.
			// The surface in SSGI's view space, made here so neither stays live through the material body.
			let view_space_surface_position: vec3f = views.views[0].view * vec4f(
				world_space_vertex_position.x,
				world_space_vertex_position.y,
				world_space_vertex_position.z,
				1.0
			);
			let view_space_normal: vec3f = views.views[0].view * vec4f(
				f32(geometric_normal.x),
				f32(geometric_normal.y),
				f32(geometric_normal.z),
				0.0
			);
			let screen_space_indirect: vec4f = sample_screen_space_indirect_diffuse(
				pixel_coordinates,
				image_extent,
				view_space_surface_position,
				view_space_normal
			);
			// SSGI carries pre-exposed light through its half-float images. Undo that scale before combining it
			// with physical lighting, so the final exposure below applies exactly once.
			screen_space_irradiance = vec3f(screen_space_indirect.x, screen_space_indirect.y, screen_space_indirect.z)
				/ lighting_data.exposure;
			// Filter rounding can push the hit fraction past one. The clamp keeps the base of the specular fit's pow nonnegative.
			environment_visibility = clamp(1.0 - screen_space_indirect.w, 0.0, 1.0);
		}
		if (push_constant.gtao != 0) {
			// Both passes estimate the same visibility, so multiplying them would darken an occluder they both see
			// twice. The smaller one keeps an occluder that only one of them resolves, such as a crease below SSGI's
			// half resolution.
			environment_visibility = min(environment_visibility, fetch(ao, pixel_coordinates).x);
		}
	}
	// Environment maps store arbitrary units; the intensity calibrates them to the lux their Environment requests.
	let indirect_diffuse_radiance: vec3f = screen_space_irradiance
		+ sample_environment_irradiance(vec3f(normal)) * (lighting_data.environment_intensity * environment_visibility);
	// The BRDF constants come after the screen-space reads above, so they are not live while those fetches wait.
	// Preserve compact material values and normalized vectors through the BRDF.
	// Positions, shadow projections, HDR radiance, and accumulation remain f32.
	let albedo_rgb: vec3f16 = vec3f16(albedo.x, albedo.y, albedo.z);
	let one_minus_metalness: f16 = f16(1.0) - metalness;
	let F0: vec3f16 = vec3f16(0.04, 0.04, 0.04) * one_minus_metalness + albedo_rgb * metalness;
	let one_minus_f0: vec3f16 = vec3f16(1.0, 1.0, 1.0) - F0;
	let NdotV: f16 = max(dot(normal, V_material), f16(0.0));
	let roughness_alpha: f16 = roughness * roughness;
	let roughness_alpha_squared: f16 = roughness_alpha * roughness_alpha;
	let adjusted_roughness: f16 = roughness + 1.0;
	let geometry_k: f16 = adjusted_roughness * adjusted_roughness / 8.0;
	let view_fresnel_base: f16 = clamp(f16(1.0) - NdotV, f16(0.0), f16(1.0));
	let view_fresnel_squared: f16 = view_fresnel_base * view_fresnel_base;
	let view_fresnel_factor: f16 = view_fresnel_squared * view_fresnel_squared * view_fresnel_base;
	let one_minus_fresnel_n_dot_v: vec3f16 = one_minus_f0 * (f16(1.0) - view_fresnel_factor);
	// These terms depend only on the shaded pixel. Evaluate them once instead of once per light.
	let geometry_view: f16 = NdotV / (NdotV * (1.0 - geometry_k) + geometry_k);
	// Every factor of the direct diffuse weight except the Fresnel factor of each light's angle. Folding them here keeps
	// the albedo, metalness, and view Fresnel terms out of the light loop: of the shader's regions, the loop slows down
	// the most for every value held live across it (2026-10-05).
	let direct_diffuse_albedo: vec3f16 = one_minus_fresnel_n_dot_v * one_minus_f0 * one_minus_metalness * albedo_rgb
		/ f16(3.14159265359);
	// Indirect diffuse light and emission seed the diffuse sum, so neither they nor the material terms that weight them
	// stay live through the loop. Material occlusion, like baked AO, applies to indirect light only. Direct light has
	// its own shadows.
	let one_minus_roughness: f16 = f16(1.0) - roughness;
	let grazing: vec3f16 = vec3f16(max(one_minus_roughness, F0.x), max(one_minus_roughness, F0.y), max(one_minus_roughness, F0.z));
	let kD_ibl: vec3f16 = (one_minus_f0 - (grazing - F0) * view_fresnel_factor) * one_minus_metalness;
	let diffuse: vec3f = vec3f(kD_ibl * albedo_rgb) * indirect_diffuse_radiance * f32(occlusion) + vec3f(emission);
	let specular: vec3f = vec3f(0.0, 0.0, 0.0);
	let light_count: u32 = lighting_data.light_count;

	// Visit only the lights the light-cluster pass bucketed into this pixel's cluster: 16 columns, 8 rows, and 24
	// depth slices that grow exponentially with view depth. Each cluster stores one bit per light.
	let cluster_column: u32 = u32(min((f32(pixel_coordinates.x) + 0.5) * 16.0 / f32(image_extent.x), 15.0));
	let cluster_row: u32 = u32(min((f32(pixel_coordinates.y) + 0.5) * 8.0 / f32(image_extent.y), 7.0));
	let cluster_slice: u32 = u32(clamp(
		floor(log2(perspective_w / light_cluster_parameters.near) * light_cluster_parameters.depth_slice_scale),
		0.0,
		23.0
	));
	let cluster_mask_base: u32 = ((cluster_slice * 8 + cluster_row) * 16 + cluster_column) * 32;
	let cluster_mask_word_count: u32 = (light_count + 31) >> 5;

	for (let mask_word: u32 = 0; mask_word < cluster_mask_word_count; mask_word = mask_word + 1) {
		// Each pass takes the lowest remaining light and clears its bit, so `continue` moves on to the next light.
		for (
			let light_bits: u32 = light_cluster_masks.words[cluster_mask_base + mask_word];
			light_bits != 0;
			light_bits = light_bits & (light_bits - 1)
		) {
			let light_index: u32 = mask_word * 32 + find_lsb(light_bits);
			let light_type: u32 = lighting_data.lights[light_index].type;
			let L: vec3f = vec3f(0.0, 0.0, 0.0);
			let attenuation: f32 = 1.0;
			let light_position: vec3f = lighting_data.lights[light_index].position;
			if (light_type == 68) {
				L = vec3f(0.0, 0.0, 0.0) - light_position;
			} else {
				let surface_to_light: vec3f = light_position - world_space_vertex_position;
				let distance_squared: f32 = dot(surface_to_light, surface_to_light);
				if (distance_squared <= 0.0) {
					continue;
				}
				L = surface_to_light * inversesqrt(distance_squared);
				attenuation = 1.0 / distance_squared;
			}

			let L_material: vec3f16 = vec3f16(L);
			let NdotL: f16 = max(dot(normal, L_material), f16(0.0));
			if (NdotL <= 0.0) {
				continue;
			}

			let occlusion_factor: f16 = 1.0;
			if (light_type == 68) {
				let shadow_view0: u32 = lighting_data.lights[light_index].shadow_views[0];
				if (shadow_view0 != 0) {
					if (push_constant.blend == 0) {
						// The sun visibility pass resolved every sun's shadow map and contact shadows for every opaque
						// pixel, sun slot `s` in channel `s`.
						let sun_slot: u32 = lighting_data.lights[light_index].shadow_layer;
						let suns_visibility: vec4f = fetch(sun_visibility, pixel_coordinates);
						let sun_visibility_value: f32 = suns_visibility.x;
						if (sun_slot == 1) {
							sun_visibility_value = suns_visibility.y;
						} else if (sun_slot == 2) {
							sun_visibility_value = suns_visibility.z;
						} else if (sun_slot == 3) {
							sun_visibility_value = suns_visibility.w;
						}
						occlusion_factor = f16(sun_visibility_value);
					} else {
						// A transparent surface lies in front of the opaque depth that pass resolved, so it filters the
						// shadow map itself, without contact shadows, which trace that depth.
						let shadow_view1: u32 = lighting_data.lights[light_index].shadow_views[1];
						let shadow_view2: u32 = lighting_data.lights[light_index].shadow_views[2];
						let shadow_view3: u32 = lighting_data.lights[light_index].shadow_views[3];
						occlusion_factor = f16(sample_directional_shadow(
							depth_shadow_map,
							shadow_view0,
							shadow_view1,
							shadow_view2,
							shadow_view3,
							lighting_data.lights[light_index].angular_radius_tangent,
							world_space_vertex_position,
							perspective_w,
							position_derivative_x,
							position_derivative_y
						));
					}
					if (occlusion_factor == 0.0) {
						continue;
					}
				}
				attenuation = 1.0;
			} else {
				match light_type {
					0 => {
						let shadow_view_index: u32 = lighting_data.lights[light_index].shadow_views[0];
						if (shadow_view_index != 0) {
							let shadow_cube_index: u32 = lighting_data.lights[light_index].shadow_layer;
							occlusion_factor = f16(sample_point_shadow(
								shadow_view_index,
								shadow_cube_index,
								world_space_vertex_position,
								light_position,
								position_derivative_x,
								position_derivative_y
							));
							if (occlusion_factor == 0.0) {
								continue;
							}
						}
					}
					1 => {
						let cone_direction: vec3f16 = vec3f16(lighting_data.lights[light_index].direction);
						let cone_cosine: f16 = dot(cone_direction, vec3f16(0.0, 0.0, 0.0) - L_material);
						let cone_factor: f16 = f16(cone_attenuation(
							f32(cone_cosine),
							lighting_data.lights[light_index].cone_cosines.x,
							lighting_data.lights[light_index].cone_cosines.y
						));
						if (cone_factor <= 0.0) {
							continue;
						}
						attenuation = attenuation * f32(cone_factor);
						let shadow_view_index: u32 = lighting_data.lights[light_index].shadow_views[0];
						if (shadow_view_index != 0) {
							let shadow_layer: u32 = lighting_data.lights[light_index].shadow_layer;
							occlusion_factor = f16(sample_cone_shadow(
								cone_shadow_map,
								shadow_view_index,
								shadow_layer,
								world_space_vertex_position,
								position_derivative_x,
								position_derivative_y
							));
							if (occlusion_factor == 0.0) {
								continue;
							}
						}
					}
					_ => {}
				}
				if (lighting_data.lights[light_index].ies_profile_texture != 4294967295) {
					let emission_direction: vec3f = vec3f(0.0, 0.0, 0.0) - L;
					let profile_axis: vec3f = lighting_data.lights[light_index].direction;
					let intensity_factor: f32 = sample_ies_profile(
						lighting_data.lights[light_index].ies_profile_texture,
						emission_direction,
						profile_axis,
						lighting_data.lights[light_index].ies_c0_tangent
					);
					if (intensity_factor <= 0.0) {
						continue;
					}
					attenuation = attenuation * intensity_factor;
				}
			}

			let H: vec3f16 = normalize(V_material + L_material);
			let half_view_fresnel_base: f16 = clamp(f16(1.0) - max(dot(H, V_material), f16(0.0)), f16(0.0), f16(1.0));
			let half_view_fresnel_squared: f16 = half_view_fresnel_base * half_view_fresnel_base;
			let half_view_fresnel_factor: f16 = half_view_fresnel_squared * half_view_fresnel_squared * half_view_fresnel_base;
			// Schlick's F0 + (1 - F0) f, written so that 1 - F0 is not kept live through the loop.
			let F: vec3f16 = F0 * (f16(1.0) - half_view_fresnel_factor)
				+ vec3f16(half_view_fresnel_factor, half_view_fresnel_factor, half_view_fresnel_factor);
			let NdotH: f16 = max(dot(normal, H), f16(0.0));
			let denominator_base: f16 = NdotH * NdotH * (roughness_alpha_squared - 1.0) + 1.0;
			let NDF: f16 = roughness_alpha_squared / (3.14159265359 * denominator_base * denominator_base);
			let geometry_light: f16 = NdotL / (NdotL * (1.0 - geometry_k) + geometry_k);
			let local_specular: vec3f16 = (NDF * geometry_view * geometry_light * F) / (4.0 * NdotV * NdotL + 0.000001);
			let light_fresnel_base: f16 = clamp(f16(1.0) - NdotL, f16(0.0), f16(1.0));
			let light_fresnel_squared: f16 = light_fresnel_base * light_fresnel_base;
			let light_fresnel_factor: f16 = light_fresnel_squared * light_fresnel_squared * light_fresnel_base;
			let local_diffuse: vec3f16 = direct_diffuse_albedo * (f16(1.0) - light_fresnel_factor);
			let light_color: vec3f = lighting_data.lights[light_index].color;
			let irradiance: vec3f = light_color * (attenuation * f32(NdotL * occlusion_factor));
			diffuse = diffuse + vec3f(local_diffuse) * irradiance;
			specular = specular + vec3f(local_specular) * irradiance;
		}
	}

	// The half-precision view vector the lights above used. Keeping the f32 one live through the light loop for this
	// alone made material evaluation 1.7 % slower (2026-10-05), and it already bends the mirror ray no more than the
	// half-precision material normal does.
	let incident: vec3f = vec3f(0.0, 0.0, 0.0) - vec3f(V_material);
	let reflection_direction: vec3f = incident - 2.0 * dot(incident, vec3f(normal)) * vec3f(normal);
	let reflection_radiance: vec3f = sample_environment_specular(reflection_direction, f32(roughness))
		* lighting_data.environment_intensity;
	// SSGI rays read this one frame later. View-dependent specular is left out: a surface receives the light that
	// leaves a neighbor toward it, not the highlight the camera sees, and highlights would turn into sparkling noise.
	// Pre-exposure keeps daylight radiance within the half-float range. SSGI rescales previous-frame light when
	// exposure changes, and the material consumer removes that scale before combining it with physical lighting.
	// Alpha keeps the view depth, the clip w of this pixel, so a ray can tell which pixel belongs to the surface it hit.
	// It is written before the reflection trace, which cannot change it, so the diffuse sum does not stay live through
	// the ray march. Only SSGI reads the diffuse light, so it is not written while SSGI is off.
	if (push_constant.blend == 0) {
		if (push_constant.ssgi != 0) {
			let diffuse_radiance: vec3f = diffuse * lighting_data.exposure;
			write(
				diffuse_radiance_map,
				pixel_coordinates,
				vec4f(diffuse_radiance.x, diffuse_radiance.y, diffuse_radiance.z, perspective_w)
			);
		}
	}
	// Visibility covers the cosine-weighted hemisphere, but a reflection gathers light from a lobe around the mirror
	// direction that narrows as roughness falls. Lagarde and de Rousiers' fit ("Moving Frostbite to PBR", 2014)
	// converts one to the other: smooth surfaces seen head-on keep more of their reflection, and grazing views lose it.
	// Reflection rays stop at nearby geometry, so the light they find carries its own occlusion.
	let specular_occlusion: f32 = clamp(
		pow(f32(NdotV) + environment_visibility, pow(2.0, 0.0 - 16.0 * f32(roughness_alpha) - 1.0))
			- 1.0
			+ environment_visibility,
		0.0,
		1.0
	);
	let c0: vec4f16 = vec4f16(0.0 - 1.0, 0.0 - 0.0275, 0.0 - 0.572, 0.022);
	let c1: vec4f16 = vec4f16(1.0, 0.0425, 1.04, 0.0 - 0.04);
	let r: vec4f16 = roughness * c0 + c1;
	let a004: f16 = min(r.x * r.x, pow(f16(2.0), (f16(0.0) - f16(9.28)) * NdotV)) * r.x + r.y;
	let env_brdf: vec2f16 = vec2f16(0.0 - 1.04, 1.04) * a004 + vec2f16(r.z, r.w);
	// The split-sum weight of reflected light, with the material occlusion that applies to indirect light.
	let specular_weight: vec3f = vec3f(F0 * env_brdf.x + env_brdf.y) * f32(occlusion);
	// Everything but the reflected light is known before the trace, so only these sums stay live through the march.
	let direct_and_diffuse: vec3f = diffuse + specular;
	let environment_specular: vec3f = reflection_radiance * specular_occlusion;
	// Screen-space reflections replace the environment where the mirror ray finds visible geometry. One mirror ray
	// cannot stand for a wide glossy lobe, so reflections fade back to the prefiltered environment from roughness 0.2
	// to 0.4. A reflection that points into the geometric surface would only find the surface itself.
	let screen_space_reflection: vec4f = vec4f(0.0, 0.0, 0.0, 0.0);
	if (f32(roughness) < 0.4 && dot(reflection_direction, vec3f(geometric_normal)) > 0.0) {
		screen_space_reflection = trace_screen_space_reflection(
			world_space_vertex_position,
			vec3f(geometric_normal),
			reflection_direction,
			views.views[0].view_projection
		);
	}
	let reflection_weight: f32 = screen_space_reflection.w * clamp((0.4 - f32(roughness)) * 5.0, 0.0, 1.0);
	let specular_radiance: vec3f = environment_specular * (1.0 - reflection_weight)
		+ vec3f(screen_space_reflection.x, screen_space_reflection.y, screen_space_reflection.z) * reflection_weight;
	// Pre-expose: store light already multiplied by the camera exposure, so real-world intensities such as a
	// 100,000 lux sun on a glossy surface stay within the half-float range of the lit map.
	let lit: vec3f = (direct_and_diffuse + specular_weight * specular_radiance) * lighting_data.exposure;
	let output_color: vec4f = vec4f(lit.x, lit.y, lit.z, 1.0);
	if (push_constant.blend != 0) {
		let source_alpha: f32 = f32(clamp(albedo.w, f16(0.0), f16(1.0)));
		let destination_color: vec4f = image_load(lit_map, pixel_coordinates);
		output_color = source_over(
			vec4f(lit.x * source_alpha, lit.y * source_alpha, lit.z * source_alpha, source_alpha),
			destination_color
		);
	}
	write(lit_map, pixel_coordinates, output_color);
	// Reflection rays read the full exposed light the camera sees, highlights included, from the radiance history.
	// It stays exposed, like the lit map, so a bright highlight fits in half-float range.
	if (push_constant.blend == 0) {
		write(radiance_history_map, pixel_coordinates, vec4f(lit.x, lit.y, lit.z, perspective_w));
	}
}
"#;

// Computes the projected receiver plane so cone PCF compares each texel against the depth at that texel's center.
// Returning no correction for a degenerate projection keeps the existing bias as a safe fallback.

pub(crate) const U16_TO_U32_SOURCE: &str = "u16_to_u32: fn (value: u16) -> u32 { return u32(value); }";

pub(crate) const CONE_ATTENUATION_SOURCE: &str = "cone_attenuation: fn (cosine: f32, inner_cosine: f32, outer_cosine: f32) -> f32 { return clamp((cosine - outer_cosine) / (inner_cosine - outer_cosine), 0.0, 1.0); }";

// Resolve all three triangle vertices together so mesh and meshlet offsets are computed once.
pub(crate) const COMPUTE_VERTEX_INDICES_SOURCE: &str = r#"
compute_vertex_indices: fn (mesh: Mesh, meshlet: Meshlet, primitive_index_base: u32) -> u32[3] {
	let vertex_index_base: u32 = mesh.base_vertex_index;
	let relative_index_base: u32 = mesh.base_primitive_index + meshlet.primitive_offset;
	let primitive_index0: u32 = u32(primitive_indices[primitive_index_base]);
	let primitive_index1: u32 = u32(primitive_indices[primitive_index_base + 1]);
	let primitive_index2: u32 = u32(primitive_indices[primitive_index_base + 2]);
	return u32[3](
		vertex_index_base + u16_to_u32(vertex_indices[relative_index_base + primitive_index0]),
		vertex_index_base + u16_to_u32(vertex_indices[relative_index_base + primitive_index1]),
		vertex_index_base + u16_to_u32(vertex_indices[relative_index_base + primitive_index2])
	);
}
"#;

// Share the clip-space basis between geometry and optional UV interpolation.
pub(crate) const COMPUTE_TRIANGLE_INTERPOLATION_SOURCE: &str = r#"
compute_triangle_interpolation: fn (
	clip_position0: vec4f,
	clip_position1: vec4f,
	clip_position2: vec4f
) -> TriangleInterpolation {
	let inverse_w: vec3f = vec3f(
		1.0 / clip_position0.w,
		1.0 / clip_position1.w,
		1.0 / clip_position2.w
	);
	let origin: vec2f = vec2f(
		clip_position0.x * inverse_w.x,
		clip_position0.y * inverse_w.x
	);
	let ndc1: vec2f = vec2f(
		clip_position1.x * inverse_w.y,
		clip_position1.y * inverse_w.y
	);
	let ndc2: vec2f = vec2f(
		clip_position2.x * inverse_w.z,
		clip_position2.y * inverse_w.z
	);
	let determinant: f32 =
		(ndc2.x - ndc1.x) * (origin.y - ndc1.y) -
		(origin.x - ndc1.x) * (ndc2.y - ndc1.y);
	let inverse_determinant: f32 = 1.0 / determinant;
	let raw_ddx: vec3f = vec3f(
		ndc1.y - ndc2.y,
		ndc2.y - origin.y,
		origin.y - ndc1.y
	) * inverse_determinant * inverse_w;
	let raw_ddy: vec3f = vec3f(
		ndc2.x - ndc1.x,
		origin.x - ndc2.x,
		ndc1.x - origin.x
	) * inverse_determinant * inverse_w;
	return TriangleInterpolation(origin, inverse_w, raw_ddx, raw_ddy);
}
"#;

// Normal-map decoding stays in the visibility module; BESL only lowers the general texture-array gradient sample.
pub(crate) const SAMPLE_VISIBILITY_NORMAL_SOURCE: &str = r#"
sample_visibility_normal: fn (
	texture_index: u32,
	uv: vec2f,
	uv_derivative_x: vec2f,
	uv_derivative_y: vec2f
) -> vec3f {
	let encoded: vec4f = sample_texture_2d_array_grad(
		textures, texture_index, uv, uv_derivative_x, uv_derivative_y
	);
	return unit_vector_from_xy(vec2f(encoded.x, encoded.y));
}
"#;
