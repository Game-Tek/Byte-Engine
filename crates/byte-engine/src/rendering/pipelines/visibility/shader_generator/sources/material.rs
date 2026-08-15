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
	if (normal_z < 0.0) {
		let fold: f32 = 0.0 - normal_z;
		// `step` returns the positive direction at zero, matching the CPU encoder's fold convention.
		return vec3f(
			octahedral.x - (step(0.0, octahedral.x) * 2.0 - 1.0) * fold,
			octahedral.y - (step(0.0, octahedral.y) * 2.0 - 1.0) * fold,
			normal_z
		);
	}
	return vec3f(octahedral.x, octahedral.y, normal_z);
}
"#;

pub(crate) const MATERIAL_EVALUATION_PREFIX_SOURCE: &str = r#"
material_evaluation_prefix: fn () -> void {
	let invocation: vec2u = thread_id();
	if (invocation.x >= material_count.material_count[push_constant.material_id]) {
		return;
	}

	let offset: u32 = material_offset.material_offset[push_constant.material_id];
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
	let meshlet: Meshlet = meshlets.meshlets[meshlet_index];
	let mesh: Mesh = meshes.meshes[instance_index];
	let material: Material = materials.materials[push_constant.material_id];

	let primitive_index_base: u32 = (mesh.base_triangle_index + meshlet.triangle_offset + meshlet_triangle_index) * 3;
	let triangle_vertex_indices: u32[3] = compute_vertex_indices(mesh, meshlet, primitive_index_base);
	let active_lanes: vec4u = subgroup_ballot(true);
	let setup_leader: u32 = subgroup_ballot_find_lsb(active_lanes);
	let leader_instance_index: u32 = subgroup_broadcast_u32(instance_index, setup_leader);
	let leader_triangle_indices: u32 = subgroup_broadcast_u32(triangle_meshlet_indices, setup_leader);
	let matching_triangle_lanes: vec4u = subgroup_ballot(
		instance_index == leader_instance_index && triangle_meshlet_indices == leader_triangle_indices
	);
	let share_triangle_setup: bool = subgroup_ballot_count(matching_triangle_lanes) == subgroup_ballot_count(active_lanes);
	let setup_lane: bool = share_triangle_setup == false || subgroup_lane_index() == setup_leader;

	let model_space_vertex_position0: vec4f = vec4f(0.0, 0.0, 0.0, 1.0);
	let model_space_vertex_position1: vec4f = vec4f(0.0, 0.0, 0.0, 1.0);
	let model_space_vertex_position2: vec4f = vec4f(0.0, 0.0, 0.0, 1.0);
	let vertex_normal0: vec4f = vec4f(0.0, 0.0, 1.0, 0.0);
	let vertex_normal1: vec4f = vec4f(0.0, 0.0, 1.0, 0.0);
	let vertex_normal2: vec4f = vec4f(0.0, 0.0, 1.0, 0.0);

	if (setup_lane && mesh.skinned_base_vertex_index != 4294967295) {
		let skinned_vertex_index0: u32 = mesh.skinned_base_vertex_index + (triangle_vertex_indices[0] - mesh.base_vertex_index);
		let skinned_vertex_index1: u32 = mesh.skinned_base_vertex_index + (triangle_vertex_indices[1] - mesh.base_vertex_index);
		let skinned_vertex_index2: u32 = mesh.skinned_base_vertex_index + (triangle_vertex_indices[2] - mesh.base_vertex_index);
		let skinned_vertex0: SkinnedVertex = skinned_vertices.vertices[skinned_vertex_index0];
		let skinned_vertex1: SkinnedVertex = skinned_vertices.vertices[skinned_vertex_index1];
		let skinned_vertex2: SkinnedVertex = skinned_vertices.vertices[skinned_vertex_index2];
		model_space_vertex_position0 = skinned_vertex0.position;
		model_space_vertex_position1 = skinned_vertex1.position;
		model_space_vertex_position2 = skinned_vertex2.position;
		vertex_normal0 = skinned_vertex0.normal;
		vertex_normal1 = skinned_vertex1.normal;
		vertex_normal2 = skinned_vertex2.normal;
	}
	if (setup_lane && mesh.skinned_base_vertex_index == 4294967295) {
		let position0: vec3f = vertex_positions.positions[triangle_vertex_indices[0]];
		let position1: vec3f = vertex_positions.positions[triangle_vertex_indices[1]];
		let position2: vec3f = vertex_positions.positions[triangle_vertex_indices[2]];
		let normal0: vec3f = decode_octahedral_normal(vertex_normals.normals[triangle_vertex_indices[0]]);
		let normal1: vec3f = decode_octahedral_normal(vertex_normals.normals[triangle_vertex_indices[1]]);
		let normal2: vec3f = decode_octahedral_normal(vertex_normals.normals[triangle_vertex_indices[2]]);
		model_space_vertex_position0 = vec4f(position0.x, position0.y, position0.z, 1.0);
		model_space_vertex_position1 = vec4f(position1.x, position1.y, position1.z, 1.0);
		model_space_vertex_position2 = vec4f(position2.x, position2.y, position2.z, 1.0);
		vertex_normal0 = vec4f(normal0.x, normal0.y, normal0.z, 0.0);
		vertex_normal1 = vec4f(normal1.x, normal1.y, normal1.z, 0.0);
		vertex_normal2 = vec4f(normal2.x, normal2.y, normal2.z, 0.0);
	}
	let nc: vec2f = make_raster_ndc_from_pixel_coordinates(pixel_coordinates, image_extent);
	let model: mat4x3f = mesh.model;
	let world_space_vertex_position0: vec3f = vec3f(0.0, 0.0, 0.0);
	let world_space_vertex_position1: vec3f = vec3f(0.0, 0.0, 0.0);
	let world_space_vertex_position2: vec3f = vec3f(0.0, 0.0, 0.0);
	let clip_space_vertex_position0: vec4f = vec4f(0.0, 0.0, 0.0, 0.0);
	let clip_space_vertex_position1: vec4f = vec4f(0.0, 0.0, 0.0, 0.0);
	let clip_space_vertex_position2: vec4f = vec4f(0.0, 0.0, 0.0, 0.0);
	let world_space_vertex_normal0: vec3f = vec3f(0.0, 0.0, 1.0);
	let world_space_vertex_normal1: vec3f = vec3f(0.0, 0.0, 1.0);
	let world_space_vertex_normal2: vec3f = vec3f(0.0, 0.0, 1.0);
	let triangle_inverse_w: vec3f = vec3f(0.0, 0.0, 0.0);
	let triangle_raw_ddx: vec3f = vec3f(0.0, 0.0, 0.0);
	let triangle_raw_ddy: vec3f = vec3f(0.0, 0.0, 0.0);
	if (setup_lane) {
		let view_projection: mat4f = views.views[0].view_projection;
		world_space_vertex_position0 = model * model_space_vertex_position0;
		world_space_vertex_position1 = model * model_space_vertex_position1;
		world_space_vertex_position2 = model * model_space_vertex_position2;
		clip_space_vertex_position0 = view_projection * vec4f(world_space_vertex_position0.x, world_space_vertex_position0.y, world_space_vertex_position0.z, 1.0);
		clip_space_vertex_position1 = view_projection * vec4f(world_space_vertex_position1.x, world_space_vertex_position1.y, world_space_vertex_position1.z, 1.0);
		clip_space_vertex_position2 = view_projection * vec4f(world_space_vertex_position2.x, world_space_vertex_position2.y, world_space_vertex_position2.z, 1.0);
		world_space_vertex_normal0 = normalize(model * vertex_normal0);
		world_space_vertex_normal1 = normalize(model * vertex_normal1);
		world_space_vertex_normal2 = normalize(model * vertex_normal2);
	}

	// Share perspective-correct interpolation planes instead of only transformed vertices. The
	// fallback computes the same planes per lane when a SIMD group contains more than one triangle.
	let interpolation_origin: vec2f = vec2f(0.0, 0.0);
	let inverse_w_origin: f32 = 0.0;
	let inverse_w_dx: f32 = 0.0;
	let inverse_w_dy: f32 = 0.0;
	let position_numerator_origin: vec3f = vec3f(0.0, 0.0, 0.0);
	let position_numerator_dx: vec3f = vec3f(0.0, 0.0, 0.0);
	let position_numerator_dy: vec3f = vec3f(0.0, 0.0, 0.0);
	let normal_numerator_origin: vec3f = vec3f(0.0, 0.0, 0.0);
	let normal_numerator_dx: vec3f = vec3f(0.0, 0.0, 0.0);
	let normal_numerator_dy: vec3f = vec3f(0.0, 0.0, 0.0);
	if (setup_lane) {
		let triangle_interpolation: TriangleInterpolation = compute_triangle_interpolation(
			clip_space_vertex_position0,
			clip_space_vertex_position1,
			clip_space_vertex_position2
		);
		interpolation_origin = triangle_interpolation.origin;
		triangle_inverse_w = triangle_interpolation.inverse_w;
		triangle_raw_ddx = triangle_interpolation.raw_ddx;
		triangle_raw_ddy = triangle_interpolation.raw_ddy;
		inverse_w_origin = triangle_inverse_w.x;
		inverse_w_dx = dot(triangle_raw_ddx, vec3f(1.0, 1.0, 1.0));
		inverse_w_dy = dot(triangle_raw_ddy, vec3f(1.0, 1.0, 1.0));
		position_numerator_origin = world_space_vertex_position0 * triangle_inverse_w.x;
		position_numerator_dx = interpolate_vec3f_with_deriv(
			triangle_raw_ddx,
			world_space_vertex_position0,
			world_space_vertex_position1,
			world_space_vertex_position2
		);
		position_numerator_dy = interpolate_vec3f_with_deriv(
			triangle_raw_ddy,
			world_space_vertex_position0,
			world_space_vertex_position1,
			world_space_vertex_position2
		);
		normal_numerator_origin = world_space_vertex_normal0 * triangle_inverse_w.x;
		normal_numerator_dx = interpolate_vec3f_with_deriv(
			triangle_raw_ddx,
			world_space_vertex_normal0,
			world_space_vertex_normal1,
			world_space_vertex_normal2
		);
		normal_numerator_dy = interpolate_vec3f_with_deriv(
			triangle_raw_ddy,
			world_space_vertex_normal0,
			world_space_vertex_normal1,
			world_space_vertex_normal2
		);
	}
	if (share_triangle_setup) {
		interpolation_origin = vec2f(subgroup_broadcast_f32(interpolation_origin.x, setup_leader), subgroup_broadcast_f32(interpolation_origin.y, setup_leader));
		inverse_w_origin = subgroup_broadcast_f32(inverse_w_origin, setup_leader);
		inverse_w_dx = subgroup_broadcast_f32(inverse_w_dx, setup_leader);
		inverse_w_dy = subgroup_broadcast_f32(inverse_w_dy, setup_leader);
		position_numerator_origin = vec3f(subgroup_broadcast_f32(position_numerator_origin.x, setup_leader), subgroup_broadcast_f32(position_numerator_origin.y, setup_leader), subgroup_broadcast_f32(position_numerator_origin.z, setup_leader));
		position_numerator_dx = vec3f(subgroup_broadcast_f32(position_numerator_dx.x, setup_leader), subgroup_broadcast_f32(position_numerator_dx.y, setup_leader), subgroup_broadcast_f32(position_numerator_dx.z, setup_leader));
		position_numerator_dy = vec3f(subgroup_broadcast_f32(position_numerator_dy.x, setup_leader), subgroup_broadcast_f32(position_numerator_dy.y, setup_leader), subgroup_broadcast_f32(position_numerator_dy.z, setup_leader));
		normal_numerator_origin = vec3f(subgroup_broadcast_f32(normal_numerator_origin.x, setup_leader), subgroup_broadcast_f32(normal_numerator_origin.y, setup_leader), subgroup_broadcast_f32(normal_numerator_origin.z, setup_leader));
		normal_numerator_dx = vec3f(subgroup_broadcast_f32(normal_numerator_dx.x, setup_leader), subgroup_broadcast_f32(normal_numerator_dx.y, setup_leader), subgroup_broadcast_f32(normal_numerator_dx.z, setup_leader));
		normal_numerator_dy = vec3f(subgroup_broadcast_f32(normal_numerator_dy.x, setup_leader), subgroup_broadcast_f32(normal_numerator_dy.y, setup_leader), subgroup_broadcast_f32(normal_numerator_dy.z, setup_leader));
	}

	let interpolation_delta: vec2f = nc - interpolation_origin;
	let inverse_w_at_pixel: f32 = inverse_w_origin + interpolation_delta.x * inverse_w_dx + interpolation_delta.y * inverse_w_dy;
	let perspective_w: f32 = 1.0 / inverse_w_at_pixel;
	let ndc_step_x: f32 = 2.0 / f32(image_extent.x);
	let ndc_step_y: f32 = 2.0 / f32(image_extent.y);
	let position_numerator: vec3f = position_numerator_origin + interpolation_delta.x * position_numerator_dx + interpolation_delta.y * position_numerator_dy;
	let normal_numerator: vec3f = normal_numerator_origin + interpolation_delta.x * normal_numerator_dx + interpolation_delta.y * normal_numerator_dy;
	let world_space_vertex_position: vec3f = position_numerator * perspective_w;
	let world_space_vertex_normal: vec3f = normalize(normal_numerator * perspective_w);
	let N: vec3f = world_space_vertex_normal;
	let camera_position: vec3f = views.views[0].inverse_view * vec4f(0.0, 0.0, 0.0, 1.0);
	let V: vec3f = normalize(camera_position - world_space_vertex_position);
	let position_derivative_x: vec3f =
		(position_numerator + position_numerator_dx * ndc_step_x) /
		(inverse_w_at_pixel + inverse_w_dx * ndc_step_x) - world_space_vertex_position;
	let position_derivative_y: vec3f =
		(position_numerator + position_numerator_dy * ndc_step_y) /
		(inverse_w_at_pixel + inverse_w_dy * ndc_step_y) - world_space_vertex_position;
}
"#;

