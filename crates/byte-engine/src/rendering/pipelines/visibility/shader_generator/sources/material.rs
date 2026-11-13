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

/// Picks the pixel a thread of a per-material evaluation pipeline shades from its slot's pixel list, and the instance
/// that covers it. The surface prefix follows.
pub(crate) const MATERIAL_EVALUATION_PIXEL_LIST_SOURCE: &str = r#"
material_evaluation_pixel_list: fn (input: StageInput) -> void {
	let invocation: vec2u = input.thread_id;
	// One dispatch shades the pixel list of an evaluation slot, which holds the pixels of every material that compiled to
	// this pipeline. Each pixel reads its own material below.
	if (invocation.x >= visibility_buffer.material_count.material_count[push_constant.evaluation_index]) {
		return;
	}

	let offset: u32 = visibility_buffer.material_offset.material_offset[push_constant.evaluation_index];
	let packed_pixel_coordinates: vec2u16 = visibility_buffer.pixel_mapping.pixel_mapping[offset + invocation.x];
	let raw_pixel_coordinates: vec2u = vec2u(
		u32(packed_pixel_coordinates.x),
		u32(packed_pixel_coordinates.y)
	);
	if (raw_pixel_coordinates.x == 0 || raw_pixel_coordinates.y == 0) {
		return;
	}
	let pixel_coordinates: vec2u = raw_pixel_coordinates - vec2u(1, 1);
	let image_extent: vec2u = image_size(visibility_buffer.triangle_index);
	if (pixel_coordinates.x >= image_extent.x || pixel_coordinates.y >= image_extent.y) {
		return;
	}
	let instance_index: u32 = image_load_u32(visibility_buffer.instance_index_render_target, pixel_coordinates);
	let mesh: Mesh = scene.meshes[instance_index];
}
"#;

/// Fills the opaque uber shader's workgroup window of indirect diffuse texels, then waits for every thread of the tile.
///
/// Each 8 x 8 tile's pixels read their indirect diffuse taps from one window of at most 6 x 6 half-resolution texels,
/// so 36 threads fetch the window once instead of every pixel fetching its four taps. The window starts at the first
/// texel the tile's first pixel reads, as `indirect_diffuse_window_origin` finds it, and holds texels clamped to the
/// image the same way the taps clamp them, so every tap of the tile lands inside it. The fill reads
/// `push_constant.indirect_diffuse`, so a project without an indirect diffuse source drops it. Under lightmaps, value 3,
/// pixels read no window. Every thread reaches the barrier, those past the image's edge too.
pub(crate) const MATERIAL_EVALUATION_TILE_WINDOW_SOURCE: &str = r#"
material_evaluation_tile_window: fn (input: StageInput) -> void {
	if (push_constant.indirect_diffuse != 0 && push_constant.indirect_diffuse != 3) {
		if (input.thread_idx < 36) {
			let window_extent: vec2u = texture_size(material_evaluation.ssgi_history);
			let window_origin: vec2u = indirect_diffuse_window_origin(input.thread_id, image_size(material_evaluation.lit_map), window_extent);
			let window_texel: vec2u = vec2u(
				min(window_origin.x + input.thread_idx % 6, window_extent.x - 1),
				min(window_origin.y + input.thread_idx / 6, window_extent.y - 1)
			);
			// The same three images, at the same precision, as the taps of `sample_screen_space_indirect_diffuse`.
			tile_indirect_depths[input.thread_idx] = fetch(material_evaluation.depth_pyramid, window_texel).x;
			let window_normal: vec4f = fetch(material_evaluation.ssgi_normals, window_texel);
			tile_indirect_normals[input.thread_idx] = vec2f(window_normal.x, window_normal.y);
			tile_indirect_lights[input.thread_idx] = fetch(material_evaluation.ssgi_history, window_texel);
		}
	}
	workgroup_barrier();
}
"#;

/// Picks the pixel a thread of an uber evaluation shader shades: its own pixel of the 8 x 8 tile its workgroup
/// covers. A pixel no surface covers returns; the shader that sets `clears_background`, one of the opaque phase's,
/// first writes the zeros the per-material mode's pixel mapping writes, so the background and the next frame's rays
/// never read light of an older frame there. The surface prefix follows.
pub(crate) const MATERIAL_EVALUATION_TILE_SOURCE: &str = r#"
material_evaluation_tile: fn (input: StageInput) -> void {
	let pixel_coordinates: vec2u = input.thread_id;
	let image_extent: vec2u = image_size(visibility_buffer.triangle_index);
	if (pixel_coordinates.x >= image_extent.x || pixel_coordinates.y >= image_extent.y) {
		return;
	}
	let instance_index: u32 = image_load_u32(visibility_buffer.instance_index_render_target, pixel_coordinates);
	if (instance_index == 4294967295) {
		if (clears_background) {
			let zero: vec4f = vec4f(0.0, 0.0, 0.0, 0.0);
			write(material_evaluation.lit_map, pixel_coordinates, zero);
			write(material_evaluation.diffuse_radiance_map, pixel_coordinates, zero);
			write(material_evaluation.subsurface_albedo, pixel_coordinates, zero);
		}
		return;
	}
	let mesh: Mesh = scene.meshes[instance_index];
	// The arm key of the pixel's material, which selects its surface code below.
	let material_arm: u32 = scene.material_evaluations.evaluation_indices[mesh.material & MESH_MATERIAL_MASK];
}
"#;

/// Leaves an opaque uber shader's thread at a pixel the phase's other shader shades: `translucent_arm_bit` is the
/// lowest bit of this shader's arm keys, which `uber_arm_key` sets for translucent materials alone.
pub(crate) const MATERIAL_EVALUATION_TILE_KIND_SOURCE: &str = r#"
material_evaluation_tile_kind: fn () -> void {
	if ((material_arm & 1) != translucent_arm_bit) {
		return;
	}
}
"#;

