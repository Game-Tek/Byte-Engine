//! Bakes `.particles` assets and runs their generated kernels in the BESL VM and the platform shader compiler.

use besl::vm::{Buffer, DescriptorBindings, ExecutableProgram, ExecutionConfig, ResourceSlot, Value, WorkgroupState};

use super::{ParticleSystemAssetHandler, generator, schema::ParticleSystemSource};
use crate::{
	asset::{self, handler::implementations::besl::ShaderCompiler, manager::AssetManager},
	r#async, resource,
	resource::resource_manager::ResourceManager,
	resources::{
		material::{Shader, ShaderArtifact, ShaderInterface},
		particle_system::ParticleSystem,
		pipeline::{Pipeline, PipelineKind},
	},
	shader::ShaderGenerationSettings,
	types::ShaderTypes,
};

/// Sparks that leave a point emitter inside a cone whose half angle has a cosine of 0.9, with no forces.
const SPARKS: &str = r#"{
	"capacity": 4096,
	"lifetime": [1.0, 1.0],
	"spawn": { "rate": 100 },
	"initialize": [{ "type": "cone", "angle": 0.45102681, "speed": [2.0, 4.0] }],
	"render": {
		"shape": { "type": "streak", "width": 0.01, "stretch": 0.02 },
		"color": [{ "age": 0.0, "radiance": [100.0, 50.0, 10.0] }, { "age": 1.0, "radiance": [0.0, 0.0, 0.0] }]
	}
}"#;

/// Smoke that uses every other module and the billboard shape.
const SMOKE: &str = r#"{
	"capacity": 1024,
	"lifetime": [2.0, 4.0],
	"initialize": [{ "type": "sphere", "radius": 0.5 }, { "type": "cone", "angle": 3.14159, "speed": [0.0, 0.2] }],
	"update": [{ "type": "acceleration", "value": [0.5, 1.0, 0.0] }, { "type": "drag", "coefficient": 1.5 }],
	"render": {
		"shape": { "type": "billboard", "size": 0.4 },
		"color": [{ "age": 0.0, "radiance": [2.0, 2.0, 2.0] }, { "age": 0.5, "radiance": [1.0, 1.0, 1.0] }, { "age": 1.0, "radiance": [0.0, 0.0, 0.0] }]
	}
}"#;

const FRAME_SLOT: ResourceSlot = ResourceSlot::new(0);
const PARTICLES_SLOT: ResourceSlot = ResourceSlot::new(1);
const DRAWS_SLOT: ResourceSlot = ResourceSlot::new(2);
const INSTRUCTION_LIMIT: usize = 4_000_000;
const CAPACITY: usize = 4096;
/// Emitter slot 3 sits at (1, 2, 3) and fires along +Z.
const EMITTER_SLOT: usize = 3;
const EMITTER_POSITION: [f32; 3] = [1.0, 2.0, 3.0];

/// The `LinkingCompiler` struct stands in for the platform compiler and only checks that generated BESL links.
struct LinkingCompiler;

impl ShaderCompiler for LinkingCompiler {
	fn compile<'a>(
		&'a self,
		id: &'a str,
		source: &'a str,
		_generator: Option<(
			&'a dyn crate::asset::handler::implementations::bema::ProgramGenerator,
			&'a crate::asset::JsonObject,
		)>,
		stage: ShaderTypes,
		_settings: ShaderGenerationSettings,
	) -> crate::r#async::BoxedFuture<'a, Result<(Shader, Box<[u8]>), String>> {
		Box::pin(async move {
			besl::lex(besl::parse(source).map_err(|error| format!("{error:?}"))?).map_err(|error| format!("{error:?}"))?;
			Ok((
				Shader {
					id: id.to_string(),
					stage,
					interface: ShaderInterface {
						workgroup_size: None,
						bindings: Vec::new(),
					},
					artifact: ShaderArtifact::Spirv,
				},
				b"compiled-shader".to_vec().into_boxed_slice(),
			))
		})
	}
}

