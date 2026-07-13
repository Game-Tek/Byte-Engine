use std::collections::HashSet;

use besl::vm::{
	builtin_position_slot, input_slot, output_slot, Buffer, DescriptorBindings, DescriptorSlot, ExecutableProgram, Texture,
	Value, VmError,
};
use besl::{compile_to_besl, BindingTypes, Node};

fn compile_program(source: &str, root: Node) -> Result<ExecutableProgram, VmError> {
	let program = compile_to_besl(source, Some(root)).expect("Expected lexed program");
	ExecutableProgram::compile(program)
}

#[test]
fn real_descriptor_slots_do_not_alias_virtual_interface_slots() {
	let virtual_slots = [input_slot(3), output_slot(3), builtin_position_slot()];
	let mut slots = HashSet::new();

	for virtual_slot in virtual_slots {
		let descriptor_slot = DescriptorSlot::new(virtual_slot.set(), virtual_slot.binding());
		assert_ne!(descriptor_slot, virtual_slot);
		slots.insert(descriptor_slot);
		slots.insert(virtual_slot);
	}

	assert_eq!(slots.len(), 6);
}

#[test]
fn maximum_descriptor_slot_does_not_alias_push_constants() {
	let mut root = Node::root();
	let f32_type = root.get_child("f32").expect("Expected f32 type");
	root.add_children(vec![
		Node::binding(
			"descriptor",
			BindingTypes::Buffer {
				members: vec![Node::member("value", f32_type.clone()).into()],
			},
			u32::MAX,
			u32::MAX,
			true,
			false,
		)
		.into(),
		Node::push_constant(vec![Node::member("value", f32_type.clone()).into()]).into(),
		Node::binding(
			"result",
			BindingTypes::Buffer {
				members: vec![Node::member("value", f32_type).into()],
			},
			0,
			0,
			false,
			true,
		)
		.into(),
	]);

	let executable = compile_program(
		r#"
		main: fn () -> void {
			result.value = descriptor.value + push_constant.value;
		}
		"#,
		root,
	)
	.expect("Expected descriptor and push constant layouts to coexist");

	let descriptor_slot = DescriptorSlot::new(u32::MAX, u32::MAX);
	let result_slot = DescriptorSlot::new(0, 0);
	let mut descriptor = Buffer::new(
		executable
			.buffer_layout(descriptor_slot)
			.expect("Expected maximum descriptor layout")
			.clone(),
	);
	let mut push_constant = Buffer::new(
		executable
			.push_constant_layout()
			.expect("Expected push constant layout")
			.clone(),
	);
	let mut result = Buffer::new(executable.buffer_layout(result_slot).expect("Expected result layout").clone());
	descriptor.write("value", Value::F32(2.0)).expect("Expected descriptor write");
	push_constant
		.write("value", Value::F32(3.0))
		.expect("Expected push constant write");

	let mut descriptors = DescriptorBindings::new();
	descriptors.bind_buffer(descriptor_slot, &mut descriptor);
	descriptors.bind_push_constant(&mut push_constant);
	descriptors.bind_buffer(result_slot, &mut result);
	executable
		.run_main(&mut descriptors)
		.expect("Expected isolated slot execution");

	assert_eq!(result.read("value").expect("Expected result value"), Value::F32(5.0));
}

