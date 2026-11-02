// Screen-space indirect diffuse helpers. Opaque material evaluation reconstructs each pixel's indirect diffuse light
// from the half-resolution SSGI history with the pixel's own surface position and normal, so SSGI writes no
// full-resolution image.
//
// The helpers read the `ssgi_view`, `ssgi_history`, and `ssgi_normals` bindings that
// `screen_space_indirect_diffuse_scope` declares, and the shared `depth_pyramid` binding. Each helper only calls
// helpers declared above it.

// Decodes a normal the SSGI trace stored as an octahedral pair in RG, or returns zero for the pair (0, 0), which marks
// a texel without a normal. That pair encodes +z, which points away from the camera and so is never a stored normal.
// It decodes at half precision, which the normal agreement test tolerates, so the upsample keeps its four pairs in
// half-width registers. `decode_stored_normal` in `layouts/screen_trace.besl` is the same decoder at full precision;
// keep both in sync with `encode_stored_normal` there.
pub(crate) const SSGI_STORED_NORMAL_SOURCE: &str = r#"
ssgi_stored_normal: fn (stored: vec2f16) -> vec3f16 {
	if (stored.x == 0.0 && stored.y == 0.0) {
		return vec3f16(0.0, 0.0, 0.0);
	}
	let z: f16 = f16(1.0) - abs(stored.x) - abs(stored.y);
	let fold: f16 = max(-z, f16(0.0));
	// The positive direction at zero matches the trace's encoder.
	let normal: vec3f16 = vec3f16(
		stored.x - (if (stored.x >= 0.0) { fold } else { -fold }),
		stored.y - (if (stored.y >= 0.0) { fold } else { -fold }),
		z
	);
	return normalize(normal);
}
"#;

// Returns how much a history texel belongs to the pixel's surface: one on the pixel's tangent plane with a similar
// normal, falling to zero for another surface. Depth alone cannot separate surfaces that touch: where a boot stands
// on a floor, both have the same depth, but the boot's normal and its distance from the floor's plane differ. Without
// a texel normal, it compares depth only.
//
// `inverse_depth` is 1 / z of the pixel's view-space position, `scaled_normal` its normal times that, and
// `plane_offset` its tangent plane's offset times that, so a texel's distance from the plane, relative to the pixel's
// depth, takes one dot product and a subtraction.
pub(crate) const SSGI_SURFACE_WEIGHT_SOURCE: &str = r#"
ssgi_surface_weight: fn (
	normal: vec3f16,
	scaled_normal: vec3f,
	plane_offset: f32,
	inverse_depth: f32,
	tap_normal: vec3f16,
	tap_position: vec3f
) -> f32 {
	let has_normal: bool = dot(tap_normal, tap_normal) != 0.0;
	// A 2% relative-depth sigma gives a Gaussian inverse variance of 1,250.
	let relative_delta: f32 = tap_position.z * inverse_depth - 1.0;
	// A 1% sigma, relative to the pixel's depth, for the texel's distance from the pixel's tangent plane.
	let plane_distance: f32 = dot(scaled_normal, tap_position) - plane_offset;
	// Texels whose normal differs from the pixel's by more than about 45 degrees lie on another surface. Up to about
	// 25 degrees, as across a curved limb, they count fully.
	let normal_agreement: f16 = clamp((dot(normal, tap_normal) - f16(0.7)) * f16(5.0), f16(0.0), f16(1.0));
	// Both cases share one exponential instead of each taking its own.
	let exponent: f32 = if (has_normal) { plane_distance * plane_distance * 10000.0 } else { relative_delta * relative_delta * 1250.0 };
	return (if (has_normal) { f32(normal_agreement) } else { 1.0 }) * exp(-exponent);
}
"#;

// Returns the half-resolution texel position a full-resolution `pixel` of an `extent` image reads its indirect
// diffuse taps around, in a history of `source_extent`: odd full extents do not divide exactly by two, so texel
// centers are matched by normalized screen position. Its floor is the first of the four taps.
pub(crate) const INDIRECT_DIFFUSE_SOURCE_POSITION_SOURCE: &str = r#"
indirect_diffuse_source_position: fn (pixel: vec2u, extent: vec2u, source_extent: vec2u) -> vec2f {
	return vec2f(
		clamp((f32(pixel.x) + 0.5) * f32(source_extent.x) / f32(extent.x) - 0.5, 0.0, f32(source_extent.x - 1)),
		clamp((f32(pixel.y) + 0.5) * f32(source_extent.y) / f32(extent.y) - 0.5, 0.0, f32(source_extent.y - 1))
	);
}
"#;

