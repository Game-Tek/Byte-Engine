//! Ray tracing in BESL: the shaders the language accepts, and how the VM runs their ray queries and pipelines.

use std::sync::Arc;

use besl::lexer::LexError;
use besl::vm::{
	AccelerationStructure, BottomLevelStructure, Buffer, DescriptorBindings, ExecutableProgram, Instance, RayTracingPipeline,
	ResourceSlot, Value, VmError,
};
use besl::{CompilationError, compile_to_besl};

/// The transform that leaves a structure in place.
const IDENTITY: [[f32; 4]; 3] = [[1.0, 0.0, 0.0, 0.0], [0.0, 1.0, 0.0, 0.0], [0.0, 0.0, 1.0, 0.0]];

/// The slot of the scene descriptor that every fixture declares.
const SCENE: ResourceSlot = ResourceSlot::new(0);

/// The slot of the buffer that every fixture writes its results to.
const RESULTS: ResourceSlot = ResourceSlot::new(1);

/// A triangle off the z axis, which every fixture ray misses.
const TRIANGLE_ASIDE: [[f32; 3]; 3] = [[5.0, 5.0, 1.0], [6.0, 5.0, 1.0], [5.0, 6.0, 1.0]];

/// Shades one ray for each launch point: ray `x` starts at `(10x, 0, 0)`, points along +z, and runs the closest hit
/// of what it finds.
const RAY_GENERATION: &str = r#"
	SurfaceHit: struct {
		albedo: vec3f,
		distance: f32,
	}

	scene: descriptor<{ type: AccelerationStructure, binding: 0, access: read }>;
	surfaces: descriptor<{ type: SurfaceHit[], binding: 1, access: write }>;

	main: fn (input: StageInput) -> void {
		let origin: vec3f = vec3f(f32(input.launch_index.x) * 10.0, 0.0, 0.0);
		let hit: Hit = intersect(scene, 255, origin, vec3f(0.0, 0.0, 1.0), 0.0, 100.0);
		surfaces[input.launch_index.x] = invoke(hit, SurfaceHit(vec3f(0.25, 0.5, 0.75), -1.0));
	}
"#;

/// Returns a closest hit that reports `albedo` and the distance of its hit.
fn closest_hit(albedo: &str) -> String {
	format!(
		r#"
		SurfaceHit: struct {{
			albedo: vec3f,
			distance: f32,
		}}

		main: fn (input: StageInput, payload: SurfaceHit) -> SurfaceHit {{
			let surface: SurfaceHit = payload;
			surface.albedo = {albedo};
			surface.distance = input.hit_distance;
			return surface;
		}}
		"#
	)
}

/// Returns a triangle in the plane at `z` that a ray from `(x, 0, 0)` along +z hits at barycentrics `(0.25, 0.25)`.
fn triangle_ahead_of(x: f32, z: f32) -> [[f32; 3]; 3] {
	[[x - 1.0, -1.0, z], [x + 3.0, -1.0, z], [x - 1.0, 3.0, z]]
}

/// Places a structure of `geometries` with `transform`. Hits on it report `custom_index`, and their records start at
/// `hit_group_offset`. Every ray query tests it.
fn instance(
	transform: [[f32; 4]; 3],
	custom_index: u32,
	hit_group_offset: u32,
	geometries: Vec<Vec<[[f32; 3]; 3]>>,
) -> Instance {
	Instance {
		transform,
		custom_index,
		mask: 0xFF,
		hit_group_offset,
		bottom_level: Arc::new(BottomLevelStructure::new(geometries)),
	}
}

/// Lexes and compiles a shader that the language accepts.
fn compile(source: &str) -> ExecutableProgram {
	let program = compile_to_besl(source, None).expect("Expected the shader to lex");
	ExecutableProgram::compile(program).expect("Expected the shader to compile")
}

/// Allocates `count` elements of the results array that `executable` declares.
fn results(executable: &ExecutableProgram, count: usize) -> Buffer {
	let layout = executable.buffer_layout(RESULTS).expect("Expected a results buffer").clone();
	Buffer::new_array(layout, count).expect("Expected results storage")
}

