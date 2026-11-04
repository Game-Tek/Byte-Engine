//! Runs the lightmap bake's kernels in the BESL VM: the trace against small scenes, and the resolve and dilation on a
//! small image.

use std::sync::Arc;

use besl::vm::{AccelerationStructure, BottomLevelStructure, Buffer, DescriptorBindings, ExecutableProgram, Instance, RayTracingPipeline, Texture, Value};

use crate::rendering::shader_vm_test::{array_buffer, asset_source, buffer, compile, run_at, slot, texture_2d, with_engine_modules};

/// The paths each fixture texel traces.
const SAMPLES: u32 = 64;
/// What an image texel that holds no light holds.
const EMPTY: [f32; 4] = [0.0, 0.0, 0.0, -1.0];
/// The traced texel an image texel without one holds. Matches `NO_LIGHTMAP_TEXEL` in `layouts/lightmap_bake.besl`.
const NO_TEXEL: u32 = u32::MAX;

fn kernel(name: &str) -> ExecutableProgram {
	compile(
		besl::lex(with_engine_modules(asset_source(&format!("lightmaps/{name}"))))
			.expect("the lightmap kernel should link"),
	)
}

/// Returns a closest hit that reports a black surface emitting `emission` nits, on its front when `front` is set.
fn hit(emission: f32, front: bool) -> ExecutableProgram {
	let source = format!(
		"
		use ::byte_engine::rendering::visibility::layouts::ray_traced_scene::*;
		main: fn (input: StageInput, payload: HitSurface) -> HitSurface {{
			let hit_surface: HitSurface = payload;
			hit_surface.albedo = vec3f16(0.0, 0.0, 0.0);
			hit_surface.front_face = {};
			hit_surface.emission = vec3f({emission:?}, {emission:?}, {emission:?});
			hit_surface.normal = vec3f16(normalize(vec3f(0.0, 0.0, 0.0) - input.ray_direction));
			return hit_surface;
		}}
		",
		u32::from(front)
	);
	compile(besl::lex(with_engine_modules(&source)).expect("the fixture hit should link"))
}

/// A scene of one horizontal square at height `y`, 200 meters on a side, which covers all but a ten-thousandth of a
/// cosine-weighted hemisphere above a texel at the origin.
///
/// The square sits a little off center, so the diagonal its two triangles share doesn't pass over the texel, where a
/// ray straight up could slip between them.
fn ceiling_at(y: f32) -> AccelerationStructure {
	quad(y, -100.0 + 0.3, 100.0 + 0.3)
}

/// A scene of one horizontal rectangle at height `y`, from `left` to `right` along x and 200 meters along z, centered
/// on z.
fn quad(y: f32, left: f32, right: f32) -> AccelerationStructure {
	let corner = |x: f32, z: f32| [x, y, z];
	let [a, b, c, d] = [corner(left, -100.0), corner(right, -100.0), corner(right, 100.0), corner(left, 100.0)];
	AccelerationStructure::new(vec![Instance {
		transform: [[1.0, 0.0, 0.0, 0.0], [0.0, 1.0, 0.0, 0.0], [0.0, 0.0, 1.0, 0.0]],
		custom_index: 0,
		mask: 0xFF,
		hit_group_offset: 0,
		bottom_level: Arc::new(BottomLevelStructure::new(vec![vec![[a, b, c], [a, c, d]]])),
	}])
}

