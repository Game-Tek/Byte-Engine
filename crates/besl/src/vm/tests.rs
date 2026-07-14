//! Focused regressions for the VM's private instruction and numeric semantics.

use super::{
	input_slot, output_slot, reflect_vector, Buffer, DescriptorBindings, DescriptorSlot, ExecutableProgram, ExecutionConfig,
	MeshOutputs, SpecializationValues, Texture, Value, VmError,
};
use crate::{compile_to_besl, BindingTypes, Expressions, Node, NodeReference, Operators};

fn read_f32s(buffer: &Buffer, count: usize) -> Vec<f32> {
	buffer
		.bytes()
		.chunks_exact(4)
		.take(count)
		.map(|chunk| f32::from_ne_bytes(chunk.try_into().expect("Expected four bytes")))
		.collect()
}

fn read_u32s(buffer: &Buffer, count: usize) -> Vec<u32> {
	buffer
		.bytes()
		.chunks_exact(4)
		.take(count)
		.map(|chunk| u32::from_ne_bytes(chunk.try_into().expect("Expected four bytes")))
		.collect()
}

fn compile_test_program(script: &str, root: Option<Node>) -> ExecutableProgram {
	let program = compile_to_besl(script, root).expect("Expected lexed program");
	ExecutableProgram::compile(program).expect("Expected runnable program")
}

fn compile_test_root_program(root: NodeReference) -> ExecutableProgram {
	ExecutableProgram::compile(root).expect("Expected runnable program")
}

fn buffer_for_slot(executable: &ExecutableProgram, slot: DescriptorSlot) -> Buffer {
	let layout = executable.buffer_layout(slot).expect("Expected buffer layout").clone();
	Buffer::new(layout)
}

fn interface_buffer_for_input(executable: &ExecutableProgram, location: u8) -> Buffer {
	let layout = executable.input_layout(location).expect("Expected input layout").clone();
	Buffer::new(layout)
}

fn interface_buffer_for_output(executable: &ExecutableProgram, location: u8) -> Buffer {
	let layout = executable.output_layout(location).expect("Expected output layout").clone();
	Buffer::new(layout)
}

fn run_with_buffer(executable: &ExecutableProgram, slot: DescriptorSlot, buffer: &mut Buffer) {
	let mut descriptors = DescriptorBindings::new();
	descriptors.bind_buffer(slot, buffer);
	executable.run_main(&mut descriptors).expect("Expected execution to succeed");
}

fn write_texture(texture: &mut Texture, texels: &[([u32; 2], [f32; 4])]) {
	for (coord, value) in texels {
		texture.write(*coord, *value).expect("Expected texture write to succeed");
	}
}

#[test]
fn executable_program_runs_main_and_writes_a_bound_buffer_member() {
	let script = r#"
	main: fn () -> void {
		buff.value = 42.0;
	}
	"#;

	let mut root = Node::root();
	let float_type = root.get_child("f32").expect("Expected f32");

	root.add_child(
		Node::binding(
			"buff",
			BindingTypes::Buffer {
				members: vec![Node::member("value", float_type).into()],
			},
			0,
			0,
			true,
			true,
		)
		.into(),
	);

	let executable = compile_test_program(script, Some(root));

	let slot = DescriptorSlot::new(0, 0);
	let mut buffer = buffer_for_slot(&executable, slot);
	run_with_buffer(&executable, slot, &mut buffer);

	assert_eq!(buffer.read_f32("value").expect("Expected f32 member"), 42.0);
}

#[test]
fn executable_program_reads_locals_before_writing_to_a_bound_buffer_member() {
	let script = r#"
	main: fn () -> void {
		let value: f32 = 7.5;
		buff.value = value;
	}
	"#;

	let mut root = Node::root();
	let float_type = root.get_child("f32").expect("Expected f32");

	root.add_child(
		Node::binding(
			"buff",
			BindingTypes::Buffer {
				members: vec![Node::member("value", float_type).into()],
			},
			1,
			3,
			true,
			true,
		)
		.into(),
	);

	let executable = compile_test_program(script, Some(root));

	let slot = DescriptorSlot::new(1, 3);
	let mut buffer = buffer_for_slot(&executable, slot);
	run_with_buffer(&executable, slot, &mut buffer);

	assert_eq!(buffer.read_f32("value").expect("Expected f32 member"), 7.5);
}

