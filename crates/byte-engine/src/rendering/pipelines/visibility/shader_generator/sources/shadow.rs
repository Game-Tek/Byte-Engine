pub(crate) const SHADOW_RECEIVER_PLANE_SOURCE: &str = r#"
shadow_receiver_plane_depth_gradient: fn (
	shadow_view_projection: mat4f,
	surface_light_clip_position: vec4f,
	surface_light_ndc_position: vec3f,
	world_space_position_derivative_x: vec3f,
	world_space_position_derivative_y: vec3f
) -> vec2f {
	let light_clip_derivative_x: vec4f = shadow_view_projection * vec4f(
		world_space_position_derivative_x.x,
		world_space_position_derivative_x.y,
		world_space_position_derivative_x.z,
		0.0
	);
	let light_clip_derivative_y: vec4f = shadow_view_projection * vec4f(
		world_space_position_derivative_y.x,
		world_space_position_derivative_y.y,
		world_space_position_derivative_y.z,
		0.0
	);
	let light_ndc_derivative_x: vec3f = (
		vec3f(light_clip_derivative_x.x, light_clip_derivative_x.y, light_clip_derivative_x.z)
		- surface_light_ndc_position * light_clip_derivative_x.w
	) / surface_light_clip_position.w;
	let light_ndc_derivative_y: vec3f = (
		vec3f(light_clip_derivative_y.x, light_clip_derivative_y.y, light_clip_derivative_y.z)
		- surface_light_ndc_position * light_clip_derivative_y.w
	) / surface_light_clip_position.w;
	let shadow_uv_derivative_x: vec2f = vec2f(
		light_ndc_derivative_x.x * 0.5,
		0.0 - light_ndc_derivative_x.y * 0.5
	);
	let shadow_uv_derivative_y: vec2f = vec2f(
		light_ndc_derivative_y.x * 0.5,
		0.0 - light_ndc_derivative_y.y * 0.5
	);
	let shadow_uv_determinant: f32 = shadow_uv_derivative_x.x * shadow_uv_derivative_y.y
		- shadow_uv_derivative_y.x * shadow_uv_derivative_x.y;
	if (abs(shadow_uv_determinant) <= 0.0000000001) {
		return vec2f(0.0, 0.0);
	}
	return vec2f(
		(light_ndc_derivative_x.z * shadow_uv_derivative_y.y
			- light_ndc_derivative_y.z * shadow_uv_derivative_x.y) / shadow_uv_determinant,
		(shadow_uv_derivative_x.x * light_ndc_derivative_y.z
			- shadow_uv_derivative_y.x * light_ndc_derivative_x.z) / shadow_uv_determinant
	);
}
"#;

pub(crate) const SHADOW_TAP_SOURCE: &str = r#"
sample_shadow_tap: fn (
	shadow_map: ArrayTexture2D,
	shadow_uv: vec2f,
	surface_depth: f32,
	receiver_plane_depth_gradient: vec2f,
	offset: vec2f,
	shadow_layer: u32,
	shadow_map_extent: vec2u
) -> f32 {
	let offset_shadow_uv: vec2f = shadow_uv + offset;
	if (offset_shadow_uv.x < 0.0 || offset_shadow_uv.x > 1.0 || offset_shadow_uv.y < 0.0 || offset_shadow_uv.y > 1.0) {
		return 1.0;
	}
	if (surface_depth < 0.0 || surface_depth > 1.0) {
		return 1.0;
	}

	let maximum_texel: vec2u = shadow_map_extent - vec2u(1, 1);
	let shadow_texel: vec2u = vec2u(
		u32(clamp(offset_shadow_uv.x * f32(shadow_map_extent.x), 0.0, f32(maximum_texel.x))),
		u32(clamp(offset_shadow_uv.y * f32(shadow_map_extent.y), 0.0, f32(maximum_texel.y)))
	);
	let texel_center_uv: vec2f = (vec2f(f32(shadow_texel.x), f32(shadow_texel.y)) + vec2f(0.5, 0.5))
		/ vec2f(f32(shadow_map_extent.x), f32(shadow_map_extent.y));
	let tap_surface_depth: f32 = surface_depth + dot(receiver_plane_depth_gradient, texel_center_uv - shadow_uv);
	if (tap_surface_depth < 0.0 || tap_surface_depth > 1.0) {
		return 1.0;
	}
	let closest_depth: f32 = fetch(shadow_map, shadow_texel, shadow_layer).x;
	return step(closest_depth, tap_surface_depth);
}
"#;

