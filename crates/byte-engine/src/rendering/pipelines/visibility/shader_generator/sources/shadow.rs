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

// Filters shadow maps with a tent over a 4x4 grid of taps spaced `texel_spacing` texels apart, so it reaches two
// spacings to each side of the receiver. The grid is anchored to the shadow map, not to the receiver, and each tap reads
// the one texel at its grid point. Each tap's weight changes smoothly as the receiver moves, so penumbrae are smooth
// ramps with no noise, and they stay still from frame to frame because cascades are snapped to the texel grid. On each
// axis the four weights are 1 - f, 2 - f, 1 + f, and f, where f is the receiver's position within its grid cell. They
// always sum to four, so the filter never needs renormalizing.
//
// A spacing of one filters every texel under a two-texel tent. Wider spacings give wider penumbrae at the same cost,
// and resolve occluder outlines at the spacing, which the wider tent smooths the same way.
//
// This version checks every tap against the map's edges and treats taps outside the map as lit. Cone shadows and
// directional receivers near a cascade's edge use it.
pub(crate) const SHADOW_TENT_SOURCE: &str = r#"
sample_shadow_tent: fn (
	shadow_map: ArrayTexture2D,
	shadow_uv: vec2f,
	surface_depth: f32,
	receiver_plane_depth_gradient: vec2f,
	shadow_layer: u32,
	shadow_map_extent: vec2u,
	texel_spacing: f32
) -> f32 {
	let shadow_map_extent_f: vec2f = vec2f(f32(shadow_map_extent.x), f32(shadow_map_extent.y));
	let shadow_texel_position: vec2f = shadow_uv * shadow_map_extent_f;
	let grid_position: vec2f = shadow_texel_position / texel_spacing;
	// Grid coordinates can be negative here, so they stay in floats until `sample_shadow_tap` bounds them.
	let first_cell: vec2f = vec2f(floor(grid_position.x - 1.5), floor(grid_position.y - 1.5));
	let lit: f32 = 0.0;
	for (let row: u32 = 0; row < 4; row = row + 1) {
		let cell_center_y: f32 = first_cell.y + f32(row) + 0.5;
		let weight_y: f32 = 2.0 - abs(cell_center_y - grid_position.y);
		for (let column: u32 = 0; column < 4; column = column + 1) {
			let cell_center: vec2f = vec2f(first_cell.x + f32(column) + 0.5, cell_center_y);
			let weight: f32 = (2.0 - abs(cell_center.x - grid_position.x)) * weight_y;
			let texel_center: vec2f = vec2f(
				floor(cell_center.x * texel_spacing),
				floor(cell_center.y * texel_spacing)
			) + vec2f(0.5, 0.5);
			lit = lit + weight * sample_shadow_tap(
				shadow_map,
				shadow_uv,
				surface_depth,
				receiver_plane_depth_gradient,
				(texel_center - shadow_texel_position) / shadow_map_extent_f,
				shadow_layer,
				shadow_map_extent
			);
		}
	}
	return lit / 16.0;
}
"#;

