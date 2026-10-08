//! Focused regressions for the VM's private instruction and numeric semantics.

use super::{
	Buffer, BufferLayout, DescriptorBindings, ExecutableProgram, ExecutionConfig, MeshOutputs, ResourceSlot, Sampler,
	SamplerReductionMode, SpecializationValues, TaskOutputs, Texture, Value, VmError, WorkgroupState,
	builtin_instance_index_slot, builtin_vertex_index_slot, f16, input_slot, output_slot, reflect_vector,
};
use crate::{BindingTypes, Expressions, Node, Operators, compile_to_besl};

fn read_f32s(buffer: &Buffer, count: usize) -> Vec<f32> {
	buffer
		.bytes()
		.as_chunks::<4>()
		.0
		.iter()
		.take(count)
		.map(|chunk| f32::from_ne_bytes(*chunk))
		.collect()
}

fn read_u32s(buffer: &Buffer, count: usize) -> Vec<u32> {
	buffer
		.bytes()
		.as_chunks::<4>()
		.0
		.iter()
		.take(count)
		.map(|chunk| u32::from_ne_bytes(*chunk))
		.collect()
}

fn compile_test_program(script: &str, root: Option<Node>) -> ExecutableProgram {
	let program = compile_to_besl(script, root).expect("Expected lexed program");
	ExecutableProgram::compile(program).expect("Expected runnable program")
}

fn buffer_for_slot(executable: &ExecutableProgram, slot: ResourceSlot) -> Buffer {
	let layout = executable.buffer_layout(slot).expect("Expected buffer layout").clone();
	Buffer::new(layout)
}

/// Allocates the stage-interface buffer that `layout`, from [`ExecutableProgram::input_layout`] or
/// [`ExecutableProgram::output_layout`], describes.
fn interface_buffer(layout: Option<&BufferLayout>) -> Buffer {
	Buffer::new(layout.expect("Expected interface layout").clone())
}

fn run_with_buffer(executable: &ExecutableProgram, slot: ResourceSlot, buffer: &mut Buffer) {
	let mut descriptors = DescriptorBindings::new();
	descriptors.bind_buffer(slot, buffer);
	executable.run_main(&mut descriptors).expect("Expected execution to succeed");
}

/// Builds a root with one read-write buffer binding `name` at `slot`, whose members have the named built-in types.
fn buffer_root(name: &str, slot: u32, members: &[(&str, &str)]) -> Node {
	let mut root = Node::root();
	let members = members
		.iter()
		.map(|(member, type_name)| Node::member(member, root.get_child(type_name).expect("Expected built-in type")).into())
		.collect();
	root.add_child(Node::binding(name, BindingTypes::Buffer { members }, slot, true, true).into());
	root
}

/// Runs `main` with a fresh buffer bound at `slot` and returns that buffer.
fn run_slot(executable: &ExecutableProgram, slot: u32) -> Buffer {
	let slot = ResourceSlot::new(slot);
	let mut buffer = buffer_for_slot(executable, slot);
	run_with_buffer(executable, slot, &mut buffer);
	buffer
}

fn write_texture(texture: &mut Texture, texels: &[([u32; 2], [f32; 4])]) {
	for (coord, value) in texels {
		texture.write(*coord, *value).expect("Expected texture write to succeed");
	}
}

#[test]
fn discard_terminates_the_current_invocation_across_function_calls() {
	let script = r#"
	discard_after_write: fn () -> void {
		buff.value = 1.0;
		discard;
		buff.value = 2.0;
	}

	main: fn () -> void {
		buff.value = 3.0;
		discard_after_write();
		buff.value = 4.0;
	}
	"#;

	let root = buffer_root("buff", 0, &[("value", "f32")]);

	let executable = compile_test_program(script, Some(root));
	let buffer = run_slot(&executable, 0);

	assert_eq!(buffer.read_f32("value").expect("Expected f32 member"), 1.0);
}

#[test]
fn apply_arithmetic_supports_all_basic_scalar_operations() {
	use super::{ArithmeticOperator::*, apply_arithmetic};

	let identity = [1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0];
	for (operator, left, right, expected) in [
		(Add, Value::U32(2), Value::U32(3), Value::U32(5)),
		(Subtract, Value::I32(9), Value::I32(4), Value::I32(5)),
		(Multiply, Value::U16(6), Value::U16(7), Value::U16(42)),
		(Divide, Value::F32(9.0), Value::F32(2.0), Value::F32(4.5)),
		(Modulo, Value::U8(20), Value::U8(6), Value::U8(2)),
		(
			Add,
			Value::Vec3F([1.0, 2.0, 3.0]),
			Value::Vec3F([4.0, 5.0, 6.0]),
			Value::Vec3F([5.0, 7.0, 9.0]),
		),
		(
			Multiply,
			Value::Vec4F([1.0, 2.0, 3.0, 4.0]),
			Value::F32(2.0),
			Value::Vec4F([2.0, 4.0, 6.0, 8.0]),
		),
		(
			Add,
			Value::Mat4F(identity),
			Value::F32(1.0),
			Value::Mat4F(identity.map(|value| value + 1.0)),
		),
	] {
		assert_eq!(apply_arithmetic(operator, &left, &right), Ok(expected), "{operator:?}");
	}
}

#[test]
fn executable_program_round_trips_vec4u16_construction_arithmetic_and_member_access() {
	let script = r#"
	main: fn () -> void {
		let left: vec4u16 = vec4u16(1, 2, 3, 4);
		let right: vec4u16 = vec4u16(4, 3, 2, 1);
		buff.value = left + right;
		buff.last = buff.value.w;
	}
	"#;

	let root = buffer_root("buff", 30, &[("value", "vec4u16"), ("last", "u16")]);

	let executable = compile_test_program(script, Some(root));
	let slot = ResourceSlot::new(30);
	let layout = executable.buffer_layout(slot).expect("Expected vec4u16 buffer layout");

	assert_eq!(layout.member("value").unwrap().value_type().size(), 8);
	assert_eq!(layout.member("last").unwrap().offset(), 8);
	let mut buffer = Buffer::new(layout.clone());
	run_with_buffer(&executable, slot, &mut buffer);

	assert_eq!(buffer.read("value").unwrap(), Value::Vec4U16([5, 5, 5, 5]));
	assert_eq!(buffer.read("last").unwrap(), Value::U16(5));
}

#[test]
fn executable_program_round_trips_f16_arithmetic_casts_and_packed_buffer_values() {
	let script = r#"
	main: fn () -> void {
		let left: vec2f16 = vec2f16(1.25, 2.0);
		let right: vec2f16 = vec2f16(0.5, 0.25);
		let source: vec2f = vec2f(3.5, 4.25);
		buff.value = left + right * 2.0;
		buff.narrowed = vec2f16(source);
		buff.widened = vec2f(buff.narrowed);
		buff.component = buff.value.y;
		buff.as_f32 = f32(buff.component);
		buff.as_u32 = u32(f16(7.8));
		let literal: f16 = 0.25;
		buff.literal = literal;
	}
	"#;

	let root = buffer_root(
		"buff",
		31,
		&[
			("value", "vec2f16"),
			("narrowed", "vec2f16"),
			("widened", "vec2f"),
			("component", "f16"),
			("as_f32", "f32"),
			("as_u32", "u32"),
			("literal", "f16"),
		],
	);

	let executable = compile_test_program(script, Some(root));
	let slot = ResourceSlot::new(31);
	let layout = executable.buffer_layout(slot).expect("Expected f16 buffer layout");

	assert_eq!(layout.member("value").unwrap().value_type().size(), 4);
	assert_eq!(layout.member("component").unwrap().value_type().size(), 2);
	assert_eq!(layout.member("component").unwrap().offset(), 16);
	let mut buffer = Buffer::new(layout.clone());

	run_with_buffer(&executable, slot, &mut buffer);

	let half = |value| super::f16::from_f32(value);

	assert_eq!(buffer.read("value").unwrap(), Value::Vec2F16([half(2.25), half(2.5)]));
	assert_eq!(buffer.read("narrowed").unwrap(), Value::Vec2F16([half(3.5), half(4.25)]));
	assert_eq!(buffer.read("widened").unwrap(), Value::Vec2F([3.5, 4.25]));
	assert_eq!(buffer.read_f16("component").unwrap(), half(2.5));
	assert_eq!(buffer.read_f32("as_f32").unwrap(), 2.5);
	assert_eq!(buffer.read("as_u32").unwrap(), Value::U32(7));
	assert_eq!(buffer.read_f16("literal").unwrap(), half(0.25));
	assert_eq!(
		u16::from_ne_bytes(buffer.bytes()[16..18].try_into().expect("Expected f16 component bytes")),
		half(2.5).to_bits()
	);
}

#[test]
fn executable_program_evaluates_mat4f_arithmetic_before_writing_to_a_bound_buffer_member() {
	let script = r#"
	main: fn () -> void {
		let lhs: mat4f = mat4f(
			vec4f(1.0, 0.0, 0.0, 0.0),
			vec4f(0.0, 1.0, 0.0, 0.0),
			vec4f(0.0, 0.0, 1.0, 0.0),
			vec4f(0.0, 0.0, 0.0, 1.0)
		);
		let rhs: mat4f = mat4f(
			vec4f(1.0, 1.0, 1.0, 1.0),
			vec4f(1.0, 1.0, 1.0, 1.0),
			vec4f(1.0, 1.0, 1.0, 1.0),
			vec4f(1.0, 1.0, 1.0, 1.0)
		);
		buff.value = lhs + rhs;
	}
	"#;

	let root = buffer_root("buff", 4, &[("value", "mat4f")]);

	let executable = compile_test_program(script, Some(root));

	let buffer = run_slot(&executable, 4);

	assert_eq!(
		read_f32s(&buffer, 16),
		vec![2.0, 1.0, 1.0, 1.0, 1.0, 2.0, 1.0, 1.0, 1.0, 1.0, 2.0, 1.0, 1.0, 1.0, 1.0, 2.0,]
	);
}

#[test]
fn executable_program_indexes_mat4f_and_mat4x3f_columns() {
	let script = r#"
	main: fn () -> void {
		let projection: mat4f = mat4f(
			vec4f(1.0, 2.0, 3.0, 4.0),
			vec4f(5.0, 6.0, 7.0, 8.0),
			vec4f(9.0, 10.0, 11.0, 12.0),
			vec4f(13.0, 14.0, 15.0, 16.0)
		);
		let model: mat4x3f = mat4x3f(
			vec3f(1.0, 2.0, 3.0),
			vec3f(4.0, 5.0, 6.0),
			vec3f(7.0, 8.0, 9.0),
			vec3f(10.0, 11.0, 12.0)
		);
		result.projection_column = projection[2];
		result.model_column = model[3];
	}
	"#;
	let root = buffer_root("result", 41, &[("projection_column", "vec4f"), ("model_column", "vec3f")]);
	let executable = compile_test_program(script, Some(root));
	let result = run_slot(&executable, 41);

	assert_eq!(
		result
			.read("projection_column")
			.expect("Missing indexed mat4f column. The most likely cause is broken matrix indexing or result-buffer storage.",),
		Value::Vec4F([9.0, 10.0, 11.0, 12.0])
	);
	assert_eq!(
		result.read("model_column").expect(
			"Missing indexed mat4x3f column. The most likely cause is broken matrix indexing or result-buffer storage.",
		),
		Value::Vec3F([10.0, 11.0, 12.0])
	);
}

#[test]
fn executable_program_reads_and_writes_vector_components_by_runtime_index() {
	let script = r#"
	main: fn () -> void {
		let words: vec4u = vec4u(1, 2, 3, 4);
		let index: u32 = result.index;
		words[index] = words[index] | 16;
		words[index + 1] = 7;
		result.words = words;
		result.selected = words[index];
	}
	"#;
	let root = buffer_root("result", 42, &[("index", "u32"), ("words", "vec4u"), ("selected", "u32")]);
	let executable = compile_test_program(script, Some(root));
	let slot = ResourceSlot::new(42);
	let mut result = buffer_for_slot(&executable, slot);
	result
		.write("index", Value::U32(1))
		.expect("Failed to write the component index. The most likely cause is a mismatched result layout.");
	run_with_buffer(&executable, slot, &mut result);

	assert_eq!(
		result.read("words").expect("Missing vector result"),
		Value::Vec4U([1, 18, 7, 4])
	);
	assert_eq!(result.read("selected").expect("Missing component result"), Value::U32(18));

	// The last component has no successor, so `words[index + 1]` must fail instead of writing past the vector.
	result
		.write("index", Value::U32(3))
		.expect("Failed to write the component index. The most likely cause is a mismatched result layout.");
	let mut descriptors = DescriptorBindings::new();
	descriptors.bind_buffer(slot, &mut result);
	assert!(matches!(
		executable.run_main(&mut descriptors),
		Err(VmError::BufferArrayIndexOutOfBounds { index: 4, count: 4 })
	));
}

#[test]
fn executable_program_calls_function_with_parameters_and_return_value() {
	let script = r#"
	add: fn (lhs: f32, rhs: f32) -> f32 {
		return lhs + rhs;
	}

	main: fn () -> void {
		buff.value = add(3.0, 4.5);
	}
	"#;

	let root = buffer_root("buff", 5, &[("value", "f32")]);

	let executable = compile_test_program(script, Some(root));

	let slot = ResourceSlot::new(5);
	let mut buffer = buffer_for_slot(&executable, slot);
	let mut descriptors = DescriptorBindings::new();
	descriptors.bind_buffer(slot, &mut buffer);
	for _ in 0..2 {
		executable
			.run_main(&mut descriptors)
			.expect("Expected repeated function-call execution to succeed");
	}
	drop(descriptors);

	assert_eq!(buffer.read_f32("value").expect("Expected f32 member"), 7.5);
}

