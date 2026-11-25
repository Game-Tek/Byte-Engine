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

pub(crate) const SHADOW_POISSON_ROTATION_SOURCE: &str = r#"
rotate_shadow_poisson_offset: fn (poisson_offset: vec2f16, rotation: vec2f16) -> vec2f16 {
	return vec2f16(
		poisson_offset.x * rotation.x - poisson_offset.y * rotation.y,
		poisson_offset.x * rotation.y + poisson_offset.y * rotation.x
	);
}
"#;

// Rotates the point-light Poisson kernel. The rotation is keyed to the shadow-map texel the receiver falls in, not to
// the screen pixel or the exact surface point: a pixel sees a slightly different surface point every frame the camera
// moves, and a rotation keyed to that point would change every frame. `texel_direction` is the texel-center direction
// from `point_shadow_texel_direction`, which is identical for every receiver in the same texel.
pub(crate) const SHADOW_ROTATION_SOURCE: &str = r#"
compute_shadow_rotation: fn (texel_direction: vec3f) -> vec2f16 {
	let rotation_noise: f32 = fract(
		sin(dot(vec2f(texel_direction.x, texel_direction.z) + texel_direction.y, vec2f(12.9898, 78.233))) * 43758.5453
	);
	let rotation_angle: f32 = rotation_noise * 6.2831853;
	let rotation_sine_cosine: vec2f = sincos(rotation_angle);
	return vec2f16(rotation_sine_cosine.y, rotation_sine_cosine.x);
}
"#;

// Adds one texel of the cone tent's 4x4 block to `lit`: its tent weight when the receiver plane at its center lies in
// front of `stored_depth`, or when the texel lies off the map or the plane there lies past the far plane, which count as
// lit. `texel` is the texel's position in texels, an integer, and `shadow_texel_position` the
// receiver's.
pub(crate) const CONE_SHADOW_TENT_TEXEL_SOURCE: &str = r#"
cone_shadow_tent_texel: fn (
	lit: f32,
	stored_depth: f32,
	texel: vec2f,
	shadow_texel_position: vec2f,
	shadow_uv: vec2f,
	surface_depth: f32,
	receiver_plane_depth_gradient: vec2f,
	shadow_map_extent_f: vec2f
) -> f32 {
	let weight: f32 = (2.0 - abs(texel.x + 0.5 - shadow_texel_position.x)) * (2.0 - abs(texel.y + 0.5 - shadow_texel_position.y));
	let inside: bool = texel.x >= 0.0 && texel.y >= 0.0 && texel.x < shadow_map_extent_f.x && texel.y < shadow_map_extent_f.y;
	let tap_depth: f32 = surface_depth
		+ dot(receiver_plane_depth_gradient, (texel + vec2f(0.5, 0.5)) / shadow_map_extent_f - shadow_uv);
	return lit + weight * (if (inside && tap_depth >= 0.0) { step(stored_depth, tap_depth) } else { 1.0 });
}
"#;

// Reads one row of the cone tent's 4x4 block, left to right: the texels in columns `columns` of row `row`.
pub(crate) const CONE_SHADOW_ROW_DEPTHS_SOURCE: &str = r#"
cone_shadow_row_depths: fn (shadow_map: ArrayTexture2D, columns: vec4u, row: u32, shadow_layer: u32) -> vec4f {
	return vec4f(
		fetch(shadow_map, vec2u(columns.x, row), shadow_layer).x,
		fetch(shadow_map, vec2u(columns.y, row), shadow_layer).x,
		fetch(shadow_map, vec2u(columns.z, row), shadow_layer).x,
		fetch(shadow_map, vec2u(columns.w, row), shadow_layer).x
	);
}
"#;