// Weighs the four history texels around a pixel, which the caller read into `depths`, `stored_normals`, and `lights`
// in the order x then y from `base`, into the pixel's pre-exposed indirect diffuse light: bilinearly, and by how well
// each lies on the pixel's surface, so light does not bleed across edges. RGB is the light of rays that hit on-screen
// geometry and alpha the fraction of rays that hit. Without a matching texel, every ray counts as missed, so the
// environment lights the pixel instead of a neighbor, unless `nearest_fallback` is set: then the pixel takes the light
// and hit fraction of the tap nearest to it in depth, the usual nearest-depth upsample. A surface thinner than a
// history texel, such as a rod or a bevel, has no texel of its own, and its neighbors' light stands in for it better
// than none.
//
// `position` and `normal` are the pixel's view-space surface position and geometric normal, in the space the SSGI
// images use.
pub(crate) const WEIGH_SCREEN_SPACE_INDIRECT_DIFFUSE_SOURCE: &str = r#"
weigh_screen_space_indirect_diffuse: fn (
	material_evaluation: MaterialEvaluation,
	base: vec2f,
	fraction: vec2f,
	source_extent: vec2u,
	depths: f32[4],
	stored_normals: vec2f16[4],
	lights: vec4f16[4],
	position: vec3f,
	normal: vec3f,
	nearest_fallback: bool
) -> vec4f {
	// The pixel's terms of the plane test, computed once for the four taps.
	let inverse_depth: f32 = 1.0 / position.z;
	let scaled_normal: vec3f = normal * inverse_depth;
	let plane_offset: f32 = dot(normal, position) * inverse_depth;
	let normal16: vec3f16 = vec3f16(normal);
	let sum: vec4f = vec4f(0.0, 0.0, 0.0, 0.0);
	let total_weight: f32 = 0.0;
	for (let tap: u32 = 0; tap < 4; tap = tap + 1) {
		let z: f32 = depths[tap];
		// The texel again, in float so it is rebuilt from `base` here rather than kept from the reads.
		let tap_texel: vec2f = vec2f(
			min(base.x + f32(tap % 2), f32(source_extent.x - 1)),
			min(base.y + f32(tap / 2), f32(source_extent.y - 1))
		);
		let ray: vec2f = tap_texel * material_evaluation.ssgi_view.pixel_to_ray_mul + material_evaluation.ssgi_view.pixel_to_ray_add;
		let tap_normal: vec3f16 = ssgi_stored_normal(stored_normals[tap]);
		let bilinear: f32 = mix(1.0 - fraction.x, fraction.x, f32(tap % 2)) * mix(1.0 - fraction.y, fraction.y, f32(tap / 2));
		// A small floor keeps a texel that lands exactly on the far side of the bilinear footprint eligible. A texel
		// without a surface has zero depth and gets no weight; a factor instead of a branch keeps the four taps in one
		// straight run of code that the compiler can interleave with the reads.
		let weight: f32 = step(0.000001, z) * max(bilinear, 0.001) * ssgi_surface_weight(
			normal16,
			scaled_normal,
			plane_offset,
			inverse_depth,
			tap_normal,
			vec3f(ray.x * z, ray.y * z, z)
		);
		// The light is weighted at the half precision its image holds and summed at full precision, so a bright sum
		// does not overflow.
		sum = sum + vec4f(lights[tap] * f16(weight));
		total_weight = total_weight + weight;
	}
	if (total_weight > 0.00001) {
		return sum * (1.0 / total_weight);
	}
	let nearest: vec4f = vec4f(0.0, 0.0, 0.0, 0.0);
	if (nearest_fallback) {
		// The relative depth difference of the nearest tap so far. A texel without a surface has zero depth and is
		// never chosen.
		let nearest_delta: f32 = 0.0;
		let found: bool = false;
		for (let tap: u32 = 0; tap < 4; tap = tap + 1) {
			let delta: f32 = abs(depths[tap] * inverse_depth - 1.0);
			if (depths[tap] > 0.0 && (!found || delta < nearest_delta)) {
				nearest = vec4f(lights[tap]);
				nearest_delta = delta;
				found = true;
			}
		}
	}
	return nearest;
}
"#;