#[test]
fn executable_program_reads_a_bound_buffer_inside_main() {
	let script = r#"
	main: fn () -> void {
		let value: f32 = input.value;
		output.value = value;
	}
	"#;

	let mut root = Node::root();
	let float_type = root.get_child("f32").expect("Expected f32");

	root.add_child(
		Node::binding(
			"input",
			BindingTypes::Buffer {
				members: vec![Node::member("value", float_type.clone()).into()],
			},
			0,
			7,
			true,
			false,
		)
		.into(),
	);

	root.add_child(
		Node::binding(
			"output",
			BindingTypes::Buffer {
				members: vec![Node::member("value", float_type).into()],
			},
			0,
			8,
			false,
			true,
		)
		.into(),
	);

	let executable = compile_test_program(script, Some(root));

	let input_slot = DescriptorSlot::new(0, 7);
	let output_slot = DescriptorSlot::new(0, 8);
	let mut input = buffer_for_slot(&executable, input_slot);
	let mut output = buffer_for_slot(&executable, output_slot);
	input
		.write("value", Value::F32(9.25))
		.expect("Expected host buffer write to succeed");

	{
		let mut descriptors = DescriptorBindings::new();
		descriptors.bind_buffer(input_slot, &mut input);
		descriptors.bind_buffer(output_slot, &mut output);
		executable.run_main(&mut descriptors).expect("Expected execution to succeed");
	}

	assert_eq!(output.read_f32("value").expect("Expected f32 member"), 9.25);
}

#[test]
fn executable_program_evaluates_addition_before_writing_to_a_bound_buffer_member() {
	let script = r#"
	main: fn () -> void {
		let value: f32 = 7.5;
		buff.value = value + 4.5;
	}
	"#;

	let mut root = Node::root();
	let float_type = root.get_child("f32").expect("Expected f32");

	root.add_child(
		Node::binding(
			"buff",
			BindingTypes::Buffer {
				members: vec![Node::member("value", float_type).into()],
			},
			0,
			1,
			true,
			true,
		)
		.into(),
	);

	let executable = compile_test_program(script, Some(root));

	let slot = DescriptorSlot::new(0, 1);
	let mut buffer = buffer_for_slot(&executable, slot);
	run_with_buffer(&executable, slot, &mut buffer);

	assert_eq!(buffer.read_f32("value").expect("Expected f32 member"), 12.0);
}

#[test]
fn apply_arithmetic_supports_all_basic_scalar_operations() {
	assert_eq!(
		super::apply_arithmetic(super::ArithmeticOperator::Add, &super::Value::U32(2), &super::Value::U32(3))
			.expect("Expected addition to succeed"),
		super::Value::U32(5)
	);
	assert_eq!(
		super::apply_arithmetic(
			super::ArithmeticOperator::Subtract,
			&super::Value::I32(9),
			&super::Value::I32(4)
		)
		.expect("Expected subtraction to succeed"),
		super::Value::I32(5)
	);
	assert_eq!(
		super::apply_arithmetic(
			super::ArithmeticOperator::Multiply,
			&super::Value::U16(6),
			&super::Value::U16(7)
		)
		.expect("Expected multiplication to succeed"),
		super::Value::U16(42)
	);
	assert_eq!(
		super::apply_arithmetic(
			super::ArithmeticOperator::Divide,
			&super::Value::F32(9.0),
			&super::Value::F32(2.0)
		)
		.expect("Expected division to succeed"),
		super::Value::F32(4.5)
	);
	assert_eq!(
		super::apply_arithmetic(super::ArithmeticOperator::Modulo, &super::Value::U8(20), &super::Value::U8(6))
			.expect("Expected modulo to succeed"),
		super::Value::U8(2)
	);
	assert_eq!(
		super::apply_arithmetic(
			super::ArithmeticOperator::Add,
			&super::Value::Vec3F([1.0, 2.0, 3.0]),
			&super::Value::Vec3F([4.0, 5.0, 6.0])
		)
		.expect("Expected vec3f addition to succeed"),
		super::Value::Vec3F([5.0, 7.0, 9.0])
	);
	assert_eq!(
		super::apply_arithmetic(
			super::ArithmeticOperator::Multiply,
			&super::Value::Vec4F([1.0, 2.0, 3.0, 4.0]),
			&super::Value::F32(2.0)
		)
		.expect("Expected vec4f scalar broadcast to succeed"),
		super::Value::Vec4F([2.0, 4.0, 6.0, 8.0])
	);
	assert_eq!(
		super::apply_arithmetic(
			super::ArithmeticOperator::Add,
			&super::Value::Mat4F([1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0,]),
			&super::Value::F32(1.0)
		)
		.expect("Expected mat4f scalar broadcast to succeed"),
		super::Value::Mat4F([2.0, 1.0, 1.0, 1.0, 1.0, 2.0, 1.0, 1.0, 1.0, 1.0, 2.0, 1.0, 1.0, 1.0, 1.0, 2.0,])
	);
}

#[test]
fn executable_program_evaluates_vec3f_arithmetic_before_writing_to_a_bound_buffer_member() {
	let script = r#"
	main: fn () -> void {
		let lhs: vec3f = vec3f(1.0, 2.0, 3.0);
		let rhs: vec3f = vec3f(4.0, 5.0, 6.0);
		buff.value = lhs + rhs;
	}
	"#;

	let mut root = Node::root();
	let vec3f_type = root.get_child("vec3f").expect("Expected vec3f");

	root.add_child(
		Node::binding(
			"buff",
			BindingTypes::Buffer {
				members: vec![Node::member("value", vec3f_type.clone()).into()],
			},
			0,
			2,
			true,
			true,
		)
		.into(),
	);

	let executable = compile_test_program(script, Some(root));

	let slot = DescriptorSlot::new(0, 2);
	let mut buffer = buffer_for_slot(&executable, slot);
	run_with_buffer(&executable, slot, &mut buffer);

	assert_eq!(read_f32s(&buffer, 3), vec![5.0, 7.0, 9.0]);
}