// The same tent filter as `sample_shadow_tent`, for directional receivers. When the whole footprint, which reaches
// `2 * texel_spacing` texels from the receiver on each axis, lies inside the map, it skips the per-tap bounds checks
// and works in texel space with the receiver plane expressed per texel. Otherwise it falls back to
// `sample_shadow_tent`.
pub(crate) const DIRECTIONAL_SHADOW_TENT_SOURCE: &str = r#"
sample_directional_shadow_tent: fn (
	shadow_map: ArrayTexture2D,
	shadow_uv: vec2f,
	surface_depth: f32,
	receiver_plane_depth_gradient: vec2f,
	shadow_layer: u32,
	shadow_map_extent: vec2u,
	texel_spacing: f32
) -> f32 {
	let shadow_map_extent_f: vec2f = vec2f(f32(shadow_map_extent.x), f32(shadow_map_extent.y));
	let shadow_texel_position: vec2f = shadow_uv * shadow_map_extent_f;
	let reach: f32 = 2.0 * texel_spacing;
	if (shadow_texel_position.x < reach
		|| shadow_texel_position.y < reach
		|| shadow_texel_position.x > shadow_map_extent_f.x - 1.0 - reach
		|| shadow_texel_position.y > shadow_map_extent_f.y - 1.0 - reach) {
		return sample_shadow_tent(
			shadow_map, shadow_uv, surface_depth, receiver_plane_depth_gradient, shadow_layer, shadow_map_extent, texel_spacing
		);
	}
	let depth_gradient_per_texel: vec2f = receiver_plane_depth_gradient / shadow_map_extent_f;
	let grid_position: vec2f = shadow_texel_position / texel_spacing;
	let first_cell: vec2f = vec2f(floor(grid_position.x - 1.5), floor(grid_position.y - 1.5));
	let lit: f32 = 0.0;
	for (let row: u32 = 0; row < 4; row = row + 1) {
		let cell_center_y: f32 = first_cell.y + f32(row) + 0.5;
		let weight_y: f32 = 2.0 - abs(cell_center_y - grid_position.y);
		for (let column: u32 = 0; column < 4; column = column + 1) {
			let cell_center_x: f32 = first_cell.x + f32(column) + 0.5;
			let weight: f32 = (2.0 - abs(cell_center_x - grid_position.x)) * weight_y;
			let shadow_texel: vec2u = vec2u(u32(cell_center_x * texel_spacing), u32(cell_center_y * texel_spacing));
			let texel_center: vec2f = vec2f(f32(shadow_texel.x), f32(shadow_texel.y)) + vec2f(0.5, 0.5);
			let closest_depth: f32 = fetch(shadow_map, shadow_texel, shadow_layer).x;
			// The map holds each texel's depth at its center, so the receiver is compared at that same point on its
			// plane.
			let tap_surface_depth: f32 = surface_depth + dot(depth_gradient_per_texel, texel_center - shadow_texel_position);
			lit = lit + weight * step(closest_depth, tap_surface_depth);
		}
	}
	return lit / 16.0;
}
"#;


// Proves every texel the directional blocker search and penumbra filter can read is no closer to the light than the
// receiver's center, so the receiver is fully lit. It covers the 4x4 block of eight-texel max-depth cells that
// `directional_shadow_blocker_depth` searches with four maximum-reduction samples, each on the corner shared by four
// cells.
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
	// Four cascades packed at an eighth of their resolution make the pyramid W/8 cells wide and 4H/8 cells tall.
	let cell_extent: vec2u = shadow_map_extent / vec2u(8, 8);
	let layer_offset: f32 = f32(shadow_layer * cell_extent.y);
	let pyramid_extent: vec2f = vec2f(f32(cell_extent.x), f32(cell_extent.y * 4));
	let first_cell: vec2f = vec2f(
		floor(shadow_texel_position.x / 8.0 - 1.5),
		floor(shadow_texel_position.y / 8.0 - 1.5)
	);
	// Corners are clamped to the cascade, so no sample reads a neighboring cascade's cells.
	let maximum_corner: vec2f = vec2f(f32(cell_extent.x - 1), f32(cell_extent.y - 1));
	let block_maximum: f32 = 0.0;
	for (let quad_row: u32 = 0; quad_row < 2; quad_row = quad_row + 1) {
		for (let quad_column: u32 = 0; quad_column < 2; quad_column = quad_column + 1) {
			let corner: vec2f = vec2f(
				clamp(first_cell.x + f32(quad_column * 2 + 1), 1.0, maximum_corner.x),
				clamp(first_cell.y + f32(quad_row * 2 + 1), 1.0, maximum_corner.y)
			);
			block_maximum = max(block_maximum, downsample_max(
				directional_shadow_depth_pyramid,
				vec2f(corner.x, corner.y + layer_offset) / pyramid_extent,
				0.0
			));
		}
	}
	return surface_depth >= block_maximum;
}
"#;