// A two-texel tent for cone shadows: its sixteen taps are the 4x4 block of adjacent texels around the receiver, each
// weighing two less its distance from the receiver on each axis, so a hard edge becomes a smooth ramp. All sixteen are
// read before any is compared, so the reads are in flight together instead of one after another in a loop. Four gathers would read the block too, but they made material evaluation 0.6 % slower by day, when
// no cone light runs, and saved only 1 % more than these fetches in a spotlight's view (2026-10-06).
pub(crate) const CONE_SHADOW_TENT_SOURCE: &str = r#"
sample_cone_shadow_tent: fn (
	shadow_map: ArrayTexture2D,
	shadow_uv: vec2f,
	surface_depth: f32,
	receiver_plane_depth_gradient: vec2f,
	shadow_layer: u32,
	shadow_map_extent: vec2u
) -> f32 {
	let extent: vec2f = vec2f(f32(shadow_map_extent.x), f32(shadow_map_extent.y));
	let receiver: vec2f = shadow_uv * extent;
	let first: vec2f = vec2f(floor(receiver.x - 1.5), floor(receiver.y - 1.5));
	// Texels off the map are read at its edge, and `cone_shadow_tent_texel` counts them as lit.
	let columns: vec4u = vec4u(
		u32(clamp(first.x, 0.0, extent.x - 1.0)),
		u32(clamp(first.x + 1.0, 0.0, extent.x - 1.0)),
		u32(clamp(first.x + 2.0, 0.0, extent.x - 1.0)),
		u32(clamp(first.x + 3.0, 0.0, extent.x - 1.0))
	);
	let row0: vec4f = cone_shadow_row_depths(shadow_map, columns, u32(clamp(first.y, 0.0, extent.y - 1.0)), shadow_layer);
	let row1: vec4f = cone_shadow_row_depths(shadow_map, columns, u32(clamp(first.y + 1.0, 0.0, extent.y - 1.0)), shadow_layer);
	let row2: vec4f = cone_shadow_row_depths(shadow_map, columns, u32(clamp(first.y + 2.0, 0.0, extent.y - 1.0)), shadow_layer);
	let row3: vec4f = cone_shadow_row_depths(shadow_map, columns, u32(clamp(first.y + 3.0, 0.0, extent.y - 1.0)), shadow_layer);
	let gradient: vec2f = receiver_plane_depth_gradient;
	let lit: f32 = 0.0;
	lit = cone_shadow_tent_texel(lit, row0.x, first, receiver, shadow_uv, surface_depth, gradient, extent);
	lit = cone_shadow_tent_texel(lit, row0.y, first + vec2f(1.0, 0.0), receiver, shadow_uv, surface_depth, gradient, extent);
	lit = cone_shadow_tent_texel(lit, row0.z, first + vec2f(2.0, 0.0), receiver, shadow_uv, surface_depth, gradient, extent);
	lit = cone_shadow_tent_texel(lit, row0.w, first + vec2f(3.0, 0.0), receiver, shadow_uv, surface_depth, gradient, extent);
	lit = cone_shadow_tent_texel(lit, row1.x, first + vec2f(0.0, 1.0), receiver, shadow_uv, surface_depth, gradient, extent);
	lit = cone_shadow_tent_texel(lit, row1.y, first + vec2f(1.0, 1.0), receiver, shadow_uv, surface_depth, gradient, extent);
	lit = cone_shadow_tent_texel(lit, row1.z, first + vec2f(2.0, 1.0), receiver, shadow_uv, surface_depth, gradient, extent);
	lit = cone_shadow_tent_texel(lit, row1.w, first + vec2f(3.0, 1.0), receiver, shadow_uv, surface_depth, gradient, extent);
	lit = cone_shadow_tent_texel(lit, row2.x, first + vec2f(0.0, 2.0), receiver, shadow_uv, surface_depth, gradient, extent);
	lit = cone_shadow_tent_texel(lit, row2.y, first + vec2f(1.0, 2.0), receiver, shadow_uv, surface_depth, gradient, extent);
	lit = cone_shadow_tent_texel(lit, row2.z, first + vec2f(2.0, 2.0), receiver, shadow_uv, surface_depth, gradient, extent);
	lit = cone_shadow_tent_texel(lit, row2.w, first + vec2f(3.0, 2.0), receiver, shadow_uv, surface_depth, gradient, extent);
	lit = cone_shadow_tent_texel(lit, row3.x, first + vec2f(0.0, 3.0), receiver, shadow_uv, surface_depth, gradient, extent);
	lit = cone_shadow_tent_texel(lit, row3.y, first + vec2f(1.0, 3.0), receiver, shadow_uv, surface_depth, gradient, extent);
	lit = cone_shadow_tent_texel(lit, row3.z, first + vec2f(2.0, 3.0), receiver, shadow_uv, surface_depth, gradient, extent);
	lit = cone_shadow_tent_texel(lit, row3.w, first + vec2f(3.0, 3.0), receiver, shadow_uv, surface_depth, gradient, extent);
	return lit / 16.0;
}
"#;