// Reconstructs a pixel's pre-exposed indirect diffuse light from the half-resolution SSGI history: the four history
// texels around the pixel, weighed by `weigh_screen_space_indirect_diffuse`.
//
// `pixel` is the full-resolution pixel of an `extent` image. `position` and `normal` are its view-space surface
// position and smooth normal, in the space the SSGI images use. Ray-traced light retries with `geometric_normal` if
// the smooth normal rejects every tap, then keeps the nearest-depth tap for a surface thinner than a history texel.
// A smooth normal can reject a covered facet, while a thin bevel may have no history texel of its own. The retry
// prevents either case from replacing ray-traced visibility with unoccluded environment light. Both attempts reuse
// the same reads and retain the normal and tangent-plane tests.
pub(crate) const SAMPLE_SCREEN_SPACE_INDIRECT_DIFFUSE_SOURCE: &str = r#"
sample_screen_space_indirect_diffuse: fn (
	material_evaluation: MaterialEvaluation,
	pixel: vec2u,
	extent: vec2u,
	position: vec3f,
	normal: vec3f,
	geometric_normal: vec3f,
	ray_traced: bool
) -> vec4f {
	let source_extent: vec2u = texture_size(material_evaluation.ssgi_history);
	let source_position: vec2f = indirect_diffuse_source_position(pixel, extent, source_extent);
	let base: vec2f = vec2f(floor(source_position.x), floor(source_position.y));
	// Every texel is read before any weight is computed, so the gather and eight fetches can overlap instead of
	// waiting on each other: the material shader's register footprint leaves few other threads to hide their latency. Each tap keeps
	// only what its fetches returned, the depth at full precision and the stored normal pair and light at the half
	// precision their 16-bit images hold, and derives its position and decoded normal when it is weighted, so the
	// fetch window holds as little as possible. Keeping the pairs and the light at half precision took the window's
	// register peak, the highest of the opaque material shader without local lights, from 86 to 74, and material
	// evaluation 0.8 to 1.4 % faster on the Sponza hall view (2026-10-06).
	// One gather reads the same clamped 2x2 depth footprint as the four independent texel fetches. The corner
	// lies between the texel centers; gather returns bottom-left, bottom-right, top-right, top-left.
	let depth_quad: vec4f = gather(material_evaluation.depth_pyramid, (base + vec2f(1.0, 1.0)) / vec2f(f32(source_extent.x), f32(source_extent.y)));
	let depths: f32[4] = f32[4](depth_quad.w, depth_quad.z, depth_quad.x, depth_quad.y);
	let zero_pair: vec2f16 = vec2f16(0.0, 0.0);
	let stored_normals: vec2f16[4] = vec2f16[4](zero_pair, zero_pair, zero_pair, zero_pair);
	let zero_light: vec4f16 = vec4f16(0.0, 0.0, 0.0, 0.0);
	let lights: vec4f16[4] = vec4f16[4](zero_light, zero_light, zero_light, zero_light);
	for (let tap: u32 = 0; tap < 4; tap = tap + 1) {
		let texel: vec2u = vec2u(
			min(u32(base.x) + tap % 2, source_extent.x - 1),
			min(u32(base.y) + tap / 2, source_extent.y - 1)
		);
		let stored: vec4f = fetch(material_evaluation.ssgi_normals, texel);
		stored_normals[tap] = vec2f16(vec2f(stored.x, stored.y));
		lights[tap] = vec4f16(fetch(material_evaluation.ssgi_history, texel));
	}
	let indirect: vec4f = weigh_screen_space_indirect_diffuse(
		material_evaluation,
		base,
		source_position - base,
		source_extent,
		depths,
		stored_normals,
		lights,
		position,
		normal,
		false
	);
	if (!ray_traced || dot(indirect, indirect) != 0.0) {
		return indirect;
	}
	// Retry the same texels against the triangle plane, with the nearest-depth fallback for a thin surface.
	// Keeping the original reads avoids a second texture window without changing either reconstruction's tests.
	return weigh_screen_space_indirect_diffuse(
		material_evaluation,
		base, source_position - base, source_extent, depths, stored_normals, lights, position, geometric_normal, true
	);
}
"#;