// Returns the depth of the occluders near a directional receiver, for sizing its penumbra, or zero when nothing near it
// rises above the receiver's plane. Zero is never a blocker's depth, because blockers lie closer to the light than a
// receiver whose depth is at least zero. Occluders that touch the receiver can be missed, and they need the sharpest
// penumbra anyway.
//
// The search reads the 4x4 max-depth cells of the cascade's depth pyramid, eight texels each, around the receiver: the
// cells up to 32 texels wide that also hold every tap of the widest directional filter. A cell's maximum is the depth of
// its occluder closest to the light, so the search never misses an occluder, and it leans toward the taller parts of
// one, which widens penumbrae slightly.
//
// The estimate changes smoothly as the receiver moves, so penumbrae do not jump in width. Cells are weighted by the
// same kind of tent as the filter. A cell's occluder fades in over its first five centimeters above the receiver's
// plane, `depth_per_meter` converting that distance to stored depth, instead of counting all at once. And while the
// occluders found carry less than one unit of weight, the result moves toward the receiver's own depth, so a penumbra
// grows from zero as an occluder enters the search.
pub(crate) const DIRECTIONAL_SHADOW_BLOCKER_SOURCE: &str = r#"
directional_shadow_blocker_depth: fn (
	shadow_texel_position: vec2f,
	surface_depth: f32,
	depth_gradient_per_texel: vec2f,
	depth_per_meter: f32,
	shadow_layer: u32,
	shadow_map_extent: vec2u
) -> f32 {
	// Four cascades packed at an eighth of their resolution make the pyramid W/8 cells wide and 4H/8 cells tall.
	let cell_extent: vec2u = shadow_map_extent / vec2u(8, 8);
	let layer_offset: f32 = f32(shadow_layer * cell_extent.y);
	let grid_position: vec2f = shadow_texel_position / 8.0;
	let first_cell: vec2f = vec2f(floor(grid_position.x - 1.5), floor(grid_position.y - 1.5));

	// Within one cell the receiver's own texel centers lie up to 3.5 texels from the cell center on each axis, so its
	// plane rises by up to 3.5 texels of slope there. Occluders start counting only above four texels of slope, so the
	// receiver never counts as its own blocker.
	let cell_plane_rise: f32 = 4.0 * (abs(depth_gradient_per_texel.x) + abs(depth_gradient_per_texel.y));
	let fade_depth: f32 = 0.05 * depth_per_meter;
	let weighted_depth: f32 = 0.0;
	let total_weight: f32 = 0.0;
	for (let row: u32 = 0; row < 4; row = row + 1) {
		let cell_y: f32 = first_cell.y + f32(row);
		let weight_y: f32 = 2.0 - abs(cell_y + 0.5 - grid_position.y);
		for (let column: u32 = 0; column < 4; column = column + 1) {
			let cell_x: f32 = first_cell.x + f32(column);
			// Cells past the cascade's edge hold no occluders for it.
			if (cell_x >= 0.0 && cell_y >= 0.0 && cell_x < f32(cell_extent.x) && cell_y < f32(cell_extent.y)) {
				let cell_maximum: f32 = fetch(
					directional_shadow_depth_pyramid,
					vec2u(u32(cell_x), u32(cell_y + layer_offset))
				).x;
				let cell_center: vec2f = vec2f(cell_x + 0.5, cell_y + 0.5) * 8.0;
				let plane_depth: f32 = surface_depth + dot(depth_gradient_per_texel, cell_center - shadow_texel_position);
				let blocker_share: f32 = clamp((cell_maximum - plane_depth - cell_plane_rise) / fade_depth, 0.0, 1.0);
				let weight: f32 = (2.0 - abs(cell_x + 0.5 - grid_position.x)) * weight_y * blocker_share;
				weighted_depth = weighted_depth + weight * cell_maximum;
				total_weight = total_weight + weight;
			}
		}
	}
	if (total_weight <= 0.0) {
		return 0.0;
	}
	return mix(surface_depth, weighted_depth / total_weight, min(total_weight, 1.0));
}
"#;

// Returns how many shadow-map texels one meter spans in a directional cascade. Normalized device x spans two units
// across the map, and the length of the first row of the cascade's orthographic projection is how many of those units
// one meter spans.
pub(crate) const DIRECTIONAL_SHADOW_TEXELS_PER_METER_SOURCE: &str = r#"
directional_shadow_texels_per_meter: fn (shadow_view_projection: mat4f, shadow_map_width: f32) -> f32 {
	let world_x: vec4f = shadow_view_projection * vec4f(1.0, 0.0, 0.0, 0.0);
	let world_y: vec4f = shadow_view_projection * vec4f(0.0, 1.0, 0.0, 0.0);
	let world_z: vec4f = shadow_view_projection * vec4f(0.0, 0.0, 1.0, 0.0);
	return length(vec3f(world_x.x, world_y.x, world_z.x)) * 0.5 * shadow_map_width;
}
"#;