// Cone maps use two positive Depth16Unorm steps as a reverse-Z comparison margin after receiver-plane correction.
pub(crate) const CONE_SHADOW_SOURCE: &str = r#"
sample_cone_shadow: fn (
	scene: Scene,
	shadow_map: ArrayTexture2D,
	shadow_view_index: u32,
	shadow_layer: u32,
	world_space_position: vec3f,
	world_space_position_derivative_x: vec3f,
	world_space_position_derivative_y: vec3f
) -> f32 {
	// Avoid materializing the full View record. Cone projection only needs this matrix.
	let shadow_view_projection: mat4f = scene.views.views[shadow_view_index].view_projection;
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
	return sample_cone_shadow_tent(
		shadow_map,
		shadow_uv,
		surface_depth,
		receiver_plane_depth_gradient,
		shadow_layer,
		shadow_map_extent
	);
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
		// The cube projection has no valid receiver depth at or beyond its far plane.
		return 1.0;
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
// `shadow_map_resolution` is the cube faces' texels per side.
pub(crate) const POINT_SHADOW_TEXEL_DIRECTION_SOURCE: &str = r#"
point_shadow_texel_direction: fn (sample_direction: vec3f, shadow_map_resolution: f32) -> vec3f {
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
		} else if (sample_direction.x < 0.0) {
			face = 1;
			face_coordinate = vec2f(sample_direction.z, 0.0 - sample_direction.y) / absolute_direction.x;
		}
	} else if (absolute_direction.y >= absolute_direction.z) {
		if (sample_direction.y >= 0.0) {
			face = 2;
			face_coordinate = vec2f(sample_direction.x, sample_direction.z) / absolute_direction.y;
		} else if (sample_direction.y < 0.0) {
			face = 3;
			face_coordinate = vec2f(sample_direction.x, 0.0 - sample_direction.z) / absolute_direction.y;
		}
	} else {
		if (sample_direction.z >= 0.0) {
			face = 4;
			face_coordinate = vec2f(sample_direction.x, 0.0 - sample_direction.y) / absolute_direction.z;
		} else if (sample_direction.z < 0.0) {
			face = 5;
			face_coordinate = vec2f(0.0 - sample_direction.x, 0.0 - sample_direction.y) / absolute_direction.z;
		}
	}

	let texel_position: vec2f = (face_coordinate * 0.5 + vec2f(0.5, 0.5)) * shadow_map_resolution;
	let texel: vec2u = vec2u(
		u32(clamp(texel_position.x, 0.0, shadow_map_resolution - 1.0)),
		u32(clamp(texel_position.y, 0.0, shadow_map_resolution - 1.0))
	);
	let texel_center: vec2f = ((vec2f(f32(texel.x), f32(texel.y)) + vec2f(0.5, 0.5)) / shadow_map_resolution) * 2.0
		- vec2f(1.0, 1.0);
	// Face zero keeps the initial direction.
	let snapped_direction: vec3f = vec3f(1.0, 0.0 - texel_center.y, 0.0 - texel_center.x);
	match face {
		1 => snapped_direction = vec3f(0.0 - 1.0, 0.0 - texel_center.y, texel_center.x),
		2 => snapped_direction = vec3f(texel_center.x, 1.0, texel_center.y),
		3 => snapped_direction = vec3f(texel_center.x, 0.0 - 1.0, 0.0 - texel_center.y),
		4 => snapped_direction = vec3f(texel_center.x, 0.0 - texel_center.y, 1.0),
		5 => snapped_direction = vec3f(0.0 - texel_center.x, 0.0 - texel_center.y, 0.0 - 1.0),
		_ => {}
	}
	return normalize(snapped_direction);
}
"#;