pub(crate) const SHADOW_POISSON_ROTATION_SOURCE: &str = r#"
rotate_shadow_poisson_offset: fn (poisson_offset: vec2f16, rotation: vec2f16) -> vec2f16 {
	return vec2f16(
		poisson_offset.x * rotation.x - poisson_offset.y * rotation.y,
		poisson_offset.x * rotation.y + poisson_offset.y * rotation.x
	);
}
"#;

pub(crate) const ROTATED_SHADOW_TAP_SOURCE: &str = r#"
sample_rotated_shadow_tap: fn (
	shadow_map: ArrayTexture2D,
	shadow_uv: vec2f,
	surface_depth: f32,
	receiver_plane_depth_gradient: vec2f,
	poisson_offset: vec2f16,
	rotation: vec2f16,
	texel_size: vec2f16,
	shadow_layer: u32,
	shadow_map_extent: vec2u
) -> f32 {
	let rotated_offset: vec2f16 = rotate_shadow_poisson_offset(poisson_offset, rotation) * texel_size * f16(1.5);
	return sample_shadow_tap(
		shadow_map,
		shadow_uv,
		surface_depth,
		receiver_plane_depth_gradient,
		vec2f(rotated_offset),
		shadow_layer,
		shadow_map_extent
	);
}
"#;

// Directional shadows have one depth for the whole PCF kernel. Interior taps stay
// in texel space after one kernel-wide bounds check, avoiding eight normalize,
// bounds, clamp, receiver-plane, and denormalize sequences.
pub(crate) const DIRECTIONAL_SHADOW_TAP_SOURCE: &str = r#"
sample_directional_shadow_tap: fn (
	shadow_map: ArrayTexture2D,
	shadow_texel_position: vec2f,
	surface_depth: f32,
	poisson_offset: vec2f16,
	rotation: vec2f16,
	shadow_layer: u32
) -> f32 {
	let rotated_offset: vec2f16 = rotate_shadow_poisson_offset(poisson_offset, rotation) * f16(1.5);
	let tap_position: vec2f = shadow_texel_position + vec2f(rotated_offset);
	let shadow_texel: vec2u = vec2u(u32(tap_position.x), u32(tap_position.y));
	let closest_depth: f32 = fetch(shadow_map, shadow_texel, shadow_layer).x;
	return step(closest_depth, surface_depth);
}
"#;

// Proves one directional PCF footprint is fully lit from the 4x4 max-depth level.
// The gather covers every reduction cell touched by the rotated tap footprint.
pub(crate) const DIRECTIONAL_SHADOW_DEPTH_PROBE_SOURCE: &str = r#"
directional_shadow_area_is_fully_lit: fn (
	shadow_uv: vec2f,
	surface_depth: f32,
	shadow_layer: u32,
	shadow_map_extent: vec2u
) -> bool {
	if (shadow_uv.x <= 0.0 || shadow_uv.x >= 1.0 || shadow_uv.y <= 0.0 || shadow_uv.y >= 1.0) {
		return false;
	}

	let shadow_texel_position: vec2f = shadow_uv * vec2f(f32(shadow_map_extent.x), f32(shadow_map_extent.y));
	// One maximum-reduction sample reads the one, two, or four 4x4 cells touched by the footprint.
	let fine_first_cell: vec2u = vec2u(
		u32(max(shadow_texel_position.x - 1.5, 0.0)) / 4,
		u32(max(shadow_texel_position.y - 1.5, 0.0)) / 4
	);
	let fine_last_cell: vec2u = vec2u(
		u32(min(shadow_texel_position.x + 1.5, f32(shadow_map_extent.x - 1))) / 4,
		u32(min(shadow_texel_position.y + 1.5, f32(shadow_map_extent.y - 1))) / 4
	);
	let fine_layer_offset: u32 = shadow_layer * (shadow_map_extent.y / 4);
	// Four cascades packed at quarter resolution make the atlas width W/4 and height H.
	let fine_probe_uv: vec2f = vec2f(
		f32(fine_first_cell.x + fine_last_cell.x) + 1.0,
		f32(fine_first_cell.y + fine_last_cell.y + fine_layer_offset + fine_layer_offset) + 1.0
	) * 0.5 / vec2f(f32(shadow_map_extent.x) * 0.25, f32(shadow_map_extent.y));
	let fine_maximum_depth: f32 = downsample_max(
		directional_shadow_depth_pyramid,
		fine_probe_uv,
		0.0
	);
	return surface_depth >= fine_maximum_depth;
}
"#;

