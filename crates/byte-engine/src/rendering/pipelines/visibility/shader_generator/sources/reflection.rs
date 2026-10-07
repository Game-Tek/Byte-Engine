// Screen-space reflection helpers. Material evaluation traces one mirror ray per pixel through this frame's linear
// depth pyramid, and reads the light at the hit from last frame's radiance history.
//
// The helpers read the `reflection_parameters` and `previous_radiance` bindings that `screen_space_reflection_scope`
// declares, and the shared `depth_pyramid` binding. Each helper only calls helpers declared above it.

// Returns whether the ray lies behind the depth buffer at `fraction` of its screen path. Screen position and 1/z are
// both linear along the path, so the test compares inverse depths: the ray is behind a surface when its 1/z is the
// smaller one, `scene_z * inverse_z < 1`, which needs no division. Background holds zero depth and nothing to hit.
//
// The pyramid's point sampler reads the texel under the coordinate and clamps to the image's edge, so a step needs no
// rounding, clamping, or conversion to integer coordinates of its own. Together with the bit-mask scan in
// `trace_screen_space_reflection`, this made material evaluation 2.2 % faster on the Sponza hall view, and fetching the
// texel at converted integer coordinates instead was 3 % slower than sampling it (2026-10-06).
pub(crate) const REFLECTION_RAY_IS_BEHIND_SOURCE: &str = r#"
reflection_ray_is_behind: fn (ray: ReflectionRay, fraction: f32) -> bool {
	// Mip zero of the pyramid holds half-resolution positive linear depth.
	let scene_z: f32 = texture_lod(depth_pyramid, mix(ray.start_uv, ray.end_uv, fraction), 0.0).x;
	return scene_z != 0.0 && scene_z * mix(ray.inverse_start_z, ray.inverse_end_z, fraction) < 1.0;
}
"#;

// Converts a fraction of a ray's screen path into the fraction of its world-space length. Depth is linear along the
// world-space ray, while 1/depth is linear along its screen path.
pub(crate) const REFLECTION_WORLD_FRACTION_SOURCE: &str = r#"
reflection_world_fraction: fn (screen_fraction: f32, start_z: f32, end_z: f32) -> f32 {
	return screen_fraction * start_z / ((1.0 - screen_fraction) * end_z + screen_fraction * start_z);
}
"#;

// Reads last frame's light where the previous camera saw `world_position`, converted back to unexposed radiance.
// Returns zero alpha when the previous frame did not see that surface.
//
// The trace finds hits in half-resolution depth, which keeps the nearest of each 2x2 pixel block, while radiance is
// stored per full-resolution pixel. Next to an object's edge, the pixel under the hit can belong to the background.
// So the four pixels around the hit are read, and only the one whose stored depth matches the hit supplies light.
pub(crate) const REFLECTION_HISTORY_RADIANCE_SOURCE: &str = r#"
reflection_history_radiance: fn (world_position: vec3f) -> vec4f {
	let miss: vec4f = vec4f(0.0, 0.0, 0.0, 0.0);
	let clip: vec4f = reflection_parameters.world_to_previous_clip * vec4f(
		world_position.x,
		world_position.y,
		world_position.z,
		1.0
	);
	if (clip.w <= 0.0) {
		return miss;
	}
	let uv: vec2f = vec2f(0.5 + 0.5 * clip.x / clip.w, 0.5 - 0.5 * clip.y / clip.w);
	if (uv.x < 0.0 || uv.x > 1.0 || uv.y < 0.0 || uv.y > 1.0) {
		return miss;
	}
	let radiance_extent: vec2u = texture_size(previous_radiance);
	let texel_position: vec2f = vec2f(
		uv.x * f32(radiance_extent.x) - 0.5,
		uv.y * f32(radiance_extent.y) - 0.5
	);
	let color: vec4f = miss;
	// A pixel belongs to the hit only when its stored depth is within this fraction of the hit's depth.
	let best_error: f32 = 0.05;
	for (let tap: u32 = 0; tap < 4; tap = tap + 1) {
		let tap_texel: vec2u = vec2u(
			u32(clamp(floor(texel_position.x) + f32(tap % 2), 0.0, f32(radiance_extent.x - 1))),
			u32(clamp(floor(texel_position.y) + f32(tap / 2), 0.0, f32(radiance_extent.y - 1)))
		);
		let tap_color: vec4f = fetch(previous_radiance, tap_texel);
		// The previous camera's clip w is its view depth, the same quantity each pixel stored in alpha.
		let error: f32 = abs(tap_color.w - clip.w) / clip.w;
		if (error < best_error) {
			best_error = error;
			color = tap_color;
		}
	}
	if (color.w == 0.0) {
		return miss;
	}
	// The history holds light already multiplied by last frame's exposure, which keeps highlights in half-float range.
	let radiance: vec3f = vec3f(color.x, color.y, color.z) / reflection_parameters.previous_exposure;
	// One non-finite texel would otherwise spread through every later frame's reflections.
	if (is_finite(radiance.x + radiance.y + radiance.z) == false) {
		return miss;
	}
	return vec4f(max(radiance.x, 0.0), max(radiance.y, 0.0), max(radiance.z, 0.0), 1.0);
}
"#;