/// Runs `executable` as a compute shader against `scene`, writing to `results`.
fn run(executable: &ExecutableProgram, scene: &AccelerationStructure, results: &mut Buffer) -> Result<(), VmError> {
	let mut descriptors = DescriptorBindings::new();
	descriptors.bind_acceleration_structure(SCENE, scene);
	descriptors.bind_buffer(RESULTS, results);
	executable.run_main(&mut descriptors)
}

/// Traces a `width` by 1 grid through `pipeline` against `scene`, writing to `surfaces`.
fn trace(
	pipeline: &RayTracingPipeline,
	scene: &AccelerationStructure,
	records: &[u32],
	width: u32,
	surfaces: &mut Buffer,
) -> Result<(), VmError> {
	let mut descriptors = DescriptorBindings::new();
	descriptors.bind_acceleration_structure(SCENE, scene);
	descriptors.bind_buffer(RESULTS, surfaces);
	pipeline.trace_rays(&mut descriptors, records, [width, 1, 1])
}

/// Returns the message of a language rule that `source` breaks.
fn rule_violation(source: &str) -> String {
	match compile_to_besl(source, None) {
		Err(CompilationError::Lex(LexError::Invalid { message })) => message,
		Err(error) => panic!("Expected a broken language rule, but found {error:?}"),
		Ok(_) => panic!("Expected the shader to break a language rule"),
	}
}

#[test]
fn intersect_reports_the_nearest_triangle_and_its_record() {
	let executable = compile(
		r#"
		scene: descriptor<{ type: AccelerationStructure, binding: 0, access: read }>;
		hits: descriptor<{ type: Hit[], binding: 1, access: write }>;

		main: fn () -> void {
			hits[0] = intersect(scene, 255, vec3f(0.0, 0.0, 0.0), vec3f(0.0, 0.0, 1.0), 0.0, 100.0);
		}
		"#,
	);
	// The nearest triangle is the second one of the second geometry of the second instance, which the instance's
	// transform moves from z = 6 to z = 4. The other triangles along the ray lie at z = 8 and z = 5.
	let lowered = [[1.0, 0.0, 0.0, 0.0], [0.0, 1.0, 0.0, 0.0], [0.0, 0.0, 1.0, -2.0]];
	let scene = AccelerationStructure::new(vec![
		instance(IDENTITY, 7, 0, vec![vec![triangle_ahead_of(0.0, 8.0)]]),
		instance(
			lowered,
			9,
			4,
			vec![
				vec![triangle_ahead_of(0.0, 7.0)],
				vec![TRIANGLE_ASIDE, triangle_ahead_of(0.0, 6.0)],
			],
		),
	]);
	let mut hits = results(&executable, 1);

	run(&executable, &scene, &mut hits).expect("Expected the query to run");

	let member = |name| hits.read_array_member(0, name).expect("Expected a hit member");
	// The record is the instance's hit group offset plus the geometry index.
	assert_eq!(member("record"), Value::U32(5));
	assert_eq!(member("distance"), Value::F32(4.0));
	assert_eq!(member("custom_index"), Value::U32(9));
	assert_eq!(member("geometry_index"), Value::U32(1));
	assert_eq!(member("primitive_index"), Value::U32(1));
	assert_eq!(member("barycentrics"), Value::Vec2F([0.25, 0.25]));
	assert_eq!(member("origin"), Value::Vec3F([0.0, 0.0, 0.0]));
	assert_eq!(member("direction"), Value::Vec3F([0.0, 0.0, 1.0]));
}

#[test]
fn ray_queries_skip_instances_whose_mask_shares_no_bit() {
	let executable = compile(
		r#"
		scene: descriptor<{ type: AccelerationStructure, binding: 0, access: read }>;
		distances: descriptor<{ type: f32[], binding: 1, access: write }>;

		main: fn () -> void {
			let origin: vec3f = vec3f(0.0, 0.0, 0.0);
			let direction: vec3f = vec3f(0.0, 0.0, 1.0);
			let far: Hit = intersect(scene, 2, origin, direction, 0.0, 100.0);
			let beyond_low_bits: Hit = intersect(scene, 256, origin, direction, 0.0, 100.0);
			let any: Hit = intersect(scene, 255, origin, direction, 0.0, 100.0);
			distances[0] = far.distance;
			distances[1] = beyond_low_bits.distance;
			distances[2] = any.distance;
		}
		"#,
	);
	let scene = AccelerationStructure::new(vec![
		Instance {
			mask: 0b01,
			..instance(IDENTITY, 0, 0, vec![vec![triangle_ahead_of(0.0, 3.0)]])
		},
		Instance {
			mask: 0b10,
			..instance(IDENTITY, 0, 0, vec![vec![triangle_ahead_of(0.0, 5.0)]])
		},
	]);
	let mut distances = results(&executable, 3);

	run(&executable, &scene, &mut distances).expect("Expected the queries to run");

	// A query tests only the low 8 bits of its mask, and a miss reports a distance of -1.
	assert_eq!(distances.read_array_element(0), Ok(Value::F32(5.0)));
	assert_eq!(distances.read_array_element(1), Ok(Value::F32(-1.0)));
	assert_eq!(distances.read_array_element(2), Ok(Value::F32(3.0)));
}