pub(crate) const MATERIAL_EVALUATION_UV_SOURCE: &str = r#"
material_evaluation_uv: fn () -> void {
	// Runtime UVs use half-float storage and are expanded only for materials that sample them.
	let uv_numerator_origin: vec2f = vec2f(0.0, 0.0);
	let uv_numerator_dx: vec2f = vec2f(0.0, 0.0);
	let uv_numerator_dy: vec2f = vec2f(0.0, 0.0);
	if (setup_lane) {
		let vertex_uv0: vec2f = decode_f16_vec2(vertex_uvs.uvs[triangle_vertex_indices[0]]);
		let vertex_uv1: vec2f = decode_f16_vec2(vertex_uvs.uvs[triangle_vertex_indices[1]]);
		let vertex_uv2: vec2f = decode_f16_vec2(vertex_uvs.uvs[triangle_vertex_indices[2]]);
		uv_numerator_origin = vertex_uv0 * triangle_inverse_w.x;
		uv_numerator_dx = interpolate_vec2f_with_deriv(triangle_raw_ddx, vertex_uv0, vertex_uv1, vertex_uv2);
		uv_numerator_dy = interpolate_vec2f_with_deriv(triangle_raw_ddy, vertex_uv0, vertex_uv1, vertex_uv2);
	}
	if (share_triangle_setup) {
		uv_numerator_origin = vec2f(subgroup_broadcast_f32(uv_numerator_origin.x, setup_leader), subgroup_broadcast_f32(uv_numerator_origin.y, setup_leader));
		uv_numerator_dx = vec2f(subgroup_broadcast_f32(uv_numerator_dx.x, setup_leader), subgroup_broadcast_f32(uv_numerator_dx.y, setup_leader));
		uv_numerator_dy = vec2f(subgroup_broadcast_f32(uv_numerator_dy.x, setup_leader), subgroup_broadcast_f32(uv_numerator_dy.y, setup_leader));
	}
	let uv_numerator: vec2f = uv_numerator_origin + interpolation_delta.x * uv_numerator_dx + interpolation_delta.y * uv_numerator_dy;
	let vertex_uv: vec2f = uv_numerator * perspective_w;
	let uv_derivative_x: vec2f =
		(uv_numerator + uv_numerator_dx * ndc_step_x) /
		(inverse_w_at_pixel + inverse_w_dx * ndc_step_x) - vertex_uv;
	let uv_derivative_y: vec2f =
		(uv_numerator + uv_numerator_dy * ndc_step_y) /
		(inverse_w_at_pixel + inverse_w_dy * ndc_step_y) - vertex_uv;
}
"#;