#[test]
fn executable_program_calls_function_and_returns_scalar_array() {
	let script = r#"
	mirror_indices: fn (indices: u32[3]) -> u32[3] {
		return indices;
	}

	main: fn () -> void {
		let indices: u32[3] = mirror_indices(u32[3](4, 8, 15));
		buff.value = indices[1];
	}
	"#;

	let root = buffer_root("buff", 6, &[("value", "u32")]);

	let executable = compile_test_program(script, Some(root));
	let buffer = run_slot(&executable, 6);

	assert_eq!(buffer.read("value").expect("Expected selected array element"), Value::U32(8));
}

#[test]
fn executable_program_fetches_texture_texels_into_a_bound_buffer_member() {
	let script = r#"
	main: fn () -> void {
		let coord: vec2u = vec2u(1, 0);
		buff.value = fetch(texture, coord);
	}
	"#;

	let mut root = buffer_root("buff", 8, &[("value", "vec4f")]);
	root.add_child(
		Node::binding(
			"texture",
			BindingTypes::CombinedImageSampler { format: String::new() },
			7,
			true,
			false,
		)
		.into(),
	);

	let executable = compile_test_program(script, Some(root));

	let texture_slot = ResourceSlot::new(7);
	let buffer_slot = ResourceSlot::new(8);
	let mut texture = Texture::new(2, 2).expect("Expected texture allocation");
	let mut buffer = buffer_for_slot(&executable, buffer_slot);
	write_texture(
		&mut texture,
		&[
			([0, 0], [1.0, 0.0, 0.0, 1.0]),
			([1, 0], [0.0, 1.0, 0.0, 1.0]),
			([0, 1], [0.0, 0.0, 1.0, 1.0]),
			([1, 1], [1.0, 1.0, 1.0, 1.0]),
		],
	);

	{
		let mut descriptors = DescriptorBindings::new();
		descriptors.bind_texture(texture_slot, &mut texture);
		descriptors.bind_buffer(buffer_slot, &mut buffer);
		executable.run_main(&mut descriptors).expect("Expected execution to succeed");
	}

	assert_eq!(read_f32s(&buffer, 4), vec![0.0, 1.0, 0.0, 1.0]);
}

#[test]
fn executable_program_samples_textures_inside_arithmetic_expressions() {
	let script = r#"
	main: fn () -> void {
		let color: vec4f = sample(texture_sampler, vec2f(0.5, 0.5));
		buff.value = color * 2.0;
	}
	"#;

	let mut root = buffer_root("buff", 10, &[("value", "vec4f")]);
	root.add_child(
		Node::binding(
			"texture_sampler",
			BindingTypes::CombinedImageSampler { format: String::new() },
			9,
			true,
			false,
		)
		.into(),
	);

	let executable = compile_test_program(script, Some(root));

	let texture_slot = ResourceSlot::new(9);
	let buffer_slot = ResourceSlot::new(10);
	let mut texture = Texture::new(2, 2).expect("Expected texture allocation");
	let mut buffer = buffer_for_slot(&executable, buffer_slot);
	write_texture(
		&mut texture,
		&[
			([0, 0], [0.0, 0.0, 0.0, 1.0]),
			([1, 0], [1.0, 0.0, 0.0, 1.0]),
			([0, 1], [0.0, 1.0, 0.0, 1.0]),
			([1, 1], [1.0, 1.0, 0.0, 1.0]),
		],
	);

	{
		let mut descriptors = DescriptorBindings::new();
		descriptors.bind_texture(texture_slot, &mut texture);
		descriptors.bind_buffer(buffer_slot, &mut buffer);
		executable.run_main(&mut descriptors).expect("Expected execution to succeed");
	}

	assert_eq!(read_f32s(&buffer, 4), vec![1.0, 1.0, 0.0, 2.0]);
}

#[test]
fn combined_sampler_reduction_modes_select_weighted_minimum_and_maximum_footprints() {
	let script = r#"
	main: fn () -> void {
		buff.value = texture_lod(texture_sampler, vec2f(0.5, 0.5), 0.0);
	}
	"#;
	let mut root = buffer_root("buff", 10, &[("value", "vec4f")]);
	root.add_child(
		Node::binding(
			"texture_sampler",
			BindingTypes::CombinedImageSampler { format: String::new() },
			9,
			true,
			false,
		)
		.into(),
	);
	let executable = compile_test_program(script, Some(root));
	let texture_slot = ResourceSlot::new(9);
	let buffer_slot = ResourceSlot::new(10);
	let mut texture = Texture::new(2, 2).expect("Expected texture allocation");
	write_texture(
		&mut texture,
		&[
			([0, 0], [0.0, 8.0, 2.0, 1.0]),
			([1, 0], [2.0, 6.0, 4.0, 1.0]),
			([0, 1], [6.0, 4.0, 8.0, 1.0]),
			([1, 1], [8.0, 2.0, 6.0, 1.0]),
		],
	);

	let mut sample = |reduction_mode| {
		let mut buffer = buffer_for_slot(&executable, buffer_slot);
		{
			let mut descriptors = DescriptorBindings::new();
			descriptors.bind_texture_with_sampler(texture_slot, &mut texture, Sampler::new(reduction_mode));
			descriptors.bind_buffer(buffer_slot, &mut buffer);
			executable
				.run_main(&mut descriptors)
				.expect("Expected combined sampler execution to succeed");
		}
		read_f32s(&buffer, 4)
	};

	assert_eq!(sample(SamplerReductionMode::WeightedAverage), vec![4.0, 5.0, 5.0, 1.0]);
	assert_eq!(sample(SamplerReductionMode::Min), vec![0.0, 2.0, 2.0, 1.0]);
	assert_eq!(sample(SamplerReductionMode::Max), vec![8.0, 8.0, 8.0, 1.0]);
}

#[test]
fn nearest_sampler_reads_the_texel_under_the_coordinate_and_clamps_to_the_edge() {
	let script = r#"
	main: fn () -> void {
		buff.inside = texture_lod(texture_sampler, vec2f(0.74, 0.26), 0.0).x;
		buff.outside = texture_lod(texture_sampler, vec2f(1.5, -0.5), 0.0).x;
	}
	"#;
	let mut root = buffer_root("buff", 10, &[("inside", "f32"), ("outside", "f32")]);
	root.add_child(
		Node::binding(
			"texture_sampler",
			BindingTypes::CombinedImageSampler { format: String::new() },
			9,
			true,
			false,
		)
		.into(),
	);
	let executable = compile_test_program(script, Some(root));
	let mut texture = Texture::new(2, 2).expect("Expected texture allocation");
	write_texture(
		&mut texture,
		&[
			([0, 0], [1.0, 0.0, 0.0, 0.0]),
			([1, 0], [2.0, 0.0, 0.0, 0.0]),
			([0, 1], [3.0, 0.0, 0.0, 0.0]),
			([1, 1], [4.0, 0.0, 0.0, 0.0]),
		],
	);
	let mut buffer = buffer_for_slot(&executable, ResourceSlot::new(10));
	{
		let mut descriptors = DescriptorBindings::new();
		descriptors.bind_texture_with_sampler(ResourceSlot::new(9), &mut texture, Sampler::nearest());
		descriptors.bind_buffer(ResourceSlot::new(10), &mut buffer);
		executable.run_main(&mut descriptors).expect("Expected execution to succeed");
	}

	// Both coordinates select texel (1, 0): the first lies inside it and the second clamps to it from outside the
	// image. A linear sampler would blend texels instead.
	assert_eq!(read_f32s(&buffer, 2), vec![2.0, 2.0]);
}

#[test]
fn downsample_intrinsics_select_the_requested_reduction_independent_of_sampler_state() {
	let script = r#"
	main: fn () -> void {
		buff.minimum = downsample_min(texture_sampler, vec2f(0.5, 0.5), 0.0);
		buff.maximum = downsample_max(texture_sampler, vec2f(0.5, 0.5), 0.0);
	}
	"#;
	let mut root = buffer_root("buff", 10, &[("minimum", "f32"), ("maximum", "f32")]);
	root.add_child(
		Node::binding(
			"texture_sampler",
			BindingTypes::CombinedImageSampler { format: String::new() },
			9,
			true,
			false,
		)
		.into(),
	);
	let executable = compile_test_program(script, Some(root));
	let texture_slot = ResourceSlot::new(9);
	let buffer_slot = ResourceSlot::new(10);
	let mut texture = Texture::new(2, 2).expect("Expected texture allocation");
	write_texture(
		&mut texture,
		&[
			([0, 0], [1.0, 0.0, 0.0, 1.0]),
			([1, 0], [7.0, 0.0, 0.0, 1.0]),
			([0, 1], [3.0, 0.0, 0.0, 1.0]),
			([1, 1], [5.0, 0.0, 0.0, 1.0]),
		],
	);
	let mut buffer = buffer_for_slot(&executable, buffer_slot);
	let mut descriptors = DescriptorBindings::new();
	descriptors.bind_texture_with_sampler(
		texture_slot,
		&mut texture,
		Sampler::new(SamplerReductionMode::WeightedAverage),
	);
	descriptors.bind_buffer(buffer_slot, &mut buffer);
	executable
		.run_main(&mut descriptors)
		.expect("Expected conservative downsample execution to succeed");

	assert_eq!(read_f32s(&buffer, 2), vec![1.0, 7.0]);
}

#[test]
fn gather_returns_the_texel_quad_in_platform_order() {
	let script = r#"
	main: fn () -> void {
		let quad: vec4f = gather(texture_sampler, vec2f(0.5, 0.5));
		let layer_quad: vec4f = gather(array_sampler, vec2f(0.5, 0.5), 1);
		buff.x = quad.x;
		buff.y = quad.y;
		buff.z = quad.z;
		buff.w = quad.w;
		buff.layer_x = layer_quad.x;
	}
	"#;
	let mut root = buffer_root(
		"buff",
		10,
		&[("x", "f32"), ("y", "f32"), ("z", "f32"), ("w", "f32"), ("layer_x", "f32")],
	);
	root.add_child(
		Node::binding(
			"texture_sampler",
			BindingTypes::CombinedImageSampler { format: String::new() },
			9,
			true,
			false,
		)
		.into(),
	);
	root.add_child(
		Node::binding(
			"array_sampler",
			BindingTypes::CombinedImageSampler {
				format: "ArrayTexture2D".to_string(),
			},
			8,
			true,
			false,
		)
		.into(),
	);
	let executable = compile_test_program(script, Some(root));
	let mut texture = Texture::new(2, 2).expect("Expected texture allocation");
	write_texture(
		&mut texture,
		&[
			([0, 0], [1.0, 0.0, 0.0, 1.0]),
			([1, 0], [7.0, 0.0, 0.0, 1.0]),
			([0, 1], [3.0, 0.0, 0.0, 1.0]),
			([1, 1], [5.0, 0.0, 0.0, 1.0]),
		],
	);
	let mut array = Texture::new_3d(2, 2, 2).expect("Expected array texture allocation");
	array
		.write_3d([0, 1, 1], [9.0, 0.0, 0.0, 1.0])
		.expect("Expected array texel write");
	let mut buffer = buffer_for_slot(&executable, ResourceSlot::new(10));
	let mut descriptors = DescriptorBindings::new();
	descriptors.bind_texture(ResourceSlot::new(9), &mut texture);
	descriptors.bind_texture(ResourceSlot::new(8), &mut array);
	descriptors.bind_buffer(ResourceSlot::new(10), &mut buffer);
	executable
		.run_main(&mut descriptors)
		.expect("Expected gather execution to succeed");

	// The quad around the center: x and y from the second row, left then right, z and w from the first, right then
	// left; the array layer reads its own texels.
	assert_eq!(read_f32s(&buffer, 5), vec![3.0, 5.0, 7.0, 1.0, 9.0]);
}

#[test]
fn executable_program_writes_a_pixel_to_a_bound_image() {
	let script = r#"
	main: fn () -> void {
		write(image, vec2u(1, 0), vec4f(0.25, 0.5, 0.75, 1.0));
	}
	"#;

	let mut root = Node::root();
	root.add_child(
		Node::binding(
			"image",
			BindingTypes::Image {
				format: "rgba8".to_string(),
			},
			11,
			false,
			true,
		)
		.into(),
	);

	let executable = compile_test_program(script, Some(root));

	let image_slot = ResourceSlot::new(11);
	let mut image = Texture::new(2, 2).expect("Expected texture allocation");

	{
		let mut descriptors = DescriptorBindings::new();
		descriptors.bind_image(image_slot, &mut image);
		executable.run_main(&mut descriptors).expect("Expected execution to succeed");
	}

	assert_eq!(
		image.fetch([1, 0]).expect("Expected image fetch"),
		Value::Vec4F([0.25, 0.5, 0.75, 1.0])
	);
}

#[test]
fn executable_program_reads_and_writes_buffer_array_elements() {
	let script = r#"
	main: fn () -> void {
		let index: u32 = 1;
		buff.values[index] = 7.5;
		buff.value = buff.values[index];
	}
	"#;

	let mut root = Node::root();
	let float_type = root.get_child("f32").expect("Expected f32");

	root.add_child(
		Node::binding(
			"buff",
			BindingTypes::Buffer {
				members: vec![
					Node::array("values", float_type.clone(), 3),
					Node::member("value", float_type).into(),
				],
			},
			12,
			true,
			true,
		)
		.into(),
	);

	let executable = compile_test_program(script, Some(root));

	let slot = ResourceSlot::new(12);
	let layout = executable.buffer_layout(slot).expect("Expected buffer layout").clone();
	let mut buffer = Buffer::new(layout.clone());
	let values_member = layout.member("values").expect("Expected values member");
	buffer
		.write_value(
			values_member.offset() + values_member.value_type().size(),
			values_member.value_type(),
			&Value::F32(2.5),
		)
		.expect("Expected array element write to succeed");

	run_with_buffer(&executable, slot, &mut buffer);

	assert_eq!(read_f32s(&buffer, 4), vec![0.0, 7.5, 0.0, 7.5]);
}