fn asset_manager(
	assets: asset::storage_backend::tests::TestStorageBackend,
	resources: resource::storage_backend::tests::TestStorageBackend,
) -> AssetManager {
	let mut asset_manager = AssetManager::new(assets, resources);
	asset_manager.add_asset_handler(ParticleSystemAssetHandler {
		compiler: Box::new(LinkingCompiler),
	});
	asset_manager
}

/// Verifies requesting one generated pipeline bakes the whole system, and that its pipelines name its shaders.
#[r#async::test]
async fn baking_a_system_stores_its_pipelines_and_shaders() {
	let assets = asset::storage_backend::tests::TestStorageBackend::new();
	let resources = resource::storage_backend::tests::TestStorageBackend::new();
	assets.add_file("effects/sparks.particles", SPARKS.as_bytes());

	asset_manager(assets, resources.clone())
		.bake("effects/sparks.particles#draw")
		.await
		.expect("the particle system should bake");

	let resource_manager = ResourceManager::new(resources);
	let system = resource_manager
		.request::<ParticleSystem>("effects/sparks.particles")
		.await
		.expect("the baked particle system should be requested");
	assert_eq!(
		system.resource(),
		&ParticleSystem {
			capacity: 4096,
			longest_life: 1.0,
			rate: 100.0,
			burst: 0,
			simulate_pipeline: "effects/sparks.particles#simulate".to_string(),
			draw_pipeline: "effects/sparks.particles#draw".to_string(),
		}
	);

	let simulate = resource_manager
		.request::<Pipeline>(&system.resource().simulate_pipeline)
		.await
		.expect("the simulation pipeline should be stored");
	let PipelineKind::Compute { shader, .. } = &simulate.resource().kind else {
		panic!("the simulation pipeline should be a compute pipeline");
	};
	let draw = resource_manager
		.request::<Pipeline>(&system.resource().draw_pipeline)
		.await
		.expect("the draw pipeline should be stored");
	let PipelineKind::Raster { shaders, .. } = &draw.resource().kind else {
		panic!("the draw pipeline should be a raster pipeline");
	};
	for id in std::iter::once(shader).chain(shaders) {
		resource_manager
			.request::<Shader>(id)
			.await
			.unwrap_or_else(|error| panic!("pipeline shader '{id}' should be stored: {error:?}"));
	}
}

/// Verifies systems the kernels cannot run fail to bake instead of producing broken shaders.
#[r#async::test]
async fn systems_that_cannot_run_are_rejected() {
	for (name, source) in [
		("empty", SPARKS.replace("\"capacity\": 4096", "\"capacity\": 0")),
		("unknown module", SPARKS.replace("\"cone\"", "\"vortex\"")),
		("unsorted colors", SPARKS.replace("\"age\": 1.0", "\"age\": 0.0")),
	] {
		let assets = asset::storage_backend::tests::TestStorageBackend::new();
		assets.add_file("bad.particles", source.as_bytes());
		let result = asset_manager(assets, resource::storage_backend::tests::TestStorageBackend::new())
			.bake("bad.particles")
			.await;
		assert!(result.is_err(), "a system with {name} should not bake");
	}
}

/// The VM buffers one frame of a particle system reads and writes.
struct FrameBuffers {
	frame: Buffer,
	particles: Buffer,
	draws: Buffer,
}