pub(crate) const POINT_SHADOW_TAP_SOURCE: &str = r#"
sample_point_shadow_tap: fn (
	shadow_map: TextureCubeArray,
	shadow_map_resolution: f32,
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
		* f16(1.5 * 2.0 / shadow_map_resolution);
	let sample_direction: vec3f = point_shadow_texel_direction(
		center_direction + tangent * f32(tap_offset.x) + bitangent * f32(tap_offset.y),
		shadow_map_resolution
	);
	let receiver_vector: vec3f = point_shadow_receiver_vector(
		sample_direction,
		center_receiver_vector,
		receiver_plane_normal
	);
	let face_distance: f32 = max(max(abs(receiver_vector.x), abs(receiver_vector.y)), abs(receiver_vector.z));
	let receiver_depth: f32 = point_shadow_receiver_depth(receiver_vector, near, far);
	let closest_depth: f32 = texture_cube_array_lod(shadow_map, sample_direction, shadow_cube_index, 0.0).x;
	return point_shadow_occlusion(closest_depth, receiver_depth, face_distance, near, far);
}
"#;

// Returns the outward normal of the cube face that a lookup direction selects, with the tie-breaking of
// `point_shadow_texel_direction`: x before y before z.
pub(crate) const POINT_SHADOW_FACE_NORMAL_SOURCE: &str = r#"
point_shadow_face_normal: fn (direction: vec3f) -> vec3f {
	let absolute_direction: vec3f = vec3f(abs(direction.x), abs(direction.y), abs(direction.z));
	if (absolute_direction.x >= absolute_direction.y && absolute_direction.x >= absolute_direction.z) {
		return vec3f(if (direction.x >= 0.0) { 1.0 } else { -1.0 }, 0.0, 0.0);
	}
	if (absolute_direction.y >= absolute_direction.z) {
		return vec3f(0.0, if (direction.y >= 0.0) { 1.0 } else { -1.0 }, 0.0);
	}
	return vec3f(0.0, 0.0, if (direction.z >= 0.0) { 1.0 } else { -1.0 });
}
"#;

// One tap of `sample_point_shadow` for a receiver whose taps all stay on its own cube face. It reads the same texel and
// compares the same receiver-plane depth as `sample_point_shadow_tap`, but works in the coordinates of that face: x and
// y run along the face axes `face_u` and `face_v`, and z along `face_normal`. A direction there is the face position it
// points at, scaled by z, so the tap needs no face choice and no normalization. `center_direction`, `tangent`,
// `bitangent`, and `receiver_plane_normal` are given in these coordinates. `receiver_plane_distance` is the receiver
// plane's distance from the light along its normal, and `inverse_center_face_distance` the reciprocal of the receiver's
// own face distance, which a tap compares when its ray runs along the plane or meets it behind the light.
pub(crate) const POINT_SHADOW_FACE_TAP_SOURCE: &str = r#"
sample_point_shadow_face_tap: fn (
	face_u: vec3f,
	face_v: vec3f,
	face_normal: vec3f,
	center_direction: vec3f,
	tangent: vec3f,
	bitangent: vec3f,
	receiver_plane_normal: vec3f,
	receiver_plane_distance: f32,
	inverse_receiver_plane_distance: f32,
	inverse_center_face_distance: f32,
	near: f32,
	far: f32,
	shadow_map: TextureCubeArray,
	shadow_map_resolution: f32,
	shadow_cube_index: u32,
	poisson_offset: vec2f16,
	pcf_rotation: vec2f16
) -> f32 {
	let tap_offset: vec2f16 = rotate_shadow_poisson_offset(poisson_offset, pcf_rotation)
		* f16(1.5 * 2.0 / shadow_map_resolution);
	let ray: vec3f = center_direction + tangent * f32(tap_offset.x) + bitangent * f32(tap_offset.y);
	// The texel `point_shadow_texel_direction` snaps the ray to, and its center on the face, from -1 to 1.
	let texel_position: vec2f = (vec2f(ray.x, ray.y) / ray.z * 0.5 + vec2f(0.5, 0.5)) * shadow_map_resolution;
	let texel_center: vec2f = vec2f(
		floor(clamp(texel_position.x, 0.0, shadow_map_resolution - 1.0)) + 0.5,
		floor(clamp(texel_position.y, 0.0, shadow_map_resolution - 1.0)) + 0.5
	) / (shadow_map_resolution * 0.5) - vec2f(1.0, 1.0);
	// The ray through the texel center has a z of one, so the distance along it to the receiver plane is the face distance
	// there. The plane test of `point_shadow_receiver_vector` uses the unit ray, which is this one divided by its length.
	let ray_alignment: f32 = dot(receiver_plane_normal, vec3f(texel_center.x, texel_center.y, 1.0));
	let meets_plane: bool = ray_alignment * ray_alignment > 0.000000000001 * (1.0 + dot(texel_center, texel_center))
		&& ray_alignment * receiver_plane_distance > 0.0;
	let inverse_face_distance: f32 = if (meets_plane) {
		ray_alignment * inverse_receiver_plane_distance
	} else {
		inverse_center_face_distance
	};
	// As in `point_shadow_occlusion`, a tap whose face distance lies outside the shadow's range is lit. A receiver depth of
	// two lies in front of every stored depth.
	let in_range: bool = inverse_face_distance * near < 1.0 && inverse_face_distance * far > 1.0;
	let receiver_depth: f32 = if (in_range) { (near * far * inverse_face_distance - near) / (far - near) + 2.0 / 65535.0 } else { 2.0 };
	let closest_depth: f32 = texture_cube_array_lod(
		shadow_map,
		face_normal + face_u * texel_center.x + face_v * texel_center.y,
		shadow_cube_index,
		0.0
	).x;
	return step(closest_depth, receiver_depth);
}
"#;