#[test]
fn executable_program_evaluates_vec4f_scalar_broadcast_arithmetic() {
	let script = r#"
	main: fn () -> void {
		let value: vec4f = vec4f(1.0, 2.0, 3.0, 4.0);
		buff.value = value * 2.0;
	}
	"#;

	let mut root = Node::root();
	let vec4f_type = root.get_child("vec4f").expect("Expected vec4f");

	root.add_child(
		Node::binding(
			"buff",
			BindingTypes::Buffer {
				members: vec![Node::member("value", vec4f_type).into()],
			},
			0,
			3,
			true,
			true,
		)
		.into(),
	);

	let executable = compile_test_program(script, Some(root));

	let slot = DescriptorSlot::new(0, 3);
	let mut buffer = buffer_for_slot(&executable, slot);
	run_with_buffer(&executable, slot, &mut buffer);

	assert_eq!(read_f32s(&buffer, 4), vec![2.0, 4.0, 6.0, 8.0]);
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

	let mut root = Node::root();
	let vec4u16_type = root.get_child("vec4u16").expect("Expected vec4u16");
	let u16_type = root.get_child("u16").expect("Expected u16");
	root.add_child(
		Node::binding(
			"buff",
			BindingTypes::Buffer {
				members: vec![
					Node::member("value", vec4u16_type).into(),
					Node::member("last", u16_type).into(),
				],
			},
			0,
			30,
			true,
			true,
		)
		.into(),
	);

	let executable = compile_test_program(script, Some(root));
	let slot = DescriptorSlot::new(0, 30);
	let layout = executable.buffer_layout(slot).expect("Expected vec4u16 buffer layout");
	assert_eq!(layout.member("value").unwrap().value_type().size(), 8);
	assert_eq!(layout.member("last").unwrap().offset(), 8);
	let mut buffer = Buffer::new(layout.clone());
	run_with_buffer(&executable, slot, &mut buffer);

	assert_eq!(buffer.read("value").unwrap(), Value::Vec4U16([5, 5, 5, 5]));
	assert_eq!(buffer.read("last").unwrap(), Value::U16(5));
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

	let mut root = Node::root();
	let mat4f_type = root.get_child("mat4f").expect("Expected mat4f");

	root.add_child(
		Node::binding(
			"buff",
			BindingTypes::Buffer {
				members: vec![Node::member("value", mat4f_type).into()],
			},
			0,
			4,
			true,
			true,
		)
		.into(),
	);

	let executable = compile_test_program(script, Some(root));

	let slot = DescriptorSlot::new(0, 4);
	let mut buffer = buffer_for_slot(&executable, slot);
	run_with_buffer(&executable, slot, &mut buffer);

	assert_eq!(
		read_f32s(&buffer, 16),
		vec![2.0, 1.0, 1.0, 1.0, 1.0, 2.0, 1.0, 1.0, 1.0, 1.0, 2.0, 1.0, 1.0, 1.0, 1.0, 2.0,]
	);
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

	let mut root = Node::root();
	let float_type = root.get_child("f32").expect("Expected f32");

	root.add_child(
		Node::binding(
			"buff",
			BindingTypes::Buffer {
				members: vec![Node::member("value", float_type).into()],
			},
			0,
			5,
			true,
			true,
		)
		.into(),
	);

	let executable = compile_test_program(script, Some(root));

	let slot = DescriptorSlot::new(0, 5);
	let mut buffer = buffer_for_slot(&executable, slot);
	run_with_buffer(&executable, slot, &mut buffer);

	assert_eq!(buffer.read_f32("value").expect("Expected f32 member"), 7.5);
}

#[test]
fn executable_program_calls_function_and_returns_vec3f() {
	let script = r#"
	double: fn (value: vec3f) -> vec3f {
		return value * 2.0;
	}

	main: fn () -> void {
		buff.value = double(vec3f(1.0, 2.0, 3.0));
	}
	"#;

	let mut root = Node::root();
	let vec3f_type = root.get_child("vec3f").expect("Expected vec3f");

	root.add_child(
		Node::binding(
			"buff",
			BindingTypes::Buffer {
				members: vec![Node::member("value", vec3f_type.clone()).into()],
			},
			0,
			6,
			true,
			true,
		)
		.into(),
	);

	let executable = compile_test_program(script, Some(root));

	let slot = DescriptorSlot::new(0, 6);
	let mut buffer = buffer_for_slot(&executable, slot);
	run_with_buffer(&executable, slot, &mut buffer);

	assert_eq!(read_f32s(&buffer, 3), vec![2.0, 4.0, 6.0]);
}