// Cone maps use two positive Depth16Unorm steps as a reverse-Z comparison margin after receiver-plane correction.
pub(crate) const SHADOW_ROTATION_SOURCE: &str = r#"
compute_shadow_rotation: fn (world_space_position: vec3f) -> vec2f16 {
	let rotation_noise: f32 = fract(
		sin(dot(vec2f(world_space_position.x, world_space_position.z) + world_space_position.y, vec2f(12.9898, 78.233))) * 43758.5453
	);
	let rotation_angle: f32 = rotation_noise * 6.2831853;
	let rotation_sine_cosine: vec2f = sincos(rotation_angle);
	return vec2f16(rotation_sine_cosine.y, rotation_sine_cosine.x);
}
"#;

pub(crate) const CONE_SHADOW_SOURCE: &str = r#"
sample_cone_shadow: fn (
	shadow_map: ArrayTexture2D,
	shadow_view_index: u32,
	shadow_layer: u32,
	pcf_rotation: vec2f16,
	world_space_position: vec3f,
	world_space_position_derivative_x: vec3f,
	world_space_position_derivative_y: vec3f
) -> f32 {
	// Avoid materializing the full View record. Cone projection only needs this matrix.
	let shadow_view_projection: mat4f = views.views[shadow_view_index].view_projection;
	let surface_light_clip_position: vec4f = shadow_view_projection * vec4f(
		world_space_position.x,
		world_space_position.y,
		world_space_position.z,
		1.0
	);
	let surface_light_ndc_position: vec3f = vec3f(
		surface_light_clip_position.x,
		surface_light_clip_position.y,
		surface_light_clip_position.z
	) / surface_light_clip_position.w;
	let shadow_uv: vec2f = vec2f(
		surface_light_ndc_position.x * 0.5 + 0.5,
		0.5 - surface_light_ndc_position.y * 0.5
	);
	let surface_depth_bias: f32 = 2.0 / 65535.0;
	let receiver_plane_depth_gradient: vec2f = shadow_receiver_plane_depth_gradient(
		shadow_view_projection,
		surface_light_clip_position,
		surface_light_ndc_position,
		world_space_position_derivative_x,
		world_space_position_derivative_y
	);
	let surface_depth: f32 = surface_light_ndc_position.z + surface_depth_bias;
	if (surface_depth < 0.0 || surface_depth > 1.0) {
		return 1.0;
	}

	let shadow_map_extent: vec2u = texture_size(shadow_map);
	// Cone shadows need per-tap border handling and receiver-plane depth correction.
	let texel_size: vec2f16 = vec2f16(1.0, 1.0) / vec2f16(f32(shadow_map_extent.x), f32(shadow_map_extent.y));
	let occlusion: f32 = 0.0;
	occlusion = occlusion + sample_rotated_shadow_tap(shadow_map, shadow_uv, surface_depth, receiver_plane_depth_gradient, vec2f16(0.0 - 0.613392, 0.617481), pcf_rotation, texel_size, shadow_layer, shadow_map_extent);
	occlusion = occlusion + sample_rotated_shadow_tap(shadow_map, shadow_uv, surface_depth, receiver_plane_depth_gradient, vec2f16(0.170019, 0.0 - 0.040254), pcf_rotation, texel_size, shadow_layer, shadow_map_extent);
	occlusion = occlusion + sample_rotated_shadow_tap(shadow_map, shadow_uv, surface_depth, receiver_plane_depth_gradient, vec2f16(0.0 - 0.299417, 0.791925), pcf_rotation, texel_size, shadow_layer, shadow_map_extent);
	occlusion = occlusion + sample_rotated_shadow_tap(shadow_map, shadow_uv, surface_depth, receiver_plane_depth_gradient, vec2f16(0.645680, 0.493210), pcf_rotation, texel_size, shadow_layer, shadow_map_extent);
	occlusion = occlusion + sample_rotated_shadow_tap(shadow_map, shadow_uv, surface_depth, receiver_plane_depth_gradient, vec2f16(0.0 - 0.651784, 0.717887), pcf_rotation, texel_size, shadow_layer, shadow_map_extent);
	occlusion = occlusion + sample_rotated_shadow_tap(shadow_map, shadow_uv, surface_depth, receiver_plane_depth_gradient, vec2f16(0.421003, 0.027070), pcf_rotation, texel_size, shadow_layer, shadow_map_extent);
	occlusion = occlusion + sample_rotated_shadow_tap(shadow_map, shadow_uv, surface_depth, receiver_plane_depth_gradient, vec2f16(0.0 - 0.817194, 0.0 - 0.271096), pcf_rotation, texel_size, shadow_layer, shadow_map_extent);
	occlusion = occlusion + sample_rotated_shadow_tap(shadow_map, shadow_uv, surface_depth, receiver_plane_depth_gradient, vec2f16(0.0 - 0.705374, 0.0 - 0.668203), pcf_rotation, texel_size, shadow_layer, shadow_map_extent);
	return occlusion / 8.0;
}
"#;