// Traces one reflection ray from a surface and returns the unexposed radiance it finds in RGB and how much to trust
// it in alpha. Zero alpha means the ray found nothing on screen, so the caller keeps the environment.
//
// `position` is the world-space surface point, `normal` its geometric normal, and `direction` the unit reflection
// direction. `view_projection` is the camera that drew this frame's depth.
pub(crate) const TRACE_SCREEN_SPACE_REFLECTION_SOURCE: &str = r#"
trace_screen_space_reflection: fn (
	position: vec3f,
	normal: vec3f,
	direction: vec3f,
	view_projection: mat4f
) -> vec4f {
	let miss: vec4f = vec4f(0.0, 0.0, 0.0, 0.0);
	if (reflection_parameters.history_valid == 0) {
		return miss;
	}
	// View depth changes along the ray at this rate per world unit. A ray heading toward the camera can only reach
	// the sides of objects that face away from the camera, which the depth buffer does not hold, so it fades out.
	let direction_clip: vec4f = view_projection * vec4f(direction.x, direction.y, direction.z, 0.0);
	let facing_fade: f32 = clamp(1.0 + 2.0 * direction_clip.w, 0.0, 1.0);
	if (facing_fade == 0.0) {
		return miss;
	}

	// Move the origin off the surface, relative to its distance, so the first steps do not hit the start pixel.
	let surface_clip: vec4f = view_projection * vec4f(position.x, position.y, position.z, 1.0);
	let origin: vec3f = position + normal * (surface_clip.w * 0.002);
	// World-space reach of one ray. Reflections of geometry further away than this come from the environment.
	let max_distance: f32 = 32.0;
	let ray_length: f32 = max_distance;
	let far_point: vec3f = origin + direction * ray_length;
	let start_clip: vec4f = view_projection * vec4f(origin.x, origin.y, origin.z, 1.0);
	let end_clip: vec4f = view_projection * vec4f(far_point.x, far_point.y, far_point.z, 1.0);
	// Stop rays in front of the camera plane so every step keeps a positive depth.
	let minimum_z: f32 = start_clip.w * 0.05;
	if (end_clip.w < minimum_z) {
		let kept_fraction: f32 = (start_clip.w - minimum_z) / (start_clip.w - end_clip.w);
		end_clip = mix(start_clip, end_clip, kept_fraction);
		ray_length = ray_length * kept_fraction;
	}

	// Rays march the half-resolution depth in normalized image coordinates, which the depth pyramid's texels divide
	// evenly at any resolution.
	let origin_uv: vec2f = vec2f(0.5 + 0.5 * start_clip.x / start_clip.w, 0.5 - 0.5 * start_clip.y / start_clip.w);
	let far_uv: vec2f = vec2f(0.5 + 0.5 * end_clip.x / end_clip.w, 0.5 - 0.5 * end_clip.y / end_clip.w);
	// Cut the screen path where it leaves the image, so every step lands on screen and none are wasted.
	let path: vec2f = far_uv - origin_uv;
	let exit_fraction: f32 = 1.0;
	if (path.x > 0.0) {
		exit_fraction = min(exit_fraction, (1.0 - origin_uv.x) / path.x);
	} else if (path.x < 0.0) {
		exit_fraction = min(exit_fraction, -origin_uv.x / path.x);
	}
	if (path.y > 0.0) {
		exit_fraction = min(exit_fraction, (1.0 - origin_uv.y) / path.y);
	} else if (path.y < 0.0) {
		exit_fraction = min(exit_fraction, -origin_uv.y / path.y);
	}
	if (exit_fraction <= 0.0) {
		return miss;
	}
	// Screen position and 1/z are both linear along the projected ray, so cutting both at the same fraction keeps the
	// ray on its line.
	ray_length = ray_length * reflection_world_fraction(exit_fraction, start_clip.w, end_clip.w);
	let inverse_end_z: f32 = mix(1.0 / start_clip.w, 1.0 / end_clip.w, exit_fraction);
	let ray: ReflectionRay = ReflectionRay(
		origin_uv,
		mix(origin_uv, far_uv, exit_fraction),
		1.0 / start_clip.w,
		inverse_end_z
	);
	let end_z: f32 = 1.0 / inverse_end_z;

	// The half-resolution pyramid keeps the nearest depth of each 2x2 block, which can put the ray's own surface in
	// front of its first steps. A crossing only counts once the ray was seen in front of the depth buffer.
	let previous_fraction: f32 = 0.0;
	let was_in_front: bool = false;
	let step_count: u32 = 32;
	// Steps are read eight at a time. The eight depth reads of a batch do not depend on each other, so they overlap
	// instead of each waiting for the last, which the material shader's few resident threads cannot hide. The steps
	// are still judged in order, and the march resumes right after a crossing that turns out not to be a hit, so the
	// result is the same as stepping one read at a time. On the Sponza hall view this made material evaluation about
	// 14 % faster than single steps; batches of 16 were no faster and batches of 32 spilled registers (2026-10-05).
	let next_step: u32 = 0;
	for (let batch: u32 = 0; batch < step_count; batch = batch + 1) {
		if (next_step >= step_count) {
			break;
		}
		// Bit `lane` is set when that step of the batch lies behind the depth buffer. Steps past the end read the last
		// step again; the valid mask below ignores them.
		let behind_steps: u32 = 0;
		let first_fraction: f32 = f32(next_step + 1) / f32(step_count);
		for (let lane: u32 = 0; lane < 8; lane = lane + 1) {
			let lane_fraction: f32 = min(first_fraction + f32(lane) / f32(step_count), 1.0);
			behind_steps = behind_steps | (reflection_ray_is_behind(ray, lane_fraction) ? 1 << lane : 0);
		}
		// The steps are judged in order with bit operations instead of a loop over them. A crossing is a step behind the
		// depth buffer after the ray was seen in front of it: in an earlier batch, or at an earlier step of this one. A
		// ray that is still behind the surface it went behind at an earlier step crosses nothing new.
		let remaining_steps: u32 = step_count - next_step;
		let valid_steps: u32 = remaining_steps < 8 ? (1 << remaining_steps) - 1 : 255;
		let front_steps: u32 = valid_steps & ~behind_steps;
		let crossable_steps: u32 = was_in_front ? valid_steps : (front_steps != 0 ? 255 << (find_lsb(front_steps) + 1) : 0);
		let crossing: u32 = find_lsb(behind_steps & valid_steps & crossable_steps);
		if (crossing == 4294967295) {
			// Every valid step after the first one in front is in front too, so the last valid step is.
			if (front_steps != 0) {
				previous_fraction = f32(next_step + min(remaining_steps, 8)) / f32(step_count);
				was_in_front = true;
			}
			next_step = next_step + 8;
			continue;
		}
		// Every step between the first one in front and the crossing is in front, so the step before the crossing is
		// the last one in front; a crossing at the first step keeps the earlier batch's.
		if (crossing > 0) {
			previous_fraction = f32(next_step + crossing) / f32(step_count);
		}
		let fraction: f32 = f32(next_step + crossing + 1) / f32(step_count);
		next_step = next_step + crossing + 1;
		was_in_front = false;

		// The ray went behind the depth buffer since the last step. Bisect for where it crossed.
		let front: f32 = previous_fraction;
		let behind: f32 = fraction;
		for (let refinement: u32 = 0; refinement < 5; refinement = refinement + 1) {
			let middle: f32 = 0.5 * (front + behind);
			if (reflection_ray_is_behind(ray, middle)) {
				behind = middle;
				continue;
			}
			front = middle;
		}
		// The depth buffer stores only front faces. A ray further behind a surface than this passed behind the
		// object instead of hitting it, and keeps marching.
		let hit_uv: vec2f = mix(ray.start_uv, ray.end_uv, behind);
		let scene_z: f32 = texture_lod(depth_pyramid, hit_uv, 0.0).x;
		if (1.0 / mix(ray.inverse_start_z, ray.inverse_end_z, behind) - scene_z >= 0.05 + 0.02 * scene_z) {
			continue;
		}

		let hit_distance: f32 = ray_length * reflection_world_fraction(behind, start_clip.w, end_z);
		let radiance: vec4f = reflection_history_radiance(origin + direction * hit_distance);
		if (radiance.w == 0.0) {
			return miss;
		}
		// Fade hits near the image border and near the end of the ray's reach, so reflections do not end in a hard
		// line where the screen or the ray runs out.
		let border_distance: f32 = min(min(hit_uv.x, 1.0 - hit_uv.x), min(hit_uv.y, 1.0 - hit_uv.y));
		let border_fade: f32 = clamp(border_distance * 20.0, 0.0, 1.0);
		let distance_fade: f32 = clamp((max_distance - hit_distance) / (0.25 * max_distance), 0.0, 1.0);
		return vec4f(radiance.x, radiance.y, radiance.z, facing_fade * border_fade * distance_fade);
	}
	return miss;
}
"#;