#[test]
fn executable_program_reads_and_writes_same_named_buffer_members() {
	let script = r#"
	main: fn () -> void {
		pixel_mapping.pixel_mapping[0] = meshes.meshes[1];
	}
	"#;

	let mut root = Node::root();
	let u32_type = root.get_child("u32").expect("Expected u32");

	root.add_children(vec![
		Node::binding(
			"meshes",
			BindingTypes::Buffer {
				members: vec![Node::array("meshes", u32_type.clone(), 2)],
			},
			24,
			true,
			false,
		)
		.into(),
		Node::binding(
			"pixel_mapping",
			BindingTypes::Buffer {
				members: vec![Node::array("pixel_mapping", u32_type, 2)],
			},
			25,
			false,
			true,
		)
		.into(),
	]);

	let executable = compile_test_program(script, Some(root));

	let input_slot = ResourceSlot::new(24);
	let output_slot = ResourceSlot::new(25);
	let mut input = buffer_for_slot(&executable, input_slot);
	input
		.write_array_element(1, Value::U32(42))
		.expect("Expected array element write to succeed");

	let mut output = buffer_for_slot(&executable, output_slot);

	{
		let mut descriptors = DescriptorBindings::new();
		descriptors.bind_buffer(input_slot, &mut input);
		descriptors.bind_buffer(output_slot, &mut output);
		executable.run_main(&mut descriptors).expect("Expected execution to succeed");
	}

	assert_eq!(read_u32s(&output, 2), vec![42, 0]);
}

#[test]
fn executable_program_compile_rejects_raw_code_blocks() {
	let script = r#"
	main: fn () -> void {}
	"#;

	let program = compile_to_besl(script, None).expect("Expected lexed program");
	let main = program.get_descendant("main").expect("Expected main function");
	main.borrow_mut()
		.add_child(Node::raw(Some("gl_Position = vec4(0);".to_string()), None, None, vec![], vec![]).into());

	match ExecutableProgram::compile(program) {
		Err(error) => assert_eq!(error, super::VmError::UnsupportedRawCode),
		Ok(_) => panic!("Expected raw code rejection"),
	}
}

#[test]
fn executable_program_requires_bound_push_constant() {
	let script = r#"
	main: fn () -> void {
		buff.value = push_constant.material_id;
	}
	"#;

	let mut root = buffer_root("buff", 15, &[("value", "f32")]);
	let float_type = root.get_child("f32").expect("Expected f32");
	root.add_child(Node::push_constant(vec![Node::member("material_id", float_type).into()]).into());

	let executable = compile_test_program(script, Some(root));

	let slot = ResourceSlot::new(15);
	let mut buffer = buffer_for_slot(&executable, slot);

	let error = {
		let mut descriptors = DescriptorBindings::new();
		descriptors.bind_buffer(slot, &mut buffer);
		executable
			.run_main(&mut descriptors)
			.expect_err("Expected missing push constant error")
	};

	assert_eq!(error, VmError::MissingPushConstant);
}

#[test]
fn executable_program_reads_implicit_vertex_invocation_indices() {
	let script = r#"
	out_vertex_index: output<u32, 0>;
	out_instance_index: output<u32, 1>;
	main: fn () -> void {
		out_vertex_index = vertex_index;
		out_instance_index = instance_index;
	}
	"#;

	let executable = compile_test_program(script, None);
	let mut vertex_index = buffer_for_slot(&executable, builtin_vertex_index_slot());
	let mut instance_index = buffer_for_slot(&executable, builtin_instance_index_slot());
	let mut out_vertex_index = interface_buffer(executable.output_layout(0));
	let mut out_instance_index = interface_buffer(executable.output_layout(1));
	vertex_index
		.write("vertex_index", Value::U32(17))
		.expect("Expected vertex index write to succeed");
	instance_index
		.write("instance_index", Value::U32(23))
		.expect("Expected instance index write to succeed");

	{
		let mut descriptors = DescriptorBindings::new();
		descriptors.bind_buffer(builtin_vertex_index_slot(), &mut vertex_index);
		descriptors.bind_buffer(builtin_instance_index_slot(), &mut instance_index);
		descriptors.bind_buffer(output_slot(0), &mut out_vertex_index);
		descriptors.bind_buffer(output_slot(1), &mut out_instance_index);
		executable.run_main(&mut descriptors).expect("Expected execution to succeed");
	}

	assert_eq!(out_vertex_index.read("out_vertex_index"), Ok(Value::U32(17)));
	assert_eq!(out_instance_index.read("out_instance_index"), Ok(Value::U32(23)));
}

#[test]
fn executable_program_rejects_writing_to_input_interfaces() {
	let script = r#"
	main: fn () -> void {
		in_color = vec4f(1.0, 0.0, 0.0, 1.0);
	}
	"#;

	let mut root = Node::root();
	let vec4f_type = root.get_child("vec4f").expect("Expected vec4f");
	root.add_child(Node::input("in_color", vec4f_type, 0).into());

	let program = compile_to_besl(script, Some(root)).expect("Expected lexed program");
	let error = match ExecutableProgram::compile(program) {
		Err(error) => error,
		Ok(_) => panic!("Expected input write rejection"),
	};

	assert!(matches!(
		error,
		VmError::UnsupportedAssignmentTarget { .. } | VmError::UnsupportedExpression { .. }
	));
}

#[test]
fn executable_program_rejects_reading_from_output_interfaces() {
	let script = r#"
	main: fn () -> void {
		let color: vec4f = out_color;
	}
	"#;

	let mut root = Node::root();
	let vec4f_type = root.get_child("vec4f").expect("Expected vec4f");
	root.add_child(Node::output("out_color", vec4f_type, 0).into());

	let program = compile_to_besl(script, Some(root)).expect("Expected lexed program");
	let error = match ExecutableProgram::compile(program) {
		Err(error) => error,
		Ok(_) => panic!("Expected output read rejection"),
	};

	assert!(matches!(error, VmError::UnsupportedExpression { .. }));
}

#[test]
fn executable_program_supports_vertex_to_fragment_interface_workflows() {
	let vertex_script = r#"
	main: fn () -> void {
		out_color = in_color * 0.5;
	}
	"#;
	let fragment_script = r#"
	main: fn () -> void {
		out_color = in_color + vec4f(0.25, 0.0, 0.0, 0.0);
	}
	"#;

	let mut vertex_root = Node::root();
	let vertex_vec4f = vertex_root.get_child("vec4f").expect("Expected vec4f");
	vertex_root.add_child(Node::input("in_color", vertex_vec4f.clone(), 0).into());
	vertex_root.add_child(Node::output("out_color", vertex_vec4f, 0).into());

	let mut fragment_root = Node::root();
	let fragment_vec4f = fragment_root.get_child("vec4f").expect("Expected vec4f");
	fragment_root.add_child(Node::input("in_color", fragment_vec4f.clone(), 0).into());
	fragment_root.add_child(Node::output("out_color", fragment_vec4f, 0).into());

	let vertex_executable = compile_test_program(vertex_script, Some(vertex_root));
	let fragment_executable = compile_test_program(fragment_script, Some(fragment_root));

	let mut vertex_input = interface_buffer(vertex_executable.input_layout(0));
	let mut vertex_output = interface_buffer(vertex_executable.output_layout(0));
	let mut fragment_output = interface_buffer(fragment_executable.output_layout(0));

	vertex_input
		.write("in_color", Value::Vec4F([0.8, 0.4, 0.2, 1.0]))
		.expect("Expected vertex input write");

	{
		let mut descriptors = DescriptorBindings::new();
		descriptors.bind_buffer(input_slot(0), &mut vertex_input);
		descriptors.bind_buffer(output_slot(0), &mut vertex_output);
		vertex_executable
			.run_main(&mut descriptors)
			.expect("Expected vertex execution to succeed");
	}

	{
		let mut descriptors = DescriptorBindings::new();
		descriptors.bind_buffer(input_slot(0), &mut vertex_output);
		descriptors.bind_buffer(output_slot(0), &mut fragment_output);
		fragment_executable
			.run_main(&mut descriptors)
			.expect("Expected fragment execution to succeed");
	}

	assert_eq!(
		fragment_output.read("out_color").expect("Expected fragment output"),
		Value::Vec4F([0.65, 0.2, 0.1, 0.5])
	);
}

#[test]
fn executable_program_evaluates_dot_intrinsics() {
	let script = r#"
	main: fn () -> void {
		buff.value = dot(vec3f(1.0, 2.0, 3.0), vec3f(4.0, 5.0, 6.0));
	}
	"#;

	let root = buffer_root("buff", 17, &[("value", "f32")]);

	let executable = compile_test_program(script, Some(root));

	let buffer = run_slot(&executable, 17);

	assert_eq!(buffer.read_f32("value").expect("Expected f32 member"), 32.0);
}

#[test]
fn executable_program_converts_i32_to_f32_and_u32() {
	let script = r#"
	main: fn () -> void {
		let signed: i32 = 3 - 7;
		buff.float_value = f32(signed);
		buff.unsigned_value = u32(signed);
	}
	"#;

	let root = buffer_root("buff", 35, &[("float_value", "f32"), ("unsigned_value", "u32")]);

	let executable = compile_test_program(script, Some(root));
	let buffer = run_slot(&executable, 35);

	assert_eq!(buffer.read("float_value").expect("Expected converted f32"), Value::F32(-4.0));
	assert_eq!(
		buffer.read("unsigned_value").expect("Expected converted u32"),
		Value::U32(u32::MAX - 3)
	);
}

#[test]
fn executable_program_evaluates_cross_intrinsics() {
	let script = r#"
	main: fn () -> void {
		buff.value = cross(vec3f(1.0, 0.0, 0.0), vec3f(0.0, 1.0, 0.0));
	}
	"#;

	let root = buffer_root("buff", 18, &[("value", "vec3f")]);

	let executable = compile_test_program(script, Some(root));

	let buffer = run_slot(&executable, 18);

	assert_eq!(read_f32s(&buffer, 3), vec![0.0, 0.0, 1.0]);
}

#[test]
fn executable_program_evaluates_vector_mix_integer_ordering_and_scalar_round() {
	let script = r#"
	main: fn () -> void {
		buff.magnitude = length(vec2f(3.0, 4.0));
		buff.blended = mix(vec2f(0.0, 2.0), vec2f(4.0, 6.0), 0.25);
		buff.rounded = round(2.5);
		buff.blended3 = mix(vec3f(0.0, 2.0, 4.0), vec3f(4.0, 6.0, 8.0), 0.5);
		buff.blended4 = mix(vec4f(0.0, 2.0, 4.0, 6.0), vec4f(4.0, 6.0, 8.0, 10.0), 0.5);
		let negative: i32 = 0 - 3;
		let positive: i32 = 5;
		let low: i32 = 0 - 4;
		let high: i32 = 4;
		let three: u32 = 3;
		let five: u32 = 5;
		let zero: u32 = 0;
		let four: u32 = 4;
		buff.smallest = min(negative, positive);
		buff.largest = max(three, five);
		buff.held_i = clamp(low - 3, low, high);
		buff.held_u = clamp(five + 2, zero, four);
	}
	"#;

	let root = buffer_root(
		"buff",
		19,
		&[
			("magnitude", "f32"),
			("blended", "vec2f"),
			("rounded", "f32"),
			("blended3", "vec3f"),
			("blended4", "vec4f"),
			("smallest", "i32"),
			("largest", "u32"),
			("held_i", "i32"),
			("held_u", "u32"),
		],
	);

	let executable = compile_test_program(script, Some(root));

	let buffer = run_slot(&executable, 19);

	assert_eq!(buffer.read_f32("magnitude").expect("Expected f32 member"), 5.0);
	assert_eq!(
		buffer.read("blended").expect("Expected vec2f member"),
		Value::Vec2F([1.0, 3.0])
	);
	assert_eq!(buffer.read_f32("rounded").expect("Expected f32 member"), 3.0);
	assert_eq!(
		buffer.read("blended3").expect("Expected vec3f member"),
		Value::Vec3F([2.0, 4.0, 6.0])
	);
	assert_eq!(
		buffer.read("blended4").expect("Expected vec4f member"),
		Value::Vec4F([2.0, 4.0, 6.0, 8.0])
	);
	assert_eq!(buffer.read("smallest").expect("Expected i32 member"), Value::I32(-3));
	assert_eq!(buffer.read("largest").expect("Expected u32 member"), Value::U32(5));
	assert_eq!(buffer.read("held_i").expect("Expected i32 member"), Value::I32(-4));
	assert_eq!(buffer.read("held_u").expect("Expected u32 member"), Value::U32(4));
}