// Point shadows use native cube-array addressing so taps can cross cube-face boundaries.
pub(crate) const POINT_SHADOW_RECEIVER_DEPTH_SOURCE: &str = r#"
point_shadow_receiver_depth: fn (light_to_surface: vec3f, near: f32, far: f32) -> f32 {
	let face_distance: f32 = max(max(abs(light_to_surface.x), abs(light_to_surface.y)), abs(light_to_surface.z));
	return (near * far / face_distance - near) / (far - near);
}
"#;

pub(crate) const POINT_SHADOW_OCCLUSION_SOURCE: &str = r#"
point_shadow_occlusion: fn (
	closest_depth: f32,
	receiver_depth: f32,
	face_distance: f32,
	near: f32,
	far: f32
) -> f32 {
	if (face_distance <= near || closest_depth <= 0.0) {
		return 1.0;
	}
	if (face_distance >= far) {
		return 0.0;
	}
	return step(closest_depth, receiver_depth + 2.0 / 65535.0);
}
"#;

pub(crate) const POINT_SHADOW_RECEIVER_VECTOR_SOURCE: &str = r#"
point_shadow_receiver_vector: fn (
	sample_direction: vec3f,
	center_receiver_vector: vec3f,
	receiver_plane_normal: vec3f
) -> vec3f {
	let ray_alignment: f32 = dot(receiver_plane_normal, sample_direction);
	if (abs(ray_alignment) <= 0.000001) {
		return center_receiver_vector;
	}
	let intersection_distance: f32 = dot(receiver_plane_normal, center_receiver_vector) / ray_alignment;
	if (intersection_distance <= 0.0) {
		return center_receiver_vector;
	}
	return sample_direction * intersection_distance;
}
"#;

// Keeps the receiver plane independent of the screen-space derivative scale as the camera moves.
pub(crate) const POINT_SHADOW_RECEIVER_PLANE_NORMAL_SOURCE: &str = r#"
point_shadow_receiver_plane_normal: fn (
	position_derivative_x: vec3f,
	position_derivative_y: vec3f
) -> vec3f {
	let receiver_plane_normal: vec3f = cross(position_derivative_x, position_derivative_y);
	let length_squared: f32 = dot(receiver_plane_normal, receiver_plane_normal);
	if (length_squared <= 0.0) {
		return vec3f(0.0, 0.0, 0.0);
	}
	return receiver_plane_normal * inversesqrt(length_squared);
}
"#;