// Returns how many units of stored depth one meter along the light spans in a directional cascade: the length of the
// third row of its orthographic projection.
pub(crate) const DIRECTIONAL_SHADOW_DEPTH_PER_METER_SOURCE: &str = r#"
directional_shadow_depth_per_meter: fn (shadow_view_projection: mat4f) -> f32 {
	let world_x: vec4f = shadow_view_projection * vec4f(1.0, 0.0, 0.0, 0.0);
	let world_y: vec4f = shadow_view_projection * vec4f(0.0, 1.0, 0.0, 0.0);
	let world_z: vec4f = shadow_view_projection * vec4f(0.0, 0.0, 1.0, 0.0);
	return length(vec3f(world_x.z, world_y.z, world_z.z));
}
"#;

// Returns the view index of one directional cascade from the light's four cascade views.
pub(crate) const DIRECTIONAL_SHADOW_CASCADE_VIEW_SOURCE: &str = r#"
directional_shadow_cascade_view: fn (cascade: u32, view0: u32, view1: u32, view2: u32, view3: u32) -> u32 {
	if (cascade == 1) {
		return view1;
	}
	if (cascade == 2) {
		return view2;
	}
	if (cascade == 3) {
		return view3;
	}
	return view0;
}
"#;

// Returns one cascade's texels per meter from the four cascades' scales, cascade zero in x through cascade three in w.
pub(crate) const DIRECTIONAL_SHADOW_CASCADE_SCALE_SOURCE: &str = r#"
directional_shadow_cascade_scale: fn (cascade: u32, texels_per_meter: vec4f) -> f32 {
	if (cascade == 1) {
		return texels_per_meter.y;
	}
	if (cascade == 2) {
		return texels_per_meter.z;
	}
	if (cascade == 3) {
		return texels_per_meter.w;
	}
	return texels_per_meter.x;
}
"#;

// Returns the first cascade from `first_cascade` through `last_cascade` in which a distance of `reach_meters` spans at
// most `limit_texels` texels, or `last_cascade` when none does. `texels_per_meter` holds each cascade's scale, from
// cascade zero in x to cascade three in w. Directional shadows use it to search for occluders in a cascade whose search
// window is wide enough, and to filter a penumbra in the finest cascade that can hold it.
pub(crate) const DIRECTIONAL_SHADOW_FITTING_CASCADE_SOURCE: &str = r#"
directional_shadow_fitting_cascade: fn (
	first_cascade: u32,
	last_cascade: u32,
	reach_meters: f32,
	texels_per_meter: vec4f,
	limit_texels: f32
) -> u32 {
	for (let cascade: u32 = first_cascade; cascade < last_cascade; cascade = cascade + 1) {
		if (reach_meters * directional_shadow_cascade_scale(cascade, texels_per_meter) <= limit_texels) {
			return cascade;
		}
	}
	return last_cascade;
}
"#;

// Projects a world-space receiver into one directional cascade. Returns its shadow-map uv in x and y and its stored
// depth, with a small margin, in z. W is one when the receiver lies inside the cascade's depth range, and zero when it
// lies outside and so is lit.
pub(crate) const DIRECTIONAL_SHADOW_RECEIVER_SOURCE: &str = r#"
directional_shadow_receiver: fn (shadow_view_projection: mat4f, cascade_index: u32, world_space_position: vec3f) -> vec4f {
	let clip_position: vec4f = shadow_view_projection * vec4f(
		world_space_position.x,
		world_space_position.y,
		world_space_position.z,
		1.0
	);
	let ndc_position: vec3f = vec3f(clip_position.x, clip_position.y, clip_position.z) / clip_position.w;
	// PCF taps compare the receiver's own plane at each fetched texel center, which is exact for flat receivers at
	// any light angle and texel size. The constant margin absorbs the 16-bit rounding of stored depth, at most one step
	// of 1/65535, and rounding between the shadow and material transforms. Coarser cascades span more depth per
	// texel, so it grows with the cascade, from one and a half steps.
	let surface_depth: f32 = ndc_position.z + 1.5 / 65535.0 * f32(cascade_index + 1);
	let inside: f32 = 1.0;
	if (surface_depth < 0.0 || surface_depth > 1.0) {
		inside = 0.0;
	}
	return vec4f(ndc_position.x * 0.5 + 0.5, 0.5 - ndc_position.y * 0.5, surface_depth, inside);
}
"#;