/// Verifies normalized material vectors and angular terms stay in the half-precision execution path.
#[test]
fn executable_program_evaluates_f16_vector_intrinsics() {
	let script = r#"
	main: fn () -> void {
		buff.direction = normalize(vec3f16(3.0, 4.0, 0.0));
		buff.alignment = dot(buff.direction, vec3f16(0.0, 1.0, 0.0));
		buff.fused = fma(f16(2.0), f16(3.0), f16(1.0));
		buff.fused_vector = fma(vec3f16(2.0, 3.0, 4.0), vec3f16(3.0, 4.0, 5.0), vec3f16(1.0, 2.0, 3.0));
	}
	"#;

	let root = buffer_root(
		"buff",
		21,
		&[
			("direction", "vec3f16"),
			("alignment", "f16"),
			("fused", "f16"),
			("fused_vector", "vec3f16"),
		],
	);

	let executable = compile_test_program(script, Some(root));
	let buffer = run_slot(&executable, 21);

	assert_eq!(
		buffer.read("direction").expect("Expected f16 direction"),
		Value::Vec3F16([f16::from_f32(0.6), f16::from_f32(0.8), f16::from_f32(0.0)])
	);
	assert_eq!(
		buffer.read_f16("alignment").expect("Expected f16 alignment"),
		f16::from_f32(0.8)
	);
	assert_eq!(buffer.read_f16("fused").expect("Expected scalar f16 FMA"), f16::from_f32(7.0));
	assert_eq!(
		buffer.read("fused_vector").expect("Expected vector f16 FMA"),
		Value::Vec3F16([f16::from_f32(7.0), f16::from_f32(14.0), f16::from_f32(23.0)])
	);
}

/// Verifies half FMA rounds the exact product and sum directly to binary16.
#[test]
fn executable_program_avoids_intermediate_f32_rounding_in_f16_fma() {
	let executable = compile_test_program(
		r#"
		FmaCase: struct {
			first: f16,
			second: f16,
			third: f16,
			result: f16,
			first_vector: vec4f16,
			second_vector: vec4f16,
			third_vector: vec4f16,
			result_vector: vec4f16,
		}
		fma_case: descriptor<{ type: FmaCase, binding: 50, access: read_write }>;

		main: fn () -> void {
			fma_case.result = fma(fma_case.first, fma_case.second, fma_case.third);
			fma_case.result_vector = fma(fma_case.first_vector, fma_case.second_vector, fma_case.third_vector);
		}
		"#,
		None,
	);
	let slot = ResourceSlot::new(50);
	let mut buffer = buffer_for_slot(&executable, slot);
	buffer
		.write("first", Value::F16(f16::from_bits(0x852b)))
		.expect("Expected first half input to fit the buffer");
	buffer
		.write("second", Value::F16(f16::from_bits(0x7603)))
		.expect("Expected second half input to fit the buffer");
	buffer
		.write("third", Value::F16(f16::from_bits(0x87f1)))
		.expect("Expected third half input to fit the buffer");
	buffer
		.write("first_vector", Value::Vec4F16([f16::from_bits(0x852b); 4]))
		.expect("Expected first half vector to fit the buffer");
	buffer
		.write("second_vector", Value::Vec4F16([f16::from_bits(0x7603); 4]))
		.expect("Expected second half vector to fit the buffer");
	buffer
		.write("third_vector", Value::Vec4F16([f16::from_bits(0x87f1); 4]))
		.expect("Expected third half vector to fit the buffer");

	run_with_buffer(&executable, slot, &mut buffer);

	assert_eq!(
		buffer.read_f16("result").expect("Expected half FMA result").to_bits(),
		0xbfc5,
		"FMA must round once to binary16 instead of first rounding to binary32"
	);
	assert_eq!(
		buffer.read("result_vector").expect("Expected half FMA vector"),
		Value::Vec4F16([f16::from_bits(0xbfc5); 4]),
		"Every vector FMA component must round once to binary16"
	);
}

#[test]
fn executable_program_evaluates_reflect_intrinsics() {
	let mut root = buffer_root("buff", 21, &[("value", "vec3f")]);
	let void_type = root.get_child("void").expect("Expected void");
	let vec3f_type = root.get_child("vec3f").expect("Expected vec3f");
	let reflect = root.get_child("reflect").expect("Expected reflect intrinsic");
	root.add_child(
		Node::function(
			"main",
			Vec::new(),
			void_type,
			vec![
				Node::expression(Expressions::Operator {
					operator: Operators::Assignment,
					left: Node::expression(Expressions::Accessor {
						left: Node::expression(Expressions::Member {
							name: "buff".to_string(),
							source: root.get_child("buff").expect("Expected buff binding"),
						})
						.into(),
						right: Node::expression(Expressions::Member {
							name: "value".to_string(),
							source: root.get_child("buff").expect("Expected buff binding"),
						})
						.into(),
					})
					.into(),
					right: Node::expression(Expressions::IntrinsicCall {
						intrinsic: reflect,
						arguments: vec![
							Node::expression(Expressions::FunctionCall {
								function: vec3f_type.clone().into(),
								parameters: vec![
									Node::expression(Expressions::Literal {
										value: "1.0".to_string(),
									})
									.into(),
									Node::expression(Expressions::Literal {
										value: "-1.0".to_string(),
									})
									.into(),
									Node::expression(Expressions::Literal {
										value: "0.0".to_string(),
									})
									.into(),
								],
							})
							.into(),
							Node::expression(Expressions::FunctionCall {
								function: vec3f_type.into(),
								parameters: vec![
									Node::expression(Expressions::Literal {
										value: "0.0".to_string(),
									})
									.into(),
									Node::expression(Expressions::Literal {
										value: "1.0".to_string(),
									})
									.into(),
									Node::expression(Expressions::Literal {
										value: "0.0".to_string(),
									})
									.into(),
								],
							})
							.into(),
						],
						elements: vec![],
					})
					.into(),
				})
				.into(),
			],
		)
		.into(),
	);

	let executable = ExecutableProgram::compile(root.into()).expect("Expected runnable program");

	let buffer = run_slot(&executable, 21);

	assert_eq!(read_f32s(&buffer, 3), vec![1.0, 1.0, 0.0]);
}

#[test]
fn executable_program_executes_continue_and_comparisons() {
	let script = r#"
	main: fn () -> void {
		let sum: u32 = 0;
		for (let i: u32 = 0; i <= 4; i = i + 1) {
			if (i >= 2) {
				continue;
			}
			sum = sum + i;
		}
		buff.sum = sum;
	}
	"#;

	assert_eq!(run_sum_program(script), Value::U32(1));
}

/// Verifies each value reaches exactly one branch of an `if`/`else if`/`else` chain.
#[test]
fn executable_program_executes_else_chains() {
	let script = r#"
	main: fn () -> void {
		let sum: u32 = 0;
		for (let i: u32 = 0; i <= 4; i = i + 1) {
			if (i < 1) {
				sum = sum + 1;
			} else if (i < 3) {
				sum = sum + 10;
			} else {
				sum = sum + 100;
			}
		}
		buff.sum = sum;
	}
	"#;

	// i = 0 takes the if branch, i = 1..2 the else-if branch, and i = 3..4 the else branch.
	assert_eq!(run_sum_program(script), Value::U32(221));
}

/// Runs `script` with a read-write `buff` buffer at slot 25 that holds one `u32` member, `sum`, and returns it.
fn run_sum_program(script: &str) -> Value {
	let executable = compile_test_program(script, Some(buffer_root("buff", 25, &[("sum", "u32")])));
	run_slot(&executable, 25).read("sum").expect("Expected sum value")
}

/// Verifies each value runs the first arm that matches it, and `_` catches the rest.
#[test]
fn executable_program_executes_match_arms() {
	let sum = run_sum_program(
		r#"
	main: fn () -> void {
		let sum: u32 = 0;
		for (let i: u32 = 0; i < 6; i = i + 1) {
			match i {
				0 => sum = sum + 1,
				1 | 2 => {
					sum = sum + 10;
				}
				2 => sum = sum + 1000,
				_ => sum = sum + 100,
			}
		}
		buff.sum = sum;
	}
	"#,
	);

	// i = 0 takes the first arm, i = 1..2 the second, and i = 3..5 the wildcard. The `2` arm is unreachable.
	assert_eq!(sum, Value::U32(321));
}

/// Verifies `break` and `continue` inside a match arm act on the enclosing loop, as in Rust.
#[test]
fn executable_program_match_arms_break_and_continue_the_enclosing_loop() {
	let sum = run_sum_program(
		r#"
	main: fn () -> void {
		let sum: u32 = 0;
		for (let i: u32 = 0; i < 10; i = i + 1) {
			match i {
				1 => continue,
				3 => {
					break;
				}
				_ => {}
			}
			sum = sum + 1;
		}
		buff.sum = sum;
	}
	"#,
	);

	// Only i = 0 and i = 2 reach the end of the loop body.
	assert_eq!(sum, Value::U32(2));
}

/// Verifies `bool` and signed matches, including an exhaustive `bool` match without `_`.
#[test]
fn executable_program_matches_bool_and_signed_values() {
	let sum = run_sum_program(
		r#"
	main: fn () -> void {
		let sum: u32 = 0;
		let signed: i32 = 7;
		for (let i: u32 = 0; i < 3; i = i + 1) {
			match i < 1 {
				true => sum = sum + 1,
				false => sum = sum + 10,
			}
		}
		match signed {
			-1 => sum = sum + 1000,
			7 => sum = sum + 100,
			_ => {}
		}
		buff.sum = sum;
	}
	"#,
	);

	assert_eq!(sum, Value::U32(121));
}

/// Verifies `break` leaves only the innermost loop and execution resumes after it.
#[test]
fn executable_program_breaks_out_of_the_innermost_loop() {
	let script = r#"
	main: fn () -> void {
		let sum: u32 = 0;
		for (let i: u32 = 0; i < 3; i = i + 1) {
			for (let j: u32 = 0; j < 10; j = j + 1) {
				if (j == 2) {
					break;
				}
				sum = sum + 1;
			}
			sum = sum + 10;
		}
		buff.sum = sum;
	}
	"#;

	// Each outer iteration counts two inner iterations before the break, then its own ten.
	assert_eq!(run_sum_program(script), Value::U32(36));
}

#[test]
fn executable_program_evaluates_scalar_math_intrinsics() {
	let script = r#"
	main: fn () -> void {
		buff.abs_value = abs(0.0 - 2.5);
		buff.sqrt_value = sqrt(9.0);
		buff.exp_value = exp(1.0);
		buff.sin_value = sin(0.0);
		buff.cos_value = cos(0.0);
		buff.tan_value = tan(0.0);
		buff.asin_value = asin(1.0);
		buff.atan2_value = atan2(1.0, 0.0);
		buff.floor_value = floor(1.75);
		buff.fract_value = fract(1.25);
		buff.radians_value = radians(180.0);
		buff.inverse_sqrt_value = inversesqrt(4.0);
		buff.smoothstep_value = smoothstep(0.0, 1.0, 0.5);
		buff.mix_value = mix(2.0, 4.0, 0.25);
	}
	"#;

	let root = buffer_root(
		"buff",
		26,
		&[
			("abs_value", "f32"),
			("sqrt_value", "f32"),
			("exp_value", "f32"),
			("sin_value", "f32"),
			("cos_value", "f32"),
			("tan_value", "f32"),
			("asin_value", "f32"),
			("atan2_value", "f32"),
			("floor_value", "f32"),
			("fract_value", "f32"),
			("radians_value", "f32"),
			("inverse_sqrt_value", "f32"),
			("smoothstep_value", "f32"),
			("mix_value", "f32"),
		],
	);

	let executable = compile_test_program(script, Some(root));
	let buffer = run_slot(&executable, 26);

	use std::f32::consts::{E, FRAC_PI_2, PI};
	let expected = [
		(2.5, 1e-6),
		(3.0, 1e-6),
		(E, 1e-5),
		(0.0, 1e-6),
		(1.0, 1e-6),
		(0.0, 1e-6),
		(FRAC_PI_2, 1e-6),
		(FRAC_PI_2, 1e-6),
		(1.0, 1e-6),
		(0.25, 1e-6),
		(PI, 1e-6),
		(0.5, 1e-6),
		(0.5, 1e-6),
		(2.5, 1e-6),
	];
	for (index, (value, (expected, tolerance))) in read_f32s(&buffer, 14).into_iter().zip(expected).enumerate() {
		assert!((value - expected).abs() < tolerance, "member {index}: {value} != {expected}");
	}
}

#[test]
fn executable_program_evaluates_paired_trigonometry_fma_and_signed_rounding() {
	let script = r#"
	main: fn () -> void {
		buff.trigonometry = sincos(0.0);
		buff.fused = fma(vec2f(2.0, 3.0), vec2f(4.0, 5.0), vec2f(1.0, 2.0));
		buff.rounded = round_to_i32(vec2f(0.0 - 1.6, 2.4));
	}
	"#;

	let root = buffer_root(
		"buff",
		36,
		&[("trigonometry", "vec2f"), ("fused", "vec2f"), ("rounded", "vec2i")],
	);

	let executable = compile_test_program(script, Some(root));
	let buffer = run_slot(&executable, 36);

	assert_eq!(
		buffer.read("trigonometry").expect("Expected paired trigonometry"),
		Value::Vec2F([0.0, 1.0])
	);
	assert_eq!(
		buffer.read("fused").expect("Expected fused result"),
		Value::Vec2F([9.0, 17.0])
	);
	assert_eq!(
		buffer.read("rounded").expect("Expected rounded result"),
		Value::Vec2I([-2, 2])
	);
}

#[test]
fn executable_program_evaluates_scalar_max_and_clamp() {
	let script = r#"
	main: fn () -> void {
		buff.max_value = max(1.5, 2.5);
		buff.clamp_value = clamp(1.5, 0.0, 1.0);
	}
	"#;

	let root = buffer_root("buff", 27, &[("max_value", "f32"), ("clamp_value", "f32")]);

	let executable = compile_test_program(script, Some(root));
	let buffer = run_slot(&executable, 27);

	let values = read_f32s(&buffer, 2);

	assert!((values[0] - 2.5).abs() < 1e-6);
	assert!((values[1] - 1.0).abs() < 1e-6);
}