// Snaps a cube lookup ray so depth sampling and receiver-plane correction use the same texel-center ray.
pub(crate) const POINT_SHADOW_TEXEL_DIRECTION_SOURCE: &str = r#"
point_shadow_texel_direction: fn (sample_direction: vec3f) -> vec3f {
	let absolute_direction: vec3f = vec3f(
		abs(sample_direction.x),
		abs(sample_direction.y),
		abs(sample_direction.z)
	);
	let face: u32 = 0;
	let face_coordinate: vec2f = vec2f(0.0, 0.0);
	if (absolute_direction.x >= absolute_direction.y && absolute_direction.x >= absolute_direction.z) {
		if (sample_direction.x >= 0.0) {
			face = 0;
			face_coordinate = vec2f(0.0 - sample_direction.z, 0.0 - sample_direction.y) / absolute_direction.x;
		}
		if (sample_direction.x < 0.0) {
			face = 1;
			face_coordinate = vec2f(sample_direction.z, 0.0 - sample_direction.y) / absolute_direction.x;
		}
	}
	if (absolute_direction.y > absolute_direction.x && absolute_direction.y >= absolute_direction.z) {
		if (sample_direction.y >= 0.0) {
			face = 2;
			face_coordinate = vec2f(sample_direction.x, sample_direction.z) / absolute_direction.y;
		}
		if (sample_direction.y < 0.0) {
			face = 3;
			face_coordinate = vec2f(sample_direction.x, 0.0 - sample_direction.z) / absolute_direction.y;
		}
	}
	if (absolute_direction.z > absolute_direction.x && absolute_direction.z > absolute_direction.y) {
		if (sample_direction.z >= 0.0) {
			face = 4;
			face_coordinate = vec2f(sample_direction.x, 0.0 - sample_direction.y) / absolute_direction.z;
		}
		if (sample_direction.z < 0.0) {
			face = 5;
			face_coordinate = vec2f(0.0 - sample_direction.x, 0.0 - sample_direction.y) / absolute_direction.z;
		}
	}

	let texel_position: vec2f = (face_coordinate * 0.5 + vec2f(0.5, 0.5)) * 1024.0;
	let texel: vec2u = vec2u(
		u32(clamp(texel_position.x, 0.0, 1023.0)),
		u32(clamp(texel_position.y, 0.0, 1023.0))
	);
	let texel_center: vec2f = ((vec2f(f32(texel.x), f32(texel.y)) + vec2f(0.5, 0.5)) / 1024.0) * 2.0
		- vec2f(1.0, 1.0);
	let snapped_direction: vec3f = vec3f(1.0, 0.0 - texel_center.y, 0.0 - texel_center.x);
	if (face == 1) {
		snapped_direction = vec3f(0.0 - 1.0, 0.0 - texel_center.y, texel_center.x);
	}
	if (face == 2) {
		snapped_direction = vec3f(texel_center.x, 1.0, texel_center.y);
	}
	if (face == 3) {
		snapped_direction = vec3f(texel_center.x, 0.0 - 1.0, 0.0 - texel_center.y);
	}
	if (face == 4) {
		snapped_direction = vec3f(texel_center.x, 0.0 - texel_center.y, 1.0);
	}
	if (face == 5) {
		snapped_direction = vec3f(0.0 - texel_center.x, 0.0 - texel_center.y, 0.0 - 1.0);
	}
	return normalize(snapped_direction);
}
"#;