// Returns how stored depth changes across a receiver's plane per unit of shadow-map uv in one directional cascade, from
// the receiver's screen-space position derivatives.
pub(crate) const DIRECTIONAL_SHADOW_RECEIVER_GRADIENT_SOURCE: &str = r#"
directional_shadow_receiver_gradient: fn (
	shadow_view_projection: mat4f,
	world_space_position: vec3f,
	world_space_position_derivative_x: vec3f,
	world_space_position_derivative_y: vec3f
) -> vec2f {
	let clip_position: vec4f = shadow_view_projection * vec4f(
		world_space_position.x,
		world_space_position.y,
		world_space_position.z,
		1.0
	);
	let ndc_position: vec3f = vec3f(clip_position.x, clip_position.y, clip_position.z) / clip_position.w;
	return shadow_receiver_plane_depth_gradient(
		shadow_view_projection,
		clip_position,
		ndc_position,
		world_space_position_derivative_x,
		world_space_position_derivative_y
	);
}
"#;

// Searches one directional cascade for the occluders near a receiver. Returns how far above the receiver they are, in
// meters along the light, for sizing its penumbra: zero when none rises clearly above the receiver's plane, and -1
// when the cascade proves the receiver fully lit.
pub(crate) const DIRECTIONAL_SHADOW_OCCLUDER_DISTANCE_SOURCE: &str = r#"
directional_shadow_occluder_distance: fn (
	shadow_view_projection: mat4f,
	shadow_layer: u32,
	world_space_position: vec3f,
	world_space_position_derivative_x: vec3f,
	world_space_position_derivative_y: vec3f,
	shadow_map_extent: vec2u
) -> f32 {
	let receiver: vec4f = directional_shadow_receiver(shadow_view_projection, shadow_layer, world_space_position);
	let shadow_uv: vec2f = vec2f(receiver.x, receiver.y);
	if (receiver.w == 0.0 || directional_shadow_area_is_fully_lit(shadow_uv, receiver.z, shadow_layer, shadow_map_extent)) {
		return 0.0 - 1.0;
	}
	let shadow_map_extent_f: vec2f = vec2f(f32(shadow_map_extent.x), f32(shadow_map_extent.y));
	let depth_gradient: vec2f = directional_shadow_receiver_gradient(
		shadow_view_projection,
		world_space_position,
		world_space_position_derivative_x,
		world_space_position_derivative_y
	);
	let depth_per_meter: f32 = directional_shadow_depth_per_meter(shadow_view_projection);
	let blocker_depth: f32 = directional_shadow_blocker_depth(
		shadow_uv * shadow_map_extent_f,
		receiver.z,
		depth_gradient / shadow_map_extent_f,
		depth_per_meter,
		shadow_layer,
		shadow_map_extent
	);
	if (blocker_depth <= 0.0) {
		return 0.0;
	}
	return max(blocker_depth - receiver.z, 0.0) / depth_per_meter;
}
"#;