#[test]
fn execution_limit_stops_an_infinite_loop() {
	let executable = compile_test_program(
		r#"
		main: fn () -> void {
			for (let i: u32 = 0; i >= 0; i = i + 1) {
				i = i;
			}
		}
		"#,
		None,
	);
	let mut descriptors = DescriptorBindings::new();
	let error = executable
		.run_main_with_config(&mut descriptors, &ExecutionConfig::new(32))
		.expect_err("An infinite loop must exhaust its explicit instruction budget");

	assert_eq!(error, VmError::InstructionLimitExceeded { limit: 32 });
}

#[test]
fn reflect_preserves_the_exact_non_unit_normal_semantics() {
	assert_eq!(
		reflect_vector([1.0, 2.0], [2.0, 0.0]).expect("Reflect is defined for every finite normal"),
		[-7.0, 2.0]
	);
}

#[test]
fn texture_descriptor_handles_flow_through_function_parameters() {
	let script = r#"
	read_source: fn (source: Texture2D) -> vec4f {
		return fetch(source, vec2u(0, 0));
	}
	main: fn () -> void {
		result.color = read_source(source_texture);
	}
	"#;
	let mut root = buffer_root("result", 31, &[("color", "vec4f")]);
	root.add_child(
		Node::binding(
			"source_texture",
			BindingTypes::CombinedImageSampler { format: String::new() },
			30,
			true,
			false,
		)
		.into(),
	);
	let executable = compile_test_program(script, Some(root));
	let mut texture = Texture::new(1, 1).expect("Expected texture");
	texture.write([0, 0], [0.25, 0.5, 0.75, 1.0]).expect("Expected texel write");
	let mut result = buffer_for_slot(&executable, ResourceSlot::new(31));
	{
		let mut descriptors = DescriptorBindings::new();
		descriptors.bind_texture(ResourceSlot::new(30), &mut texture);
		descriptors.bind_buffer(ResourceSlot::new(31), &mut result);
		executable
			.run_main(&mut descriptors)
			.expect("Expected descriptor-handle execution");
	}

	assert_eq!(
		result.read("color").expect("Expected color"),
		Value::Vec4F([0.25, 0.5, 0.75, 1.0])
	);
}

const TEXTURE_DESCRIPTOR_ARRAY_SHADER: &str = r#"
textures: descriptor<{ type: Texture2D, binding: 5, access: read, count: 3 }>;

main: fn (pipeline_input: interface { index: u32, uv: vec2f }) -> output { color: vec4f } {
	return { color: sample(textures[pipeline_input.index], pipeline_input.uv) };
}
"#;

#[test]
fn parsed_texture_descriptor_arrays_select_runtime_resources() {
	let executable = compile_test_program(TEXTURE_DESCRIPTOR_ARRAY_SHADER, None);
	let colors = [[1.0, 0.0, 0.0, 1.0], [0.0, 1.0, 0.0, 0.5], [0.0, 0.0, 1.0, 0.25]];
	let mut textures = colors.map(|color| {
		let mut texture = Texture::new(1, 1).expect("Expected texture allocation");
		texture.write([0, 0], color).expect("Expected texel write");
		texture
	});
	let mut index_input = interface_buffer(executable.input_layout(0));
	let mut uv_input = interface_buffer(executable.input_layout(1));
	let mut output = interface_buffer(executable.output_layout(0));
	uv_input
		.write("_besl_interface_uv", Value::Vec2F([0.5, 0.5]))
		.expect("Expected UV input");
	for index in [2, 0, 1] {
		index_input
			.write("_besl_interface_index", Value::U32(index))
			.expect("Expected texture index");
		{
			let mut descriptors = DescriptorBindings::new();
			// Inactive array elements need no host texture for this invocation.
			descriptors.bind_texture(ResourceSlot::new(5 + index), &mut textures[index as usize]);
			descriptors.bind_buffer(input_slot(0), &mut index_input);
			descriptors.bind_buffer(input_slot(1), &mut uv_input);
			descriptors.bind_buffer(output_slot(0), &mut output);
			executable
				.run_main(&mut descriptors)
				.expect("Expected indexed texture sampling");
		}
		assert_eq!(
			output.read("_besl_output_color").expect("Expected sampled color"),
			Value::Vec4F(colors[index as usize])
		);
	}
}

#[test]
fn texture_descriptor_array_indices_stay_inside_the_declared_range() {
	let executable = compile_test_program(TEXTURE_DESCRIPTOR_ARRAY_SHADER, None);
	let mut index_input = interface_buffer(executable.input_layout(0));
	let mut uv_input = interface_buffer(executable.input_layout(1));
	let mut output = interface_buffer(executable.output_layout(0));
	let mut adjacent_texture = Texture::new(1, 1).expect("Expected texture allocation");
	index_input
		.write("_besl_interface_index", Value::U32(3))
		.expect("Expected texture index");
	uv_input
		.write("_besl_interface_uv", Value::Vec2F([0.5, 0.5]))
		.expect("Expected UV input");
	let mut descriptors = DescriptorBindings::new();
	// A bound resource after the array must remain inaccessible through its index.
	descriptors.bind_texture(ResourceSlot::new(8), &mut adjacent_texture);
	descriptors.bind_buffer(input_slot(0), &mut index_input);
	descriptors.bind_buffer(input_slot(1), &mut uv_input);
	descriptors.bind_buffer(output_slot(0), &mut output);
	assert_eq!(
		executable.run_main(&mut descriptors),
		Err(VmError::DescriptorArrayIndexOutOfBounds {
			slot: ResourceSlot::new(5),
			index: 3,
			count: 3
		})
	);
}

#[test]
fn dynamic_const_array_indices_select_runtime_elements() {
	let script = r#"
	WEIGHTS: const f32[3] = f32[3](0.25, 0.5, 0.75);
	main: fn () -> void {
		let index: u32 = 2;
		result.value = WEIGHTS[index];
	}
	"#;
	let root = buffer_root("result", 32, &[("value", "f32")]);
	let executable = compile_test_program(script, Some(root));
	let result = run_slot(&executable, 32);

	assert_eq!(result.read("value").expect("Expected selected weight"), Value::F32(0.75));
}

#[test]
fn mesh_intrinsics_capture_geometry_and_indexed_outputs() {
	let script = r#"
	main: fn () -> void {
		set_mesh_output_counts(1, 1);
		set_mesh_vertex_position(0, vec4f(1.0, 2.0, 3.0, 1.0));
		set_mesh_triangle(0, vec3u(0, 0, 0));
		set_mesh_primitive_render_target_array_index(0, 3);
		out_index[0] = 17;
	}
	"#;
	let mut root = Node::root();
	let u32_type = root.get_child("u32").expect("Expected u32");
	root.add_child(Node::output_array("out_index", u32_type, 0, std::num::NonZeroUsize::new(1), false).into());
	let executable = compile_test_program(script, Some(root));
	let mut output = interface_buffer(executable.output_layout(0));
	let mut mesh_outputs = MeshOutputs::new();
	{
		let mut descriptors = DescriptorBindings::new();
		descriptors.bind_buffer(output_slot(0), &mut output);
		descriptors.bind_mesh_outputs(&mut mesh_outputs);
		executable
			.run_main(&mut descriptors)
			.expect("Expected mesh capture execution");
	}

	assert_eq!(mesh_outputs.vertex_count(), 1);
	assert_eq!(mesh_outputs.primitive_count(), 1);
	assert_eq!(mesh_outputs.vertex_position(0), Some([1.0, 2.0, 3.0, 1.0]));
	assert_eq!(mesh_outputs.triangle(0), Some([0, 0, 0]));
	assert_eq!(mesh_outputs.render_target_array_index(0), Some(3));
	assert_eq!(
		output.read_indexed("out_index", 0).expect("Expected indexed output"),
		Value::U32(17)
	);
}

#[test]
fn authored_mesh_shader_reads_bound_task_payload_elements() {
	let executable = compile_test_program(
		r#"
		Result: struct {
			meshlet_index: u32,
		}
		result: descriptor<{ type: Result, binding: 40, access: read_write }>;
		visible_meshlets: task_payload<u32, 32>;

		main: fn () -> void {
			result.meshlet_index = visible_meshlets[threadgroup_position()];
		}
		"#,
		None,
	);
	let mut result = buffer_for_slot(&executable, ResourceSlot::new(40));
	let mut descriptors = DescriptorBindings::new();
	descriptors.bind_buffer(ResourceSlot::new(40), &mut result);
	descriptors.bind_task_payload("visible_meshlets", [Value::U32(5)]);
	executable
		.run_main(&mut descriptors)
		.expect("Expected the authored mesh shader to read its bound task payload");

	assert_eq!(
		result.read("meshlet_index").expect("Expected captured meshlet index"),
		Value::U32(5)
	);
}

#[test]
fn task_stage_intrinsics_capture_payload_and_mesh_output_count() {
	let executable = compile_test_program(
		r#"
		visible_meshlets: task_payload<u32, 4>;
		visible_count: workgroup<atomicu32>;

		main: fn () -> void {
			if (thread_idx() == 0) {
				atomic_store(visible_count, 0);
			}
			workgroup_barrier();
			let payload_index: u32 = atomic_add(visible_count, 1);
			visible_meshlets[payload_index] = thread_position();
			workgroup_barrier();
			if (thread_idx() == 0) {
				set_task_mesh_output_count(atomic_load(visible_count));
			}
		}
		"#,
		None,
	);
	let mut outputs = TaskOutputs::new();
	let mut workgroup = WorkgroupState::new();
	let configs = [
		ExecutionConfig::new(128).with_thread_idx(0).with_thread_position(7),
		ExecutionConfig::new(128).with_thread_idx(1).with_thread_position(8),
		ExecutionConfig::new(128).with_thread_idx(2).with_thread_position(9),
	];
	{
		let mut descriptors = DescriptorBindings::new();
		descriptors.bind_task_outputs(&mut outputs);
		descriptors.bind_workgroup_state(&mut workgroup);
		executable
			.run_workgroup(&mut descriptors, &configs)
			.expect("Task workgroup execution failed. The most likely cause is broken barrier or shared-atomic scheduling.");
	}

	assert_eq!(configs[0].thread_position(), 7);
	assert_eq!(outputs.mesh_output_count(), Some(3));
	assert_eq!(outputs.payload_value("visible_meshlets", 0), Some(&Value::U32(7)));
	assert_eq!(outputs.payload_value("visible_meshlets", 1), Some(&Value::U32(8)));
	assert_eq!(outputs.payload_value("visible_meshlets", 2), Some(&Value::U32(9)));
}

#[test]
fn task_workgroup_reuse_clears_stale_payload_values() {
	let executable = compile_test_program(
		r#"
		visible_meshlets: task_payload<u32, 4>;
		visible_count: workgroup<atomicu32>;

		main: fn () -> void {
			if (thread_idx() == 0) {
				atomic_store(visible_count, 0);
			}
			workgroup_barrier();
			if (thread_position() == 7) {
				let payload_index: u32 = atomic_add(visible_count, 1);
				visible_meshlets[payload_index] = thread_position();
			}
			workgroup_barrier();
			if (thread_idx() == 0) {
				set_task_mesh_output_count(atomic_load(visible_count));
			}
		}
		"#,
		None,
	);
	let mut outputs = TaskOutputs::new();
	let mut workgroup = WorkgroupState::new();

	for (position, expected_count) in [(7, 1), (8, 0)] {
		let config = ExecutionConfig::new(128).with_thread_idx(0).with_thread_position(position);
		let mut descriptors = DescriptorBindings::new();
		descriptors.bind_task_outputs(&mut outputs);
		descriptors.bind_workgroup_state(&mut workgroup);
		executable
			.run_workgroup(&mut descriptors, &[config])
			.expect("Task capture reuse failed. The most likely cause is stale workgroup or task-output state.");

		assert_eq!(outputs.mesh_output_count(), Some(expected_count));
	}

	assert_eq!(outputs.payload_value("visible_meshlets", 0), None);
}

#[test]
fn task_mesh_output_counts_respect_execution_limits() {
	let executable = compile_test_program(
		r#"
		main: fn () -> void {
			set_task_mesh_output_count(2);
		}
		"#,
		None,
	);
	let mut outputs = TaskOutputs::new();
	let config = ExecutionConfig::new(32).with_max_task_mesh_output_count(1);
	let error = {
		let mut descriptors = DescriptorBindings::new();
		descriptors.bind_task_outputs(&mut outputs);
		executable
			.run_workgroup(&mut descriptors, &[config])
			.expect_err("Task output limit was ignored. The most likely cause is missing task-count validation.")
	};

	assert_eq!(error, VmError::TaskMeshOutputCountLimitExceeded { requested: 2, limit: 1 });
}

#[test]
fn task_workgroup_rejects_barrier_divergence() {
	let executable = compile_test_program(
		r#"
		main: fn () -> void {
			if (thread_idx() == 1) {
				workgroup_barrier();
			}
		}
		"#,
		None,
	);
	let configs = [
		ExecutionConfig::new(32).with_thread_idx(0),
		ExecutionConfig::new(32).with_thread_idx(1),
	];
	let mut descriptors = DescriptorBindings::new();
	let error = executable
		.run_workgroup(&mut descriptors, &configs)
		.expect_err("Barrier divergence was accepted. The most likely cause is a broken task rendezvous check.");

	assert!(matches!(
		error,
		VmError::DivergentWorkgroupBarrier {
			lane: 0,
			found_instruction: None,
			..
		}
	));
}