// Returns the first half-resolution texel the indirect diffuse taps of `pixel`'s 8 x 8 tile read: the first tap of
// the tile's first pixel. Uber material evaluation's tiles start at multiples of eight, so the tile's first pixel is
// `pixel` rounded down to one. The tile's other taps lie at most five texels further on along each axis, because
// the history has half the resolution, rounded up.
pub(crate) const INDIRECT_DIFFUSE_WINDOW_ORIGIN_SOURCE: &str = r#"
indirect_diffuse_window_origin: fn (pixel: vec2u, extent: vec2u, source_extent: vec2u) -> vec2u {
	let tile_origin: vec2u = vec2u(pixel.x - pixel.x % 8, pixel.y - pixel.y % 8);
	let source_position: vec2f = indirect_diffuse_source_position(tile_origin, extent, source_extent);
	return vec2u(u32(source_position.x), u32(source_position.y));
}
"#;

// The uber shaders' `sample_screen_space_indirect_diffuse`, in place of `SAMPLE_SCREEN_SPACE_INDIRECT_DIFFUSE_SOURCE`.
// It reconstructs a pixel's pre-exposed indirect diffuse light from the window of history texels its uber material
// evaluation tile loaded into workgroup memory instead of from the images. The window holds the texels at the same
// clamped positions the images give the taps, so the result is the same.
pub(crate) const SAMPLE_TILE_INDIRECT_DIFFUSE_SOURCE: &str = r#"
sample_screen_space_indirect_diffuse: fn (
	material_evaluation: MaterialEvaluation,
	pixel: vec2u,
	extent: vec2u,
	position: vec3f,
	normal: vec3f,
	geometric_normal: vec3f,
	ray_traced: bool
) -> vec4f {
	let source_extent: vec2u = texture_size(material_evaluation.ssgi_history);
	let source_position: vec2f = indirect_diffuse_source_position(pixel, extent, source_extent);
	let base: vec2f = vec2f(floor(source_position.x), floor(source_position.y));
	let window_origin: vec2u = indirect_diffuse_window_origin(pixel, extent, source_extent);
	let depths: f32[4] = f32[4](0.0, 0.0, 0.0, 0.0);
	let zero_pair: vec2f16 = vec2f16(0.0, 0.0);
	let stored_normals: vec2f16[4] = vec2f16[4](zero_pair, zero_pair, zero_pair, zero_pair);
	let zero_light: vec4f16 = vec4f16(0.0, 0.0, 0.0, 0.0);
	let lights: vec4f16[4] = vec4f16[4](zero_light, zero_light, zero_light, zero_light);
	for (let tap: u32 = 0; tap < 4; tap = tap + 1) {
		// The clamp keeps a tap that rounding moved one texel off the window inside the array.
		let window_texel: vec2u = vec2u(
			min(min(u32(base.x) + tap % 2, source_extent.x - 1) - window_origin.x, 5),
			min(min(u32(base.y) + tap / 2, source_extent.y - 1) - window_origin.y, 5)
		);
		let window_index: u32 = window_texel.y * 6 + window_texel.x;
		depths[tap] = tile_indirect_depths[window_index];
		stored_normals[tap] = vec2f16(tile_indirect_normals[window_index]);
		lights[tap] = vec4f16(tile_indirect_lights[window_index]);
	}
	let indirect: vec4f = weigh_screen_space_indirect_diffuse(
		material_evaluation,
		base,
		source_position - base,
		source_extent,
		depths,
		stored_normals,
		lights,
		position,
		normal,
		false
	);
	if (!ray_traced || dot(indirect, indirect) != 0.0) {
		return indirect;
	}
	// Retry the same texels against the triangle plane, with the nearest-depth fallback for a thin surface.
	// Keeping the original reads avoids a second texture window without changing either reconstruction's tests.
	return weigh_screen_space_indirect_diffuse(
		material_evaluation,
		base, source_position - base, source_extent, depths, stored_normals, lights, position, geometric_normal, true
	);
}
"#;