// Returns tap `tap`, from zero to seven, of the eight-tap Poisson disk that point shadows rotate per receiver.
pub(crate) const POINT_SHADOW_POISSON_OFFSET_SOURCE: &str = r#"
point_shadow_poisson_offset: fn (tap: u32) -> vec2f16 {
	match tap {
		1 => return vec2f16(0.170019, -0.040254),
		2 => return vec2f16(-0.299417, 0.791925),
		3 => return vec2f16(0.645680, 0.493210),
		4 => return vec2f16(-0.651784, 0.717887),
		5 => return vec2f16(0.421003, 0.027070),
		6 => return vec2f16(-0.817194, -0.271096),
		7 => return vec2f16(-0.705374, -0.668203),
		_ => return vec2f16(-0.613392, 0.617481),
	}
}
"#;

// Filters a point light's shadow with eight Poisson taps around the receiver, rotated per shadow-map texel. Taps of a
// receiver inside its cube face run in that face's coordinates with `sample_point_shadow_face_tap`, which costs far less
// than choosing a face for every tap. Near a face edge, where a tap can cross onto the next face, every tap chooses its
// own face with `sample_point_shadow_tap`. `shadow_map` is the point image of the light's size class, whose cube faces
// have `shadow_map_resolution` texels per side.
pub(crate) const POINT_SHADOW_SOURCE: &str = r#"
sample_point_shadow: fn (
	scene: Scene,
	shadow_map: TextureCubeArray,
	shadow_map_resolution: f32,
	shadow_view_index: u32,
	shadow_cube_index: u32,
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
	let view: View = scene.views.views[shadow_view_index];
	let receiver_distance: f32 = sqrt(distance_squared);
	let center_direction: vec3f = light_to_surface / receiver_distance;
	let pcf_rotation: vec2f16 = compute_shadow_rotation(point_shadow_texel_direction(center_direction, shadow_map_resolution));
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
	// The receiver's cube face and the axes its face coordinates run along, as cube maps lay them out. Each axis is a unit
	// vector along x, y, or z, so moving a vector into face coordinates only reorders and negates its components, exactly.
	let face_normal: vec3f = point_shadow_face_normal(center_direction);
	let face_u: vec3f = vec3f(abs(face_normal.y) + face_normal.z, 0.0, -face_normal.x);
	let face_v: vec3f = vec3f(0.0, -abs(face_normal.x) - abs(face_normal.z), face_normal.y);
	let face_center: vec3f = vec3f(
		dot(center_direction, face_u),
		dot(center_direction, face_v),
		dot(center_direction, face_normal)
	);
	let occlusion: f32 = 0.0;
	// A tap lies at most 3 / resolution from the center direction, which moves the gap between the face component and
	// another component by at most 4.25 / resolution. A receiver whose gaps both exceed 5.12 / resolution, 0.005 at 1024
	// texels, keeps every tap on its face.
	let face_gap: f32 = 5.12 / shadow_map_resolution;
	if (face_center.z - abs(face_center.x) > face_gap && face_center.z - abs(face_center.y) > face_gap) {
		let face_tangent: vec3f = vec3f(dot(tangent, face_u), dot(tangent, face_v), dot(tangent, face_normal));
		let face_bitangent: vec3f = vec3f(dot(bitangent, face_u), dot(bitangent, face_v), dot(bitangent, face_normal));
		let face_plane_normal: vec3f = vec3f(
			dot(receiver_plane_normal, face_u),
			dot(receiver_plane_normal, face_v),
			dot(receiver_plane_normal, face_normal)
		);
		let plane_distance: f32 = dot(receiver_plane_normal, light_to_surface);
		let inverse_plane_distance: f32 = 1.0 / plane_distance;
		let inverse_center_face_distance: f32 = 1.0
			/ max(max(abs(light_to_surface.x), abs(light_to_surface.y)), abs(light_to_surface.z));
		// The taps are written out, so their eight cube reads are in flight together.
		occlusion = occlusion + sample_point_shadow_face_tap(face_u, face_v, face_normal, face_center, face_tangent, face_bitangent, face_plane_normal, plane_distance, inverse_plane_distance, inverse_center_face_distance, view.near, view.far, shadow_map, shadow_map_resolution, shadow_cube_index, point_shadow_poisson_offset(0), pcf_rotation);
		occlusion = occlusion + sample_point_shadow_face_tap(face_u, face_v, face_normal, face_center, face_tangent, face_bitangent, face_plane_normal, plane_distance, inverse_plane_distance, inverse_center_face_distance, view.near, view.far, shadow_map, shadow_map_resolution, shadow_cube_index, point_shadow_poisson_offset(1), pcf_rotation);
		occlusion = occlusion + sample_point_shadow_face_tap(face_u, face_v, face_normal, face_center, face_tangent, face_bitangent, face_plane_normal, plane_distance, inverse_plane_distance, inverse_center_face_distance, view.near, view.far, shadow_map, shadow_map_resolution, shadow_cube_index, point_shadow_poisson_offset(2), pcf_rotation);
		occlusion = occlusion + sample_point_shadow_face_tap(face_u, face_v, face_normal, face_center, face_tangent, face_bitangent, face_plane_normal, plane_distance, inverse_plane_distance, inverse_center_face_distance, view.near, view.far, shadow_map, shadow_map_resolution, shadow_cube_index, point_shadow_poisson_offset(3), pcf_rotation);
		occlusion = occlusion + sample_point_shadow_face_tap(face_u, face_v, face_normal, face_center, face_tangent, face_bitangent, face_plane_normal, plane_distance, inverse_plane_distance, inverse_center_face_distance, view.near, view.far, shadow_map, shadow_map_resolution, shadow_cube_index, point_shadow_poisson_offset(4), pcf_rotation);
		occlusion = occlusion + sample_point_shadow_face_tap(face_u, face_v, face_normal, face_center, face_tangent, face_bitangent, face_plane_normal, plane_distance, inverse_plane_distance, inverse_center_face_distance, view.near, view.far, shadow_map, shadow_map_resolution, shadow_cube_index, point_shadow_poisson_offset(5), pcf_rotation);
		occlusion = occlusion + sample_point_shadow_face_tap(face_u, face_v, face_normal, face_center, face_tangent, face_bitangent, face_plane_normal, plane_distance, inverse_plane_distance, inverse_center_face_distance, view.near, view.far, shadow_map, shadow_map_resolution, shadow_cube_index, point_shadow_poisson_offset(6), pcf_rotation);
		occlusion = occlusion + sample_point_shadow_face_tap(face_u, face_v, face_normal, face_center, face_tangent, face_bitangent, face_plane_normal, plane_distance, inverse_plane_distance, inverse_center_face_distance, view.near, view.far, shadow_map, shadow_map_resolution, shadow_cube_index, point_shadow_poisson_offset(7), pcf_rotation);
	} else {
		// One pass per set bit, like the light loop. The compiler cannot count these passes, so this rare path stays a
		// compact loop. Written out, its eight taps doubled the kernel's spill and added reloads to the light loop, which made
		// material evaluation 0.3 % slower by day, when no point light runs; counted loops compiled to light-loop reloads
		// too (2026-10-06).
		for (let taps: u32 = 255; taps != 0; taps = taps & (taps - 1)) {
			occlusion = occlusion + sample_point_shadow_tap(
				shadow_map,
				shadow_map_resolution,
				shadow_cube_index,
				center_direction,
				tangent,
				bitangent,
				light_to_surface,
				receiver_plane_normal,
				view.near,
				view.far,
				point_shadow_poisson_offset(find_lsb(taps)),
				pcf_rotation
			);
		}
	}
	return occlusion / 8.0;
}
"#;