// Filters a directional receiver's shadow in one cascade with a penumbra `penumbra_meters` wide on each side of an
// occluder edge. When `check_fully_lit` is true, it first proves the receiver fully lit in this cascade when it can:
// callers set it when a coarser cascade found no occluders, because a thin occluder can vanish at that resolution.
pub(crate) const DIRECTIONAL_SHADOW_CASCADE_SOURCE: &str = r#"
sample_directional_shadow_cascade: fn (
	shadow_map: ArrayTexture2D,
	shadow_view_projection: mat4f,
	shadow_layer: u32,
	world_space_position: vec3f,
	world_space_position_derivative_x: vec3f,
	world_space_position_derivative_y: vec3f,
	shadow_map_extent: vec2u,
	penumbra_meters: f32,
	check_fully_lit: bool
) -> f32 {
	let receiver: vec4f = directional_shadow_receiver(shadow_view_projection, shadow_layer, world_space_position);
	let shadow_uv: vec2f = vec2f(receiver.x, receiver.y);
	if (receiver.w == 0.0) {
		return 1.0;
	}
	if (check_fully_lit && directional_shadow_area_is_fully_lit(shadow_uv, receiver.z, shadow_layer, shadow_map_extent)) {
		return 1.0;
	}
	let depth_gradient: vec2f = directional_shadow_receiver_gradient(
		shadow_view_projection,
		world_space_position,
		world_space_position_derivative_x,
		world_space_position_derivative_y
	);
	let penumbra_radius: f32 = penumbra_meters
		* directional_shadow_texels_per_meter(shadow_view_projection, f32(shadow_map_extent.x));
	return sample_directional_shadow_penumbra(
		shadow_map,
		shadow_uv,
		receiver.z,
		depth_gradient,
		shadow_layer,
		shadow_map_extent,
		penumbra_radius
	);
}
"#;

