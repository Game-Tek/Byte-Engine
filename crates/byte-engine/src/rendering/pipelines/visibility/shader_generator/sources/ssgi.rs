// Screen-space indirect diffuse helpers. Opaque material evaluation reconstructs each pixel's indirect diffuse light
// from the half-resolution SSGI history with the pixel's own surface position and normal, so SSGI writes no
// full-resolution image.
//
// The helpers read the `ssgi_view`, `ssgi_history`, and `ssgi_normals` bindings that
// `screen_space_indirect_diffuse_scope` declares, and the shared `depth_pyramid` binding. Each helper only calls
// helpers declared above it.

// Decodes a normal the SSGI trace stored as an octahedral pair in RG, or returns zero for the pair (0, 0), which marks
// a texel without a normal. That pair encodes +z, which points away from the camera and so is never a stored normal.
// `ssgi-temporal.besl` holds the same decoder; keep both in sync with `encode_stored_normal` in `ssgi-trace.besl`.
pub(crate) const SSGI_STORED_NORMAL_SOURCE: &str = r#"
ssgi_stored_normal: fn (stored: vec4f) -> vec3f {
	if (stored.x == 0.0 && stored.y == 0.0) {
		return vec3f(0.0, 0.0, 0.0);
	}
	let z: f32 = 1.0 - abs(stored.x) - abs(stored.y);
	let fold: f32 = max(0.0 - z, 0.0);
	// `step` returns the positive direction at zero, matching the trace's encoder.
	let normal: vec3f = vec3f(
		stored.x - (step(0.0, stored.x) * 2.0 - 1.0) * fold,
		stored.y - (step(0.0, stored.y) * 2.0 - 1.0) * fold,
		z
	);
	return normal * inversesqrt(dot(normal, normal));
}
"#;

// Returns how much a history texel belongs to the pixel's surface: one on the pixel's tangent plane with a similar
// normal, falling to zero for another surface. Depth alone cannot separate surfaces that touch: where a boot stands
// on a floor, both have the same depth, but the boot's normal and its distance from the floor's plane differ. Without
// a texel normal, it compares depth only.
pub(crate) const SSGI_SURFACE_WEIGHT_SOURCE: &str = r#"
ssgi_surface_weight: fn (normal: vec3f, position: vec3f, tap_normal: vec3f, tap_position: vec3f) -> f32 {
	if (dot(tap_normal, tap_normal) == 0.0) {
		// A 2% relative-depth sigma gives a Gaussian inverse variance of 1,250.
		let relative_delta: f32 = (tap_position.z - position.z) / position.z;
		return exp(0.0 - relative_delta * relative_delta * 1250.0);
	}
	// A 1% sigma, relative to the pixel's depth, for the texel's distance from the pixel's tangent plane.
	let plane_distance: f32 = dot(normal, tap_position - position) / position.z;
	// Texels whose normal differs from the pixel's by more than about 45 degrees lie on another surface. Up to about
	// 25 degrees, as across a curved limb, they count fully.
	let normal_agreement: f32 = clamp((dot(normal, tap_normal) - 0.7) / (0.9 - 0.7), 0.0, 1.0);
	return normal_agreement * exp(0.0 - plane_distance * plane_distance * 10000.0);
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
	// only what its fetches returned, the depth and the stored normal pair, and derives its position and decoded
	// normal when it is weighted, so the fetch window holds as little as possible.
	let depths: f32[4] = f32[4](0.0, 0.0, 0.0, 0.0);
	let stored_normals: vec2f[4] = vec2f[4](vec2f(0.0, 0.0), vec2f(0.0, 0.0), vec2f(0.0, 0.0), vec2f(0.0, 0.0));
	let zero: vec4f = vec4f(0.0, 0.0, 0.0, 0.0);
	let lights: vec4f[4] = vec4f[4](zero, zero, zero, zero);
	for (let tap: u32 = 0; tap < 4; tap = tap + 1) {
		let texel: vec2u = vec2u(
			min(u32(base.x) + tap % 2, source_extent.x - 1),
			min(u32(base.y) + tap / 2, source_extent.y - 1)
		);
		// Mip zero of the depth pyramid holds positive linear depth at the history's resolution.
		depths[tap] = fetch(depth_pyramid, texel).x;
		let stored: vec4f = fetch(ssgi_normals, texel);
		stored_normals[tap] = vec2f(stored.x, stored.y);
		lights[tap] = fetch(ssgi_history, texel);
	}
	let sum: vec4f = zero;
	let total_weight: f32 = 0.0;
	for (let tap: u32 = 0; tap < 4; tap = tap + 1) {
		let z: f32 = depths[tap];
		// The texel again, in float so it is rebuilt from `base` here rather than kept from the fetch loop.
		let tap_texel: vec2f = vec2f(
			min(base.x + f32(tap % 2), f32(source_extent.x - 1)),
			min(base.y + f32(tap / 2), f32(source_extent.y - 1))
		);
		let ray: vec2f = tap_texel * ssgi_view.pixel_to_ray_mul + ssgi_view.pixel_to_ray_add;
		let stored: vec2f = stored_normals[tap];
		let tap_normal: vec3f = ssgi_stored_normal(vec4f(stored.x, stored.y, 0.0, 0.0));
		let bilinear: f32 = mix(1.0 - fraction.x, fraction.x, f32(tap % 2)) * mix(1.0 - fraction.y, fraction.y, f32(tap / 2));
		// A small floor keeps a texel that lands exactly on the far side of the bilinear footprint eligible. A texel
		// without a surface has zero depth and gets no weight; a factor instead of a branch keeps the four taps in one
		// straight run of code that the compiler can interleave with the fetches.
		let weight: f32 = step(0.000001, z) * max(bilinear, 0.001) * ssgi_surface_weight(
			normal,
			position,
			tap_normal,
			vec3f(ray.x * z, ray.y * z, z)
		);
		sum = sum + lights[tap] * weight;
		total_weight = total_weight + weight;
	}
	if (total_weight > 0.00001) {
		return sum * (1.0 / total_weight);
	}
	return zero;
}
"#;
