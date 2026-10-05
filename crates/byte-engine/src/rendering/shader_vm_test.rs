//! Shared fixtures for executing production rendering shaders through the BESL VM.

use besl::vm::{Buffer, DescriptorBindings, ExecutableProgram, ExecutionConfig, ResourceSlot, Texture, Value};

const TEST_INSTRUCTION_LIMIT: usize = 4_000_000;
const TEST_CALL_DEPTH_LIMIT: usize = 128;

/// A column-major identity matrix in the BESL VM representation.
pub(crate) const IDENTITY_MATRIX: [f32; 16] = [1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0];

/// Converts a row-major matrix to the column-major element order the BESL VM multiplies with.
pub(crate) fn column_major(matrix: maths_rs::Mat4f) -> [f32; 16] {
	std::array::from_fn(|index| matrix[(index % 4) * 4 + index / 4])
}

/// Creates a VM input buffer from a compiled shader interface.
pub(crate) fn input_buffer(program: &ExecutableProgram, index: u8) -> Buffer {
	Buffer::new(program.input_layout(index).expect("Missing VM input layout.").clone())
}

/// Creates a VM output buffer from a compiled shader interface.
pub(crate) fn output_buffer(program: &ExecutableProgram, index: u8) -> Buffer {
	Buffer::new(program.output_layout(index).expect("Missing VM output layout.").clone())
}

/// Creates the builtin-position output buffer from a compiled vertex shader.
pub(crate) fn builtin_position_buffer(program: &ExecutableProgram) -> Buffer {
	Buffer::new(
		program
			.builtin_position_layout()
			.expect("Missing VM position output.")
			.clone(),
	)
}

/// Creates a push-constant buffer from a compiled shader interface.
pub(crate) fn push_constant_buffer(program: &ExecutableProgram) -> Buffer {
	Buffer::new(
		program
			.push_constant_layout()
			.expect("Missing VM push-constant layout.")
			.clone(),
	)
}

/// Links one checked-in BESL shader through the frontend production baking uses.
///
/// Returns the program rather than its `main`, because the program owns every function it calls. Pass the result to
/// [`compile`].
pub(crate) fn link_program(source: &str, name: &str) -> besl::NodeReference {
	let program = besl::compile_to_besl(source, None).unwrap_or_else(|error| {
		panic!("Failed to link {name}: {error:?}. The most likely cause is invalid syntax in the checked-in BESL asset.")
	});
	program.get_main().unwrap_or_else(|| {
		panic!("Missing {name} entry point. The most likely cause is that the checked-in BESL asset has no `main` function.")
	});
	program
}

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
pub(crate) fn buffer(program: &ExecutableProgram, slot: ResourceSlot) -> Buffer {
	Buffer::new(buffer_layout(program, slot))
}

fn buffer_layout(program: &ExecutableProgram, slot: ResourceSlot) -> besl::vm::BufferLayout {
	program
		.buffer_layout(slot)
		.expect(
			"Missing VM buffer layout. The most likely cause is that the production shader did not retain the expected binding.",
		)
		.clone()
}

/// Creates `element_count` elements of a production runtime-array buffer, such as `vertex_positions: vec3f[]`.
pub(crate) fn array_buffer(program: &ExecutableProgram, slot: ResourceSlot, element_count: usize) -> Buffer {
	Buffer::new_array(buffer_layout(program, slot), element_count).expect(
		"Failed to allocate a VM runtime-array buffer. The most likely cause is an element count that overflows memory.",
	)
}

/// Returns the configuration of workgroup lane `thread_idx`, bounded so a runaway shader fails its test instead of
/// hanging it.
///
/// Workgroup fixtures add the lane's coordinates and pass one per lane to [`ExecutableProgram::run_workgroup`].
pub(crate) fn lane_config(thread_idx: u32) -> ExecutionConfig {
	ExecutionConfig::new(TEST_INSTRUCTION_LIMIT)
		.with_call_depth_limit(TEST_CALL_DEPTH_LIMIT)
		.with_thread_idx(thread_idx)
}

/// Executes one bounded shader invocation at the requested two-dimensional thread coordinate.
pub(crate) fn run_at(program: &ExecutableProgram, descriptors: &mut DescriptorBindings<'_>, thread_id: [u32; 2]) {
	program.run_main_with_config(descriptors, &lane_config(0).with_thread_id(thread_id)).expect(
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

/// Runs a one-texel image-to-image compute program and returns the written color.
///
/// Image-transform passes share this: each binds a 1x1 source at slot 0 and a 1x1 result at slot 1,
/// then reads the single output texel back.
pub(crate) fn run_image_transform_vm(program: &ExecutableProgram, source_color: [f32; 4]) -> [f32; 4] {
	let mut source = texture_2d(1, 1, &[source_color]);
	let mut result = empty_image(1, 1);
	let mut descriptors = DescriptorBindings::new();
	descriptors.bind_image(ResourceSlot::new(0), &mut source);
	descriptors.bind_image(ResourceSlot::new(1), &mut result);
	run_at(program, &mut descriptors, [0, 0]);
	drop(descriptors);
	rgba(&result, [0, 0])
}
