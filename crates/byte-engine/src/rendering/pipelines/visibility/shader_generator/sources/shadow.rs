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

// Proves every texel the directional blocker search and penumbra filter can read is no closer to the light than the
// receiver's center, so the receiver is fully lit. It covers the 4x4 block of eight-texel max-depth cells that
// `directional_shadow_blocker_depth` searches with four maximum-reduction samples, each on the corner shared by four
// cells.
pub(crate) const DIRECTIONAL_SHADOW_DEPTH_PROBE_SOURCE: &str = r#"
directional_shadow_area_is_fully_lit: fn (
	material_evaluation: MaterialEvaluation,
	shadow_uv: vec2f,
	surface_depth: f32,
	shadow_layer: u32,
	shadow_map_extent: vec2u
) -> bool {
	if (shadow_uv.x <= 0.0 || shadow_uv.x >= 1.0 || shadow_uv.y <= 0.0 || shadow_uv.y >= 1.0) {
		return false;
	}

	let shadow_texel_position: vec2f = shadow_uv * vec2f(f32(shadow_map_extent.x), f32(shadow_map_extent.y));
	// Every cascade layer packed at an eighth of its resolution, one below the other, makes the pyramid W/8 cells wide
	// and H/8 cells tall per layer.
	let cell_extent: vec2u = shadow_map_extent / vec2u(8, 8);
	let layer_offset: f32 = f32(shadow_layer * cell_extent.y);
	let pyramid_size: vec2u = texture_size(material_evaluation.directional_shadow_depth_pyramid);
	let pyramid_extent: vec2f = vec2f(f32(pyramid_size.x), f32(pyramid_size.y));
	let first_cell: vec2f = vec2f(
		floor(shadow_texel_position.x / 8.0 - 1.5),
		floor(shadow_texel_position.y / 8.0 - 1.5)
	);
	// Corners are clamped to the cascade, so no sample reads a neighboring cascade's cells.
	let maximum_corner: vec2f = vec2f(f32(cell_extent.x - 1), f32(cell_extent.y - 1));
	let corner0: vec2f = vec2f(
		clamp(first_cell.x + 1.0, 1.0, maximum_corner.x),
		clamp(first_cell.y + 1.0, 1.0, maximum_corner.y) + layer_offset
	) / pyramid_extent;
	let corner1: vec2f = vec2f(
		clamp(first_cell.x + 3.0, 1.0, maximum_corner.x),
		clamp(first_cell.y + 1.0, 1.0, maximum_corner.y) + layer_offset
	) / pyramid_extent;
	let corner2: vec2f = vec2f(
		clamp(first_cell.x + 1.0, 1.0, maximum_corner.x),
		clamp(first_cell.y + 3.0, 1.0, maximum_corner.y) + layer_offset
	) / pyramid_extent;
	let corner3: vec2f = vec2f(
		clamp(first_cell.x + 3.0, 1.0, maximum_corner.x),
		clamp(first_cell.y + 3.0, 1.0, maximum_corner.y) + layer_offset
	) / pyramid_extent;
	// Issue the independent probes before reducing them, so one fetch does not gate the next.
	let probe0: f32 = downsample_max(material_evaluation.directional_shadow_depth_pyramid, corner0, 0.0);
	let probe1: f32 = downsample_max(material_evaluation.directional_shadow_depth_pyramid, corner1, 0.0);
	let probe2: f32 = downsample_max(material_evaluation.directional_shadow_depth_pyramid, corner2, 0.0);
	let probe3: f32 = downsample_max(material_evaluation.directional_shadow_depth_pyramid, corner3, 0.0);
	let block_maximum: f32 = max(max(max(max(0.0, probe0), probe1), probe2), probe3);
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
	material_evaluation: MaterialEvaluation,
	shadow_texel_position: vec2f,
	surface_depth: f32,
	depth_gradient_per_texel: vec2f,
	depth_per_meter: f32,
	shadow_layer: u32,
	shadow_map_extent: vec2u
) -> f32 {
	// Every cascade layer packed at an eighth of its resolution, one below the other, makes the pyramid W/8 cells wide
	// and H/8 cells tall per layer.
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
					material_evaluation.directional_shadow_depth_pyramid,
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
	match cascade {
		1 => return view1,
		2 => return view2,
		3 => return view3,
		_ => return view0,
	}
}
"#;