#[test]
fn task_workgroup_rejects_different_static_barriers_in_one_phase() {
	let executable = compile_test_program(
		r#"
		main: fn () -> void {
			if (thread_idx() == 0) {
				workgroup_barrier();
			}
			if (thread_idx() == 1) {
				workgroup_barrier();
			}
		}
		"#,
		None,
	);
	let configs = [
		ExecutionConfig::new(32).with_thread_idx(0),
		ExecutionConfig::new(32).with_thread_idx(1),
	];
	let mut descriptors = DescriptorBindings::new();
	let error = executable
		.run_workgroup(&mut descriptors, &configs)
		.expect_err("Static barrier divergence was accepted. The most likely cause is a broken rendezvous phase check.");

	assert!(matches!(
		error,
		VmError::DivergentWorkgroupBarrier {
			lane: 1,
			found_instruction: Some(_),
			..
		}
	));
}

#[test]
fn bitwise_xor_flips_shared_bits() {
	let executable = compile_test_program(
		r#"
		Result: struct {
			values: u32[2],
		}
		result: descriptor<{ type: Result, binding: 43, access: read_write }>;

		main: fn () -> void {
			let mask: u32 = 12;
			result.values[0] = mask ^ 10;
			result.values[1] = 1 | mask ^ 6 & 3;
		}
		"#,
		None,
	);
	let result = run_slot(&executable, 43);

	assert_eq!(read_u32s(&result, 2), [6, 1 | (12 ^ (6 & 3))]);
}

#[test]
fn find_lsb_returns_the_lowest_set_bit_or_all_ones_for_zero() {
	let executable = compile_test_program(
		r#"
		Result: struct {
			values: u32[4],
			logarithm: f32,
		}
		result: descriptor<{ type: Result, binding: 43, access: read_write }>;

		main: fn () -> void {
			let empty: u32 = 0;
			let top: u32 = 1;
			top = top << 31;
			result.values[0] = find_lsb(empty);
			result.values[1] = find_lsb(top);
			result.values[2] = find_lsb(40);
			result.values[3] = find_lsb(1);
			result.logarithm = log2(8.0);
		}
		"#,
		None,
	);
	let result = run_slot(&executable, 43);

	assert_eq!(read_u32s(&result, 4), [u32::MAX, 31, 3, 0]);
	assert_eq!(result.read_f32("logarithm").expect("scalar log2 result"), 3.0);
}

#[test]
fn compute_subgroup_collectives_partition_two_subgroups_and_preserve_masks() {
	let executable = compile_test_program(
		r#"
		Result: struct {
			values: u32[64],
			floats: f32[64],
			lane_indices: u32[64],
		}
		result: descriptor<{ type: Result, binding: 43, access: read_write }>;

		main: fn () -> void {
			let lane: u32 = thread_idx();
			let active: vec4u = subgroup_ballot((lane & 3) != 0);
			let leader: u32 = subgroup_ballot_find_lsb(active);
			let leader_lane: u32 = subgroup_broadcast_u32(lane, leader);
			let leader_float: f32 = subgroup_broadcast_f32(f32(lane) * 0.5, leader);
			let removed: vec4u = subgroup_ballot((lane & 7) == 1);
			let remaining: vec4u = subgroup_ballot_and_not(active, removed);

			if (subgroup_ballot_any(remaining)) {
				result.values[lane] = leader_lane + subgroup_ballot_count(remaining);
				result.floats[lane] = leader_float;
				result.lane_indices[lane] = subgroup_lane_index();
			}
		}
		"#,
		None,
	);
	let mut result = buffer_for_slot(&executable, ResourceSlot::new(43));
	let configs = (0..64)
		.map(|lane| ExecutionConfig::new(512).with_thread_idx(lane).with_subgroup_size(32))
		.collect::<Vec<_>>();
	{
		let mut descriptors = DescriptorBindings::new();
		descriptors.bind_buffer(ResourceSlot::new(43), &mut result);
		executable
			.run_workgroup(&mut descriptors, &configs)
			.expect("Subgroup mask execution failed. The most likely cause is broken per-subgroup collective scheduling.");
	}

	for lane in 0..32 {
		assert_eq!(
			result.read_indexed("values", lane).expect("Expected first subgroup result"),
			Value::U32(21)
		);
		assert_eq!(result.read_indexed("floats", lane), Ok(Value::F32(0.5)));
		assert_eq!(result.read_indexed("lane_indices", lane), Ok(Value::U32(lane as u32)));
	}
	for lane in 32..64 {
		assert_eq!(
			result.read_indexed("values", lane).expect("Expected second subgroup result"),
			Value::U32(53)
		);
		assert_eq!(result.read_indexed("floats", lane), Ok(Value::F32(16.5)));
		assert_eq!(result.read_indexed("lane_indices", lane), Ok(Value::U32((lane - 32) as u32)));
	}
}

#[test]
fn compute_subgroup_collectives_reject_divergent_lanes() {
	let executable = compile_test_program(
		r#"
		main: fn () -> void {
			if (thread_idx() == 0) {
				let mask: vec4u = subgroup_ballot(true);
				mask;
			}
		}
		"#,
		None,
	);
	let configs = [
		ExecutionConfig::new(32).with_thread_idx(0).with_subgroup_size(32),
		ExecutionConfig::new(32).with_thread_idx(1).with_subgroup_size(32),
	];
	let mut descriptors = DescriptorBindings::new();
	let error = executable.run_workgroup(&mut descriptors, &configs).expect_err(
		"Divergent subgroup collective was accepted. The most likely cause is missing subgroup rendezvous validation.",
	);

	assert!(matches!(
		error,
		VmError::DivergentSubgroupCollective {
			lane: 1,
			found_instruction: None,
			..
		}
	));
}

/// Verifies each lane reads the lane whose index differs by the mask, inside its own subgroup only.
#[test]
fn compute_subgroup_shuffle_xor_reads_the_masked_lane_of_each_subgroup() {
	let executable = compile_test_program(
		r#"
		Result: struct {
			near: f32[64],
			far: f32[64],
		}
		result: descriptor<{ type: Result, binding: 43, access: read_write }>;

		main: fn () -> void {
			let lane: u32 = thread_idx();
			result.near[lane] = subgroup_shuffle_xor_f32(f32(lane), 1);
			result.far[lane] = subgroup_shuffle_xor_f32(f32(lane) * 0.5, 18);
		}
		"#,
		None,
	);
	let mut result = buffer_for_slot(&executable, ResourceSlot::new(43));
	let configs = (0..64)
		.map(|lane| ExecutionConfig::new(512).with_thread_idx(lane).with_subgroup_size(32))
		.collect::<Vec<_>>();
	{
		let mut descriptors = DescriptorBindings::new();
		descriptors.bind_buffer(ResourceSlot::new(43), &mut result);
		executable
			.run_workgroup(&mut descriptors, &configs)
			.expect("Subgroup shuffle execution failed. The most likely cause is broken per-subgroup collective scheduling.");
	}

	// Lane indices restart in the second subgroup, so a mask never crosses into the other subgroup.
	for lane in 0..64usize {
		assert_eq!(result.read_indexed("near", lane), Ok(Value::F32((lane ^ 1) as f32)));
		assert_eq!(result.read_indexed("far", lane), Ok(Value::F32((lane ^ 18) as f32 * 0.5)));
	}
}

/// Verifies a shuffle rejects masks that differ between lanes, which Metal leaves undefined, and masks that reach a
/// lane the subgroup does not run.
#[test]
fn compute_subgroup_shuffle_xor_rejects_divergent_masks_and_inactive_sources() {
	let run = |mask: &str| {
		let executable = compile_test_program(
			&format!("main: fn () -> void {{ let shuffled: f32 = subgroup_shuffle_xor_f32(1.0, {mask}); shuffled; }}"),
			None,
		);
		let configs = (0..4)
			.map(|lane| ExecutionConfig::new(64).with_thread_idx(lane).with_subgroup_size(32))
			.collect::<Vec<_>>();
		let mut descriptors = DescriptorBindings::new();
		executable.run_workgroup(&mut descriptors, &configs)
	};

	// Lanes 0 and 1 pass mask 1, lanes 2 and 3 pass mask 2.
	assert_eq!(
		run("1 + thread_idx() / 2"),
		Err(VmError::DivergentSubgroupShuffleMask {
			lane: 2,
			expected: 1,
			found: 2,
		})
	);
	assert_eq!(
		run("4"),
		Err(VmError::SubgroupShuffleSourceInactive { lane: 0, source_lane: 4 })
	);
}

#[test]
fn task_workgroup_reuse_clears_shared_storage() {
	let executable = compile_test_program(
		r#"
		shared_count: workgroup<atomicu32>;

		main: fn () -> void {
			if (thread_position() == 0) {
				atomic_store(shared_count, 1);
			}
			set_task_mesh_output_count(atomic_load(shared_count));
		}
		"#,
		None,
	);
	let mut outputs = TaskOutputs::new();
	let mut workgroup = WorkgroupState::new();
	{
		let mut descriptors = DescriptorBindings::new();
		descriptors.bind_task_outputs(&mut outputs);
		descriptors.bind_workgroup_state(&mut workgroup);
		executable
			.run_workgroup(&mut descriptors, &[ExecutionConfig::new(32).with_thread_position(0)])
			.expect("Initial workgroup execution failed. The most likely cause is broken workgroup storage initialization.");
	}

	assert_eq!(outputs.mesh_output_count(), Some(1));

	let error = {
		let mut descriptors = DescriptorBindings::new();
		descriptors.bind_task_outputs(&mut outputs);
		descriptors.bind_workgroup_state(&mut workgroup);
		executable
			.run_workgroup(&mut descriptors, &[ExecutionConfig::new(32).with_thread_position(1)])
			.expect_err(
				"Stale workgroup storage was visible. The most likely cause is that scheduler reuse did not clear shared values.",
			)
	};

	assert_eq!(
		error,
		VmError::UninitializedWorkgroupValue {
			name: "shared_count".to_string()
		}
	);
}

#[test]
fn compute_workgroup_array_shares_values_across_a_barrier() {
	let executable = compile_test_program(
		r#"
		Result: struct {
			values: u32[2],
		}
		result: descriptor<{ type: Result, binding: 41, access: read_write }>;
		scratch: workgroup<u32, 2>;

		main: fn () -> void {
			let lane: u32 = thread_idx();
			scratch[lane] = lane + 10;
			workgroup_barrier();
			result.values[lane] = scratch[1 - lane];
		}
		"#,
		None,
	);
	let mut result = buffer_for_slot(&executable, ResourceSlot::new(41));
	let mut workgroup = WorkgroupState::new();
	let configs = [
		ExecutionConfig::new(32).with_thread_idx(0),
		ExecutionConfig::new(32).with_thread_idx(1),
	];
	{
		let mut descriptors = DescriptorBindings::new();
		descriptors.bind_buffer(ResourceSlot::new(41), &mut result);
		descriptors.bind_workgroup_state(&mut workgroup);
		executable.run_workgroup(&mut descriptors, &configs).expect(
			"Compute workgroup exchange failed. The most likely cause is broken indexed shared storage or barrier scheduling.",
		);
	}

	assert_eq!(
		result.read_array_element(0).expect("Expected lane zero result"),
		Value::U32(11)
	);
	assert_eq!(
		result.read_array_element(1).expect("Expected lane one result"),
		Value::U32(10)
	);
}

#[test]
fn atomic_compare_exchange_returns_previous_value_on_success_and_failure() {
	let executable = compile_test_program(
		r#"
		Result: struct {
			previous: u32[2],
			observed: u32[2],
		}
		result: descriptor<{ type: Result, binding: 42, access: read_write }>;
		shared_value: workgroup<atomicu32>;

		main: fn () -> void {
			let lane: u32 = thread_idx();
			if (lane == 0) {
				atomic_store(shared_value, 5);
			}
			workgroup_barrier();
			result.previous[lane] = atomic_compare_exchange(shared_value, 5, 9);
			result.observed[lane] = atomic_load(shared_value);
		}
		"#,
		None,
	);
	let mut result = buffer_for_slot(&executable, ResourceSlot::new(42));
	let mut workgroup = WorkgroupState::new();
	let configs = [
		ExecutionConfig::new(64).with_thread_idx(0),
		ExecutionConfig::new(64).with_thread_idx(1),
	];
	{
		let mut descriptors = DescriptorBindings::new();
		descriptors.bind_buffer(ResourceSlot::new(42), &mut result);
		descriptors.bind_workgroup_state(&mut workgroup);
		executable.run_workgroup(&mut descriptors, &configs).expect(
			"Compare exchange failed. The most likely cause is broken shared-atomic scheduling or previous-value handling.",
		);
	}

	assert_eq!(
		result
			.read_indexed("previous", 0)
			.expect("Expected successful previous value"),
		Value::U32(5)
	);
	assert_eq!(
		result.read_indexed("previous", 1).expect("Expected failed previous value"),
		Value::U32(9)
	);
	assert_eq!(
		result.read_indexed("observed", 0).expect("Expected lane zero observation"),
		Value::U32(9)
	);
	assert_eq!(
		result.read_indexed("observed", 1).expect("Expected lane one observation"),
		Value::U32(9)
	);
}