#[test]
fn descriptor_using_dynamic_resource_set_remains_a_real_descriptor() {
	let dynamic_resource_set = u32::MAX - 4;
	let texture_slot = DescriptorSlot::new(dynamic_resource_set, 0);
	let result_slot = DescriptorSlot::new(0, 0);
	let mut root = Node::root();
	let vec4f_type = root.get_child("vec4f").expect("Expected vec4f type");
	root.add_children(vec![
		Node::binding(
			"source",
			BindingTypes::CombinedImageSampler { format: String::new() },
			dynamic_resource_set,
			0,
			true,
			false,
		)
		.into(),
		Node::binding(
			"result",
			BindingTypes::Buffer {
				members: vec![Node::member("color", vec4f_type).into()],
			},
			0,
			0,
			false,
			true,
		)
		.into(),
	]);
	let executable = compile_program(
		r#"
		main: fn () -> void {
			result.color = fetch(source, vec2u(0, 0));
		}
		"#,
		root,
	)
	.expect("Expected real descriptor in the reserved numeric range");

	let mut texture = Texture::new(1, 1).expect("Expected texture");
	texture.write([0, 0], [0.25, 0.5, 0.75, 1.0]).expect("Expected texel write");
	let mut result = Buffer::new(executable.buffer_layout(result_slot).expect("Expected result layout").clone());
	let mut descriptors = DescriptorBindings::new();
	descriptors.bind_texture(texture_slot, &mut texture);
	descriptors.bind_buffer(result_slot, &mut result);
	executable
		.run_main(&mut descriptors)
		.expect("Expected real descriptor lookup");

	assert_eq!(
		result.read("color").expect("Expected sampled color"),
		Value::Vec4F([0.25, 0.5, 0.75, 1.0])
	);
}