#[test]
fn trace_occlusion_reports_hits_within_the_interval() {
	let executable = compile(
		r#"
		scene: descriptor<{ type: AccelerationStructure, binding: 0, access: read }>;
		occluded: descriptor<{ type: u32[], binding: 1, access: write }>;

		main: fn () -> void {
			let origin: vec3f = vec3f(0.0, 0.0, 0.0);
			let direction: vec3f = vec3f(0.0, 0.0, 1.0);
			if (trace_occlusion(scene, 255, origin, direction, 0.0, 4.5)) {
				occluded[0] = 1;
			}
			if (trace_occlusion(scene, 255, origin, direction, 0.0, 5.0)) {
				occluded[1] = 1;
			}
			if (trace_occlusion(scene, 255, origin, direction, 5.0, 100.0)) {
				occluded[2] = 1;
			}
			if (trace_occlusion(scene, 255, origin, direction, 5.5, 100.0)) {
				occluded[3] = 1;
			}
		}
		"#,
	);
	let scene = AccelerationStructure::new(vec![instance(IDENTITY, 0, 0, vec![vec![triangle_ahead_of(0.0, 5.0)]])]);
	let mut occluded = results(&executable, 4);

	run(&executable, &scene, &mut occluded).expect("Expected the queries to run");

	// The triangle lies at distance 5, and both ends of an interval belong to it.
	let occluded = (0..4)
		.map(|index| occluded.read_array_element(index).expect("Expected a result"))
		.collect::<Vec<_>>();
	assert_eq!(occluded, [Value::U32(0), Value::U32(1), Value::U32(1), Value::U32(0)]);
}

#[test]
fn ray_queries_reject_an_interval_the_gpu_leaves_undefined() {
	let executable = compile(
		r#"
		scene: descriptor<{ type: AccelerationStructure, binding: 0, access: read }>;
		occluded: descriptor<{ type: u32[], binding: 1, access: write }>;

		main: fn () -> void {
			if (trace_occlusion(scene, 255, vec3f(0.0, 0.0, 0.0), vec3f(0.0, 0.0, 1.0), 2.0, 1.0)) {
				occluded[0] = 1;
			}
		}
		"#,
	);
	let scene = AccelerationStructure::new(Vec::new());
	let mut occluded = results(&executable, 1);

	assert_eq!(
		run(&executable, &scene, &mut occluded),
		Err(VmError::InvalidRayInterval {
			t_min: "2.0".to_string(),
			t_max: "1.0".to_string(),
		})
	);
}

#[test]
fn invoke_runs_the_closest_hit_of_the_hit_record() {
	let ray_generation = compile(RAY_GENERATION);
	let mut surfaces = results(&ray_generation, 2);
	let pipeline = RayTracingPipeline::new(
		ray_generation,
		vec![
			compile(&closest_hit("vec3f(1.0, 0.0, 0.0)")),
			compile(&closest_hit("vec3f(0.0, 1.0, 0.0)")),
		],
	)
	.expect("Expected a valid pipeline");
	// Ray 0 hits geometry 0 at distance 5, and ray 1 hits geometry 1 at distance 3.
	let scene = AccelerationStructure::new(vec![instance(
		IDENTITY,
		0,
		0,
		vec![vec![triangle_ahead_of(0.0, 5.0)], vec![triangle_ahead_of(10.0, 3.0)]],
	)]);

	// Record 0 runs hit group 1, and record 1 runs hit group 0.
	trace(&pipeline, &scene, &[1, 0], 2, &mut surfaces).expect("Expected the trace to run");

	assert_eq!(surfaces.read_array_member(0, "albedo"), Ok(Value::Vec3F([0.0, 1.0, 0.0])));
	assert_eq!(surfaces.read_array_member(0, "distance"), Ok(Value::F32(5.0)));
	assert_eq!(surfaces.read_array_member(1, "albedo"), Ok(Value::Vec3F([1.0, 0.0, 0.0])));
	assert_eq!(surfaces.read_array_member(1, "distance"), Ok(Value::F32(3.0)));
}