#[test]
fn relaxed_integer_atomics_return_previous_values_for_signed_and_unsigned_storage() {
	let executable = compile_test_program(
		r#"
		Result: struct {
			unsigned_previous: u32[10],
			signed_previous: i32[10],
			unsigned_final: u32,
			signed_final: i32,
		}
		result: descriptor<{ type: Result, binding: 43, access: read_write }>;
		shared_unsigned: workgroup<atomicu32>;
		shared_signed: workgroup<atomici32>;

		main: fn () -> void {
			let negative_ten: i32 = 0 - 10;
			let negative_five: i32 = 0 - 5;
			let negative_two: i32 = 0 - 2;
			let negative_nine: i32 = 0 - 9;
			let signed_three: i32 = 3;
			let signed_four: i32 = 4;
			let signed_seven: i32 = 7;
			let signed_eight: i32 = 8;
			let signed_fifteen: i32 = 15;
			let signed_twenty: i32 = 20;
			atomic_store(shared_unsigned, 10);
			result.unsigned_previous[0] = atomic_load(shared_unsigned);
			result.unsigned_previous[1] = atomic_exchange(shared_unsigned, 20);
			result.unsigned_previous[2] = atomic_add(shared_unsigned, 5);
			result.unsigned_previous[3] = atomic_sub(shared_unsigned, 3);
			result.unsigned_previous[4] = atomic_min(shared_unsigned, 30);
			result.unsigned_previous[5] = atomic_max(shared_unsigned, 40);
			result.unsigned_previous[6] = atomic_and(shared_unsigned, 15);
			result.unsigned_previous[7] = atomic_or(shared_unsigned, 3);
			result.unsigned_previous[8] = atomic_xor(shared_unsigned, 1);
			result.unsigned_previous[9] = atomic_compare_exchange(shared_unsigned, 10, 77);
			result.unsigned_final = atomic_load(shared_unsigned);

			atomic_store(shared_signed, negative_ten);
			result.signed_previous[0] = atomic_load(shared_signed);
			result.signed_previous[1] = atomic_exchange(shared_signed, signed_twenty);
			result.signed_previous[2] = atomic_add(shared_signed, negative_five);
			result.signed_previous[3] = atomic_sub(shared_signed, signed_three);
			result.signed_previous[4] = atomic_min(shared_signed, negative_two);
			result.signed_previous[5] = atomic_max(shared_signed, signed_four);
			result.signed_previous[6] = atomic_and(shared_signed, signed_seven);
			result.signed_previous[7] = atomic_or(shared_signed, signed_eight);
			result.signed_previous[8] = atomic_xor(shared_signed, signed_three);
			result.signed_previous[9] = atomic_compare_exchange(shared_signed, signed_fifteen, negative_nine);
			result.signed_final = atomic_load(shared_signed);
		}
		"#,
		None,
	);
	let slot = ResourceSlot::new(43);
	let mut result = buffer_for_slot(&executable, slot);
	let mut workgroup = WorkgroupState::new();
	{
		let mut descriptors = DescriptorBindings::new();
		descriptors.bind_buffer(slot, &mut result);
		descriptors.bind_workgroup_state(&mut workgroup);
		executable
			.run_workgroup(&mut descriptors, &[ExecutionConfig::new(512)])
			.expect("Relaxed integer atomic execution should succeed");
	}

	let expected_unsigned = [10, 10, 20, 25, 22, 22, 40, 8, 11, 10];
	for (index, expected) in expected_unsigned.into_iter().enumerate() {
		assert_eq!(
			result
				.read_indexed("unsigned_previous", index)
				.expect("Expected unsigned previous value"),
			Value::U32(expected)
		);
	}
	let expected_signed = [-10, -10, 20, 15, 12, -2, 4, 4, 12, 15];
	for (index, expected) in expected_signed.into_iter().enumerate() {
		assert_eq!(
			result
				.read_indexed("signed_previous", index)
				.expect("Expected signed previous value"),
			Value::I32(expected)
		);
	}
	assert_eq!(result.read("unsigned_final").unwrap(), Value::U32(77));
	assert_eq!(result.read("signed_final").unwrap(), Value::I32(-9));
}

#[test]
fn relaxed_integer_atomics_update_signed_and_unsigned_buffer_members() {
	let executable = compile_test_program(
		r#"
		State: struct {
			unsigned_value: atomicu32,
			signed_value: atomici32,
			unsigned_previous: u32[9],
			signed_previous: i32[9],
		}
		state: descriptor<{ type: State, binding: 45, access: read_write }>;
		main: fn () -> void {
			let negative_ten: i32 = 0 - 10;
			let negative_five: i32 = 0 - 5;
			let negative_two: i32 = 0 - 2;
			let signed_three: i32 = 3;
			let signed_four: i32 = 4;
			let signed_seven: i32 = 7;
			let signed_eight: i32 = 8;
			let signed_fifteen: i32 = 15;
			let signed_twenty: i32 = 20;

			atomic_store(state.unsigned_value, 10);
			state.unsigned_previous[0] = atomic_exchange(state.unsigned_value, 20);
			state.unsigned_previous[1] = atomic_add(state.unsigned_value, 5);
			state.unsigned_previous[2] = atomic_sub(state.unsigned_value, 3);
			state.unsigned_previous[3] = atomic_min(state.unsigned_value, 30);
			state.unsigned_previous[4] = atomic_max(state.unsigned_value, 40);
			state.unsigned_previous[5] = atomic_and(state.unsigned_value, 15);
			state.unsigned_previous[6] = atomic_or(state.unsigned_value, 3);
			state.unsigned_previous[7] = atomic_xor(state.unsigned_value, 1);
			state.unsigned_previous[8] = atomic_compare_exchange(state.unsigned_value, 10, 77);

			atomic_store(state.signed_value, negative_ten);
			state.signed_previous[0] = atomic_exchange(state.signed_value, signed_twenty);
			state.signed_previous[1] = atomic_add(state.signed_value, negative_five);
			state.signed_previous[2] = atomic_sub(state.signed_value, signed_three);
			state.signed_previous[3] = atomic_min(state.signed_value, negative_two);
			state.signed_previous[4] = atomic_max(state.signed_value, signed_four);
			state.signed_previous[5] = atomic_and(state.signed_value, signed_seven);
			state.signed_previous[6] = atomic_or(state.signed_value, signed_eight);
			state.signed_previous[7] = atomic_xor(state.signed_value, signed_three);
			state.signed_previous[8] = atomic_compare_exchange(state.signed_value, signed_fifteen, negative_ten);
		}
		"#,
		None,
	);
	let state = run_slot(&executable, 45);

	for (index, expected) in [10, 20, 25, 22, 22, 40, 8, 11, 10].into_iter().enumerate() {
		assert_eq!(
			state
				.read_indexed("unsigned_previous", index)
				.expect("Expected unsigned buffer atomic result"),
			Value::U32(expected)
		);
	}
	for (index, expected) in [-10, 20, 15, 12, -2, 4, 4, 12, 15].into_iter().enumerate() {
		assert_eq!(
			state
				.read_indexed("signed_previous", index)
				.expect("Expected signed buffer atomic result"),
			Value::I32(expected)
		);
	}
	assert_eq!(state.read("unsigned_value").unwrap(), Value::U32(77));
	assert_eq!(state.read("signed_value").unwrap(), Value::I32(-10));
}

#[test]
fn executable_program_classifies_f16_and_f32_values() {
	let executable = compile_test_program(
		r#"
		Result: struct {
			nan_f16: bool,
			infinite_f16: bool,
			finite_f16: bool,
			normal_f16: bool,
			nan_f32: bool,
			infinite_f32: bool,
			finite_f32: bool,
			normal_f32: bool,
		}
		result: descriptor<{ type: Result, binding: 44, access: read_write }>;
		main: fn () -> void {
			let zero_f16: f16 = f16(0.0);
			let one_f16: f16 = f16(1.0);
			result.nan_f16 = is_nan(zero_f16 / zero_f16);
			result.infinite_f16 = is_infinite(one_f16 / zero_f16);
			result.finite_f16 = is_finite(one_f16);
			result.normal_f16 = is_normal(one_f16);
			result.nan_f32 = is_nan(0.0 / 0.0);
			result.infinite_f32 = is_infinite(1.0 / 0.0);
			result.finite_f32 = is_finite(1.0);
			result.normal_f32 = is_normal(1.0);
		}
		"#,
		None,
	);
	let result = run_slot(&executable, 44);

	for member in [
		"nan_f16",
		"infinite_f16",
		"finite_f16",
		"normal_f16",
		"nan_f32",
		"infinite_f32",
		"finite_f32",
		"normal_f32",
	] {
		assert_eq!(
			result.read(member).expect("Expected classification result"),
			Value::Bool(true)
		);
	}
}

#[test]
fn compute_workgroup_array_rejects_out_of_bounds_indices() {
	let executable = compile_test_program(
		r#"
		scratch: workgroup<u32, 2>;
		main: fn () -> void {
			scratch[2] = 7;
		}
		"#,
		None,
	);
	let mut workgroup = WorkgroupState::new();
	let mut descriptors = DescriptorBindings::new();
	descriptors.bind_workgroup_state(&mut workgroup);
	let error = executable
		.run_workgroup(&mut descriptors, &[ExecutionConfig::new(32)])
		.expect_err("Out-of-bounds workgroup storage access should fail");

	assert_eq!(
		error,
		VmError::WorkgroupIndexOutOfBounds {
			name: "scratch".to_string(),
			index: 2,
			count: 2,
		}
	);
}

#[test]
fn single_invocation_execution_preserves_barriers_in_called_functions() {
	let executable = compile_test_program(
		r#"
		wait_for_peers: fn () -> void {
			workgroup_barrier();
		}

		main: fn () -> void {
			wait_for_peers();
		}
		"#,
		None,
	);
	let mut descriptors = DescriptorBindings::new();
	executable.run_main(&mut descriptors).expect(
		"Single-invocation helper barrier failed. The most likely cause is that ordinary VM execution attempted task rendezvous.",
	);
}

#[test]
fn task_payload_writes_respect_the_declared_count() {
	let executable = compile_test_program(
		r#"
		visible_meshlets: task_payload<u32, 4>;

		main: fn () -> void {
			visible_meshlets[4] = 9;
		}
		"#,
		None,
	);
	let mut outputs = TaskOutputs::new();
	let error = {
		let mut descriptors = DescriptorBindings::new();
		descriptors.bind_task_outputs(&mut outputs);
		executable.run_main(&mut descriptors).expect_err(
			"Out-of-bounds task payload write was accepted. The most likely cause is missing payload declaration bounds checking.",
		)
	};

	assert_eq!(
		error,
		VmError::TaskPayloadOutputIndexOutOfBounds {
			name: "visible_meshlets".to_string(),
			index: 4,
			count: 4,
		}
	);
}

#[test]
fn mesh_output_counts_clear_reused_capture_ranges() {
	let mut outputs = MeshOutputs::new();
	outputs.set_counts(2, 2, 2, 2, false).expect("Expected bounded mesh outputs");
	outputs.vertex_positions[0] = [1.0, 2.0, 3.0, 1.0];
	outputs.triangles[0] = [4, 5, 6];
	outputs.render_target_array_indices[0] = 3;

	outputs.set_counts(2, 2, 2, 2, true).expect("Expected capture reuse");

	assert_eq!(outputs.vertex_positions, vec![[0.0; 4]; 2]);
	assert_eq!(outputs.triangles, vec![[0; 3]; 2]);
	assert_eq!(outputs.render_target_array_indices, vec![0; 2]);
}

#[test]
fn mesh_output_counts_respect_execution_limits_before_resizing() {
	let executable = compile_test_program(
		r#"
		main: fn () -> void {
			set_mesh_output_counts(2, 3);
		}
		"#,
		None,
	);
	let mut outputs = MeshOutputs::new();
	outputs.set_counts(1, 1, 1, 1, false).expect("Expected initial capture");
	outputs.vertex_positions[0] = [1.0, 2.0, 3.0, 1.0];
	outputs.triangles[0] = [7, 8, 9];
	let config = ExecutionConfig::new(32)
		.with_max_mesh_vertex_count(1)
		.with_max_mesh_primitive_count(3);

	assert_eq!(config.max_mesh_vertex_count(), 1);
	assert_eq!(config.max_mesh_primitive_count(), 3);

	let error = {
		let mut descriptors = DescriptorBindings::new();
		descriptors.bind_mesh_outputs(&mut outputs);
		executable
			.run_main_with_config(&mut descriptors, &config)
			.expect_err("Shader-controlled mesh counts must be bounded")
	};

	assert_eq!(
		error,
		VmError::MeshOutputCountLimitExceeded {
			kind: "vertex",
			requested: 2,
			limit: 1,
		}
	);
	assert_eq!(outputs.vertex_count(), 1);
	assert_eq!(outputs.primitive_count(), 1);
	assert_eq!(outputs.vertex_position(0), Some([1.0, 2.0, 3.0, 1.0]));
	assert_eq!(outputs.triangle(0), Some([7, 8, 9]));

	let primitive_config = ExecutionConfig::new(32)
		.with_max_mesh_vertex_count(2)
		.with_max_mesh_primitive_count(2);
	let primitive_error = {
		let mut descriptors = DescriptorBindings::new();
		descriptors.bind_mesh_outputs(&mut outputs);
		executable
			.run_main_with_config(&mut descriptors, &primitive_config)
			.expect_err("Primitive counts must use their independent limit")
	};

	assert_eq!(
		primitive_error,
		VmError::MeshOutputCountLimitExceeded {
			kind: "primitive",
			requested: 3,
			limit: 2,
		}
	);
}

#[test]
fn descriptor_binding_errors_report_resource_kinds_consistently() {
	let slot = ResourceSlot::new(2);
	let mut texture = Texture::new(1, 1).expect("Expected texture");
	let mut descriptors = DescriptorBindings::new();
	descriptors.bind_texture(slot, &mut texture);

	assert_eq!(
		descriptors.buffer_mut(slot).expect_err("A texture is not a buffer"),
		VmError::DescriptorTypeMismatch {
			slot,
			expected: "buffer",
			found: "texture",
		}
	);
	assert!(
		VmError::UnboundDescriptor { slot }
			.to_string()
			.contains("no resource was bound")
	);
}