pub(crate) const MATERIAL_EVALUATION_TANGENT_SOURCE: &str = r#"
material_evaluation_tangent: fn () -> void {
	let tangent_scale: f32 = 1.0 / (uv_derivative_x.x * uv_derivative_y.y - uv_derivative_y.x * uv_derivative_x.y);
	let T: vec3f = normalize(
		tangent_scale * (uv_derivative_y.y * position_derivative_x - uv_derivative_x.y * position_derivative_y)
	);
	let B: vec3f = normalize(
		tangent_scale * ((0.0 - uv_derivative_y.x) * position_derivative_x + uv_derivative_x.x * position_derivative_y)
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
	normal = vec3f16(normalize(f32(normal.x) * T + f32(normal.y) * B + f32(normal.z) * N));
}
"#;

pub(crate) const MATERIAL_EVALUATION_GEOMETRY_NORMAL_SOURCE: &str = r#"
material_evaluation_normal: fn () -> void {
	normal = vec3f16(N);
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
	// Preserve compact material values and normalized vectors through the BRDF.
	// Positions, shadow projections, HDR radiance, and accumulation remain f32.
	let albedo_rgb: vec3f16 = vec3f16(albedo.x, albedo.y, albedo.z);
	let V_material: vec3f16 = vec3f16(V);
	let one_minus_metalness: f16 = f16(1.0) - metalness;
	let F0: vec3f16 = vec3f16(0.04, 0.04, 0.04) * one_minus_metalness + albedo_rgb * metalness;
	let one_minus_f0: vec3f16 = vec3f16(1.0, 1.0, 1.0) - F0;
	let NdotV: f16 = max(dot(normal, V_material), f16(0.0));
	let roughness_alpha: f16 = roughness * roughness;
	let roughness_alpha_squared: f16 = roughness_alpha * roughness_alpha;
	let adjusted_roughness: f16 = roughness + 1.0;
	let geometry_k: f16 = adjusted_roughness * adjusted_roughness / 8.0;
	let diffuse: vec3f = vec3f(0.0, 0.0, 0.0);
	let specular: vec3f = vec3f(0.0, 0.0, 0.0);
	let ao_factor: f16 = 1.0;
	if (push_constant.blend == 0) {
		ao_factor = f16(fetch(ao, pixel_coordinates).x);
	}
	let view_fresnel_base: f16 = clamp(f16(1.0) - NdotV, f16(0.0), f16(1.0));
	let view_fresnel_squared: f16 = view_fresnel_base * view_fresnel_base;
	let view_fresnel_factor: f16 = view_fresnel_squared * view_fresnel_squared * view_fresnel_base;
	let one_minus_fresnel_n_dot_v: vec3f16 = one_minus_f0 * (f16(1.0) - view_fresnel_factor);
	// These terms depend only on the shaded pixel. Evaluate them once instead of once per light.
	let geometry_view: f16 = NdotV / (NdotV * (1.0 - geometry_k) + geometry_k);
	let light_count: u32 = lighting_data.light_count;
	// Local shadows have no hierarchy early-out, so they share one lazily generated PCF rotation.
	let local_shadow_rotation: vec2f16 = vec2f16(0.0, 0.0);
	let has_local_shadow_rotation: bool = false;

	for (let light_index: u32 = 0; light_index < light_count; light_index = light_index + 1) {
		let light_type: u32 = lighting_data.lights[light_index].type;
		let L: vec3f = vec3f(0.0, 0.0, 0.0);
		let attenuation: f32 = 1.0;
		let light_position: vec3f = vec3f(
			lighting_data.lights[light_index].position.x,
			lighting_data.lights[light_index].position.y,
			lighting_data.lights[light_index].position.z
		);
		if (light_type == 68) {
			L = vec3f(0.0, 0.0, 0.0) - light_position;
		}
		if (light_type != 68) {
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
			let view_space_surface_position: vec3f = views.views[0].view * vec4f(
				world_space_vertex_position.x,
				world_space_vertex_position.y,
				world_space_vertex_position.z,
				1.0
			);
			let shadow_view0: u32 = lighting_data.lights[light_index].shadow_views[0];
			if (shadow_view0 != 0) {
				let shadow_view1: u32 = lighting_data.lights[light_index].shadow_views[1];
				let shadow_view2: u32 = lighting_data.lights[light_index].shadow_views[2];
				let shadow_view3: u32 = lighting_data.lights[light_index].shadow_views[3];
				occlusion_factor = f16(sample_directional_shadow(
					depth_shadow_map,
					shadow_view0,
					shadow_view1,
					shadow_view2,
					shadow_view3,
					world_space_vertex_position,
					view_space_surface_position,
					world_space_vertex_normal,
					L
				));
				if (occlusion_factor == 0.0) {
					continue;
				}
			}
			attenuation = 1.0;
		}
	if (light_type != 68) {
		if (light_type == 0) {
			let shadow_view_index: u32 = lighting_data.lights[light_index].shadow_views[0];
			if (shadow_view_index != 0) {
				let shadow_cube_index: u32 = lighting_data.lights[light_index].shadow_layer;
				if (has_local_shadow_rotation == false) {
					local_shadow_rotation = compute_shadow_rotation(world_space_vertex_position);
					has_local_shadow_rotation = true;
				}
				occlusion_factor = f16(sample_point_shadow(
					shadow_view_index,
					shadow_cube_index,
					local_shadow_rotation,
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
			if (light_type == 1) {
			let cone_direction: vec3f16 = vec3f16(
				lighting_data.lights[light_index].direction.x,
				lighting_data.lights[light_index].direction.y,
				lighting_data.lights[light_index].direction.z
			);
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
				if (has_local_shadow_rotation == false) {
					local_shadow_rotation = compute_shadow_rotation(world_space_vertex_position);
					has_local_shadow_rotation = true;
				}
				occlusion_factor = f16(sample_cone_shadow(
					cone_shadow_map,
					shadow_view_index,
					shadow_layer,
					local_shadow_rotation,
					world_space_vertex_position,
					position_derivative_x,
					position_derivative_y
				));
				if (occlusion_factor == 0.0) {
					continue;
				}
			}
			}
		}
		if (light_type != 68 && lighting_data.lights[light_index].ies_profile_texture != 4294967295) {
			let emission_direction: vec3f = vec3f(0.0, 0.0, 0.0) - L;
			let profile_axis: vec3f = vec3f(
				lighting_data.lights[light_index].direction.x,
				lighting_data.lights[light_index].direction.y,
				lighting_data.lights[light_index].direction.z
			);
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

		let H: vec3f16 = normalize(V_material + L_material);
		let half_view_fresnel_base: f16 = clamp(f16(1.0) - max(dot(H, V_material), f16(0.0)), f16(0.0), f16(1.0));
		let half_view_fresnel_squared: f16 = half_view_fresnel_base * half_view_fresnel_base;
		let half_view_fresnel_factor: f16 = half_view_fresnel_squared * half_view_fresnel_squared * half_view_fresnel_base;
		let F: vec3f16 = F0 + one_minus_f0 * half_view_fresnel_factor;
		let NdotH: f16 = max(dot(normal, H), f16(0.0));
		let denominator_base: f16 = NdotH * NdotH * (roughness_alpha_squared - 1.0) + 1.0;
		let NDF: f16 = roughness_alpha_squared / (3.14159265359 * denominator_base * denominator_base);
		let geometry_light: f16 = NdotL / (NdotL * (1.0 - geometry_k) + geometry_k);
		let local_specular: vec3f16 = (NDF * geometry_view * geometry_light * F) / (4.0 * NdotV * NdotL + 0.000001);
		let light_fresnel_base: f16 = clamp(f16(1.0) - NdotL, f16(0.0), f16(1.0));
		let light_fresnel_squared: f16 = light_fresnel_base * light_fresnel_base;
		let light_fresnel_factor: f16 = light_fresnel_squared * light_fresnel_squared * light_fresnel_base;
		let kD: vec3f16 = one_minus_f0 * (f16(1.0) - light_fresnel_factor)
			* one_minus_fresnel_n_dot_v
			* one_minus_metalness;
		let local_diffuse: vec3f16 = kD * albedo_rgb / 3.14159265359;
		let light_color: vec3f = vec3f(
			lighting_data.lights[light_index].color.x,
			lighting_data.lights[light_index].color.y,
			lighting_data.lights[light_index].color.z
		);
		let irradiance: vec3f = light_color * (attenuation * f32(NdotL * occlusion_factor));
		diffuse = diffuse + vec3f(local_diffuse) * irradiance;
		specular = specular + vec3f(local_specular) * irradiance;
	}

	let ambient_irradiance: vec3f = sample_environment_irradiance(vec3f(normal));
	let incident: vec3f = vec3f(0.0, 0.0, 0.0) - V;
	let reflection_direction: vec3f = incident - 2.0 * dot(incident, vec3f(normal)) * vec3f(normal);
	let reflection_radiance: vec3f = sample_environment_specular(reflection_direction, f32(roughness));
	let one_minus_roughness: f16 = f16(1.0) - roughness;
	let grazing: vec3f16 = vec3f16(max(one_minus_roughness, F0.x), max(one_minus_roughness, F0.y), max(one_minus_roughness, F0.z));
	let kD_ibl: vec3f16 = (one_minus_f0 - (grazing - F0) * view_fresnel_factor) * one_minus_metalness;
	let ibl_diffuse: vec3f = vec3f(kD_ibl * albedo_rgb) * ambient_irradiance;

	let c0: vec4f16 = vec4f16(0.0 - 1.0, 0.0 - 0.0275, 0.0 - 0.572, 0.022);
	let c1: vec4f16 = vec4f16(1.0, 0.0425, 1.04, 0.0 - 0.04);
	let r: vec4f16 = roughness * c0 + c1;
	let a004: f16 = min(r.x * r.x, pow(f16(2.0), (f16(0.0) - f16(9.28)) * NdotV)) * r.x + r.y;
	let env_brdf: vec2f16 = vec2f16(0.0 - 1.04, 1.04) * a004 + vec2f16(r.z, r.w);
	let ibl_specular: vec3f = vec3f(F0 * env_brdf.x + env_brdf.y) * reflection_radiance;
	let ambient: vec3f = ibl_diffuse + ibl_specular;
	ao_factor = ao_factor * occlusion;
	let lit: vec3f = (diffuse + specular) * f32(ao_factor) + ambient * f32(ao_factor) + vec3f(emission);
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
}
"#;

// Computes the projected receiver plane so cone PCF compares each texel against the depth at that texel's center.
// Returning no correction for a degenerate projection keeps the existing bias as a safe fallback.