#[test]
fn invoke_leaves_the_payload_of_a_miss_unchanged() {
	let ray_generation = compile(RAY_GENERATION);
	let mut surfaces = results(&ray_generation, 1);
	let pipeline = RayTracingPipeline::new(ray_generation, vec![compile(&closest_hit("vec3f(1.0, 0.0, 0.0)"))])
		.expect("Expected a valid pipeline");
	let scene = AccelerationStructure::new(vec![instance(IDENTITY, 0, 0, vec![vec![TRIANGLE_ASIDE]])]);

	trace(&pipeline, &scene, &[0], 1, &mut surfaces).expect("Expected the trace to run");

	assert_eq!(surfaces.read_array_member(0, "albedo"), Ok(Value::Vec3F([0.25, 0.5, 0.75])));
	assert_eq!(surfaces.read_array_member(0, "distance"), Ok(Value::F32(-1.0)));
}

#[test]
fn mismatched_payload_layouts_are_an_error() {
	// The closest hit declares the payload's members in another order, so it would read the payload wrongly.
	let reordered = compile(
		r#"
		SurfaceHit: struct {
			distance: f32,
			albedo: vec3f,
		}

		main: fn (payload: SurfaceHit) -> SurfaceHit {
			return payload;
		}
		"#,
	);

	let pipeline = RayTracingPipeline::new(
		compile(RAY_GENERATION),
		vec![compile(&closest_hit("vec3f(1.0, 0.0, 0.0)")), reordered],
	);

	assert!(matches!(pipeline, Err(VmError::PayloadMismatch { hit_group: 1, .. })));
}

#[test]
fn invoke_outside_ray_generation_is_rejected() {
	// A compute shader has no shader binding table to run a closest hit from.
	let compute = compile(
		r#"
		SurfaceHit: struct {
			albedo: vec3f,
			distance: f32,
		}

		scene: descriptor<{ type: AccelerationStructure, binding: 0, access: read }>;
		surfaces: descriptor<{ type: SurfaceHit[], binding: 1, access: write }>;

		main: fn () -> void {
			let hit: Hit = Hit(0, 1.0, 0, 0, 0, vec2f(0.0, 0.0), vec3f(0.0, 0.0, 0.0), vec3f(0.0, 0.0, 1.0));
			surfaces[0] = invoke(hit, SurfaceHit(vec3f(0.0, 0.0, 0.0), -1.0));
		}
		"#,
	);
	let mut surfaces = results(&compute, 1);

	assert_eq!(
		run(&compute, &AccelerationStructure::new(Vec::new()), &mut surfaces),
		Err(VmError::InvokeOutsideRayGeneration)
	);
}

#[test]
fn closest_hits_that_trace_rays_are_rejected() {
	// A closest hit that invokes another one could recurse, and one that runs ray queries does the ray generation
	// shader's work.
	let invoking = compile(
		r#"
		SurfaceHit: struct {
			albedo: vec3f,
			distance: f32,
		}

		main: fn (input: StageInput, payload: SurfaceHit) -> SurfaceHit {
			let hit: Hit = Hit(0, 1.0, 0, 0, 0, vec2f(0.0, 0.0), input.ray_origin, input.ray_direction);
			return invoke(hit, payload);
		}
		"#,
	);
	let querying = compile(
		r#"
		SurfaceHit: struct {
			albedo: vec3f,
			distance: f32,
		}

		scene: descriptor<{ type: AccelerationStructure, binding: 0, access: read }>;

		main: fn (input: StageInput, payload: SurfaceHit) -> SurfaceHit {
			let surface: SurfaceHit = payload;
			if (trace_occlusion(scene, 255, input.ray_origin, input.ray_direction, 0.0, 1.0)) {
				surface.distance = 0.0;
			}
			return surface;
		}
		"#,
	);

	for (hit_group, tracing) in [invoking, querying].into_iter().enumerate() {
		let mut hit_groups = vec![compile(&closest_hit("vec3f(1.0, 0.0, 0.0)"))];
		hit_groups.insert(hit_group, tracing);
		assert!(matches!(
			RayTracingPipeline::new(compile(RAY_GENERATION), hit_groups),
			Err(VmError::RayTracingInClosestHit { hit_group: rejected }) if rejected == hit_group
		));
	}
}