#[test]
fn rebinding_a_descriptor_slot_replaces_its_previous_resource() {
	let slot = ResourceSlot::new(2);
	let mut previous = Texture::new(1, 1).expect("Expected previous texture");
	let mut replacement = Texture::new(1, 1).expect("Expected replacement texture");
	{
		let mut descriptors = DescriptorBindings::new();
		descriptors.bind_image(slot, &mut previous);
		descriptors.bind_image(slot, &mut replacement);
		descriptors
			.image_mut(slot)
			.expect("Expected replacement image binding")
			.write([0, 0], [1.0, 2.0, 3.0, 4.0])
			.expect("Expected replacement image write");
	}

	assert_eq!(
		previous.fetch([0, 0]).expect("Expected previous image texel"),
		Value::Vec4F([0.0; 4])
	);
	assert_eq!(
		replacement.fetch([0, 0]).expect("Expected replacement image texel"),
		Value::Vec4F([1.0, 2.0, 3.0, 4.0])
	);
}

#[test]
fn specialization_values_select_x_and_y_components() {
	for (axis, expected) in [([1.0, 0.0], 1.0), ([0.0, 1.0], 2.0)] {
		let script = r#"
		main: fn () -> void {
			result.value = axis.x + axis.y * 2.0;
		}
		"#;
		let mut root = buffer_root("result", 33, &[("value", "f32")]);
		let vec2f = root.get_child("vec2f").expect("Expected vec2f");
		root.add_child(Node::specialization("axis", vec2f, 0).into());
		let program = compile_to_besl(script, Some(root)).expect("Expected lexed specialization program");
		let mut specializations = SpecializationValues::new();
		specializations.set("axis", Value::Vec2F(axis));
		let executable = ExecutableProgram::compile_with_specializations(program, &specializations)
			.expect("Expected specialized executable");
		let result = run_slot(&executable, 33);

		assert_eq!(
			result.read("value").expect("Expected specialization result"),
			Value::F32(expected)
		);
	}
}

#[test]
fn prefix_operators_negate_flip_bits_and_invert_booleans() {
	let script = r#"
	main: fn () -> void {
		let one: u32 = 1;
		let x: f32 = 2.5;
		// Unsigned negation wraps, as in C-family shading languages.
		result.wrapped = -one;
		result.flipped = ~one ^ 3;
		// Negation binds tighter than the product, and a negative literal works as an argument.
		result.negated = -x * 2.0;
		result.vector = -vec3f(1.0, -2.0, 3.0);
		result.inverted = if (!(one == 1)) { 10 } else { 20 };
	}
	"#;
	let root = buffer_root(
		"result",
		34,
		&[
			("wrapped", "u32"),
			("flipped", "u32"),
			("negated", "f32"),
			("vector", "vec3f"),
			("inverted", "u32"),
		],
	);
	let result = run_slot(&compile_test_program(script, Some(root)), 34);

	assert_eq!(result.read("wrapped").expect("wrapped"), Value::U32(u32::MAX));
	assert_eq!(result.read("flipped").expect("flipped"), Value::U32(!1 ^ 3));
	assert_eq!(result.read("negated").expect("negated"), Value::F32(-5.0));
	assert_eq!(result.read("vector").expect("vector"), Value::Vec3F([-1.0, 2.0, -3.0]));
	assert_eq!(result.read("inverted").expect("inverted"), Value::U32(20));
}

#[test]
fn if_values_run_only_the_selected_branch() {
	let script = r#"
	main: fn () -> void {
		let zero: u32 = 0;
		let four: u32 = 4;
		// The run fails on division by zero, so a guard must keep the unselected branch from running.
		result.guarded = if (zero != 0) { four / zero } else { 7 };
		result.divisor = four / (if (zero == 0) { four } else { zero });
		result.nested = if (four < 2) { 10 } else if (four < 5) { 20 } else { 30 };
		result.selected = if (four > 3 || zero > 1) { 1.5 } else { 2.5 };
	}
	"#;
	let root = buffer_root(
		"result",
		35,
		&[("guarded", "u32"), ("divisor", "u32"), ("nested", "u32"), ("selected", "f32")],
	);
	let result = run_slot(&compile_test_program(script, Some(root)), 35);

	assert_eq!(result.read("guarded").expect("guarded"), Value::U32(7));
	assert_eq!(result.read("divisor").expect("divisor"), Value::U32(1));
	assert_eq!(result.read("nested").expect("nested"), Value::U32(20));
	assert_eq!(result.read("selected").expect("selected"), Value::F32(1.5));
}

/// Verifies that `&&` and `||` run their right side only when the left side doesn't decide the result, as on the GPU.
#[test]
fn logical_operators_skip_the_right_side_when_the_left_decides() {
	let script = r#"
	main: fn () -> void {
		let zero: u32 = 0;
		let four: u32 = 4;
		// The run fails on division by zero, so the right side must not run when the left one decides.
		result.and = if (zero != 0 && four / zero > 1) { 1 } else { 2 };
		result.or = if (zero == 0 || four / zero > 1) { 3 } else { 4 };
		result.both = if (zero == 0 && four / 2 > 1) { 5 } else { 6 };
		result.neither = if (zero != 0 || four / 2 > 3) { 7 } else { 8 };
	}
	"#;
	let root = buffer_root(
		"result",
		38,
		&[("and", "u32"), ("or", "u32"), ("both", "u32"), ("neither", "u32")],
	);
	let result = run_slot(&compile_test_program(script, Some(root)), 38);

	assert_eq!(result.read("and").expect("and"), Value::U32(2));
	assert_eq!(result.read("or").expect("or"), Value::U32(3));
	assert_eq!(result.read("both").expect("both"), Value::U32(5));
	assert_eq!(result.read("neither").expect("neither"), Value::U32(8));
}

/// Verifies that branches with statements yield their value wherever an expression can stand, and that only the
/// taken branch runs its statements.
#[test]
fn if_and_match_values_run_their_statements_only_in_the_taken_branch() {
	let script = r#"
	pick: fn (selector: u32, value: f32) -> f32 {
		return match selector {
			0 => value,
			1 | 2 => {
				let doubled: f32 = value * 2.0;
				doubled + 1.0
			}
			_ => return 0.5,
		};
	}

	main: fn () -> void {
		let zero: u32 = 0;
		let four: u32 = 4;
		let count: u32 = 0;
		// The run fails on division by zero, so the branch not taken must not run.
		let quotient: u32 = if (zero == 0) {
			count = count + 1;
			four / 2
		} else {
			count = count + 10;
			four / zero
		};
		result.quotient = quotient;
		result.count = count;
		result.lobe = match four {
			0 => 1,
			3 | 4 => {
				let base: u32 = four * 10;
				base + count
			}
			_ => 7,
		};
		result.picked = pick(2, 1.5);
		result.fallback = pick(9, 1.5);
		result.argument = pick(if (four > 2) { let selector: u32 = zero; selector } else { 1 }, 3.0);
		result.sum = 1.0 + if (four < 2) { 0.0 } else { let side: f32 = 2.0; side * side };
		result.nested = if (four > 1) {
			match zero {
				0 => {
					let seven: u32 = 6;
					seven + 1
				}
				_ => 0,
			}
		} else {
			100
		};
		result.chain = if (four < 2) { 1 } else if (four < 5) { let next: u32 = four + 1; next * 2 } else { 3 };
		result.y = if (zero == 0) { vec2f(1.0, 2.0) } else { vec2f(3.0, 4.0) }.y;
		let total: u32 = 0;
		for (let i: u32 = 0; i < 6; i = i + 1) {
			let step: u32 = match i {
				1 => {
					continue;
				}
				4 => {
					break;
				}
				_ => i * 10,
			};
			total = total + step;
		}
		result.total = total;
	}
	"#;
	let root = buffer_root(
		"result",
		36,
		&[
			("quotient", "u32"),
			("count", "u32"),
			("lobe", "u32"),
			("picked", "f32"),
			("fallback", "f32"),
			("argument", "f32"),
			("sum", "f32"),
			("nested", "u32"),
			("chain", "u32"),
			("y", "f32"),
			("total", "u32"),
		],
	);
	let result = run_slot(&compile_test_program(script, Some(root)), 36);

	assert_eq!(result.read("quotient").expect("quotient"), Value::U32(2));
	assert_eq!(result.read("count").expect("count"), Value::U32(1));
	assert_eq!(result.read("lobe").expect("lobe"), Value::U32(41));
	assert_eq!(result.read("picked").expect("picked"), Value::F32(4.0));
	assert_eq!(result.read("fallback").expect("fallback"), Value::F32(0.5));
	assert_eq!(result.read("argument").expect("argument"), Value::F32(3.0));
	assert_eq!(result.read("sum").expect("sum"), Value::F32(5.0));
	assert_eq!(result.read("nested").expect("nested"), Value::U32(7));
	assert_eq!(result.read("chain").expect("chain"), Value::U32(10));
	assert_eq!(result.read("y").expect("y"), Value::F32(2.0));
	// `i = 1` continues and `i = 4` breaks, so only 0, 20, and 30 are added.
	assert_eq!(result.read("total").expect("total"), Value::U32(50));
}

/// Verifies that an `if` or `match` passed to a function or constructor takes the parameter's type, as a plain `if`
/// does, even when every branch is a literal.
#[test]
fn branch_value_arguments_take_the_parameter_type() {
	let script = r#"
	Pair: struct { first: u16, second: u16 }

	pick: fn (selector: u8) -> u8 {
		return selector;
	}

	main: fn () -> void {
		let k: u32 = 1;
		result.picked = u32(pick(match k { 0 => 1, _ => 2 }));
		let pair: Pair = Pair(if (k > 0) { k = k + 1; 3 } else { 4 }, u16(5));
		result.first = u32(pair.first);
	}
	"#;
	let root = buffer_root("result", 39, &[("picked", "u32"), ("first", "u32")]);
	let result = run_slot(&compile_test_program(script, Some(root)), 39);

	assert_eq!(result.read("picked").expect("picked"), Value::U32(2));
	assert_eq!(result.read("first").expect("first"), Value::U32(3));
}

/// Verifies that a branch whose every path leaves, through a nested `if` or `match` whose branches all do, needs no
/// value, as in Rust.
#[test]
fn branches_whose_paths_all_exit_need_no_value() {
	let script = r#"
	classify: fn (selector: u32) -> u32 {
		let value: u32 = if (selector > 1) {
			if (selector > 5) {
				return 50;
			} else {
				return 20;
			}
		} else if (selector == 1) {
			match selector {
				1 => return 10,
				_ => return 11,
			}
		} else {
			selector + 3
		};
		return value;
	}

	main: fn () -> void {
		result.large = classify(9);
		result.middle = classify(3);
		result.one = classify(1);
		result.zero = classify(0);
	}
	"#;
	let root = buffer_root(
		"result",
		40,
		&[("large", "u32"), ("middle", "u32"), ("one", "u32"), ("zero", "u32")],
	);
	let result = run_slot(&compile_test_program(script, Some(root)), 40);

	assert_eq!(result.read("large").expect("large"), Value::U32(50));
	assert_eq!(result.read("middle").expect("middle"), Value::U32(20));
	assert_eq!(result.read("one").expect("one"), Value::U32(10));
	assert_eq!(result.read("zero").expect("zero"), Value::U32(3));
}

/// Verifies that a value whose branches run statements doesn't change what the parts of its statement that ran
/// before it read, and that the right side of `&&` and `||` still runs only when needed.
#[test]
fn values_with_statements_keep_left_to_right_order() {
	let script = r#"
	combine: fn (left: u32, right: u32) -> u32 {
		return left * 10 + right;
	}

	main: fn () -> void {
		let zero: u32 = 0;
		let four: u32 = 4;
		let y: u32 = 1;
		result.sum = y + if (zero == 0) { y = 2; 10 } else { 0 };
		result.after = y;
		let a: u32 = 5;
		result.call = combine(a, match zero { _ => { a = 7; 1 } });
		// The index runs before the stored value, so the store lands at the old index.
		let values: u32[2] = u32[2](0, 0);
		let i: u32 = 0;
		values[i] = match zero { _ => { i = i + 1; 5 } };
		result.first = values[0];
		result.second = values[1];
		// The run fails on division by zero, so the right sides must not run.
		result.and = if (zero != 0 && match zero { _ => { let q: u32 = four / zero; q > 1 } }) { 1 } else { 2 };
		result.or = if (zero == 0 || match zero { _ => { let q: u32 = four / zero; q > 1 } }) { 3 } else { 4 };
	}
	"#;
	let root = buffer_root(
		"result",
		37,
		&[
			("sum", "u32"),
			("after", "u32"),
			("call", "u32"),
			("first", "u32"),
			("second", "u32"),
			("and", "u32"),
			("or", "u32"),
		],
	);
	let result = run_slot(&compile_test_program(script, Some(root)), 37);

	assert_eq!(result.read("sum").expect("sum"), Value::U32(11));
	assert_eq!(result.read("after").expect("after"), Value::U32(2));
	assert_eq!(result.read("call").expect("call"), Value::U32(51));
	assert_eq!(result.read("first").expect("first"), Value::U32(5));
	assert_eq!(result.read("second").expect("second"), Value::U32(0));
	assert_eq!(result.read("and").expect("and"), Value::U32(2));
	assert_eq!(result.read("or").expect("or"), Value::U32(3));
}

#[test]
fn prefix_operators_reject_operands_they_do_not_apply_to() {
	for (body, operand) in [
		("let x: f32 = 1.0; result.value = ~x;", "f32"),
		("let x: u32 = 1; result.value = !x;", "u32"),
	] {
		let script = format!("main: fn () -> void {{ {body} }}");
		let root = buffer_root("result", 36, &[("value", operand)]);
		let program = compile_to_besl(&script, Some(root)).expect("Expected lexed program");
		let Err(error) = ExecutableProgram::compile(program) else {
			panic!("Expected the VM to reject the operand of `{body}`");
		};
		assert!(matches!(error, VmError::TypeMismatch { .. }), "{body}: {error:?}");
	}
}