pub(crate) const POINT_SHADOW_TAP_SOURCE: &str = r#"
sample_point_shadow_tap: fn (
	shadow_cube_index: u32,
	center_direction: vec3f,
	tangent: vec3f,
	bitangent: vec3f,
	center_receiver_vector: vec3f,
	receiver_plane_normal: vec3f,
	near: f32,
	far: f32,
	poisson_offset: vec2f16,
	pcf_rotation: vec2f16
) -> f32 {
	let tap_offset: vec2f16 = rotate_shadow_poisson_offset(poisson_offset, pcf_rotation)
		* f16(1.5 * 2.0 / 1024.0);
	let sample_direction: vec3f = point_shadow_texel_direction(
		center_direction + tangent * f32(tap_offset.x) + bitangent * f32(tap_offset.y)
	);
	let receiver_vector: vec3f = point_shadow_receiver_vector(
		sample_direction,
		center_receiver_vector,
		receiver_plane_normal
	);
	let face_distance: f32 = max(max(abs(receiver_vector.x), abs(receiver_vector.y)), abs(receiver_vector.z));
	let receiver_depth: f32 = point_shadow_receiver_depth(receiver_vector, near, far);
	let closest_depth: f32 = texture_cube_array_lod(point_shadow_map, sample_direction, shadow_cube_index, 0.0).x;
	return point_shadow_occlusion(closest_depth, receiver_depth, face_distance, near, far);
}
"#;

pub(crate) const POINT_SHADOW_SOURCE: &str = r#"
sample_point_shadow: fn (
	shadow_view_index: u32,
	shadow_cube_index: u32,
	pcf_rotation: vec2f16,
	world_space_position: vec3f,
	light_position: vec3f,
	world_space_position_derivative_x: vec3f,
	world_space_position_derivative_y: vec3f
) -> f32 {
	let light_to_surface: vec3f = world_space_position - light_position;
	let distance_squared: f32 = dot(light_to_surface, light_to_surface);
	if (distance_squared <= 0.0) {
		return 1.0;
	}
	let view: View = views.views[shadow_view_index];
	let receiver_distance: f32 = sqrt(distance_squared);
	let center_direction: vec3f = light_to_surface / receiver_distance;
	let reference: vec3f = vec3f(0.0, 1.0, 0.0);
	if (abs(center_direction.y) > 0.99) {
		reference = vec3f(0.0, 0.0, 1.0);
	}
	let tangent: vec3f = normalize(cross(reference, center_direction));
	let bitangent: vec3f = cross(center_direction, tangent);
	let receiver_plane_normal: vec3f = point_shadow_receiver_plane_normal(
		world_space_position_derivative_x,
		world_space_position_derivative_y
	);
	let occlusion: f32 = 0.0;
	occlusion = occlusion + sample_point_shadow_tap(shadow_cube_index, center_direction, tangent, bitangent, light_to_surface, receiver_plane_normal, view.near, view.far, vec2f16(0.0 - 0.613392, 0.617481), pcf_rotation);
	occlusion = occlusion + sample_point_shadow_tap(shadow_cube_index, center_direction, tangent, bitangent, light_to_surface, receiver_plane_normal, view.near, view.far, vec2f16(0.170019, 0.0 - 0.040254), pcf_rotation);
	occlusion = occlusion + sample_point_shadow_tap(shadow_cube_index, center_direction, tangent, bitangent, light_to_surface, receiver_plane_normal, view.near, view.far, vec2f16(0.0 - 0.299417, 0.791925), pcf_rotation);
	occlusion = occlusion + sample_point_shadow_tap(shadow_cube_index, center_direction, tangent, bitangent, light_to_surface, receiver_plane_normal, view.near, view.far, vec2f16(0.645680, 0.493210), pcf_rotation);
	occlusion = occlusion + sample_point_shadow_tap(shadow_cube_index, center_direction, tangent, bitangent, light_to_surface, receiver_plane_normal, view.near, view.far, vec2f16(0.0 - 0.651784, 0.717887), pcf_rotation);
	occlusion = occlusion + sample_point_shadow_tap(shadow_cube_index, center_direction, tangent, bitangent, light_to_surface, receiver_plane_normal, view.near, view.far, vec2f16(0.421003, 0.027070), pcf_rotation);
	occlusion = occlusion + sample_point_shadow_tap(shadow_cube_index, center_direction, tangent, bitangent, light_to_surface, receiver_plane_normal, view.near, view.far, vec2f16(0.0 - 0.817194, 0.0 - 0.271096), pcf_rotation);
	occlusion = occlusion + sample_point_shadow_tap(shadow_cube_index, center_direction, tangent, bitangent, light_to_surface, receiver_plane_normal, view.near, view.far, vec2f16(0.0 - 0.705374, 0.0 - 0.668203), pcf_rotation);
	return occlusion / 8.0;
}
"#;

