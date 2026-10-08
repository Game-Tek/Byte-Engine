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
// half-width registers. `ssgi-temporal.besl` holds the same decoder at full precision; keep both in sync with
// `encode_stored_normal` in `ssgi-trace.besl`.
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

// Reconstructs a pixel's pre-exposed indirect diffuse light from the half-resolution SSGI history: the four history
// texels around the pixel, weighted bilinearly and by how well each lies on the pixel's surface, so light does not
// bleed across edges. RGB is the light of rays that hit on-screen geometry and alpha the fraction of rays that hit.
// Without a matching texel, every ray counts as missed, so the environment lights the pixel instead of a neighbor.
//
// `pixel` is the full-resolution pixel of an `extent` image. `position` and `normal` are its view-space surface
// position and geometric normal, in the space the SSGI images use.
pub(crate) const SAMPLE_SCREEN_SPACE_INDIRECT_DIFFUSE_SOURCE: &str = r#"
sample_screen_space_indirect_diffuse: fn (pixel: vec2u, extent: vec2u, position: vec3f, normal: vec3f) -> vec4f {
	let source_extent: vec2u = texture_size(ssgi_history);
	// Odd full extents do not divide exactly by two. Match texel centers by normalized screen position.
	let source_position: vec2f = vec2f(
		clamp((f32(pixel.x) + 0.5) * f32(source_extent.x) / f32(extent.x) - 0.5, 0.0, f32(source_extent.x - 1)),
		clamp((f32(pixel.y) + 0.5) * f32(source_extent.y) / f32(extent.y) - 0.5, 0.0, f32(source_extent.y - 1))
	);
	let base: vec2f = vec2f(floor(source_position.x), floor(source_position.y));
	let fraction: vec2f = source_position - base;
	// Every texel is read before any weight is computed, so the twelve fetches overlap instead of waiting on each
	// other: the material shader's register footprint leaves few other threads to hide their latency. Each tap keeps
	// only what its fetches returned, the depth at full precision and the stored normal pair and light at the half
	// precision their 16-bit images hold, and derives its position and decoded normal when it is weighted, so the
	// fetch window holds as little as possible. Keeping the pairs and the light at half precision took the window's
	// register peak, the highest of the opaque material shader without local lights, from 86 to 74, and material
	// evaluation 0.8 to 1.4 % faster on the Sponza hall view (2026-10-06).
	let depths: f32[4] = f32[4](0.0, 0.0, 0.0, 0.0);
	let zero_pair: vec2f16 = vec2f16(0.0, 0.0);
	let stored_normals: vec2f16[4] = vec2f16[4](zero_pair, zero_pair, zero_pair, zero_pair);
	let zero_light: vec4f16 = vec4f16(0.0, 0.0, 0.0, 0.0);
	let lights: vec4f16[4] = vec4f16[4](zero_light, zero_light, zero_light, zero_light);
	for (let tap: u32 = 0; tap < 4; tap = tap + 1) {
		let texel: vec2u = vec2u(
			min(u32(base.x) + tap % 2, source_extent.x - 1),
			min(u32(base.y) + tap / 2, source_extent.y - 1)
		);
		// Mip zero of the depth pyramid holds positive linear depth at the history's resolution.
		depths[tap] = fetch(depth_pyramid, texel).x;
		let stored: vec4f = fetch(ssgi_normals, texel);
		stored_normals[tap] = vec2f16(vec2f(stored.x, stored.y));
		lights[tap] = vec4f16(fetch(ssgi_history, texel));
	}
	// The pixel's terms of the plane test, computed once for the four taps.
	let inverse_depth: f32 = 1.0 / position.z;
	let scaled_normal: vec3f = normal * inverse_depth;
	let plane_offset: f32 = dot(normal, position) * inverse_depth;
	let normal16: vec3f16 = vec3f16(normal);
	let sum: vec4f = vec4f(0.0, 0.0, 0.0, 0.0);
	let total_weight: f32 = 0.0;
	for (let tap: u32 = 0; tap < 4; tap = tap + 1) {
		let z: f32 = depths[tap];
		// The texel again, in float so it is rebuilt from `base` here rather than kept from the fetch loop.
		let tap_texel: vec2f = vec2f(
			min(base.x + f32(tap % 2), f32(source_extent.x - 1)),
			min(base.y + f32(tap / 2), f32(source_extent.y - 1))
		);
		let ray: vec2f = tap_texel * ssgi_view.pixel_to_ray_mul + ssgi_view.pixel_to_ray_add;
		let tap_normal: vec3f16 = ssgi_stored_normal(stored_normals[tap]);
		let bilinear: f32 = mix(1.0 - fraction.x, fraction.x, f32(tap % 2)) * mix(1.0 - fraction.y, fraction.y, f32(tap / 2));
		// A small floor keeps a texel that lands exactly on the far side of the bilinear footprint eligible. A texel
		// without a surface has zero depth and gets no weight; a factor instead of a branch keeps the four taps in one
		// straight run of code that the compiler can interleave with the fetches.
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
	return vec4f(0.0, 0.0, 0.0, 0.0);
}
"#;