/// Traces `samples` paths of `bounces` hits from one upward texel at the origin, the first `learning` of them to learn
/// its guide and the rest by it, under an environment of radiance `sky` scaled by `intensity`, with `hit` shading
/// every surface of `scene`. Returns the texel's sum and how many of its paths' first rays reached the environment.
fn trace_texel(
	scene: &AccelerationStructure,
	hit: ExecutableProgram,
	sky: f32,
	intensity: f32,
	bounces: u32,
	[samples, learning]: [u32; 2],
) -> ([f32; 4], f32) {
	let (pipeline, trace) = (kernel("lightmap-trace.besl"), kernel("lightmap-trace.besl"));
	let pipeline = RayTracingPipeline::new(pipeline, vec![hit]).expect("the fixture hit should run in the trace");
	let mut parameters = buffer(&trace, slot(&trace, "lightmap_parameters"));
	for (member, value) in [
		("texel_count", Value::U32(1)),
		("samples", Value::U32(samples)),
		("bounces", Value::U32(bounces)),
		("environment_intensity", Value::F32(intensity)),
		("extent", Value::U32(1)),
		("pass", Value::U32(0)),
		("learning_samples", Value::U32(learning)),
		("group_count", Value::U32(1)),
	] {
		parameters.write(member, value).expect("bake parameters");
	}
	let mut texels = array_buffer(&trace, slot(&trace, "lightmap_texels"), 1);
	for (member, value) in [
		("position", Value::Vec3F([0.0; 3])),
		("pixel", Value::U32(0)),
		("normal", Value::Vec3F([0.0, 1.0, 0.0])),
		("offset", Value::F32(0.01)),
	] {
		texels.write_array_member(0, member, value).expect("texel");
	}
	let mut accumulation = array_buffer(&trace, slot(&trace, "lightmap_accumulation"), 1);
	let mut escapes = array_buffer(&trace, slot(&trace, "lightmap_escapes"), 1);
	let mut lights = buffer(&trace, slot(&trace, "scene_lights"));
	lights.write("light_count", Value::U32(0)).expect("light count");
	let mut environment: Texture = texture_2d(1, 1, &[[sky, sky, sky, 1.0]]);
	// The texel is its own guide group.
	let mut groups = array_buffer(&trace, slot(&trace, "lightmap_groups"), 1);
	groups.write_array_member(0, "first", Value::U32(0)).expect("group");
	groups.write_array_member(0, "count", Value::U32(1)).expect("group");
	groups.write_array_member(0, "axis", Value::U32(0)).expect("group");
	let mut group_texels = array_buffer(&trace, slot(&trace, "lightmap_group_texels"), 1);
	let mut texel_groups = array_buffer(&trace, slot(&trace, "lightmap_texel_groups"), 1);
	for buffer in [&mut group_texels, &mut texel_groups] {
		buffer.write_array_element(0, Value::U32(0)).expect("group index");
	}
	let mut cells = array_buffer(&trace, slot(&trace, "lightmap_guide_cells"), 64);
	let mut guides = array_buffer(&trace, slot(&trace, "lightmap_guides"), 72);

	// The learning batch, then the guided one, as the bake launches them.
	for (offset, count) in [(0, learning), (learning, samples - learning)] {
		if count == 0 {
			continue;
		}
		parameters.write("sample_offset", Value::U32(offset)).expect("sample offset");
		parameters.write("samples_per_batch", Value::U32(count)).expect("batch");
		let mut descriptors = DescriptorBindings::new();
		descriptors.bind_buffer(slot(&trace, "lightmap_parameters"), &mut parameters);
		descriptors.bind_buffer(slot(&trace, "lightmap_texels"), &mut texels);
		descriptors.bind_buffer(slot(&trace, "lightmap_accumulation"), &mut accumulation);
		descriptors.bind_buffer(slot(&trace, "lightmap_escapes"), &mut escapes);
		descriptors.bind_buffer(slot(&trace, "lightmap_groups"), &mut groups);
		descriptors.bind_buffer(slot(&trace, "lightmap_group_texels"), &mut group_texels);
		descriptors.bind_buffer(slot(&trace, "lightmap_texel_groups"), &mut texel_groups);
		descriptors.bind_buffer(slot(&trace, "lightmap_guide_cells"), &mut cells);
		descriptors.bind_buffer(slot(&trace, "lightmap_guides"), &mut guides);
		descriptors.bind_buffer(slot(&trace, "scene_lights"), &mut lights);
		descriptors.bind_texture(slot(&trace, "lightmap_environment"), &mut environment);
		descriptors.bind_acceleration_structure(slot(&trace, "scene_structure"), scene);
		pipeline
			.trace_rays(&mut descriptors, &[0], [1, 1, 1])
			.expect("the trace should run in the VM");
	}
	let Value::F32(escaped) = escapes.read_array_element(0).expect("escapes") else {
		panic!("Expected the texel's escapes as an f32.");
	};
	(read_vec4(&accumulation, 0), escaped)
}

fn read_vec4(buffer: &Buffer, index: usize) -> [f32; 4] {
	match buffer.read_array_element(index).expect("vec4 element") {
		Value::Vec4F(value) => value,
		other => panic!("Expected a vec4f, found {other:?}."),
	}
}

/// Verifies the furnace case: under an open sky of uniform radiance, every path brings back the sky times the
/// environment's intensity, so the texel's mean is that radiance, its irradiance over π.
#[test]
fn a_texel_under_an_open_sky_holds_the_skys_radiance() {
	let (sum, escaped) = trace_texel(&AccelerationStructure::new(Vec::new()), hit(0.0, true), 0.5, 2.0, 3, [SAMPLES, 0]);

	let samples = SAMPLES as f32;
	assert_eq!([sum[0] / samples, sum[1] / samples, sum[2] / samples], [1.0; 3]);
	assert_eq!(sum[3], 0.0, "no path hit a back face");
	assert_eq!(escaped, samples, "every path's first ray reached the environment");
}

/// Verifies the paths' first hits shade with the material's closest hit: a black ceiling that emits 3 nits over the
/// whole hemisphere gives the texel its emission, and hides the sky.
#[test]
fn a_texel_under_an_emissive_ceiling_holds_its_emission_and_none_of_the_sky() {
	let (sum, escaped) = trace_texel(&ceiling_at(1.0), hit(3.0, true), 10.0, 1.0, 3, [SAMPLES, 0]);

	let mean = sum[0] / SAMPLES as f32;
	assert!((mean - 3.0).abs() < 0.01, "{mean}");
	assert_eq!(escaped, 0.0, "the ceiling hides the environment");
}