/// Rebuilds the pixel's surface from the visibility buffer: its triangle, perspective-correct barycentrics and their
/// one-pixel differences, position, normal, and view vector. A pixel source declares `pixel_coordinates`,
/// `image_extent`, `instance_index`, and `mesh` first.
pub(crate) const MATERIAL_EVALUATION_PREFIX_SOURCE: &str = r#"
material_evaluation_prefix: fn (input: StageInput) -> void {
	let triangle_meshlet_indices: u32 = image_load_u32(visibility_buffer.triangle_index, pixel_coordinates);
	let meshlet_triangle_index: u32 = triangle_meshlet_indices & 255;
	let meshlet_index: u32 = triangle_meshlet_indices >> 8;
	let meshlet: Meshlet = scene.meshlets[meshlet_index];
	// Materials of one slot run the same program and differ only in the textures they bind, so the texture slots come
	// from the pixel's own material. Pixel mapping groups pixels by tile, so neighboring lanes usually share it.
	let material: Material = scene.materials.materials[mesh.material & MESH_MATERIAL_MASK];

	let primitive_index_base: u32 = (meshlet.triangle_offset + meshlet_triangle_index) * 3;
	let triangle_vertex_indices: u32[3] = compute_vertex_indices(scene, mesh, meshlet, primitive_index_base);
	// Every lane sets up its own triangle. Lanes of one SIMD group often share a triangle, but a region that only a
	// leader lane runs issues the same instructions as one every lane runs, so sharing the setup through ballots and
	// broadcasts only added their own cost: material evaluation ran 3 % faster without it (2026-10-05).
	let position0: vec3f = scene.vertex_positions[triangle_vertex_indices[0]];
	let position1: vec3f = scene.vertex_positions[triangle_vertex_indices[1]];
	let position2: vec3f = scene.vertex_positions[triangle_vertex_indices[2]];
	let model_space_vertex_positions: vec4f[3] = vec4f[3](
		vec4f(position0.x, position0.y, position0.z, 1.0),
		vec4f(position1.x, position1.y, position1.z, 1.0),
		vec4f(position2.x, position2.y, position2.z, 1.0)
	);
	// Octahedral decoding leaves a normal shorter than one. Interpolation below weights the normals as they are,
	// so they are made unit length first.
	let model_space_vertex_normals: vec3f[3] = vec3f[3](
		normalize(decode_octahedral_normal(scene.vertex_normals[triangle_vertex_indices[0]])),
		normalize(decode_octahedral_normal(scene.vertex_normals[triangle_vertex_indices[1]])),
		normalize(decode_octahedral_normal(scene.vertex_normals[triangle_vertex_indices[2]]))
	);
	let nc: vec2f = make_raster_ndc_from_pixel_coordinates(pixel_coordinates, image_extent);
	let model: mat4x3f = mesh.model;
	let view_projection: mat4f = scene.views.views[0].view_projection;
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
	// The normal is interpolated in model space and turned into world space once, instead of turning all three. The
	// turn is linear, so the direction is the same as turning each vertex normal first.
	let model_space_normal: vec3f = model_space_vertex_normals[0]
		+ barycentric.x * (model_space_vertex_normals[1] - model_space_vertex_normals[0])
		+ barycentric.y * (model_space_vertex_normals[2] - model_space_vertex_normals[0]);
	let world_space_normal: vec3f = transform_normal(model[0], model[1], model[2], model_space_normal);
	let world_space_vertex_normal: vec3f = normalize(world_space_normal);
	let N: vec3f = world_space_vertex_normal;
	let camera_position: vec3f = scene.views.views[0].inverse_view * vec4f(0.0, 0.0, 0.0, 1.0);
	let V: vec3f = normalize(camera_position - world_space_vertex_position);
	// Only double-sided materials rasterize back faces, so only they need the check.
	if ((mesh.level & MESH_FLAG_DOUBLE_SIDED) != 0) {
		N = facing_normal(N, V, world_edge1, world_edge2);
	}
	// Shading reads the view vector and the interpolated normal at half precision. The triangle normal below stays
	// at full precision for reflection visibility and the world-space ray offset.
	let V_material: vec3f16 = vec3f16(V);
	let base_normal: vec3f16 = vec3f16(N);
	// Reflection visibility and ray offsets follow the triangle plane, not its interpolated shading normals.
	// Point toward the camera so a visible back face of a double-sided surface offsets into the visible half-space.
	let geometric_normal: vec3f = normalize(cross(world_edge1, world_edge2));
	if (dot(geometric_normal, V) < 0.0) {
		geometric_normal = geometric_normal * (0.0 - 1.0);
	}
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

/// Returns the scale and bias of `F0` in a GGX lobe's directional albedo at `cosine`, the share of arriving light it
/// reflects: `E = F0 · scale + bias`. Material evaluation weighs reflected light with it, and the diffuse terms take
/// what it leaves.
///
/// It is Brian Karis's fit of the split-sum environment BRDF, based on Dimitar Lazarov's work. Epic permits use
/// without attribution: https://www.unrealengine.com/en-US/blog/physically-based-shading-on-mobile
pub(crate) const ENVIRONMENT_BRDF_SOURCE: &str = r#"
environment_brdf: fn (cosine: f16, roughness: f16) -> vec2f16 {
	let c0: vec4f16 = vec4f16(0.0 - 1.0, 0.0 - 0.0275, 0.0 - 0.572, 0.022);
	let c1: vec4f16 = vec4f16(1.0, 0.0425, 1.04, 0.0 - 0.04);
	let r: vec4f16 = roughness * c0 + c1;
	let a004: f16 = min(r.x * r.x, pow(f16(2.0), (f16(0.0) - f16(9.28)) * cosine)) * r.x + r.y;
	return vec2f16(0.0 - 1.04, 1.04) * a004 + vec2f16(r.z, r.w);
}
"#;

/// Returns the share of light arriving at `cosine` that a dielectric surface's specular lobe leaves to diffuse, one
/// minus its directional albedo with `F0` 0.04.
///
/// Diffuse light belongs to the dielectric layer, so metalness scales it apart from this. The fit overshoots one at
/// grazing angles on mirror-smooth surfaces, which the clamp absorbs.
pub(crate) const DIFFUSE_TRANSMISSION_SOURCE: &str = r#"
diffuse_transmission: fn (cosine: f16, roughness: f16) -> f16 {
	let brdf: vec2f16 = environment_brdf(cosine, roughness);
	return clamp(f16(1.0) - (f16(0.04) * brdf.x + brdf.y), f16(0.0), f16(1.0));
}
"#;

/// Returns the share of the environment's reflection a mirror direction `reflection` keeps above the geometric surface
/// whose normal is `geometric_normal`, `clamp(1 + 1.3·dot(R, Ng), 0, 1)²`.
///
/// Russell's horizon occlusion, "Horizon Occlusion for Normal Mapped Reflections" (Marmoset, 2015). A normal map can
/// tilt the shading normal so its mirror direction points toward or into the surface, where the environment can't be
/// seen; the share falls to zero once the direction is 0.77 below the surface's plane.
pub(crate) const HORIZON_OCCLUSION_SOURCE: &str = r#"
horizon_occlusion: fn (reflection: vec3f, geometric_normal: vec3f) -> f32 {
	let horizon: f32 = clamp(1.0 + 1.3 * dot(reflection, geometric_normal), 0.0, 1.0);
	return horizon * horizon;
}
"#;

/// Returns the share of a traced source's light that GTAO leaves: `gtao_visibility / source_visibility`, at most one.
///
/// Surfel GI, probe GI, and lightmaps trace rays, so their light already holds the occlusion those rays found, and
/// `source_visibility` is the share of them that reached the environment. Multiplying that light by GTAO's visibility
/// would darken an occluder both saw twice. This is the rule the environment's visibility follows, the smaller of the
/// two estimates, applied to light that already carries one of them; it is this engine's own rule, not a published
/// one. See the lighting design note.
pub(crate) const TRACED_LIGHT_OCCLUSION_SOURCE: &str = r#"
traced_light_occlusion: fn (gtao_visibility: f32, source_visibility: f32) -> f32 {
	return min(gtao_visibility / max(source_visibility, 0.0001), 1.0);
}
"#;

/// Returns an arc cosine accurate to about 0.007 radians with one square root, Lagarde's fit as Intel's XeGTAO and
/// Unity's HDRP use it, for [`CONE_SPECULAR_OCCLUSION_SOURCE`]'s cone angles.
pub(crate) const FAST_ACOS_SOURCE: &str = r#"
fast_acos: fn (x: f32) -> f32 {
	let magnitude: f32 = abs(x);
	let result: f32 = (1.57079632679 - 0.156583 * magnitude) * sqrt(1.0 - magnitude);
	if (x < 0.0) {
		return 3.14159265359 - result;
	}
	return result;
}
"#;

/// Returns the share of a reflection's lobe that the open directions around a pixel let through.
///
/// Jimenez, Wu, Pesce, and Jarabo's cone-cone specular occlusion, "Practical Realtime Strategies for Accurate Indirect
/// Occlusion" (2016), as Unity's HDRP writes it in `GetSpecularOcclusionFromBentAO`. The open directions form a cone
/// around `bent_normal` whose cosine `√(1 − visibility)` holds the cosine-weighted `visibility`; the lobe is a cone
/// around `reflection` with a cosine of `2^(−3.32193·α²)`, where `alpha` is the roughness squared. The share is the
/// cones' intersection, by a smoothstep fit of the solid angle two caps share, divided by the lobe's solid angle.
pub(crate) const CONE_SPECULAR_OCCLUSION_SOURCE: &str = r#"
cone_specular_occlusion: fn (bent_normal: vec3f, reflection: vec3f, visibility: f32, alpha: f32) -> f32 {
	let visibility_cos: f32 = sqrt(clamp(1.0 - visibility, 0.0, 1.0));
	let clamped_alpha: f32 = max(alpha, 0.01);
	let lobe_cos: f32 = pow(2.0, 0.0 - 3.32192809489 * clamped_alpha * clamped_alpha);
	let visibility_angle: f32 = fast_acos(visibility_cos);
	let lobe_angle: f32 = fast_acos(lobe_cos);
	let separation: f32 = fast_acos(clamp(dot(bent_normal, reflection), -1.0, 1.0));
	let difference: f32 = abs(visibility_angle - lobe_angle);
	// The cap of the smaller cone, which is all the two can share.
	let smaller_cap: f32 = 6.28318530718 * (1.0 - max(visibility_cos, lobe_cos));
	let shared: f32 = 0.0;
	if (separation <= difference) {
		shared = smaller_cap;
	} else {
		if (separation < visibility_angle + lobe_angle) {
			let overlap: f32 = 1.0 - clamp((separation - difference) / max(visibility_angle + lobe_angle - difference, 0.0001), 0.0, 1.0);
			shared = smoothstep(0.0, 1.0, overlap) * smaller_cap;
		}
	}
	return clamp(shared / (6.28318530718 * (1.0 - lobe_cos)), 0.0, 1.0);
}
"#;

/// Returns Christensen and Burley's per-channel profile scale `d = ℓ / s` for the mean free path `radius`, in meters,
/// and the multiple-scattering `albedo`, with their searchlight fit `s = 1.85 − A + 7·|A − 0.8|³`.
///
/// "Approximate Reflectance Profiles for Efficient Subsurface Scattering", Christensen and Burley, 2015.
pub(crate) const BURLEY_DISTANCE_SOURCE: &str = r#"
burley_distance: fn (radius: vec3f16, albedo: vec3f16) -> vec3f16 {
	let offset: vec3f16 = albedo - vec3f16(0.8, 0.8, 0.8);
	let magnitude: vec3f16 = vec3f16(abs(offset.x), abs(offset.y), abs(offset.z));
	let shape: vec3f16 = vec3f16(1.85, 1.85, 1.85) - albedo + f16(7.0) * magnitude * magnitude * magnitude;
	return radius / shape;
}
"#;

/// Returns the share of light per channel that crosses `thickness` meters of a material whose profile scale is
/// `distance`, `¼·e^(−t/d) + ¾·e^(−t/(3d))`, the transmittance of Christensen and Burley's profile that Golubev's
/// "Efficient Screen-Space Subsurface Scattering" (2018) uses for thin geometry.
pub(crate) const BURLEY_TRANSMITTANCE_SOURCE: &str = r#"
burley_transmittance: fn (thickness: f16, distance: vec3f16) -> vec3f16 {
	let t: vec3f = vec3f(f32(thickness), f32(thickness), f32(thickness))
		/ max(vec3f(distance), vec3f(0.000001, 0.000001, 0.000001));
	return vec3f16(
		0.25 * exp(0.0 - t.x) + 0.75 * exp(0.0 - t.x / 3.0),
		0.25 * exp(0.0 - t.y) + 0.75 * exp(0.0 - t.y / 3.0),
		0.25 * exp(0.0 - t.z) + 0.75 * exp(0.0 - t.z / 3.0)
	);
}
"#;

/// Returns the cosine-weighted average over the hemisphere of a dielectric GGX lobe's directional albedo, `E_avg`.
///
/// Direct diffuse light divides by `1 - E_avg`, so a surface lit evenly from every direction diffuses exactly the share
/// [`DIFFUSE_TRANSMISSION_SOURCE`] leaves toward the viewer. The quadratic fits the average of
/// [`ENVIRONMENT_BRDF_SOURCE`], integrated numerically, within 0.004 for every roughness.
pub(crate) const AVERAGE_SPECULAR_ALBEDO_SOURCE: &str = r#"
average_specular_albedo: fn (roughness: f16) -> f16 {
	return f16(0.0939) + roughness * (f16(0.0146) * roughness - f16(0.0962));
}
"#;

/// Returns the Trowbridge-Reitz (GGX) distribution toward `half_vector` for the lobe width `alpha`, roughness squared,
/// which lights the direct specular lobe.
///
/// It follows Filament: `1 - (n·h)²` is written as `|n × h|²`, which doesn't cancel near the peak, and it runs in f32,
/// because a smooth lobe peaks far past the half-float range. See "Smooth highlights stay finite" in the lighting design
/// note.
pub(crate) const TROWBRIDGE_REITZ_SOURCE: &str = r#"
trowbridge_reitz: fn (normal: vec3f, half_vector: vec3f, alpha: f32) -> f32 {
	let normal_cross_half: vec3f = cross(normal, half_vector);
	let alpha_n_dot_h: f32 = max(dot(normal, half_vector), 0.0) * alpha;
	let root: f32 = alpha / (dot(normal_cross_half, normal_cross_half) + alpha_n_dot_h * alpha_n_dot_h);
	return root * root * 0.31830988618;
}
"#;

pub(crate) const MATERIAL_EVALUATION_UV_SOURCE: &str = r#"
material_evaluation_uv: fn () -> void {
	// Runtime UVs use half-float storage and are expanded only for materials that sample them. They are interpolated,
	// with their one-pixel differences, from the barycentrics of the prefix.
	let vertex_uv0: vec2f = decode_f16_vec2(scene.vertex_uvs[triangle_vertex_indices[0]]);
	let uv_edge1: vec2f = decode_f16_vec2(scene.vertex_uvs[triangle_vertex_indices[1]]) - vertex_uv0;
	let uv_edge2: vec2f = decode_f16_vec2(scene.vertex_uvs[triangle_vertex_indices[2]]) - vertex_uv0;
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
	// The UV directions lie in the triangle's plane, which tilts away from an interpolated normal on curved surfaces.
	// Projecting the tangent onto the normal's plane keeps the frame orthonormal, as tangent-space normal maps assume.
	let tangent_in_plane: vec3f = tangent_direction - N * dot(N, tangent_direction);
	let tangent: vec3f = tangent_in_plane
		* inversesqrt(max(dot(tangent_in_plane, tangent_in_plane), length_squared_floor));
	// glTF and MikkTSpace bake normal maps with green pointing up the image, toward decreasing v, so the bitangent is
	// the normal crossed with the tangent, turned away from the direction v increases in.
	let normal_cross_tangent: vec3f = cross(N, tangent);
	let bitangent_sign: f32 = 1.0 - step(0.0, dot(normal_cross_tangent, bitangent_direction)) * 2.0;
	// The frame only rotates the half-precision material normal, so it is kept at half precision too.
	let T: vec3f16 = vec3f16(tangent);
	let B: vec3f16 = vec3f16(normal_cross_tangent * bitangent_sign);
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
	let transmission: vec3f = vec3f(0.0, 0.0, 0.0);
	let translucency: vec3f16 = vec3f16(0.0, 0.0, 0.0);
	// Below zero, the material sets no `subsurface_radius` or `thickness`: it doesn't scatter, or passes no light
	// through its thickness.
	let subsurface_radius: vec3f16 = vec3f16(-1.0, -1.0, -1.0);
	let subsurface_thickness: f16 = -1.0;
}
"#;

pub(crate) const MATERIAL_EVALUATION_TANGENT_NORMAL_SOURCE: &str = r#"
material_evaluation_normal: fn () -> void {
	normal = vec3f16(normalize(f32(normal.x) * vec3f(T) + f32(normal.y) * vec3f(B) + f32(normal.z) * vec3f(base_normal)));
}
"#;

pub(crate) const MATERIAL_EVALUATION_GEOMETRY_NORMAL_SOURCE: &str = r#"
material_evaluation_normal: fn () -> void {
	normal = base_normal;
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
	scene: Scene,
	profile_index: u32,
	emission_direction: vec3f,
	axis: vec3f,
	encoded_c0_tangent: vec2u16
) -> f32 {
	let uv: vec2f = ies_profile_uv(emission_direction, axis, encoded_c0_tangent);
	return max(
		sample_texture_2d_array_grad(scene.ies_profiles, profile_index, uv, vec2f(0.0, 0.0), vec2f(0.0, 0.0)).x,
		0.0
	);
}
"#;

pub(crate) const MATERIAL_EVALUATION_SUFFIX_SOURCE: &str = r#"
material_evaluation_suffix: fn () -> void {
	// The fraction of the hemisphere around the normal that nearby geometry leaves open to the environment. GTAO and
	// the indirect diffuse source each estimate it for opaque surfaces, so a transparent surface has neither, and
	// either may be off.
	let environment_visibility: f32 = 1.0;
	// The indirect diffuse source's light: what SSGI's rays hit, already weighted by the fraction of rays that hit, or
	// everything surfel GI's rays reached, the environment included.
	let screen_space_irradiance: vec3f = vec3f(0.0, 0.0, 0.0);
	// Whether the pixel adds the environment's diffuse light itself, which surfel GI's light already holds.
	let adds_environment_diffuse: bool = true;
	// The mean direction of the open part of the hemisphere, which specular occlusion centers its visibility cone on.
	// Without GTAO, the shading normal stands in.
	let bent_normal: vec3f = vec3f(normal);
	if (!transparent) {
		if (push_constant.indirect_diffuse == 3) {
			// Lightmaps light the instances they give a rectangle; the rest take the environment's light alone. The
			// branch samples `lightmaps`, so a project without lightmaps drops it.
			let lightmap_slot: u32 = (mesh.level >> 26) & 31;
			if (lightmap_slot != 0) {
				// UNORM16 lightmap UVs, interpolated like the texture coordinates, then placed in the instance's rectangle.
				let lightmap_scale: f32 = 1.0 / 65535.0;
				let lightmap_encoded0: vec2u16 = scene.vertex_lightmap_uvs[triangle_vertex_indices[0]];
				let lightmap_encoded1: vec2u16 = scene.vertex_lightmap_uvs[triangle_vertex_indices[1]];
				let lightmap_encoded2: vec2u16 = scene.vertex_lightmap_uvs[triangle_vertex_indices[2]];
				let lightmap_uv0: vec2f = vec2f(f32(u32(lightmap_encoded0.x)), f32(u32(lightmap_encoded0.y))) * lightmap_scale;
				let lightmap_uv1: vec2f = vec2f(f32(u32(lightmap_encoded1.x)), f32(u32(lightmap_encoded1.y))) * lightmap_scale;
				let lightmap_uv2: vec2f = vec2f(f32(u32(lightmap_encoded2.x)), f32(u32(lightmap_encoded2.y))) * lightmap_scale;
				let lightmap_uv: vec2f = lightmap_uv0 + barycentric.x * (lightmap_uv1 - lightmap_uv0)
					+ barycentric.y * (lightmap_uv2 - lightmap_uv0);
				// The instance's rectangle, as 0.16 fixed-point scale and offset fractions of the lightmap.
				let rectangle_scale: f32 = 1.0 / 65536.0;
				let lightmap_texel_uv: vec2f = vec2f(
					lightmap_uv.x * f32(mesh.surface.x & 65535) + f32(mesh.surface.y & 65535),
					lightmap_uv.y * f32(mesh.surface.x >> 16) + f32(mesh.surface.y >> 16)
				) * rectangle_scale;
				// The lightmap has one level, so zero derivatives read it.
				let lightmap_light: vec4f = sample_texture_2d_array_grad(
					scene.lightmaps, lightmap_slot - 1, lightmap_texel_uv, vec2f(0.0, 0.0), vec2f(0.0, 0.0)
				);
				// The bake stored irradiance over π in nits, not pre-exposed, holding the environment's light its paths
				// reached.
				screen_space_irradiance = vec3f(lightmap_light.x, lightmap_light.y, lightmap_light.z);
				adds_environment_diffuse = false;
				// Alpha is the share of the texel's paths that reached the environment, which occludes its reflections
				// as the rays' hit fraction does for the other sources.
				environment_visibility = clamp(lightmap_light.w, 0.0, 1.0);
			}
		} else if (push_constant.indirect_diffuse != 0) {
			// SSGI and surfel GI both store their light in view space at half resolution. The pixel's own surface picks
			// the history texels that lie on it. Alpha is the fraction of rays that hit, so the rest reach the
			// environment.
			// The surface in view space, made here so neither stays live through the material body.
			let view_space_surface_position: vec3f = scene.views.views[0].view * vec4f(
				world_space_vertex_position.x,
				world_space_vertex_position.y,
				world_space_vertex_position.z,
				1.0
			);
			let view_space_normal: vec3f = scene.views.views[0].view * vec4f(
				f32(base_normal.x),
				f32(base_normal.y),
				f32(base_normal.z),
				0.0
			);
			let screen_space_indirect: vec4f = vec4f(0.0, 0.0, 0.0, 0.0);
			if (push_constant.indirect_diffuse == 2) {
				let view_space_geometric_normal: vec3f = scene.views.views[0].view * vec4f(
					geometric_normal.x, geometric_normal.y, geometric_normal.z, 0.0
				);
				screen_space_indirect = sample_screen_space_indirect_diffuse(
					material_evaluation,
					pixel_coordinates, image_extent, view_space_surface_position, view_space_normal, view_space_geometric_normal, true
				);
			} else {
				screen_space_indirect = sample_screen_space_indirect_diffuse(
					material_evaluation,
					pixel_coordinates, image_extent, view_space_surface_position, view_space_normal, view_space_normal, false
				);
			}
			// Both sources carry pre-exposed light through half-float images. Undo that scale before combining it
			// with physical lighting, so the final exposure below applies exactly once.
			screen_space_irradiance = vec3f(screen_space_indirect.x, screen_space_indirect.y, screen_space_indirect.z)
				/ material_evaluation.lighting_data.exposure;
			// Filter rounding can push the hit fraction past one. The clamp keeps the base of the specular fit's pow nonnegative.
			environment_visibility = clamp(1.0 - screen_space_indirect.w, 0.0, 1.0);
		}
		// The value 2 means surfel GI lit the frame, and its rays gathered the environment's light where they missed.
		if (push_constant.indirect_diffuse == 2) {
			adds_environment_diffuse = false;
		}
		if (push_constant.gtao != 0) {
			let gtao_visibility: f32 = fetch(material_evaluation.ao, pixel_coordinates).x;
			// The half-resolution bent normal blends between texels. GTAO stores its two components across the view
			// vector in the frame t = normalize(up × V), b = V × t; the view's rotation carries that frame to world
			// space, where the view's up axis and V rebuild it. The bent normal lies in the hemisphere around V, which
			// gives its third component.
			let encoded_bent_normal: vec4f = texture_lod(
				material_evaluation.gtao_bent_normal,
				vec2f(
					(f32(pixel_coordinates.x) + 0.5) / f32(image_extent.x),
					(f32(pixel_coordinates.y) + 0.5) / f32(image_extent.y)
				),
				0.0
			);
			let bent_x: f32 = encoded_bent_normal.x * 2.0 - 1.0;
			let bent_y: f32 = encoded_bent_normal.y * 2.0 - 1.0;
			let view_up: vec3f = scene.views.views[0].inverse_view * vec4f(0.0, 1.0, 0.0, 0.0);
			let frame_t: vec3f = normalize(cross(view_up, V));
			let frame_b: vec3f = cross(V, frame_t);
			bent_normal = normalize(
				frame_t * bent_x + frame_b * bent_y + V * sqrt(max(1.0 - bent_x * bent_x - bent_y * bent_y, 0.0))
			);
			// Surfel, probe, and lightmap light already holds the occlusion their rays found, and `environment_visibility`
			// is still the share of them that reached the environment, so GTAO adds only what it finds beyond that.
			if (push_constant.indirect_diffuse == 2 || !adds_environment_diffuse) {
				screen_space_irradiance = screen_space_irradiance
					* traced_light_occlusion(gtao_visibility, environment_visibility);
			}
			// Both passes estimate the same visibility, so multiplying them would darken an occluder they both see
			// twice. The smaller one keeps an occluder that only one of them resolves, such as a crease below SSGI's
			// half resolution.
			environment_visibility = min(environment_visibility, gtao_visibility);
		}
	}
	// Environment maps store arbitrary units; the intensity calibrates them to the lux their Environment requests.
	let indirect_diffuse_radiance: vec3f = screen_space_irradiance;
	if (adds_environment_diffuse) {
		indirect_diffuse_radiance = indirect_diffuse_radiance
			+ sample_environment_irradiance(material_evaluation, vec3f(normal)) * (material_evaluation.lighting_data.environment_intensity * environment_visibility);
	}
	// The irradiance debug view sums every light's irradiance onto the indirect one; nothing else reads the sum, so a
	// bake without debug views drops it with the gated additions below.
	let total_irradiance: vec3f = indirect_diffuse_radiance;
	// The BRDF constants come after the screen-space reads above, so they are not live while those fetches wait.
	// Preserve compact material values and normalized vectors through the BRDF.
	// Positions, shadow projections, HDR radiance, and accumulation remain f32.
	let albedo_rgb: vec3f16 = vec3f16(albedo.x, albedo.y, albedo.z);
	let one_minus_metalness: f16 = f16(1.0) - metalness;
	let F0: vec3f16 = vec3f16(0.04, 0.04, 0.04) * one_minus_metalness + albedo_rgb * metalness;
	let NdotV: f16 = max(dot(normal, V_material), f16(0.0));
	let roughness_alpha: f16 = roughness * roughness;
	// The α lights see, at least Filament's single-precision floor of 0.045², so a mirror-smooth lobe, such as calm
	// water's, peaks at a finite value. See `trowbridge_reitz`.
	let light_lobe_alpha: f32 = max(f32(roughness) * f32(roughness), 0.002025);
	let adjusted_roughness: f16 = roughness + 1.0;
	let geometry_k: f16 = adjusted_roughness * adjusted_roughness / 8.0;
	// These terms depend only on the shaded pixel. Evaluate them once instead of once per light.
	let geometry_view: f16 = NdotV / (NdotV * (1.0 - geometry_k) + geometry_k);
	// Diffuse light takes the share the specular lobe leaves on the way out toward the viewer, and indirect light, which
	// arrives from the whole hemisphere, the average share on the way in. See the lighting design note.
	// A subsurface material weights its diffuse light by the square root of its albedo where the light enters, and the
	// subsurface pass applies the other square root where the light leaves, after spreading it. Both halves keep the
	// albedo's detail while the light bleeds. See the subsurface scattering design note.
	// Whether this pixel's diffuse light scatters; a transparent subsurface material doesn't.
	let subsurface_scatters: bool = !transparent && subsurface_radius.x >= f16(0.0);
	let diffuse_albedo_rgb: vec3f16 = albedo_rgb;
	if (subsurface_scatters) {
		diffuse_albedo_rgb = vec3f16(sqrt(albedo_rgb.x), sqrt(albedo_rgb.y), sqrt(albedo_rgb.z));
	}
	// Light on the far side of a subsurface material crosses its thickness, losing what the scattering profile
	// carries elsewhere, and enters weighted like the near side's light. A transparent subsurface material doesn't
	// scatter, so the share it passes is the full albedo's.
	if (subsurface_thickness >= f16(0.0)) {
		translucency = diffuse_albedo_rgb
			* burley_transmittance(subsurface_thickness, burley_distance(subsurface_radius, albedo_rgb));
	}
	let indirect_diffuse_albedo: vec3f16 = one_minus_metalness * diffuse_transmission(NdotV, roughness) * diffuse_albedo_rgb;
	// Every factor of the direct diffuse weight except the share each light's angle leaves. Folding them here keeps
	// the albedo, metalness, and view terms out of the light loop: of the shader's regions, the loop slows down the most
	// for every value held live across it (2026-10-05).
	let direct_diffuse_albedo: vec3f16 = indirect_diffuse_albedo
		/ (f16(3.14159265359) * (f16(1.0) - average_specular_albedo(roughness)));
	// Indirect diffuse light and emission seed the diffuse sum, so neither they nor the material terms that weight them
	// stay live through the loop. Material occlusion, like baked AO, applies to indirect light only. Direct light has
	// its own shadows.
	let diffuse: vec3f = vec3f(indirect_diffuse_albedo) * indirect_diffuse_radiance * f32(occlusion) + vec3f(emission);
	// A translucent surface is a thin sheet: `translucency` is the share of the light entering one face that leaves
	// the other, as the albedo is the share that leaves the face it entered. Light on the far side of the surface
	// therefore takes the diffuse weights with that share in place of the albedo. See the lighting design note.
	// A material that is not translucent drops every statement that reads these locals.
	let indirect_translucency: vec3f16 = one_minus_metalness * diffuse_transmission(NdotV, roughness) * translucency;
	let direct_translucency: vec3f16 = indirect_translucency
		/ (f16(3.14159265359) * (f16(1.0) - average_specular_albedo(roughness)));
	// Where the surface passes no light, such as a stem in a leaf atlas, far-side lights are skipped before their
	// shadows are filtered.
	let passes_light: bool = max(max(translucency.x, translucency.y), translucency.z) > 0.0;
	// The environment lights the far side too. Screen-space and ray-traced light and lightmaps are gathered on the
	// near side only, so they add nothing here.
	if (passes_light && adds_environment_diffuse) {
		diffuse = diffuse + vec3f(indirect_translucency)
			* sample_environment_irradiance(material_evaluation, vec3f(0.0, 0.0, 0.0) - vec3f(normal))
			* (material_evaluation.lighting_data.environment_intensity * environment_visibility * f32(occlusion));
	}
	let specular: vec3f = vec3f(0.0, 0.0, 0.0);
	let light_count: u32 = material_evaluation.lighting_data.light_count;

	// Visit only the lights the light-cluster pass bucketed into this pixel's cluster: 16 columns, 8 rows, and 24
	// depth slices that grow exponentially with view depth. Each cluster stores one bit per light.
	let cluster_column: u32 = u32(min((f32(pixel_coordinates.x) + 0.5) * 16.0 / f32(image_extent.x), 15.0));
	let cluster_row: u32 = u32(min((f32(pixel_coordinates.y) + 0.5) * 8.0 / f32(image_extent.y), 7.0));
	let cluster_slice: u32 = u32(clamp(
		floor(log2(perspective_w / material_evaluation.light_cluster_parameters.near) * material_evaluation.light_cluster_parameters.depth_slice_scale),
		0.0,
		23.0
	));
	let cluster_mask_base: u32 = ((cluster_slice * 8 + cluster_row) * 16 + cluster_column) * 32;
	let cluster_mask_word_count: u32 = (light_count + 31) >> 5;

	for (let mask_word: u32 = 0; mask_word < cluster_mask_word_count; mask_word = mask_word + 1) {
		// Each pass takes the lowest remaining light and clears its bit, so `continue` moves on to the next light.
		for (
			let light_bits: u32 = material_evaluation.light_cluster_masks.words[cluster_mask_base + mask_word];
			light_bits != 0;
			light_bits = light_bits & (light_bits - 1)
		) {
			let light_index: u32 = mask_word * 32 + find_lsb(light_bits);
			let light_type: u32 = material_evaluation.lighting_data.lights[light_index].type;
			let L: vec3f = vec3f(0.0, 0.0, 0.0);
			let attenuation: f32 = 1.0;
			let light_position: vec3f = material_evaluation.lighting_data.lights[light_index].position;
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
			// The cosine of the light on the side of the surface it lights. A negative cosine puts the light on the far
			// side, where it lights only a surface that passes light.
			let light_cosine: f16 = dot(normal, L_material);
			let NdotL: f16 = max(light_cosine, f16(0.0));
			if (passes_light && light_cosine < 0.0) {
				NdotL = f16(0.0) - light_cosine;
			}
			if (NdotL <= 0.0) {
				continue;
			}

			let occlusion_factor: f16 = 1.0;
			if (light_type == 68) {
				let shadow_view0: u32 = material_evaluation.lighting_data.lights[light_index].shadow_views[0];
				if (shadow_view0 != 0) {
					if (!transparent) {
						// The sun visibility pass resolved every sun's shadow map and contact shadows for every opaque
						// pixel, sun slot `s` in channel `s`.
						let sun_slot: u32 = material_evaluation.lighting_data.lights[light_index].shadow_layer;
						let suns_visibility: vec4f = fetch(material_evaluation.sun_visibility, pixel_coordinates);
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
						let shadow_view1: u32 = material_evaluation.lighting_data.lights[light_index].shadow_views[1];
						let shadow_view2: u32 = material_evaluation.lighting_data.lights[light_index].shadow_views[2];
						let shadow_view3: u32 = material_evaluation.lighting_data.lights[light_index].shadow_views[3];
						// The same normal offset as the resolve's: sin(theta) texels along the triangle normal on the
						// sun's side, where theta lies between that normal and the direction toward the sun. On the
						// camera's side, a surface lit from behind would move behind its own depth in the map.
						let sun_cosine: f32 = dot(geometric_normal, L);
						let normal_offset: vec3f = geometric_normal * sqrt(max(1.0 - sun_cosine * sun_cosine, 0.0));
						if (sun_cosine < 0.0) {
							normal_offset = normal_offset * (0.0 - 1.0);
						}
						occlusion_factor = f16(sample_directional_shadow(
							scene, material_evaluation,
							material_evaluation.depth_shadow_map,
							shadow_view0,
							shadow_view1,
							shadow_view2,
							shadow_view3,
							material_evaluation.lighting_data.lights[light_index].angular_radius_tangent,
							world_space_vertex_position,
							perspective_w,
							position_derivative_x,
							position_derivative_y,
							normal_offset
						));
					}
					if (occlusion_factor == 0.0) {
						continue;
					}
				}
				attenuation = 1.0;
			} else {
				// The cheap factors come first: the cone's falloff, then the IES profile's one texel. Either can drop the
				// light, and then the shadow filter, which reads eight or sixteen texels, never runs.
				if (light_type == 1) {
					let cone_direction: vec3f16 = vec3f16(material_evaluation.lighting_data.lights[light_index].direction);
					let cone_cosine: f16 = dot(cone_direction, vec3f16(0.0, 0.0, 0.0) - L_material);
					let cone_factor: f16 = f16(cone_attenuation(
						f32(cone_cosine),
						material_evaluation.lighting_data.lights[light_index].cone_cosines.x,
						material_evaluation.lighting_data.lights[light_index].cone_cosines.y
					));
					if (cone_factor <= 0.0) {
						continue;
					}
					attenuation = attenuation * f32(cone_factor);
				}
				if (material_evaluation.lighting_data.lights[light_index].ies_profile != 4294967295) {
					let emission_direction: vec3f = vec3f(0.0, 0.0, 0.0) - L;
					let profile_axis: vec3f = material_evaluation.lighting_data.lights[light_index].direction;
					let intensity_factor: f32 = sample_ies_profile(
						scene,
						material_evaluation.lighting_data.lights[light_index].ies_profile,
						emission_direction,
						profile_axis,
						material_evaluation.lighting_data.lights[light_index].ies_c0_tangent
					);
					if (intensity_factor <= 0.0) {
						continue;
					}
					attenuation = attenuation * intensity_factor;
				}
				let shadow_view_index: u32 = material_evaluation.lighting_data.lights[light_index].shadow_views[0];
				if (shadow_view_index != 0) {
					let shadow_layer: u32 = material_evaluation.lighting_data.lights[light_index].shadow_layer;
					let shadow_size_class: u32 = material_evaluation.lighting_data.lights[light_index].shadow_size_class;
					if (light_type == 0) {
						// Size class `c` holds cube faces of `128 << c` texels per side; `shadow_layer` is the cube.
						occlusion_factor = f16(sample_point_shadow(
							scene,
							material_evaluation.point_shadow_maps[shadow_size_class],
							f32(128 << shadow_size_class),
							shadow_view_index,
							shadow_layer,
							world_space_vertex_position,
							light_position,
							position_derivative_x,
							position_derivative_y
						));
					} else if (light_type == 1) {
						occlusion_factor = f16(sample_cone_shadow(
							scene,
							material_evaluation.cone_shadow_maps[shadow_size_class],
							shadow_view_index,
							shadow_layer,
							world_space_vertex_position,
							position_derivative_x,
							position_derivative_y
						));
					}
					if (occlusion_factor == 0.0) {
						continue;
					}
				}
			}

			// Light that crossed the surface left its specular share on the far face, so it adds diffuse light only.
			if (passes_light && light_cosine < 0.0) {
				diffuse = diffuse + vec3f(direct_translucency * diffuse_transmission(NdotL, roughness))
					* material_evaluation.lighting_data.lights[light_index].color
					* (attenuation * f32(NdotL * occlusion_factor));
				continue;
			}

			let H: vec3f16 = normalize(V_material + L_material);
			let half_view_fresnel_base: f16 = clamp(f16(1.0) - max(dot(H, V_material), f16(0.0)), f16(0.0), f16(1.0));
			let half_view_fresnel_squared: f16 = half_view_fresnel_base * half_view_fresnel_base;
			let half_view_fresnel_factor: f16 = half_view_fresnel_squared * half_view_fresnel_squared * half_view_fresnel_base;
			// Schlick's F0 + (1 - F0) f, written so that 1 - F0 is not kept live through the loop.
			let F: vec3f16 = F0 * (f16(1.0) - half_view_fresnel_factor)
				+ vec3f16(half_view_fresnel_factor, half_view_fresnel_factor, half_view_fresnel_factor);
			let NDF: f32 = trowbridge_reitz(vec3f(normal), vec3f(H), light_lobe_alpha);
			let geometry_light: f16 = NdotL / (NdotL * (1.0 - geometry_k) + geometry_k);
			let local_specular: vec3f = vec3f(F)
				* (NDF * f32(geometry_view * geometry_light) / (4.0 * f32(NdotV) * f32(NdotL) + 0.000001));
			let local_diffuse: vec3f16 = direct_diffuse_albedo * diffuse_transmission(NdotL, roughness);
			let light_color: vec3f = material_evaluation.lighting_data.lights[light_index].color;
			let irradiance: vec3f = light_color * (attenuation * f32(NdotL * occlusion_factor));
			diffuse = diffuse + vec3f(local_diffuse) * irradiance;
			specular = specular + local_specular * irradiance;
			// The irradiance view of `MATERIAL_DEBUG_VIEWS`.
			if (push_constant.debug_view_irradiance) {
				total_irradiance = total_irradiance + irradiance;
			}
		}
	}

	// The half-precision view vector the lights above used. Keeping the f32 one live through the light loop for this
	// alone made material evaluation 1.7 % slower (2026-10-05), and it already bends the mirror ray no more than the
	// half-precision material normal does.
	let incident: vec3f = vec3f(0.0, 0.0, 0.0) - vec3f(V_material);
	let reflection_direction: vec3f = incident - 2.0 * dot(incident, vec3f(normal)) * vec3f(normal);
	// SSGI rays read this one frame later. View-dependent specular is left out: a surface receives the light that
	// leaves a neighbor toward it, not the highlight the camera sees, and highlights would turn into sparkling noise.
	// Pre-exposure keeps daylight radiance within the half-float range. SSGI rescales previous-frame light when
	// exposure changes, and the material consumer removes that scale before combining it with physical lighting.
	// Alpha keeps the view depth, the clip w of this pixel, so a ray can tell which pixel belongs to the surface it hit.
	// It is written before the reflection trace, which cannot change it, so the diffuse sum does not stay live through
	// the ray march. Only SSGI reads the diffuse light, so it is written only in frames that SSGI lights.
	// A subsurface pixel leaves its diffuse light, less emission, for the subsurface pass to spread, with what the pass
	// needs to shape the spread and color the result. Every other opaque pixel marks itself as not scattering, so the
	// pass needs no clear. SSGI rays then read the light the pixel would leave unscattered: the other square root of
	// the albedo, applied in place.
	if (subsurface_scatters) {
		let subsurface_light: vec3f = diffuse - vec3f(emission);
		let subsurface_exposed: vec3f = subsurface_light * material_evaluation.lighting_data.exposure;
		write(
			material_evaluation.subsurface_diffuse,
			pixel_coordinates,
			vec4f(subsurface_exposed.x, subsurface_exposed.y, subsurface_exposed.z, 1.0)
		);
		let subsurface_distance: vec3f16 = burley_distance(subsurface_radius, albedo_rgb);
		write(
			material_evaluation.subsurface_shape,
			pixel_coordinates,
			vec4f(f32(subsurface_distance.x), f32(subsurface_distance.y), f32(subsurface_distance.z), 0.0)
		);
		write(
			material_evaluation.subsurface_albedo,
			pixel_coordinates,
			vec4f(f32(diffuse_albedo_rgb.x), f32(diffuse_albedo_rgb.y), f32(diffuse_albedo_rgb.z), 1.0)
		);
		diffuse = vec3f(emission) + subsurface_light * vec3f(diffuse_albedo_rgb);
	} else {
		if (!transparent) {
			write(material_evaluation.subsurface_albedo, pixel_coordinates, vec4f(0.0, 0.0, 0.0, 0.0));
		}
	}
	if (!transparent) {
		if (push_constant.ssgi != 0) {
			let diffuse_radiance: vec3f = diffuse * material_evaluation.lighting_data.exposure;
			write(
				material_evaluation.diffuse_radiance_map,
				pixel_coordinates,
				vec4f(diffuse_radiance.x, diffuse_radiance.y, diffuse_radiance.z, perspective_w)
			);
		}
	}
	// Visibility covers the cosine-weighted hemisphere, but a reflection gathers light from a lobe around the mirror
	// direction that narrows as roughness falls: keep the share of that lobe the open directions let through. See
	// `cone_specular_occlusion`. Reflection rays stop at nearby geometry, so the light they find carries its own occlusion.
	let specular_occlusion: f32 = cone_specular_occlusion(bent_normal, reflection_direction, environment_visibility, f32(roughness_alpha));
	let env_brdf: vec2f16 = environment_brdf(NdotV, roughness);
	// The split-sum reflectance toward the viewer, and the weight of reflected light, with the material occlusion that
	// applies to indirect light.
	let specular_reflectance: vec3f = vec3f(F0 * env_brdf.x + env_brdf.y);
	let specular_weight: vec3f = specular_reflectance * f32(occlusion);
	// The subsurface pass adds a subsurface pixel's diffuse light once it has spread it, so the lit image holds only
	// the light that doesn't scatter.
	if (subsurface_scatters) {
		diffuse = vec3f(emission);
	}
	// Everything but the reflected light is known before the trace, so only these sums stay live through the march.
	let direct_and_diffuse: vec3f = diffuse + specular;
	// Screen-space reflections replace the environment where the mirror ray finds visible geometry. One mirror ray
	// cannot stand for a wide glossy lobe, so reflections fade back to the prefiltered environment from roughness 0.2
	// to 0.4. A reflection that points into the geometric surface would only find the surface itself.
	let screen_space_reflection: vec4f = vec4f(0.0, 0.0, 0.0, 0.0);
	if (f32(roughness) < 0.4 && dot(reflection_direction, geometric_normal) > 0.0) {
		screen_space_reflection = trace_screen_space_reflection(
			material_evaluation,
			world_space_vertex_position,
			geometric_normal,
			reflection_direction,
			scene.views.views[0].view_projection
		);
	}
	let reflection_weight: f32 = screen_space_reflection.w * clamp((0.4 - f32(roughness)) * 5.0, 0.0, 1.0);
	// Sample the environment after the screen-space trace so its radiance does not stay live through the march.
	let reflection_radiance: vec3f = sample_environment_specular(material_evaluation, reflection_direction, f32(roughness))
		* material_evaluation.lighting_data.environment_intensity;
	// A normal map can tilt the mirror direction toward or past the surface the geometry has, where the environment
	// can't be seen. See `horizon_occlusion`.
	let environment_specular: vec3f = reflection_radiance
		* (specular_occlusion * horizon_occlusion(reflection_direction, geometric_normal));
	let specular_radiance: vec3f = environment_specular * (1.0 - reflection_weight)
		+ vec3f(screen_space_reflection.x, screen_space_reflection.y, screen_space_reflection.z) * reflection_weight;
	// Pre-expose: store light already multiplied by the camera exposure, so real-world intensities such as a
	// 100,000 lux sun on a glossy surface stay within the half-float range of the lit map.
	let lit: vec3f = (direct_and_diffuse + specular_weight * specular_radiance) * material_evaluation.lighting_data.exposure;
	// Light crossing the surface reaches the viewer where the surface reflects none, so the reflected and the
	// transmitted light never add to more than the light they share. A material that writes no transmission drops this
	// statement.
	lit = lit
		+ transmission * (vec3f(1.0, 1.0, 1.0) - specular_reflectance) * material_evaluation.lighting_data.exposure;
	// Only the irradiance debug view reads this; a bake without debug views leaves it dead.
	let exposed_irradiance: vec3f = total_irradiance * material_evaluation.lighting_data.exposure;
	let output_color: vec4f = vec4f(lit.x, lit.y, lit.z, 1.0);
	if (transparent) {
		let source_alpha: f32 = f32(clamp(albedo.w, f16(0.0), f16(1.0)));
		let destination_color: vec4f = image_load(material_evaluation.lit_map, pixel_coordinates);
		output_color = source_over(
			vec4f(lit.x * source_alpha, lit.y * source_alpha, lit.z * source_alpha, source_alpha),
			destination_color
		);
	}
	// The next frame's reflection rays read this light, highlights included, at the pixels their hits land on.
	write(material_evaluation.lit_map, pixel_coordinates, output_color);
}
"#;

// Computes the projected receiver plane so cone PCF compares each texel against the depth at that texel's center.
// Returning no correction for a degenerate projection keeps the existing bias as a safe fallback.

pub(crate) const U16_TO_U32_SOURCE: &str = "u16_to_u32: fn (value: u16) -> u32 { return u32(value); }";

pub(crate) const CONE_ATTENUATION_SOURCE: &str = "cone_attenuation: fn (cosine: f32, inner_cosine: f32, outer_cosine: f32) -> f32 { return clamp((cosine - outer_cosine) / (inner_cosine - outer_cosine), 0.0, 1.0); }";

// Resolve all three triangle vertices together so the instance's vertex base is read once.
pub(crate) const COMPUTE_VERTEX_INDICES_SOURCE: &str = r#"
compute_vertex_indices: fn (scene: Scene, mesh: Mesh, meshlet: Meshlet, primitive_index_base: u32) -> u32[3] {
	let vertex_index_base: u32 = instance_vertex_base(mesh, scene.mesh_levels[mesh.level & MESH_LEVEL_MASK]);
	let relative_index_base: u32 = meshlet.primitive_offset;
	let primitive_index0: u32 = u32(scene.primitive_indices[primitive_index_base]);
	let primitive_index1: u32 = u32(scene.primitive_indices[primitive_index_base + 1]);
	let primitive_index2: u32 = u32(scene.primitive_indices[primitive_index_base + 2]);
	return u32[3](
		vertex_index_base + u16_to_u32(scene.vertex_indices[relative_index_base + primitive_index0]),
		vertex_index_base + u16_to_u32(scene.vertex_indices[relative_index_base + primitive_index1]),
		vertex_index_base + u16_to_u32(scene.vertex_indices[relative_index_base + primitive_index2])
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