#[test]
fn executable_program_fetches_texture_texels_into_a_bound_buffer_member() {
	let script = r#"
	main: fn () -> void {
		let coord: vec2u = vec2u(1, 0);
		buff.value = fetch(texture, coord);
	}
	"#;

	let mut root = Node::root();
	let vec4f_type = root.get_child("vec4f").expect("Expected vec4f");

	root.add_child(
		Node::binding(
			"texture",
			BindingTypes::CombinedImageSampler { format: String::new() },
			0,
			7,
			true,
			false,
		)
		.into(),
	);
	root.add_child(
		Node::binding(
			"buff",
			BindingTypes::Buffer {
				members: vec![Node::member("value", vec4f_type).into()],
			},
			0,
			8,
			true,
			true,
		)
		.into(),
	);

	let executable = compile_test_program(script, Some(root));

	let texture_slot = DescriptorSlot::new(0, 7);
	let buffer_slot = DescriptorSlot::new(0, 8);
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

	let mut root = Node::root();
	let vec4f_type = root.get_child("vec4f").expect("Expected vec4f");

	root.add_child(
		Node::binding(
			"texture_sampler",
			BindingTypes::CombinedImageSampler { format: String::new() },
			0,
			9,
			true,
			false,
		)
		.into(),
	);
	root.add_child(
		Node::binding(
			"buff",
			BindingTypes::Buffer {
				members: vec![Node::member("value", vec4f_type).into()],
			},
			0,
			10,
			true,
			true,
		)
		.into(),
	);

	let executable = compile_test_program(script, Some(root));

	let texture_slot = DescriptorSlot::new(0, 9);
	let buffer_slot = DescriptorSlot::new(0, 10);
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
			0,
			11,
			false,
			true,
		)
		.into(),
	);

	let executable = compile_test_program(script, Some(root));

	let image_slot = DescriptorSlot::new(0, 11);
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
			0,
			12,
			true,
			true,
		)
		.into(),
	);

	let executable = compile_test_program(script, Some(root));

	let slot = DescriptorSlot::new(0, 12);
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
			0,
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
			0,
			25,
			false,
			true,
		)
		.into(),
	]);

	let executable = compile_test_program(script, Some(root));

	let input_slot = DescriptorSlot::new(0, 24);
	let output_slot = DescriptorSlot::new(0, 25);
	let input_layout = executable
		.buffer_layout(input_slot)
		.expect("Expected input buffer layout")
		.clone();
	let mut input = Buffer::new(input_layout.clone());
	let meshes_member = input_layout.member("meshes").expect("Expected meshes member");
	input
		.write_value(
			meshes_member.offset() + meshes_member.value_type().size(),
			meshes_member.value_type(),
			&Value::U32(42),
		)
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
fn executable_program_reads_push_constant_members() {
	let script = r#"
	main: fn () -> void {
		buff.value = push_constant.material_id;
	}
	"#;

	let mut root = Node::root();
	let float_type = root.get_child("f32").expect("Expected f32");
	root.add_child(Node::push_constant(vec![Node::member("material_id", float_type.clone()).into()]).into());
	root.add_child(
		Node::binding(
			"buff",
			BindingTypes::Buffer {
				members: vec![Node::member("value", float_type).into()],
			},
			0,
			14,
			true,
			true,
		)
		.into(),
	);

	let executable = compile_test_program(script, Some(root));

	let slot = DescriptorSlot::new(0, 14);
	let push_constant_layout = executable
		.push_constant_layout()
		.expect("Expected push constant layout")
		.clone();
	let mut buffer = buffer_for_slot(&executable, slot);
	let mut push_constant = Buffer::new(push_constant_layout);
	push_constant
		.write("material_id", Value::F32(3.5))
		.expect("Expected push constant write");

	{
		let mut descriptors = DescriptorBindings::new();
		descriptors.bind_buffer(slot, &mut buffer);
		descriptors.bind_push_constant(&mut push_constant);
		executable.run_main(&mut descriptors).expect("Expected execution to succeed");
	}

	assert_eq!(buffer.read_f32("value").expect("Expected f32"), 3.5);
}