impl FrameBuffers {
	/// Creates buffers for a frame that writes `side` with no time passing.
	fn new(simulate: &ExecutableProgram, side: u32) -> Self {
		let layout = |slot| {
			simulate
				.buffer_layout(slot)
				.expect("the generated simulation should declare this binding")
				.clone()
		};
		let mut frame = Buffer::new(layout(FRAME_SLOT));
		frame.write("side", Value::U32(side)).expect("side");
		frame.write("capacity", Value::U32(CAPACITY as u32)).expect("capacity");
		frame
			.write_indexed_field(
				"emitters",
				EMITTER_SLOT,
				"model",
				// Identity rotation, translated to the emitter position.
				Value::Mat4x3F(bytemuck::cast([
					[1.0f32, 0.0, 0.0],
					[0.0, 1.0, 0.0],
					[0.0, 0.0, 1.0],
					EMITTER_POSITION,
				])),
			)
			.expect("emitter model");
		Self {
			frame,
			particles: Buffer::new_array(layout(PARTICLES_SLOT), CAPACITY * 2).expect("particle buffer"),
			draws: Buffer::new(layout(DRAWS_SLOT)),
		}
	}

	/// Requests `count` new particles from the test emitter.
	fn spawn(&mut self, count: u32) {
		self.frame.write("spawn_total", Value::U32(count)).expect("spawn total");
		self.frame.write("spawn_count", Value::U32(1)).expect("spawn count");
		self.frame
			.write_indexed_field("spawns", 0, "emitter", Value::U32(EMITTER_SLOT as u32))
			.expect("spawn emitter");
	}

	/// Places live particles in the half the previous frame wrote and sets that half's draw count to match.
	fn keep(&mut self, previous: usize, lives: &[f32]) {
		for (index, life) in lives.iter().enumerate() {
			let element = previous * CAPACITY + index;
			for (member, value) in [
				("position_x", index as f32),
				("position_y", 0.0),
				("position_z", 0.0),
				("velocity_x", 0.0),
				("velocity_y", 1.0),
				("velocity_z", 0.0),
				("life", *life),
			] {
				self.particles
					.write_array_member(element, member, Value::F32(value))
					.expect("particle member");
			}
			// Slot 3, losing one whole life per second.
			self.particles
				.write_array_member(element, "packed", Value::U32(EMITTER_SLOT as u32 | (1024 << 16)))
				.expect("particle packing");
		}
		self.draws
			.write_array_element(previous * 4, Value::U32(lives.len() as u32 * 6))
			.expect("previous draw count");
	}

	/// Runs the first simulation workgroup.
	fn simulate(&mut self, simulate: &ExecutableProgram) {
		let configs: [ExecutionConfig; generator::SIMULATION_WORKGROUP_SIZE as usize] = std::array::from_fn(|lane| {
			ExecutionConfig::new(INSTRUCTION_LIMIT)
				.with_call_depth_limit(128)
				.with_thread_idx(lane as u32)
				.with_thread_id([lane as u32, 0])
		});
		let mut workgroup = WorkgroupState::new();
		let mut descriptors = DescriptorBindings::new();
		descriptors.bind_buffer(FRAME_SLOT, &mut self.frame);
		descriptors.bind_buffer(PARTICLES_SLOT, &mut self.particles);
		descriptors.bind_buffer(DRAWS_SLOT, &mut self.draws);
		descriptors.bind_workgroup_state(&mut workgroup);
		simulate
			.run_workgroup(&mut descriptors, &configs)
			.expect("Failed to run the generated particle simulation in the BESL VM.");
	}

	fn vertex_count(&self, side: usize) -> u32 {
		match self.draws.read_array_element(side * 4).expect("draw vertex count") {
			Value::U32(count) => count,
			value => panic!("Unexpected draw value: {value:?}."),
		}
	}

	/// Returns the position, velocity, and life of the particle at `index` in `side`'s half.
	fn particle(&self, side: usize, index: usize) -> ([f32; 3], [f32; 3], f32) {
		let element = side * CAPACITY + index;
		let read = |member| match self.particles.read_array_member(element, member).expect("particle member") {
			Value::F32(value) => value,
			value => panic!("Unexpected particle value: {value:?}."),
		};
		(
			[read("position_x"), read("position_y"), read("position_z")],
			[read("velocity_x"), read("velocity_y"), read("velocity_z")],
			read("life"),
		)
	}
}