pub(crate) const DIRECTIONAL_SHADOW_SOURCE: &str = r#"
sample_directional_shadow: fn (
	shadow_map: ArrayTexture2D,
	shadow_view0: u32,
	shadow_view1: u32,
	shadow_view2: u32,
	shadow_view3: u32,
	world_space_position: vec3f,
	view_space_position: vec3f,
	surface_normal: vec3f,
	surface_to_light_direction: vec3f
) -> f32 {
	let depth_value: f32 = abs(view_space_position.z);
	// Descend only while the surface lies beyond a split. This avoids testing
	// a sentinel cascade index after every successful near-cascade match.
	let cascade_index: u32 = 0;
	let shadow_view_index: u32 = shadow_view0;
	if (depth_value >= views.views[shadow_view0].far) {
		cascade_index = 1;
		shadow_view_index = shadow_view1;
		if (depth_value >= views.views[shadow_view1].far) {
			cascade_index = 2;
			shadow_view_index = shadow_view2;
			if (depth_value >= views.views[shadow_view2].far) {
				cascade_index = 3;
				shadow_view_index = shadow_view3;
			}
		}
	}
	let shadow_layer: u32 = cascade_index;
	let shadow_view_projection: mat4f = views.views[shadow_view_index].view_projection;
	let surface_light_clip_position: vec4f = shadow_view_projection * vec4f(
		world_space_position.x,
		world_space_position.y,
		world_space_position.z,
		1.0
	);
	let surface_light_ndc_position: vec3f = vec3f(
		surface_light_clip_position.x,
		surface_light_clip_position.y,
		surface_light_clip_position.z
	) / surface_light_clip_position.w;
	let shadow_uv: vec2f = vec2f(
		surface_light_ndc_position.x * 0.5 + 0.5,
		0.5 - surface_light_ndc_position.y * 0.5
	);
	let bias_scale: f32 = f32(cascade_index + 1);
	let normal_alignment: f32 = max(dot(surface_normal, surface_to_light_direction), 0.0);
	let cascade_depth_range: f32 = max(
		views.views[shadow_view_index].far - views.views[shadow_view_index].near,
		0.0001
	);
	let slope_scaled_bias: f32 = 0.0002 * bias_scale * (1.0 - normal_alignment);
	let constant_bias: f32 = 0.00002 * bias_scale;
	let cascade_range_bias: f32 = cascade_depth_range * 0.0000025;
	let surface_depth: f32 = surface_light_ndc_position.z + max(slope_scaled_bias + cascade_range_bias, constant_bias);
	if (surface_depth < 0.0 || surface_depth > 1.0) {
		return 1.0;
	}

	let shadow_map_extent: vec2u = texture_size(shadow_map);
	if (directional_shadow_area_is_fully_lit(
		shadow_uv,
		surface_depth,
		shadow_layer,
		shadow_map_extent
	)) {
		return 1.0;
	}

	// Only PCF fallbacks need a rotation; fully lit footprints return above without trigonometry.
	let pcf_rotation: vec2f16 = compute_shadow_rotation(world_space_position);
	// Poisson offsets are expressed in texels. Keep the interior fallback in texel space.
	let shadow_texel_position: vec2f = shadow_uv * vec2f(f32(shadow_map_extent.x), f32(shadow_map_extent.y));
	let shadow_map_extent_f: vec2f = vec2f(f32(shadow_map_extent.x), f32(shadow_map_extent.y));
	let footprint_is_inside: bool = shadow_texel_position.x >= 1.5
		&& shadow_texel_position.y >= 1.5
		&& shadow_texel_position.x <= shadow_map_extent_f.x - 1.5
		&& shadow_texel_position.y <= shadow_map_extent_f.y - 1.5;
	if (footprint_is_inside) {
		let occlusion: f32 = 0.0;
		occlusion = occlusion + sample_directional_shadow_tap(shadow_map, shadow_texel_position, surface_depth, vec2f16(0.0 - 0.613392, 0.617481), pcf_rotation, shadow_layer);
		occlusion = occlusion + sample_directional_shadow_tap(shadow_map, shadow_texel_position, surface_depth, vec2f16(0.170019, 0.0 - 0.040254), pcf_rotation, shadow_layer);
		occlusion = occlusion + sample_directional_shadow_tap(shadow_map, shadow_texel_position, surface_depth, vec2f16(0.0 - 0.299417, 0.791925), pcf_rotation, shadow_layer);
		occlusion = occlusion + sample_directional_shadow_tap(shadow_map, shadow_texel_position, surface_depth, vec2f16(0.645680, 0.493210), pcf_rotation, shadow_layer);
		occlusion = occlusion + sample_directional_shadow_tap(shadow_map, shadow_texel_position, surface_depth, vec2f16(0.0 - 0.651784, 0.717887), pcf_rotation, shadow_layer);
		occlusion = occlusion + sample_directional_shadow_tap(shadow_map, shadow_texel_position, surface_depth, vec2f16(0.421003, 0.027070), pcf_rotation, shadow_layer);
		occlusion = occlusion + sample_directional_shadow_tap(shadow_map, shadow_texel_position, surface_depth, vec2f16(0.0 - 0.817194, 0.0 - 0.271096), pcf_rotation, shadow_layer);
		occlusion = occlusion + sample_directional_shadow_tap(shadow_map, shadow_texel_position, surface_depth, vec2f16(0.0 - 0.705374, 0.0 - 0.668203), pcf_rotation, shadow_layer);
		return occlusion / 8.0;
	}

	let texel_size: vec2f16 = vec2f16(1.0, 1.0) / vec2f16(shadow_map_extent_f);
	let occlusion: f32 = 0.0;
	occlusion = occlusion + sample_rotated_shadow_tap(shadow_map, shadow_uv, surface_depth, vec2f(0.0, 0.0), vec2f16(0.0 - 0.613392, 0.617481), pcf_rotation, texel_size, shadow_layer, shadow_map_extent);
	occlusion = occlusion + sample_rotated_shadow_tap(shadow_map, shadow_uv, surface_depth, vec2f(0.0, 0.0), vec2f16(0.170019, 0.0 - 0.040254), pcf_rotation, texel_size, shadow_layer, shadow_map_extent);
	occlusion = occlusion + sample_rotated_shadow_tap(shadow_map, shadow_uv, surface_depth, vec2f(0.0, 0.0), vec2f16(0.0 - 0.299417, 0.791925), pcf_rotation, texel_size, shadow_layer, shadow_map_extent);
	occlusion = occlusion + sample_rotated_shadow_tap(shadow_map, shadow_uv, surface_depth, vec2f(0.0, 0.0), vec2f16(0.645680, 0.493210), pcf_rotation, texel_size, shadow_layer, shadow_map_extent);
	occlusion = occlusion + sample_rotated_shadow_tap(shadow_map, shadow_uv, surface_depth, vec2f(0.0, 0.0), vec2f16(0.0 - 0.651784, 0.717887), pcf_rotation, texel_size, shadow_layer, shadow_map_extent);
	occlusion = occlusion + sample_rotated_shadow_tap(shadow_map, shadow_uv, surface_depth, vec2f(0.0, 0.0), vec2f16(0.421003, 0.027070), pcf_rotation, texel_size, shadow_layer, shadow_map_extent);
	occlusion = occlusion + sample_rotated_shadow_tap(shadow_map, shadow_uv, surface_depth, vec2f(0.0, 0.0), vec2f16(0.0 - 0.817194, 0.0 - 0.271096), pcf_rotation, texel_size, shadow_layer, shadow_map_extent);
	occlusion = occlusion + sample_rotated_shadow_tap(shadow_map, shadow_uv, surface_depth, vec2f(0.0, 0.0), vec2f16(0.0 - 0.705374, 0.0 - 0.668203), pcf_rotation, texel_size, shadow_layer, shadow_map_extent);
	return occlusion / 8.0;
}
"#;