#[test]
fn executable_program_requires_bound_push_constant() {
	let script = r#"
	main: fn () -> void {
		buff.value = push_constant.material_id;
	}
	"#;

	let mut root = Node::root();
	let float_type = root.get_child("f32").expect("Expected f32");
	root.add_child(Node::push_constant(vec![Node::member("material_id", float_type.clone()).into()]).into());
	root.add_child(
		Node::binding(
			"buff",
			BindingTypes::Buffer {
				members: vec![Node::member("value", float_type).into()],
			},
			0,
			15,
			true,
			true,
		)
		.into(),
	);

	let executable = compile_test_program(script, Some(root));

	let slot = DescriptorSlot::new(0, 15);
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
fn executable_program_writes_vertex_shader_output_interfaces() {
	let script = r#"
	main: fn () -> void {
		out_position = vec4f(1.0, 2.0, 3.0, 1.0);
		out_instance_index = 7;
	}
	"#;

	let mut root = Node::root();
	let vec4f_type = root.get_child("vec4f").expect("Expected vec4f");
	let u32_type = root.get_child("u32").expect("Expected u32");
	root.add_child(Node::output("out_position", vec4f_type, 0).into());
	root.add_child(Node::output("out_instance_index", u32_type, 1).into());

	let executable = compile_test_program(script, Some(root));

	let mut position = interface_buffer_for_output(&executable, 0);
	let mut instance = interface_buffer_for_output(&executable, 1);

	{
		let mut descriptors = DescriptorBindings::new();
		descriptors.bind_buffer(output_slot(0), &mut position);
		descriptors.bind_buffer(output_slot(1), &mut instance);
		executable.run_main(&mut descriptors).expect("Expected execution to succeed");
	}

	assert_eq!(read_f32s(&position, 4), vec![1.0, 2.0, 3.0, 1.0]);
	assert_eq!(
		instance.read("out_instance_index").expect("Expected output value"),
		Value::U32(7)
	);
}

#[test]
fn executable_program_reads_vertex_shader_input_interfaces() {
	let script = r#"
	main: fn () -> void {
		out_color = in_color;
	}
	"#;

	let mut root = Node::root();
	let vec4f_type = root.get_child("vec4f").expect("Expected vec4f");
	root.add_child(Node::input("in_color", vec4f_type.clone(), 0).into());
	root.add_child(Node::output("out_color", vec4f_type, 0).into());

	let executable = compile_test_program(script, Some(root));

	let mut input = interface_buffer_for_input(&executable, 0);
	let mut output = interface_buffer_for_output(&executable, 0);
	input
		.write("in_color", Value::Vec4F([0.1, 0.2, 0.3, 1.0]))
		.expect("Expected input write to succeed");

	{
		let mut descriptors = DescriptorBindings::new();
		descriptors.bind_buffer(input_slot(0), &mut input);
		descriptors.bind_buffer(output_slot(0), &mut output);
		executable.run_main(&mut descriptors).expect("Expected execution to succeed");
	}

	assert_eq!(
		output.read("out_color").expect("Expected output value"),
		Value::Vec4F([0.1, 0.2, 0.3, 1.0])
	);
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

	let mut vertex_input = interface_buffer_for_input(&vertex_executable, 0);
	let mut vertex_output = interface_buffer_for_output(&vertex_executable, 0);
	let mut fragment_output = interface_buffer_for_output(&fragment_executable, 0);

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
fn executable_program_supports_fragment_style_input_attachments() {
	let script = r#"
	main: fn () -> void {
		out_color = fetch(input_attachment, vec2u(1, 0));
	}
	"#;

	let mut root = Node::root();
	let vec4f_type = root.get_child("vec4f").expect("Expected vec4f");
	root.add_child(
		Node::binding(
			"input_attachment",
			BindingTypes::CombinedImageSampler { format: String::new() },
			0,
			16,
			true,
			false,
		)
		.into(),
	);
	root.add_child(Node::output("out_color", vec4f_type, 0).into());

	let executable = compile_test_program(script, Some(root));

	let input_attachment_slot = DescriptorSlot::new(0, 16);
	let mut input_attachment = Texture::new(2, 1).expect("Expected attachment allocation");
	let mut output = interface_buffer_for_output(&executable, 0);
	write_texture(
		&mut input_attachment,
		&[([0, 0], [0.1, 0.2, 0.3, 1.0]), ([1, 0], [0.9, 0.4, 0.2, 1.0])],
	);

	{
		let mut descriptors = DescriptorBindings::new();
		descriptors.bind_texture(input_attachment_slot, &mut input_attachment);
		descriptors.bind_buffer(output_slot(0), &mut output);
		executable
			.run_main(&mut descriptors)
			.expect("Expected fragment-style execution to succeed");
	}

	assert_eq!(
		output.read("out_color").expect("Expected output value"),
		Value::Vec4F([0.9, 0.4, 0.2, 1.0])
	);
}

#[test]
fn executable_program_evaluates_dot_intrinsics() {
	let script = r#"
	main: fn () -> void {
		buff.value = dot(vec3f(1.0, 2.0, 3.0), vec3f(4.0, 5.0, 6.0));
	}
	"#;

	let mut root = Node::root();
	let float_type = root.get_child("f32").expect("Expected f32");
	root.add_child(
		Node::binding(
			"buff",
			BindingTypes::Buffer {
				members: vec![Node::member("value", float_type).into()],
			},
			0,
			17,
			true,
			true,
		)
		.into(),
	);

	let executable = compile_test_program(script, Some(root));

	let slot = DescriptorSlot::new(0, 17);
	let mut buffer = buffer_for_slot(&executable, slot);
	run_with_buffer(&executable, slot, &mut buffer);

	assert_eq!(buffer.read_f32("value").expect("Expected f32 member"), 32.0);
}

#[test]
fn executable_program_evaluates_cross_intrinsics() {
	let script = r#"
	main: fn () -> void {
		buff.value = cross(vec3f(1.0, 0.0, 0.0), vec3f(0.0, 1.0, 0.0));
	}
	"#;

	let mut root = Node::root();
	let vec3f_type = root.get_child("vec3f").expect("Expected vec3f");
	root.add_child(
		Node::binding(
			"buff",
			BindingTypes::Buffer {
				members: vec![Node::member("value", vec3f_type.clone()).into()],
			},
			0,
			18,
			true,
			true,
		)
		.into(),
	);

	let executable = compile_test_program(script, Some(root));

	let slot = DescriptorSlot::new(0, 18);
	let mut buffer = buffer_for_slot(&executable, slot);
	run_with_buffer(&executable, slot, &mut buffer);

	assert_eq!(read_f32s(&buffer, 3), vec![0.0, 0.0, 1.0]);
}

#[test]
fn executable_program_evaluates_length_intrinsics() {
	let script = r#"
	main: fn () -> void {
		buff.value = length(vec3f(3.0, 4.0, 0.0));
	}
	"#;

	let mut root = Node::root();
	let float_type = root.get_child("f32").expect("Expected f32");
	root.add_child(
		Node::binding(
			"buff",
			BindingTypes::Buffer {
				members: vec![Node::member("value", float_type).into()],
			},
			0,
			19,
			true,
			true,
		)
		.into(),
	);

	let executable = compile_test_program(script, Some(root));

	let slot = DescriptorSlot::new(0, 19);
	let mut buffer = buffer_for_slot(&executable, slot);
	run_with_buffer(&executable, slot, &mut buffer);

	assert_eq!(buffer.read_f32("value").expect("Expected f32 member"), 5.0);
}

#[test]
fn executable_program_evaluates_normalize_intrinsics() {
	let script = r#"
	main: fn () -> void {
		buff.value = normalize(vec3f(3.0, 4.0, 0.0));
	}
	"#;

	let mut root = Node::root();
	let vec3f_type = root.get_child("vec3f").expect("Expected vec3f");
	root.add_child(
		Node::binding(
			"buff",
			BindingTypes::Buffer {
				members: vec![Node::member("value", vec3f_type.clone()).into()],
			},
			0,
			20,
			true,
			true,
		)
		.into(),
	);

	let executable = compile_test_program(script, Some(root));

	let slot = DescriptorSlot::new(0, 20);
	let mut buffer = buffer_for_slot(&executable, slot);
	run_with_buffer(&executable, slot, &mut buffer);

	assert_eq!(read_f32s(&buffer, 3), vec![0.6, 0.8, 0.0]);
}

#[test]
fn executable_program_evaluates_reflect_intrinsics() {
	let mut root = Node::root();
	let void_type = root.get_child("void").expect("Expected void");
	let vec3f_type = root.get_child("vec3f").expect("Expected vec3f");
	let reflect = root.get_child("reflect").expect("Expected reflect intrinsic");
	root.add_child(
		Node::binding(
			"buff",
			BindingTypes::Buffer {
				members: vec![Node::member("value", vec3f_type.clone()).into()],
			},
			0,
			21,
			true,
			true,
		)
		.into(),
	);
	root.add_child(
		Node::function(
			"main",
			Vec::new(),
			void_type,
			vec![Node::expression(Expressions::Operator {
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
							function: vec3f_type.clone(),
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
							function: vec3f_type.clone(),
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
			.into()],
		)
		.into(),
	);

	let executable = compile_test_root_program(root.into());

	let slot = DescriptorSlot::new(0, 21);
	let mut buffer = buffer_for_slot(&executable, slot);
	run_with_buffer(&executable, slot, &mut buffer);

	assert_eq!(read_f32s(&buffer, 3), vec![1.0, 1.0, 0.0]);
}

#[test]
fn executable_program_reads_thread_idx_for_compute_style_workflows() {
	let script = r#"
	main: fn () -> void {
		buff.thread = thread_idx();
	}
	"#;

	let mut root = Node::root();
	let u32_type = root.get_child("u32").expect("Expected u32");
	root.add_child(
		Node::binding(
			"buff",
			BindingTypes::Buffer {
				members: vec![Node::member("thread", u32_type).into()],
			},
			0,
			22,
			true,
			true,
		)
		.into(),
	);

	let executable = compile_test_program(script, Some(root));

	let slot = DescriptorSlot::new(0, 22);
	let mut buffer = buffer_for_slot(&executable, slot);

	{
		let mut descriptors = DescriptorBindings::new();
		descriptors.bind_buffer(slot, &mut buffer);
		executable.run_main(&mut descriptors).expect("Expected execution to succeed");
	}

	assert_eq!(buffer.read("thread").expect("Expected thread value"), Value::U32(0));
}

#[test]
fn executable_program_reads_thread_idx_for_mesh_style_workflows() {
	let script = r#"
	main: fn () -> void {
		payload.thread = thread_idx();
	}
	"#;

	let mut root = Node::root();
	let u32_type = root.get_child("u32").expect("Expected u32");
	root.add_child(
		Node::binding(
			"payload",
			BindingTypes::Buffer {
				members: vec![Node::member("thread", u32_type).into()],
			},
			0,
			23,
			true,
			true,
		)
		.into(),
	);

	let executable = compile_test_program(script, Some(root));

	let slot = DescriptorSlot::new(0, 23);
	let mut buffer = buffer_for_slot(&executable, slot);

	{
		let mut descriptors = DescriptorBindings::new();
		descriptors.bind_buffer(slot, &mut buffer);
		executable.run_main(&mut descriptors).expect("Expected execution to succeed");
	}

	assert_eq!(buffer.read("thread").expect("Expected thread value"), Value::U32(0));
}

#[test]
fn executable_program_executes_for_loops() {
	let script = r#"
	main: fn () -> void {
		let sum: u32 = 0;
		for (let i: u32 = 0; i < 4; i = i + 1) {
			sum = sum + i;
		}
		buff.sum = sum;
	}
	"#;

	let mut root = Node::root();
	let u32_type = root.get_child("u32").expect("Expected u32");
	root.add_child(
		Node::binding(
			"buff",
			BindingTypes::Buffer {
				members: vec![Node::member("sum", u32_type).into()],
			},
			0,
			24,
			true,
			true,
		)
		.into(),
	);

	let executable = compile_test_program(script, Some(root));

	let slot = DescriptorSlot::new(0, 24);
	let mut buffer = buffer_for_slot(&executable, slot);

	{
		let mut descriptors = DescriptorBindings::new();
		descriptors.bind_buffer(slot, &mut buffer);
		executable.run_main(&mut descriptors).expect("Expected execution to succeed");
	}

	assert_eq!(buffer.read("sum").expect("Expected sum value"), Value::U32(6));
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

	let mut root = Node::root();
	let u32_type = root.get_child("u32").expect("Expected u32");
	root.add_child(
		Node::binding(
			"buff",
			BindingTypes::Buffer {
				members: vec![Node::member("sum", u32_type).into()],
			},
			0,
			25,
			true,
			true,
		)
		.into(),
	);

	let executable = compile_test_program(script, Some(root));

	let slot = DescriptorSlot::new(0, 25);
	let mut buffer = buffer_for_slot(&executable, slot);
	run_with_buffer(&executable, slot, &mut buffer);

	assert_eq!(buffer.read("sum").expect("Expected sum value"), Value::U32(1));
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
		buff.fract_value = fract(1.25);
		buff.radians_value = radians(180.0);
		buff.inverse_sqrt_value = inversesqrt(4.0);
		buff.smoothstep_value = smoothstep(0.0, 1.0, 0.5);
		buff.mix_value = mix(2.0, 4.0, 0.25);
	}
	"#;

	let mut root = Node::root();
	let f32_type = root.get_child("f32").expect("Expected f32");
	root.add_child(
		Node::binding(
			"buff",
			BindingTypes::Buffer {
				members: vec![
					Node::member("abs_value", f32_type.clone()).into(),
					Node::member("sqrt_value", f32_type.clone()).into(),
					Node::member("exp_value", f32_type.clone()).into(),
					Node::member("sin_value", f32_type.clone()).into(),
					Node::member("cos_value", f32_type.clone()).into(),
					Node::member("tan_value", f32_type.clone()).into(),
					Node::member("fract_value", f32_type.clone()).into(),
					Node::member("radians_value", f32_type.clone()).into(),
					Node::member("inverse_sqrt_value", f32_type.clone()).into(),
					Node::member("smoothstep_value", f32_type.clone()).into(),
					Node::member("mix_value", f32_type).into(),
				],
			},
			0,
			26,
			true,
			true,
		)
		.into(),
	);

	let executable = compile_test_program(script, Some(root));
	let slot = DescriptorSlot::new(0, 26);
	let mut buffer = buffer_for_slot(&executable, slot);
	run_with_buffer(&executable, slot, &mut buffer);

	let values = read_f32s(&buffer, 11);
	assert!((values[0] - 2.5).abs() < 1e-6);
	assert!((values[1] - 3.0).abs() < 1e-6);
	assert!((values[2] - std::f32::consts::E).abs() < 1e-5);
	assert!(values[3].abs() < 1e-6);
	assert!((values[4] - 1.0).abs() < 1e-6);
	assert!(values[5].abs() < 1e-6);
	assert!((values[6] - 0.25).abs() < 1e-6);
	assert!((values[7] - std::f32::consts::PI).abs() < 1e-6);
	assert!((values[8] - 0.5).abs() < 1e-6);
	assert!((values[9] - 0.5).abs() < 1e-6);
	assert!((values[10] - 2.5).abs() < 1e-6);
}

#[test]
fn executable_program_evaluates_scalar_max_and_clamp() {
	let script = r#"
	main: fn () -> void {
		buff.max_value = max(1.5, 2.5);
		buff.clamp_value = clamp(1.5, 0.0, 1.0);
	}
	"#;

	let mut root = Node::root();
	let f32_type = root.get_child("f32").expect("Expected f32");
	root.add_child(
		Node::binding(
			"buff",
			BindingTypes::Buffer {
				members: vec![
					Node::member("max_value", f32_type.clone()).into(),
					Node::member("clamp_value", f32_type).into(),
				],
			},
			0,
			27,
			true,
			true,
		)
		.into(),
	);

	let executable = compile_test_program(script, Some(root));
	let slot = DescriptorSlot::new(0, 27);
	let mut buffer = buffer_for_slot(&executable, slot);
	run_with_buffer(&executable, slot, &mut buffer);

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
	let mut root = Node::root();
	let vec4f = root.get_child("vec4f").expect("Expected vec4f");
	root.add_children(vec![
		Node::binding(
			"source_texture",
			BindingTypes::CombinedImageSampler { format: String::new() },
			0,
			30,
			true,
			false,
		)
		.into(),
		Node::binding(
			"result",
			BindingTypes::Buffer {
				members: vec![Node::member("color", vec4f).into()],
			},
			0,
			31,
			true,
			true,
		)
		.into(),
	]);
	let executable = compile_test_program(script, Some(root));
	let mut texture = Texture::new(1, 1).expect("Expected texture");
	texture.write([0, 0], [0.25, 0.5, 0.75, 1.0]).expect("Expected texel write");
	let mut result = buffer_for_slot(&executable, DescriptorSlot::new(0, 31));
	{
		let mut descriptors = DescriptorBindings::new();
		descriptors.bind_texture(DescriptorSlot::new(0, 30), &mut texture);
		descriptors.bind_buffer(DescriptorSlot::new(0, 31), &mut result);
		executable
			.run_main(&mut descriptors)
			.expect("Expected descriptor-handle execution");
	}
	assert_eq!(
		result.read("color").expect("Expected color"),
		Value::Vec4F([0.25, 0.5, 0.75, 1.0])
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
	let mut root = Node::root();
	let f32_type = root.get_child("f32").expect("Expected f32");
	root.add_child(
		Node::binding(
			"result",
			BindingTypes::Buffer {
				members: vec![Node::member("value", f32_type).into()],
			},
			0,
			32,
			true,
			true,
		)
		.into(),
	);
	let executable = compile_test_program(script, Some(root));
	let mut result = buffer_for_slot(&executable, DescriptorSlot::new(0, 32));
	run_with_buffer(&executable, DescriptorSlot::new(0, 32), &mut result);
	assert_eq!(result.read("value").expect("Expected selected weight"), Value::F32(0.75));
}

#[test]
fn mesh_intrinsics_capture_geometry_and_indexed_outputs() {
	let script = r#"
	main: fn () -> void {
		set_mesh_output_counts(1, 1);
		set_mesh_vertex_position(0, vec4f(1.0, 2.0, 3.0, 1.0));
		set_mesh_triangle(0, vec3u(0, 0, 0));
		out_index[0] = 17;
	}
	"#;
	let mut root = Node::root();
	let u32_type = root.get_child("u32").expect("Expected u32");
	root.add_child(Node::output_array("out_index", u32_type, 0, 1).into());
	let executable = compile_test_program(script, Some(root));
	let mut output = interface_buffer_for_output(&executable, 0);
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
	assert_eq!(
		output.read_indexed("out_index", 0).expect("Expected indexed output"),
		Value::U32(17)
	);
}

#[test]
fn mesh_output_counts_clear_reused_capture_ranges() {
	let mut outputs = MeshOutputs::new();
	outputs.set_counts(2, 2, 2, 2, false).expect("Expected bounded mesh outputs");
	outputs.vertex_positions[0] = [1.0, 2.0, 3.0, 1.0];
	outputs.triangles[0] = [4, 5, 6];

	outputs.set_counts(2, 2, 2, 2, true).expect("Expected capture reuse");

	assert_eq!(outputs.vertex_positions, vec![[0.0; 4]; 2]);
	assert_eq!(outputs.triangles, vec![[0; 3]; 2]);
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
	let slot = DescriptorSlot::new(4, 2);
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
	assert!(VmError::UnboundDescriptor { slot }
		.to_string()
		.contains("no resource was bound"));
}

#[test]
fn specialization_values_select_x_and_y_components() {
	for (axis, expected) in [([1.0, 0.0], 1.0), ([0.0, 1.0], 2.0)] {
		let script = r#"
		main: fn () -> void {
			result.value = axis.x + axis.y * 2.0;
		}
		"#;
		let mut root = Node::root();
		let vec2f = root.get_child("vec2f").expect("Expected vec2f");
		let f32_type = root.get_child("f32").expect("Expected f32");
		root.add_children(vec![
			Node::specialization("axis", vec2f).into(),
			Node::binding(
				"result",
				BindingTypes::Buffer {
					members: vec![Node::member("value", f32_type).into()],
				},
				0,
				33,
				true,
				true,
			)
			.into(),
		]);
		let program = compile_to_besl(script, Some(root)).expect("Expected lexed specialization program");
		let mut specializations = SpecializationValues::new();
		specializations.set("axis", Value::Vec2F(axis));
		let executable = ExecutableProgram::compile_with_specializations(program, &specializations)
			.expect("Expected specialized executable");
		let mut result = buffer_for_slot(&executable, DescriptorSlot::new(0, 33));
		run_with_buffer(&executable, DescriptorSlot::new(0, 33), &mut result);
		assert_eq!(
			result.read("value").expect("Expected specialization result"),
			Value::F32(expected)
		);
	}
}