#[test]
fn non_indexed_field_access_rejects_array_members() {
	let mut root = Node::root();
	let u32_type = root.get_child("u32").expect("Expected u32 type");
	let item_type = root.add_child(Node::r#struct("Item", vec![Node::member("value", u32_type.clone()).into()]).into());
	root.add_children(vec![
		Node::binding(
			"items",
			BindingTypes::Buffer {
				members: vec![Node::array("items", item_type, 2)],
			},
			0,
			0,
			true,
			false,
		)
		.into(),
		Node::binding(
			"result",
			BindingTypes::Buffer {
				members: vec![Node::member("value", u32_type).into()],
			},
			0,
			1,
			false,
			true,
		)
		.into(),
	]);
	let executable = compile_program(
		r#"
		main: fn () -> void {
			result.value = items.items[1].value;
		}
		"#,
		root,
	)
	.expect("Expected array-of-struct layout");
	let mut items = Buffer::new(
		executable
			.buffer_layout(DescriptorSlot::new(0, 0))
			.expect("Expected items layout")
			.clone(),
	);

	assert!(matches!(
		items.read_field("items", "value"),
		Err(VmError::UnsupportedBufferLayout { .. })
	));
	assert!(matches!(
		items.write_field("items", "value", Value::U32(7)),
		Err(VmError::UnsupportedBufferLayout { .. })
	));

	items
		.write_indexed_field("items", 1, "value", Value::U32(7))
		.expect("Expected explicit array element write");
	assert_eq!(
		items
			.read_indexed_field("items", 1, "value")
			.expect("Expected explicit array element read"),
		Value::U32(7)
	);
}

#[test]
fn texture_creation_rejects_overflowing_texel_counts() {
	let error = Texture::new_3d(u32::MAX, u32::MAX, u32::MAX).expect_err("Expected texel count overflow");
	assert_eq!(
		error,
		VmError::TextureTexelCountOverflow {
			width: u32::MAX,
			height: u32::MAX,
			depth: u32::MAX,
		}
	);
	assert_eq!(
		error.to_string(),
		format!(
			"Texture dimensions {0}x{0}x{0} are too large. The most likely cause is that their texel count exceeds addressable CPU memory.",
			u32::MAX
		)
	);
	assert_eq!(
		Texture::new_3d(u32::MAX, u32::MAX, 1).expect_err("Expected allocation capacity overflow"),
		VmError::TextureTexelCountOverflow {
			width: u32::MAX,
			height: u32::MAX,
			depth: 1,
		}
	);
}

#[test]
fn texture_access_rejects_stale_cross_format_views() {
	let mut texture = Texture::new(1, 1).expect("Expected texture");
	texture.write_u32([0, 0], 7).expect("Expected integer write");
	assert_eq!(texture.fetch_u32([0, 0]).expect("Expected integer fetch"), Value::U32(7));
	assert!(matches!(
		texture.fetch([0, 0]),
		Err(VmError::TextureFormatMismatch {
			expected: "float RGBA",
			found: "u32",
		})
	));

	texture
		.write([0, 0], [0.25, 0.5, 0.75, 1.0])
		.expect("Expected float write to replace the texel format");
	assert_eq!(
		texture.fetch([0, 0]).expect("Expected float fetch"),
		Value::Vec4F([0.25, 0.5, 0.75, 1.0])
	);
	assert!(matches!(
		texture.fetch_u32([0, 0]),
		Err(VmError::TextureFormatMismatch {
			expected: "u32",
			found: "float RGBA",
		})
	));
}

#[test]
fn nested_array_fields_are_rejected_during_layout_compilation() {
	let mut root = Node::root();
	let u32_type = root.get_child("u32").expect("Expected u32 type");
	let item_type = root.add_child(Node::r#struct("Item", vec![Node::array("values", u32_type.clone(), 2)]).into());
	root.add_children(vec![
		Node::binding(
			"items",
			BindingTypes::Buffer {
				members: vec![Node::member("item", item_type).into()],
			},
			0,
			0,
			true,
			false,
		)
		.into(),
		Node::binding(
			"result",
			BindingTypes::Buffer {
				members: vec![Node::member("value", u32_type).into()],
			},
			0,
			1,
			false,
			true,
		)
		.into(),
	]);

	let error = match compile_program(
		r#"
		main: fn () -> void {
			result.value = items.item.values[0];
		}
		"#,
		root,
	) {
		Ok(_) => panic!("Expected nested array field rejection"),
		Err(error) => error,
	};

	assert_eq!(
		error,
		VmError::UnsupportedBufferLayout {
			message: "Struct field `values` cannot be an array".to_string(),
		}
	);
}

#[test]
fn buffer_layout_rejects_overflowing_arrays() {
	let mut root = Node::root();
	let u32_type = root.get_child("u32").expect("Expected u32 type");
	root.add_children(vec![
		Node::binding(
			"values",
			BindingTypes::Buffer {
				members: vec![Node::array("values", u32_type.clone(), usize::MAX)],
			},
			0,
			0,
			true,
			false,
		)
		.into(),
		Node::binding(
			"result",
			BindingTypes::Buffer {
				members: vec![Node::member("value", u32_type).into()],
			},
			0,
			1,
			false,
			true,
		)
		.into(),
	]);

	let error = compile_error(
		r#"
		main: fn () -> void {
			result.value = values.values[0];
		}
		"#,
		root,
	);
	assert_eq!(
		error,
		VmError::UnsupportedBufferLayout {
			message: "Buffer member `values` exceeds addressable CPU memory".to_string(),
		}
	);
}

#[test]
fn buffer_layout_rejects_resource_handle_members() {
	let mut root = Node::root();
	let u32_type = root.get_child("u32").expect("Expected u32 type");
	let texture_type = root.get_child("Texture2D").expect("Expected texture type");
	root.add_children(vec![
		Node::binding(
			"invalid",
			BindingTypes::Buffer {
				members: vec![Node::member("resource", texture_type).into()],
			},
			0,
			0,
			true,
			false,
		)
		.into(),
		Node::binding(
			"result",
			BindingTypes::Buffer {
				members: vec![Node::member("value", u32_type).into()],
			},
			0,
			1,
			false,
			true,
		)
		.into(),
	]);

	let error = compile_error(
		r#"
		main: fn () -> void {
			result.value = invalid.resource;
		}
		"#,
		root,
	);
	assert_eq!(
		error,
		VmError::UnsupportedBufferLayout {
			message: "Buffer member `resource` cannot contain resource handles".to_string(),
		}
	);
}

fn compile_error(source: &str, root: Node) -> VmError {
	match compile_program(source, root) {
		Ok(_) => panic!("Expected VM compilation to fail"),
		Err(error) => error,
	}
}