/// Verifies a texel whose paths first hit back faces, as one inside a wall does, counts them and gathers no light.
#[test]
fn a_texel_inside_geometry_counts_its_back_face_hits() {
	let (sum, _) = trace_texel(&ceiling_at(1.0), hit(3.0, false), 10.0, 1.0, 3, [SAMPLES, 0]);

	assert_eq!([sum[0], sum[1], sum[2]], [0.0; 3]);
	// Only a path that slips past the ceiling's edge, about one in ten thousand, misses it.
	assert!(sum[3] >= SAMPLES as f32 - 1.0, "{}", sum[3]);
}

/// Verifies guided paths estimate the same light as cosine-weighted ones: under a black ceiling that covers the half of
/// the sky past an edge right above the texel, half the cosine-weighted hemisphere sees the sky, so the texel holds half
/// its radiance and half its directions escape, though its guide sends most guided paths toward the open half. A
/// quarter of the paths learn, enough to reach every cell of the guide, as a group of texels does in a bake.
#[test]
fn guided_paths_under_a_half_covered_sky_hold_half_its_radiance() {
	let samples = 1024;
	let (sum, escaped) = trace_texel(&quad(1.0, 0.0, 100.0), hit(0.0, true), 4.0, 1.0, 1, [samples, samples / 4]);
	let samples = samples as f32;

	let mean = sum[0] / samples;
	assert!((mean - 2.0).abs() < 0.04, "{mean}");
	assert!((escaped / samples - 0.5).abs() < 0.01, "{}", escaped / samples);
}

/// Verifies the resolve divides each traced texel's sum and escapes by the paths, leaves a texel inside geometry empty,
/// and that
/// each dilation pass fills the next ring of empty texels with the mean of their lit neighbors.
#[test]
fn the_resolve_averages_paths_and_dilation_fills_empty_texels_ring_by_ring() {
	let dilate = kernel("lightmap-dilate.besl");
	let extent = 4u32;
	let mut parameters = buffer(&dilate, slot(&dilate, "lightmap_parameters"));
	parameters.write("extent", Value::U32(extent)).expect("extent");
	parameters.write("samples", Value::U32(4)).expect("samples");
	// Texel (1, 1) holds a lit traced texel, and (3, 0) one whose paths all hit back faces.
	let mut indices = array_buffer(&dilate, slot(&dilate, "lightmap_texel_indices"), 16);
	for pixel in 0..16 {
		let traced = match pixel {
			5 => 0,
			3 => 1,
			_ => NO_TEXEL,
		};
		indices.write_array_element(pixel, Value::U32(traced)).expect("index");
	}
	let mut accumulation = array_buffer(&dilate, slot(&dilate, "lightmap_accumulation"), 2);
	accumulation.write_array_element(0, Value::Vec4F([4.0, 8.0, 12.0, 0.0])).expect("sum");
	accumulation.write_array_element(1, Value::Vec4F([4.0, 4.0, 4.0, 4.0])).expect("sum");
	// One of the lit texel's four paths reached the environment.
	let mut escapes = array_buffer(&dilate, slot(&dilate, "lightmap_escapes"), 2);
	escapes.write_array_element(0, Value::F32(1.0)).expect("escapes");
	let mut light = array_buffer(&dilate, slot(&dilate, "lightmap_light"), 16);
	let mut dilated = array_buffer(&dilate, slot(&dilate, "lightmap_dilated"), 16);

	for pass in 0..=2 {
		parameters.write("pass", Value::U32(pass)).expect("pass");
		for pixel in 0..16 {
			let mut descriptors = DescriptorBindings::new();
			descriptors.bind_buffer(slot(&dilate, "lightmap_parameters"), &mut parameters);
			descriptors.bind_buffer(slot(&dilate, "lightmap_texel_indices"), &mut indices);
			descriptors.bind_buffer(slot(&dilate, "lightmap_accumulation"), &mut accumulation);
			descriptors.bind_buffer(slot(&dilate, "lightmap_escapes"), &mut escapes);
			descriptors.bind_buffer(slot(&dilate, "lightmap_light"), &mut light);
			descriptors.bind_buffer(slot(&dilate, "lightmap_dilated"), &mut dilated);
			run_at(&dilate, &mut descriptors, [pixel, 0]);
		}
		match pass {
			0 => {
				assert_eq!(read_vec4(&light, 5), [1.0, 2.0, 3.0, 0.25], "the resolve averages the paths");
				assert_eq!(read_vec4(&light, 3), EMPTY, "a texel inside geometry holds nothing");
			}
			1 => {
				assert_eq!(read_vec4(&dilated, 0), [1.0, 2.0, 3.0, 0.25], "the first ring takes its lit neighbor");
				assert_eq!(read_vec4(&dilated, 15), EMPTY, "the second ring waits a pass");
			}
			_ => assert_eq!(read_vec4(&light, 15), [1.0, 2.0, 3.0, 0.25], "the second ring fills on the next pass"),
		}
	}
}
