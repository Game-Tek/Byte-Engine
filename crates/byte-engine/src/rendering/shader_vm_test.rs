//! Shared fixtures for executing production rendering shaders through the BESL VM.

use besl::vm::{Buffer, DescriptorBindings, DescriptorSlot, ExecutableProgram, ExecutionConfig, Texture, Value};

const TEST_INSTRUCTION_LIMIT: usize = 4_000_000;
const TEST_CALL_DEPTH_LIMIT: usize = 128;

/// Compiles the exact production shader entry point for a VM runtime test.
pub(crate) fn compile(main: besl::NodeReference) -> ExecutableProgram {
	ExecutableProgram::compile(main).expect(
		"Failed to compile a production shader with the BESL VM. The most likely cause is missing VM support for shader syntax.",
	)
}

/// Creates a tightly initialized two-dimensional texture without intermediate pixel storage.
pub(crate) fn texture_2d(width: u32, height: u32, texels: &[[f32; 4]]) -> Texture {
	assert_eq!(
		texels.len(),
		width as usize * height as usize,
		"Invalid VM texture fixture. The most likely cause is a texel count that does not match its extent."
	);
	let mut texture = Texture::new(width, height)
		.expect("Failed to create a VM texture. The most likely cause is a zero-sized test fixture.");
	for (index, texel) in texels.iter().copied().enumerate() {
		let index = index as u32;
		texture
			.write([index % width, index / width], texel)
			.expect("Failed to initialize a VM texture. The most likely cause is an invalid fixture coordinate.");
	}
	texture
}

/// Creates a zero-initialized image used as a shader output target.
pub(crate) fn empty_image(width: u32, height: u32) -> Texture {
	Texture::new(width, height).expect("Failed to create a VM image. The most likely cause is a zero-sized test fixture.")
}

/// Creates a host buffer using the layout discovered while compiling the shader.
pub(crate) fn buffer(program: &ExecutableProgram, slot: DescriptorSlot) -> Buffer {
	let layout = program
		.buffer_layout(slot)
		.expect("Missing VM buffer layout. The most likely cause is that the production shader did not retain the expected binding.")
		.clone();
	Buffer::new(layout)
}

/// Executes one bounded shader invocation at the requested two-dimensional thread coordinate.
pub(crate) fn run_at(program: &ExecutableProgram, descriptors: &mut DescriptorBindings<'_>, thread_id: [u32; 2]) {
	let config = ExecutionConfig::new(TEST_INSTRUCTION_LIMIT)
		.with_call_depth_limit(TEST_CALL_DEPTH_LIMIT)
		.with_thread_id(thread_id);
	program.run_main_with_config(descriptors, &config).expect(
		"Failed to execute a production shader with the BESL VM. The most likely cause is missing runtime support or an invalid fixture binding.",
	);
}

/// Reads one float RGBA texel from a VM texture.
pub(crate) fn rgba(texture: &Texture, coordinate: [u32; 2]) -> [f32; 4] {
	match texture
		.fetch(coordinate)
		.expect("Failed to read a VM texel. The most likely cause is an out-of-bounds assertion coordinate.")
	{
		Value::Vec4F(value) => value,
		_ => panic!("Unexpected VM texel type. The most likely cause is reading an integer image as float RGBA."),
	}
}

/// Compares finite RGBA values component by component with a caller-selected tolerance.
pub(crate) fn assert_rgba_close(actual: [f32; 4], expected: [f32; 4], tolerance: f32) {
	for (channel, (actual, expected)) in actual.into_iter().zip(expected).enumerate() {
		assert!(
			actual.is_finite() && (actual - expected).abs() <= tolerance,
			"Unexpected VM shader output in channel {channel}: expected {expected}, found {actual}. The most likely cause is a shader regression or incorrect VM semantics."
		);
	}
}
