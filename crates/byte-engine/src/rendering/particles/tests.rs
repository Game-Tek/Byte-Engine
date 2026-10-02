//! Executes the shared particle prepare kernel in the BESL VM and through the platform shader compiler.
//!
//! Resource management tests the kernels it generates from `.particles` assets.

use besl::vm::{DescriptorBindings, ExecutableProgram, ExecutionConfig, ResourceSlot, Value};

use super::shader_data;
use crate::rendering::shader_vm_test::{buffer, compile};

const FRAME_SLOT: ResourceSlot = ResourceSlot::new(shader_data::FRAME_SLOT.index());
const DRAWS_SLOT: ResourceSlot = ResourceSlot::new(shader_data::DRAWS_SLOT.index());
const DISPATCH_SLOT: ResourceSlot = ResourceSlot::new(shader_data::DISPATCH_SLOT.index());
const CAPACITY: u32 = 4096;

fn prepare_program() -> besl::NodeReference {
	besl::lex(
		besl::parse(include_str!(concat!(
			env!("CARGO_MANIFEST_DIR"),
			"/assets/rendering/particles/prepare.besl"
		)))
		.expect("prepare.besl should parse"),
	)
	.expect("prepare.besl should link")
}

/// Runs the prepare pass for a frame that writes half 0 and returns the simulation's workgroup count and the reset
/// draw record.
fn prepare(previous_vertex_count: u32, spawn_total: u32, reset: bool) -> ([u32; 3], [u32; 4]) {
	let program: ExecutableProgram = compile(prepare_program());
	let mut frame = buffer(&program, FRAME_SLOT);
	for (field, value) in [
		("side", 0),
		("capacity", CAPACITY),
		("spawn_total", spawn_total),
		("reset", u32::from(reset)),
	] {
		frame.write(field, Value::U32(value)).expect("frame field");
	}
	let mut draws = buffer(&program, DRAWS_SLOT);
	draws
		.write_array_element(4, Value::U32(previous_vertex_count))
		.expect("previous draw count");
	// Leftovers from an older frame that the pass must reset.
	draws.write_array_element(0, Value::U32(600)).expect("stale draw count");
	let mut dispatch = buffer(&program, DISPATCH_SLOT);

	let mut descriptors = DescriptorBindings::new();
	descriptors.bind_buffer(FRAME_SLOT, &mut frame);
	descriptors.bind_buffer(DRAWS_SLOT, &mut draws);
	descriptors.bind_buffer(DISPATCH_SLOT, &mut dispatch);
	program
		.run_workgroup(&mut descriptors, &[ExecutionConfig::new(1_000_000)])
		.expect("Failed to run the particle prepare pass in the BESL VM.");
	drop(descriptors);

	let group_count = match dispatch.read("group_count").expect("group count") {
		Value::Vec3U(group_count) => group_count,
		value => panic!("Unexpected dispatch value: {value:?}."),
	};
	let record = std::array::from_fn(|index| match draws.read_array_element(index).expect("draw record") {
		Value::U32(value) => value,
		value => panic!("Unexpected draw value: {value:?}."),
	});
	(group_count, record)
}

/// Verifies the simulation covers last frame's particles plus this frame's spawns, and starts an empty draw.
#[test]
fn prepare_covers_live_and_new_particles() {
	// 100 live and 28 new particles fill exactly two workgroups of 64.
	assert_eq!(prepare(100 * 6, 28, false), ([2, 1, 1], [0, 1, 0, 0]));
}

/// Verifies spawns stop at the free capacity, and that a reset frame ignores the previous half.
#[test]
fn prepare_clamps_spawns_and_honors_reset() {
	assert_eq!(prepare((CAPACITY - 3) * 6, 1000, false).0, [CAPACITY / 64, 1, 1]);
	assert_eq!(prepare((CAPACITY - 3) * 6, 10, true).0, [1, 1, 1]);
}

/// Verifies the prepare kernel compiles with the platform shader compiler, past BESL linking.
#[cfg(target_os = "macos")]
#[compio::test]
async fn prepare_lowers_to_the_platform_shader_language() {
	use resource_management::shader::ShaderGenerationSettings;
	use resource_management::shader::besl::backends::platform::PlatformShaderCompiler;

	PlatformShaderCompiler::new()
		.generate(
			&ShaderGenerationSettings::compute(utils::Extent::line(1)).name("particle_prepare".to_string()),
			&prepare_program(),
		)
		.await
		.expect("prepare.besl should compile for the platform shader language");
}