// Resolves the cascades of sun slot `sun` at a blended surface, with the filter the sun visibility resolve runs for
// opaque pixels: `sample_directional_shadow` in `layouts/directional_shadows.besl`. A blended surface lies in front of
// the depth that pass resolves, so material evaluation calls this for it, without contact shadows, which trace that
// depth. Sun slot `s` holds its cascades in views `4 s + 1` through `4 s + 4`, as the resolve reads them.
//
// `view_depth` is the surface's depth along the camera's view axis, the clip w the material shader keeps. The position
// derivatives lie along the surface, so they give the receiver's plane, and `geometric_normal` is its triangle's unit
// normal. A slot without a shadowed sun returns one, and so does the sun under virtual shadow maps, whose pages blended
// surfaces don't sample.
pub(crate) const BLENDED_SUN_SHADOW_SOURCE: &str = r#"
resolve_blended_sun_shadow: fn (
	scene: Scene,
	shadows: DirectionalShadows,
	sun: u32,
	world_space_position: vec3f,
	view_depth: f32,
	world_space_position_derivative_x: vec3f,
	world_space_position_derivative_y: vec3f,
	geometric_normal: vec3f
) -> f32 {
	if (sun >= shadows.sun_parameters.sun_count || shadows.sun_parameters.virtual_shadows != 0) {
		return 1.0;
	}
	// The suns' directions are in view space, where the contact-shadow trace marches them.
	let view_normal: vec3f = scene.views.views[0].view * vec4f(geometric_normal.x, geometric_normal.y, geometric_normal.z, 0.0);
	let sun_direction: vec4f = shadows.sun_parameters.suns[sun];
	let normal_offset: vec3f = directional_shadow_normal_offset(
		geometric_normal, dot(view_normal, vec3f(sun_direction.x, sun_direction.y, sun_direction.z))
	);
	let first_view: u32 = sun * 4 + 1;
	let shadow_map_width: f32 = f32(texture_size(shadows.depth_shadow_map).x);
	// The resolve computes each cascade's scales once per workgroup. A pixel list has no workgroup to share them with.
	let view_projection0: mat4f = scene.views.views[first_view].view_projection;
	let view_projection1: mat4f = scene.views.views[first_view + 1].view_projection;
	let view_projection2: mat4f = scene.views.views[first_view + 2].view_projection;
	let view_projection3: mat4f = scene.views.views[first_view + 3].view_projection;
	return sample_directional_shadow(
		scene, shadows,
		shadows.depth_shadow_map,
		first_view,
		first_view + 1,
		first_view + 2,
		first_view + 3,
		vec4f(
			directional_shadow_texels_per_meter(view_projection0, shadow_map_width),
			directional_shadow_texels_per_meter(view_projection1, shadow_map_width),
			directional_shadow_texels_per_meter(view_projection2, shadow_map_width),
			directional_shadow_texels_per_meter(view_projection3, shadow_map_width)
		),
		vec4f(
			directional_shadow_depth_per_meter(view_projection0),
			directional_shadow_depth_per_meter(view_projection1),
			directional_shadow_depth_per_meter(view_projection2),
			directional_shadow_depth_per_meter(view_projection3)
		),
		sun_direction.w,
		world_space_position,
		view_depth,
		world_space_position_derivative_x,
		world_space_position_derivative_y,
		normal_offset
	);
}
"#;