// Filters a directional receiver's shadow with a penumbra `penumbra_radius` texels wide on each side, from a sharp
// two-texel tent up to an eight-texel one. Between the tent spacings of one, two, and four texels it blends the two
// nearest, so the penumbra widens smoothly with the radius.
pub(crate) const DIRECTIONAL_SHADOW_PENUMBRA_SOURCE: &str = r#"
sample_directional_shadow_penumbra: fn (
	shadow_map: ArrayTexture2D,
	shadow_uv: vec2f,
	surface_depth: f32,
	receiver_plane_depth_gradient: vec2f,
	shadow_layer: u32,
	shadow_map_extent: vec2u,
	penumbra_radius: f32
) -> f32 {
	// A tent with spacing s reaches 2s texels, so the spacing that matches the radius is radius / 2.
	let level: f32 = clamp(log2(max(penumbra_radius, 2.0) * 0.5), 0.0, 2.0);
	let fine_spacing: f32 = 1.0;
	if (level >= 1.0) {
		fine_spacing = 2.0;
	}
	if (level >= 2.0) {
		fine_spacing = 4.0;
	}
	let coarse_blend: f32 = level - floor(level);
	let lit: f32 = sample_directional_shadow_tent(
		shadow_map, shadow_uv, surface_depth, receiver_plane_depth_gradient, shadow_layer, shadow_map_extent, fine_spacing
	);
	if (coarse_blend <= 0.0) {
		return lit;
	}
	let coarse_lit: f32 = sample_directional_shadow_tent(
		shadow_map, shadow_uv, surface_depth, receiver_plane_depth_gradient, shadow_layer, shadow_map_extent, fine_spacing * 2.0
	);
	return mix(lit, coarse_lit, coarse_blend);
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

// Cone maps use two positive Depth16Unorm steps as a reverse-Z comparison margin after receiver-plane correction.
pub(crate) const CONE_SHADOW_SOURCE: &str = r#"
sample_cone_shadow: fn (
	shadow_map: ArrayTexture2D,
	shadow_view_index: u32,
	shadow_layer: u32,
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
	return sample_shadow_tent(
		shadow_map,
		shadow_uv,
		surface_depth,
		receiver_plane_depth_gradient,
		shadow_layer,
		shadow_map_extent,
		1.0
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
	let pcf_rotation: vec2f16 = compute_shadow_rotation(point_shadow_texel_direction(center_direction));
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
	angular_radius_tangent: f32,
	world_space_position: vec3f,
	view_space_position: vec3f,
	world_space_position_derivative_x: vec3f,
	world_space_position_derivative_y: vec3f
) -> f32 {
	let depth_value: f32 = abs(view_space_position.z);
	// Surfaces past the last cascade lie beyond the shadow distance and receive no sun shadow.
	if (depth_value >= views.views[shadow_view3].far) {
		return 1.0;
	}
	// Descend only while the surface lies beyond a split. This avoids testing
	// a sentinel cascade index after every successful near-cascade match.
	let depth_cascade: u32 = 0;
	if (depth_value >= views.views[shadow_view0].far) {
		depth_cascade = 1;
		if (depth_value >= views.views[shadow_view1].far) {
			depth_cascade = 2;
			if (depth_value >= views.views[shadow_view2].far) {
				depth_cascade = 3;
			}
		}
	}
	let shadow_map_extent: vec2u = texture_size(shadow_map);
	let shadow_map_width: f32 = f32(shadow_map_extent.x);
	let texels_per_meter: vec4f = vec4f(
		directional_shadow_texels_per_meter(views.views[shadow_view0].view_projection, shadow_map_width),
		directional_shadow_texels_per_meter(views.views[shadow_view1].view_projection, shadow_map_width),
		directional_shadow_texels_per_meter(views.views[shadow_view2].view_projection, shadow_map_width),
		directional_shadow_texels_per_meter(views.views[shadow_view3].view_projection, shadow_map_width)
	);

	// Percentage-closer soft shadows: a disk of angular radius a leaves a penumbra reaching d * tan(a) to each side of
	// an occluder edge d meters above the receiver. The blocker search reliably covers twelve texels around the
	// receiver, so it runs in the first cascade where occluders up to two meters above the receiver cast penumbrae that
	// fit. A larger light therefore searches a coarser cascade.
	let search_cascade: u32 = directional_shadow_fitting_cascade(
		depth_cascade, 3, 2.0 * angular_radius_tangent, texels_per_meter, 12.0
	);
	let search_view: u32 = directional_shadow_cascade_view(search_cascade, shadow_view0, shadow_view1, shadow_view2, shadow_view3);
	let occluder_distance: f32 = directional_shadow_occluder_distance(
		views.views[search_view].view_projection,
		search_cascade,
		world_space_position,
		world_space_position_derivative_x,
		world_space_position_derivative_y,
		shadow_map_extent
	);
	if (occluder_distance < 0.0 && search_cascade == depth_cascade) {
		return 1.0;
	}

	// The filter reaches at most eight texels, so the penumbra is filtered in the finest cascade that holds it.
	let penumbra_meters: f32 = max(occluder_distance, 0.0) * angular_radius_tangent;
	let filter_cascade: u32 = directional_shadow_fitting_cascade(
		depth_cascade, search_cascade, penumbra_meters, texels_per_meter, 8.0
	);
	let filter_view: u32 = directional_shadow_cascade_view(filter_cascade, shadow_view0, shadow_view1, shadow_view2, shadow_view3);
	let lit: f32 = sample_directional_shadow_cascade(
		shadow_map,
		views.views[filter_view].view_projection,
		filter_cascade,
		world_space_position,
		world_space_position_derivative_x,
		world_space_position_derivative_y,
		shadow_map_extent,
		penumbra_meters,
		occluder_distance <= 0.0 && filter_cascade != search_cascade
	);
	if (filter_cascade == depth_cascade) {
		return lit;
	}

	// The two cascades sample the scene at different resolutions, so where a penumbra outgrows the finer cascade they
	// can disagree. Up to twelve of the finer cascade's texels, where its filter stays capped at eight, the result fades
	// from the finer cascade to this one, so the change of cascade leaves no seam.
	let finer_cascade: u32 = filter_cascade - 1;
	let finer_radius: f32 = penumbra_meters * directional_shadow_cascade_scale(finer_cascade, texels_per_meter);
	let coarse_share: f32 = clamp((finer_radius - 8.0) / 4.0, 0.0, 1.0);
	if (coarse_share >= 1.0) {
		return lit;
	}
	let finer_view: u32 = directional_shadow_cascade_view(finer_cascade, shadow_view0, shadow_view1, shadow_view2, shadow_view3);
	let finer_lit: f32 = sample_directional_shadow_cascade(
		shadow_map,
		views.views[finer_view].view_projection,
		finer_cascade,
		world_space_position,
		world_space_position_derivative_x,
		world_space_position_derivative_y,
		shadow_map_extent,
		penumbra_meters,
		false
	);
	return mix(finer_lit, lit, coarse_share);
}
"#;