fn programs(source: &str) -> generator::ParticlePrograms {
	let system: ParticleSystemSource = serde_json::from_str(source).expect("the test system should parse");
	system.validate().expect("the test system should be valid");
	generator::generate(&system)
}

fn simulation(source: &str) -> ExecutableProgram {
	let program = besl::lex(besl::parse(&programs(source).simulate).expect("the generated simulation should parse"))
		.expect("the generated simulation should link");
	ExecutableProgram::compile(program.get_main().expect("the generated simulation should have a main"))
		.expect("the generated simulation should run in the VM")
}

/// Verifies new particles start at the emitter inside its launch cone and speed range, and that the draw covers them.
#[test]
fn new_particles_leave_the_emitter_inside_its_cone() {
	let simulate = simulation(SPARKS);
	let mut buffers = FrameBuffers::new(&simulate, 0);
	buffers.frame.write("reset", Value::U32(1)).expect("reset");
	buffers.spawn(40);

	buffers.simulate(&simulate);

	assert_eq!(buffers.vertex_count(0), 40 * 6);
	for index in 0..40 {
		let (position, velocity, life) = buffers.particle(0, index);
		let speed = velocity.iter().map(|component| component * component).sum::<f32>().sqrt();
		assert_eq!(position, EMITTER_POSITION);
		assert_eq!(life, 1.0);
		assert!((2.0..=4.0).contains(&speed), "particle {index} launched at {speed} m/s");
		assert!(
			velocity[2] / speed >= 0.9 - 1e-5,
			"particle {index} left the cone: {velocity:?}"
		);
	}
}

/// Verifies particles whose life runs out this frame are dropped and the rest move and pack into the other half.
#[test]
fn spent_particles_are_dropped_and_survivors_packed() {
	let simulate = simulation(SPARKS);
	let mut buffers = FrameBuffers::new(&simulate, 1);
	buffers.frame.write("delta_time", Value::F32(0.5)).expect("delta time");
	buffers.keep(0, &[0.8, 0.25, 0.9, 0.5, 0.6]);

	buffers.simulate(&simulate);

	assert_eq!(buffers.vertex_count(1), 3 * 6);
	let mut survivors: Vec<_> = (0..3).map(|index| buffers.particle(1, index)).collect();
	survivors.sort_by(|a, b| a.0[0].total_cmp(&b.0[0]));
	// Particles 0, 2, and 4 survive with half a life less, half a meter higher.
	for ((position, _, life), (x, expected_life)) in survivors.into_iter().zip([(0.0, 0.3), (2.0, 0.4), (4.0, 0.1)]) {
		assert_eq!(position, [x, 0.5, 0.0]);
		assert!((life - expected_life).abs() < 1e-5, "particle at {x} has {life} life left");
	}
}

/// Verifies every generated stage of both shapes and every module compiles with the platform shader compiler.
#[cfg(target_os = "macos")]
#[r#async::test]
async fn generated_programs_lower_to_the_platform_shader_language() {
	use crate::asset::handler::implementations::besl::PlatformShaderCompilerAdapter;

	for (system, source) in [("sparks", SPARKS), ("smoke", SMOKE)] {
		let programs = programs(source);
		for (stage, source, kind, settings) in [
			(
				"simulate",
				&programs.simulate,
				ShaderTypes::Compute,
				ShaderGenerationSettings::compute(utils::Extent::line(generator::SIMULATION_WORKGROUP_SIZE)),
			),
			(
				"vertex",
				&programs.vertex,
				ShaderTypes::Vertex,
				ShaderGenerationSettings::vertex(),
			),
			(
				"fragment",
				&programs.fragment,
				ShaderTypes::Fragment,
				ShaderGenerationSettings::fragment(),
			),
		] {
			let id = format!("{system}.particles#shaders/{stage}");
			PlatformShaderCompilerAdapter
				.compile(&id, source, None, kind, settings.name(id.clone()))
				.await
				.unwrap_or_else(|error| panic!("{id} should compile for the platform shader language: {error}"));
		}
	}
}