// Returns one cascade's texels per meter from the four cascades' scales, cascade zero in x through cascade three in w.
pub(crate) const DIRECTIONAL_SHADOW_CASCADE_SCALE_SOURCE: &str = r#"
directional_shadow_cascade_scale: fn (cascade: u32, texels_per_meter: vec4f) -> f32 {
	match cascade {
		1 => return texels_per_meter.y,
		2 => return texels_per_meter.z,
		3 => return texels_per_meter.w,
		_ => return texels_per_meter.x,
	}
}
"#;

// Returns the first cascade from `first_cascade` through `last_cascade` in which a distance of `reach_meters` spans at
// most `limit_texels` texels, or `last_cascade` when none does. `texels_per_meter` holds each cascade's scale, from
// cascade zero in x to cascade three in w. Directional shadows use it to search for occluders in a cascade whose search
// window is wide enough.
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

// Returns whether a directional cascade can shadow a world-space receiver: the receiver lies inside the cascade's
// square, at least ten texels from its edge, so the penumbra filter's reach of eight texels stays inside the map when
// the receiver moves off its surface, and inside the cascade's depth range, where the map knows what lies above it.
// Each cascade fitted to the camera's opaque surfaces holds those of its own slice, but often not those of another
// slice, nor a surface drawn later, such as a transparent one.
pub(crate) const DIRECTIONAL_SHADOW_CASCADE_HOLDS_SOURCE: &str = r#"
directional_shadow_cascade_holds: fn (shadow_view_projection: mat4f, world_space_position: vec3f, shadow_map_width: f32) -> bool {
	let clip_position: vec4f = shadow_view_projection * vec4f(
		world_space_position.x,
		world_space_position.y,
		world_space_position.z,
		1.0
	);
	// A texel spans two over the width in normalized device units.
	let limit: f32 = 1.0 - 20.0 / shadow_map_width;
	// A fitted cascade ends at the receiver of its slice farthest from the light, so a receiver of a nearer slice can
	// lie past it, where its samples would read as lit. One depth step of slack keeps the slice's own farthest
	// receivers, which rounding can put just past the end and the receiver's depth margin brings back.
	return abs(clip_position.x) <= limit && abs(clip_position.y) <= limit
		&& clip_position.z >= 0.0 - 1.0 / 65535.0 && clip_position.z <= 1.0;
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

// Returns how stored depth changes across a receiver's plane per unit of shadow-map uv in one directional cascade.
// `surface_vector0` and `surface_vector1` are any two world-space vectors along the plane that are not parallel, such
// as the edges of the receiver's triangle or its screen-space position derivatives.
pub(crate) const DIRECTIONAL_SHADOW_RECEIVER_GRADIENT_SOURCE: &str = r#"
directional_shadow_receiver_gradient: fn (
	shadow_view_projection: mat4f,
	surface_vector0: vec3f,
	surface_vector1: vec3f
) -> vec2f {
	// Directional cascades are orthographic, so the vectors need no perspective correction or receiver position. The
	// projected plane normal gives its depth slope directly; shadow UV halves X and reverses Y.
	let projected0: vec4f = shadow_view_projection * vec4f(surface_vector0.x, surface_vector0.y, surface_vector0.z, 0.0);
	let projected1: vec4f = shadow_view_projection * vec4f(surface_vector1.x, surface_vector1.y, surface_vector1.z, 0.0);
	let normal: vec3f = cross(vec3f(projected0.x, projected0.y, projected0.z), vec3f(projected1.x, projected1.y, projected1.z));
	// The gradient is (-2 x, 2 y) / z of that normal, which grows without bound as the light turns parallel to the plane.
	// Taps up to eight texels away would then compare depths far off any real surface, and the plane would jump to
	// flat where z rounds to zero. So the plane's slope is capped at 20 meters of depth along the light per meter
	// across the map, a plane the light meets about 3 degrees from edge-on, keeping its direction. Shading gives such a
	// receiver almost no light, so the cap hides nothing visible. One unit of uv spans two normalized device units, so
	// in uv the cap is 40 times the stored depth per meter over the normalized device x per meter: the lengths of the
	// projection's third and first rows.
	let world_x: vec4f = shadow_view_projection * vec4f(1.0, 0.0, 0.0, 0.0);
	let world_y: vec4f = shadow_view_projection * vec4f(0.0, 1.0, 0.0, 0.0);
	let world_z: vec4f = shadow_view_projection * vec4f(0.0, 0.0, 1.0, 0.0);
	let maximum_gradient: f32 = 40.0 * length(vec3f(world_x.z, world_y.z, world_z.z))
		/ length(vec3f(world_x.x, world_y.x, world_z.x));
	let rise: vec2f = vec2f(0.0 - 2.0 * normal.x, 2.0 * normal.y);
	let denominator: f32 = max(abs(normal.z), length(rise) / maximum_gradient);
	// A triangle without area has no plane.
	if (denominator <= 0.0) {
		return vec2f(0.0, 0.0);
	}
	if (normal.z < 0.0) {
		denominator = 0.0 - denominator;
	}
	return rise / denominator;
}
"#;

// Searches one directional cascade for the occluders near a receiver. Returns how far above the receiver they are, in
// meters along the light, for sizing its penumbra: zero when none rises clearly above the receiver's plane, and -1
// when the cascade proves the receiver fully lit. `shadow_layer` is the cascade's shadow-map layer; every sun owns four
// layers, so the layer modulo four is the cascade's index, which sizes the receiver's depth margin.
pub(crate) const DIRECTIONAL_SHADOW_OCCLUDER_DISTANCE_SOURCE: &str = r#"
directional_shadow_occluder_distance: fn (
	material_evaluation: MaterialEvaluation,
	shadow_view_projection: mat4f,
	shadow_layer: u32,
	world_space_position: vec3f,
	world_space_position_derivative_x: vec3f,
	world_space_position_derivative_y: vec3f,
	shadow_map_extent: vec2u
) -> f32 {
	let receiver: vec4f = directional_shadow_receiver(shadow_view_projection, shadow_layer % 4, world_space_position);
	let shadow_uv: vec2f = vec2f(receiver.x, receiver.y);
	if (receiver.w == 0.0 || directional_shadow_area_is_fully_lit(material_evaluation, shadow_uv, receiver.z, shadow_layer, shadow_map_extent)) {
		return 0.0 - 1.0;
	}
	let shadow_map_extent_f: vec2f = vec2f(f32(shadow_map_extent.x), f32(shadow_map_extent.y));
	let depth_gradient: vec2f = directional_shadow_receiver_gradient(
		shadow_view_projection,
		world_space_position_derivative_x,
		world_space_position_derivative_y
	);
	let depth_per_meter: f32 = directional_shadow_depth_per_meter(shadow_view_projection);
	let blocker_depth: f32 = directional_shadow_blocker_depth(
		material_evaluation,
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
// `shadow_layer` is the cascade's shadow-map layer, as in `directional_shadow_occluder_distance`.
pub(crate) const DIRECTIONAL_SHADOW_CASCADE_SOURCE: &str = r#"
sample_directional_shadow_cascade: fn (
	material_evaluation: MaterialEvaluation,
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
	let receiver: vec4f = directional_shadow_receiver(shadow_view_projection, shadow_layer % 4, world_space_position);
	let shadow_uv: vec2f = vec2f(receiver.x, receiver.y);
	if (receiver.w == 0.0) {
		return 1.0;
	}
	if (check_fully_lit && directional_shadow_area_is_fully_lit(material_evaluation, shadow_uv, receiver.z, shadow_layer, shadow_map_extent)) {
		return 1.0;
	}
	let depth_gradient: vec2f = directional_shadow_receiver_gradient(
		shadow_view_projection,
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

// Filters a directional receiver's shadow with a penumbra `penumbra_radius` texels wide on each side of an occluder
// edge: a tent of that radius, from two to eight texels, over every texel whose center lies within it on both axes. Each
// texel weighs the radius less its center's distance from the receiver on each axis, so the penumbra widens smoothly
// with the radius and changes smoothly as the receiver moves. Reading every texel under the tent, instead of spreading
// a few taps apart, keeps each step of the map's outline a small ripple inside the ramp: a sparse tent draws the outline
// on a grid as coarse as its spacing.
//
// It reads the 2r x 2r texels around the receiver, r being the radius rounded up, one fetch a texel, so its cost grows
// with the penumbra: callers keep the radius at four or less where a coarser cascade holds the receiver. The
// whole area must lie inside the map: callers filter only in a cascade that holds the receiver, ten texels inside its
// edge. The sun visibility resolve's copy gathers the same texels in 4x4 blocks; it stays out of this copy, whose
// registers every material pays for, because gathers there slowed opaque material evaluation.
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
	let shadow_texel_position: vec2f = vec2f(
		shadow_uv.x * f32(shadow_map_extent.x),
		shadow_uv.y * f32(shadow_map_extent.y)
	);
	let radius: f32 = clamp(penumbra_radius, 2.0, 8.0);
	// Texels per axis: 2r of them hold every center within r texels of the receiver.
	let texels: u32 = u32(ceil(radius)) * 2;
	let reach: f32 = f32(texels / 2);
	let first_texel: vec2f = vec2f(
		floor(shadow_texel_position.x - reach + 0.5),
		floor(shadow_texel_position.y - reach + 0.5)
	);
	// The first texel's center lies between r and r - 1 texels before the receiver on each axis.
	let first_offset: vec2f = first_texel + vec2f(0.5, 0.5) - shadow_texel_position;
	let first_texel_index: vec2u = vec2u(u32(first_texel.x), u32(first_texel.y));
	// The map holds each texel's depth at its center, so the receiver plane is compared there: at the first texel's
	// center, then one gradient step per texel to the right or down.
	let step_x: f32 = receiver_plane_depth_gradient.x / f32(shadow_map_extent.x);
	let step_y: f32 = receiver_plane_depth_gradient.y / f32(shadow_map_extent.y);
	let row_depth: f32 = surface_depth + step_x * first_offset.x + step_y * first_offset.y;
	let lit: f32 = 0.0;
	// The weights sum to a constant only at whole radii, so the result divides by their sum on each axis. Every row adds
	// the same column weights, so each row starts the column sum over and the last row's is kept.
	let column_weight_sum: f32 = 0.0;
	let row_weight_sum: f32 = 0.0;
	for (let row: u32 = 0; row < texels; row = row + 1) {
		let weight_y: f32 = max(radius - abs(first_offset.y + f32(row)), 0.0);
		row_weight_sum = row_weight_sum + weight_y;
		column_weight_sum = 0.0;
		let row_lit: f32 = 0.0;
		let tap_depth: f32 = row_depth;
		for (let column: u32 = 0; column < texels; column = column + 1) {
			let weight_x: f32 = max(radius - abs(first_offset.x + f32(column)), 0.0);
			column_weight_sum = column_weight_sum + weight_x;
			let closest_depth: f32 = fetch(shadow_map, first_texel_index + vec2u(column, row), shadow_layer).x;
			row_lit = row_lit + weight_x * step(closest_depth, tap_depth);
			tap_depth = tap_depth + step_x;
		}
		lit = lit + weight_y * row_lit;
		row_depth = row_depth + step_y;
	}
	return lit / (column_weight_sum * row_weight_sum);
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

// Filters a sun's shadow at a transparent receiver across the cascades in views `shadow_view0` through
// `shadow_view3`. Directional view `v` is drawn into shadow-map layer `v - 1`, so cascade `c` lies in layer
// `shadow_view0 - 1 + c`. `view_depth` is the receiver's depth along the camera's view axis, which picks its cascade; the
// material shader passes the clip w it already keeps, so no view-space position stays live through its light loop.
// `normal_offset` is how far the receiver moves off its surface, in texels, as the sun visibility resolve moves it:
// each cascade moves it by `normal_offset` over its texels per meter.
pub(crate) const DIRECTIONAL_SHADOW_SOURCE: &str = r#"
sample_directional_shadow: fn (
	scene: Scene,
	material_evaluation: MaterialEvaluation,
	shadow_map: ArrayTexture2D,
	shadow_view0: u32,
	shadow_view1: u32,
	shadow_view2: u32,
	shadow_view3: u32,
	angular_radius_tangent: f32,
	world_space_position: vec3f,
	view_depth: f32,
	world_space_position_derivative_x: vec3f,
	world_space_position_derivative_y: vec3f,
	normal_offset: vec3f
) -> f32 {
	// Surfaces past the last cascade lie beyond the shadow distance and receive no sun shadow.
	if (view_depth >= scene.views.views[shadow_view3].far) {
		return 1.0;
	}
	// Descend only while the surface lies beyond a split. This avoids testing
	// a sentinel cascade index after every successful near-cascade match.
	let depth_cascade: u32 = 0;
	if (view_depth >= scene.views.views[shadow_view0].far) {
		depth_cascade = 1;
		if (view_depth >= scene.views.views[shadow_view1].far) {
			depth_cascade = 2;
			if (view_depth >= scene.views.views[shadow_view2].far) {
				depth_cascade = 3;
			}
		}
	}
	let shadow_map_extent: vec2u = texture_size(shadow_map);
	let shadow_map_width: f32 = f32(shadow_map_extent.x);
	let first_layer: u32 = shadow_view0 - 1;
	// A surface its cascade doesn't hold moves to the first coarser cascade that does, and is lit when none does.
	let holding_cascade: u32 = 4;
	for (let cascade: u32 = depth_cascade; cascade < 4 && holding_cascade == 4; cascade = cascade + 1) {
		let cascade_view: u32 = directional_shadow_cascade_view(cascade, shadow_view0, shadow_view1, shadow_view2, shadow_view3);
		if (directional_shadow_cascade_holds(scene.views.views[cascade_view].view_projection, world_space_position, shadow_map_width)) {
			holding_cascade = cascade;
		}
	}
	if (holding_cascade == 4) {
		return 1.0;
	}
	depth_cascade = holding_cascade;
	let texels_per_meter: vec4f = vec4f(
		directional_shadow_texels_per_meter(scene.views.views[shadow_view0].view_projection, shadow_map_width),
		directional_shadow_texels_per_meter(scene.views.views[shadow_view1].view_projection, shadow_map_width),
		directional_shadow_texels_per_meter(scene.views.views[shadow_view2].view_projection, shadow_map_width),
		directional_shadow_texels_per_meter(scene.views.views[shadow_view3].view_projection, shadow_map_width)
	);

	// Percentage-closer soft shadows: a disk of angular radius a leaves a penumbra reaching d * tan(a) to each side of
	// an occluder edge d meters above the receiver. The blocker search reliably covers twelve texels around the
	// receiver, so it runs in the first cascade where occluders up to two meters above the receiver cast penumbrae that
	// fit. A larger light therefore searches a coarser cascade.
	let search_cascade: u32 = directional_shadow_fitting_cascade(
		depth_cascade, 3, 2.0 * angular_radius_tangent, texels_per_meter, 12.0
	);
	let search_view: u32 = directional_shadow_cascade_view(search_cascade, shadow_view0, shadow_view1, shadow_view2, shadow_view3);
	// A coarser cascade fitted to its own surfaces may not hold this one; the search then stays in the surface's
	// cascade, where the penumbra it finds is cut off at the search's reach.
	if (directional_shadow_cascade_holds(scene.views.views[search_view].view_projection, world_space_position, shadow_map_width) == false) {
		search_cascade = depth_cascade;
		search_view = directional_shadow_cascade_view(search_cascade, shadow_view0, shadow_view1, shadow_view2, shadow_view3);
	}
	let occluder_distance: f32 = directional_shadow_occluder_distance(
		material_evaluation,
		scene.views.views[search_view].view_projection,
		first_layer + search_cascade,
		world_space_position + normal_offset / directional_shadow_cascade_scale(search_cascade, texels_per_meter),
		world_space_position_derivative_x,
		world_space_position_derivative_y,
		shadow_map_extent
	);
	if (occluder_distance < 0.0 && search_cascade == depth_cascade) {
		return 1.0;
	}

	// The penumbra cascade and the fade between two cascades follow the sun visibility resolve's, in
	// `sun-visibility.besl`, which says why.
	let penumbra_meters: f32 = max(occluder_distance, 0.0) * angular_radius_tangent;
	let fitted_penumbra_meters: f32 = max(penumbra_meters, 0.0001);
	let last_filter_cascade: u32 = min(search_cascade + 1, 3);
	let narrow_cascade: u32 = 4;
	let narrow_misfit: f32 = 100000000.0;
	let wide_cascade: u32 = 4;
	let wide_misfit: f32 = 100000000.0;
	for (let cascade: u32 = depth_cascade; cascade <= last_filter_cascade; cascade = cascade + 1) {
		let cascade_view: u32 = directional_shadow_cascade_view(cascade, shadow_view0, shadow_view1, shadow_view2, shadow_view3);
		if (cascade == depth_cascade
			|| cascade == search_cascade
			|| directional_shadow_cascade_holds(scene.views.views[cascade_view].view_projection, world_space_position, shadow_map_width)) {
			let width: f32 = fitted_penumbra_meters * directional_shadow_cascade_scale(cascade, texels_per_meter);
			let misfit: f32 = max(2.0 / width, width / 6.0);
			if (width * width < 12.0) {
				if (misfit < narrow_misfit) {
					narrow_cascade = cascade;
					narrow_misfit = misfit;
				}
			} else if (misfit < wide_misfit) {
				wide_cascade = cascade;
				wide_misfit = misfit;
			}
		}
	}
	let filter_cascade: u32 = wide_cascade;
	let filter_misfit: f32 = wide_misfit;
	let partner_cascade: u32 = narrow_cascade;
	let partner_misfit: f32 = narrow_misfit;
	if (narrow_misfit < wide_misfit || (narrow_misfit == wide_misfit && narrow_cascade < wide_cascade)) {
		filter_cascade = narrow_cascade;
		filter_misfit = narrow_misfit;
		partner_cascade = wide_cascade;
		partner_misfit = wide_misfit;
	}
	let filter_share: f32 = 1.0;
	let sampled_cascades: u32 = 1;
	if (partner_cascade != 4) {
		filter_share = min(partner_misfit / filter_misfit - 0.5, 1.0);
		if (filter_share < 1.0) {
			sampled_cascades = 2;
		}
	}
	let lit: f32 = 0.0;
	for (let sampled: u32 = 0; sampled < sampled_cascades; sampled = sampled + 1) {
		let cascade: u32 = filter_cascade;
		let share: f32 = filter_share;
		if (sampled == 1) {
			cascade = partner_cascade;
			share = 1.0 - filter_share;
		}
		let cascade_view: u32 = directional_shadow_cascade_view(cascade, shadow_view0, shadow_view1, shadow_view2, shadow_view3);
		lit = lit + share * sample_directional_shadow_cascade(
			material_evaluation,
			shadow_map,
			scene.views.views[cascade_view].view_projection,
			first_layer + cascade,
			world_space_position + normal_offset / directional_shadow_cascade_scale(cascade, texels_per_meter),
			world_space_position_derivative_x,
			world_space_position_derivative_y,
			shadow_map_extent,
			penumbra_meters,
			occluder_distance <= 0.0 && cascade != search_cascade
		);
	}
	return lit;
}
"#;