#[test]
fn discard_is_an_error_in_ray_tracing_shaders() {
	let ray_generation = compile(RAY_GENERATION);
	let mut surfaces = results(&ray_generation, 1);
	let discarding = compile(
		r#"
		SurfaceHit: struct {
			albedo: vec3f,
			distance: f32,
		}

		main: fn (payload: SurfaceHit) -> SurfaceHit {
			discard;
			return payload;
		}
		"#,
	);
	let pipeline = RayTracingPipeline::new(ray_generation, vec![discarding]).expect("Expected a valid pipeline");
	let scene = AccelerationStructure::new(vec![instance(IDENTITY, 0, 0, vec![vec![triangle_ahead_of(0.0, 5.0)]])]);

	assert_eq!(
		trace(&pipeline, &scene, &[0], 1, &mut surfaces),
		Err(VmError::DiscardInRayTracing)
	);
}

#[test]
fn ray_tracing_inputs_are_available_only_in_their_stage() {
	// Run as a compute shader, the ray generation shader has no launch grid to read its launch index from.
	let ray_generation = compile(RAY_GENERATION);
	let mut surfaces = results(&ray_generation, 1);

	assert_eq!(
		run(&ray_generation, &AccelerationStructure::new(Vec::new()), &mut surfaces),
		Err(VmError::RayTracingInputUnavailable { name: "launch_index" })
	);
}

#[test]
fn payloads_hold_only_scalars_and_vectors() {
	let message = rule_violation(
		r#"
		Flagged: struct {
			albedo: vec3f,
			visible: bool,
		}

		main: fn (payload: Flagged) -> Flagged {
			return payload;
		}
		"#,
	);
	assert!(
		message.starts_with("Payload `Flagged` can't carry member `visible` of type `bool`."),
		"{message}"
	);

	let message = rule_violation(
		r#"
		main: fn (payload: vec4f) -> vec4f {
			return payload;
		}
		"#,
	);
	assert!(message.starts_with("`vec4f` can't be a ray tracing payload."), "{message}");
}

#[test]
fn invoke_calls_in_one_shader_pass_one_payload_type() {
	let message = rule_violation(
		r#"
		Color: struct {
			albedo: vec3f,
		}

		Depth: struct {
			distance: f32,
		}

		scene: descriptor<{ type: AccelerationStructure, binding: 0, access: read }>;

		main: fn () -> void {
			let hit: Hit = intersect(scene, 255, vec3f(0.0, 0.0, 0.0), vec3f(0.0, 0.0, 1.0), 0.0, 100.0);
			let color: Color = invoke(hit, Color(vec3f(0.0, 0.0, 0.0)));
			let depth: Depth = invoke(hit, Depth(0.0));
		}
		"#,
	);
	assert!(
		message.starts_with("`invoke` passes payload `Depth`, but an earlier `invoke` in this shader passes `Color`."),
		"{message}"
	);
}

#[test]
fn acceleration_structure_descriptors_are_read_only_and_single() {
	let message = rule_violation(
		r#"
		scene: descriptor<{ type: AccelerationStructure, binding: 0, access: read_write }>;

		main: fn () -> void {}
		"#,
	);
	assert!(
		message.starts_with("Acceleration structure descriptor `scene` declares write access."),
		"{message}"
	);

	let message = rule_violation(
		r#"
		scenes: descriptor<{ type: AccelerationStructure, binding: 0, access: read, count: 2 }>;

		main: fn () -> void {}
		"#,
	);
	assert!(
		message.starts_with("Acceleration structure descriptor `scenes` declares a count."),
		"{message}"
	);
}
