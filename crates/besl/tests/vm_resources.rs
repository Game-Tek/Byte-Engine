use std::collections::HashSet;

use besl::vm::{
	Buffer, DescriptorBindings, ExecutableProgram, ResourceSlot, Sampler, SamplerReductionMode, Texture, Value, VmError,
	builtin_position_slot, input_slot, output_slot,
};
use besl::{BindingTypes, Node, compile_to_besl};

fn compile_program(source: &str, root: Node) -> Result<ExecutableProgram, VmError> {
	let program = compile_to_besl(source, Some(root)).expect("Expected lexed program");
	ExecutableProgram::compile(program)
}

#[test]
fn real_resource_slots_do_not_alias_virtual_interface_slots() {
	let virtual_slots = [input_slot(3), output_slot(3), builtin_position_slot()];
	let mut slots = HashSet::new();

	for virtual_slot in virtual_slots {
		let descriptor_slot = ResourceSlot::new(virtual_slot.slot());

		assert_ne!(descriptor_slot, virtual_slot);
		slots.insert(descriptor_slot);
		slots.insert(virtual_slot);
	}

	// Input and output slot 3 deliberately share one numeric real-resource counterpart.

	assert_eq!(slots.len(), 5);
}

#[test]
fn structural_position_return_uses_the_builtin_position_slot_without_shadowing_its_local() {
	let program = compile_to_besl(
		r#"
		main: fn () -> interface { position: vec4f } {
			let position: vec4f = vec4f(1.0, 2.0, 3.0, 1.0);
			return { position };
		}
		"#,
		None,
	)
	.expect("Expected structural vertex source to link");
	let executable = ExecutableProgram::compile(program).expect("Expected structural vertex source to compile");
	let mut position = Buffer::new(
		executable
			.builtin_position_layout()
			.expect("Expected VM position output")
			.clone(),
	);

	let mut descriptors = DescriptorBindings::new();
	descriptors.bind_buffer(builtin_position_slot(), &mut position);
	executable
		.run_main(&mut descriptors)
		.expect("Expected structural vertex execution");

	assert_eq!(
		position
			.read(besl::STRUCTURAL_POSITION_OUTPUT)
			.expect("Expected structural position value"),
		Value::Vec4F([1.0, 2.0, 3.0, 1.0])
	);
}

#[test]
fn maximum_resource_slot_does_not_alias_push_constants() {
	let mut root = Node::root();
	let f32_type = root.get_child("f32").expect("Expected f32 type");
	root.add_children(vec![
		Node::binding(
			"descriptor",
			BindingTypes::Buffer {
				members: vec![Node::member("value", f32_type.clone()).into()],
			},
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

	let descriptor_slot = ResourceSlot::new(u32::MAX);
	let result_slot = ResourceSlot::new(0);
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
fn resource_using_dynamic_handle_number_remains_a_real_resource() {
	let dynamic_resource_slot = u32::MAX - 4;
	let texture_slot = ResourceSlot::new(dynamic_resource_slot);
	let result_slot = ResourceSlot::new(0);
	let mut root = Node::root();
	let vec4f_type = root.get_child("vec4f").expect("Expected vec4f type");
	root.add_children(vec![
		Node::binding(
			"source",
			BindingTypes::CombinedImageSampler { format: String::new() },
			dynamic_resource_slot,
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
fn runtime_buffer_index_selects_an_array_texture_layer_with_the_bound_sampler() {
	let program = compile_to_besl(
		r#"
		Instance: struct { position: vec3f, sprite_id: u32 }
		sprites: descriptor<{ type: Texture2DArray, binding: 0, access: read }>;
		instances: descriptor<{ type: Instance[], binding: 1, access: read }>;
		main: fn () -> output { color: vec4f } {
			let color: vec4f = sample(sprites[instances[1].sprite_id], vec2f(0.5, 0.5));
			return { color };
		}
		"#,
		None,
	)
	.expect("Expected runtime buffer and layered texture source to link");
	let executable = ExecutableProgram::compile(program).expect("Expected runtime buffer and layered texture compilation");

	let instances_slot = ResourceSlot::new(1);
	let mut instances = Buffer::new_array(
		executable
			.buffer_layout(instances_slot)
			.expect("Expected runtime buffer element layout")
			.clone(),
		2,
	)
	.expect("Expected two runtime buffer elements");
	instances
		.write_array_member(1, "sprite_id", Value::U32(1))
		.expect("Expected layer selection write");

	let mut sprites = Texture::new_3d(2, 2, 2).expect("Expected two texture-array layers");
	for (coord, red) in [([0, 0, 1], 1.0), ([1, 0, 1], 3.0), ([0, 1, 1], 5.0), ([1, 1, 1], 7.0)] {
		sprites
			.write_3d(coord, [red, 0.0, 0.0, 1.0])
			.expect("Expected layered texel write");
	}
	let mut output = Buffer::new(executable.output_layout(0).expect("Expected color output").clone());
	let mut descriptors = DescriptorBindings::new();
	descriptors.bind_texture_with_sampler(ResourceSlot::new(0), &mut sprites, Sampler::new(SamplerReductionMode::Max));
	descriptors.bind_buffer(instances_slot, &mut instances);
	descriptors.bind_buffer(output_slot(0), &mut output);
	executable
		.run_main(&mut descriptors)
		.expect("Expected runtime array layer sampling");

	assert_eq!(
		output.read("_besl_output_color").expect("Expected sampled color"),
		Value::Vec4F([7.0, 0.0, 0.0, 1.0])
	);
}

#[test]
fn runtime_buffer_bounds_follow_the_bound_byte_length() {
	let program = compile_to_besl(
		r#"
		Instance: struct { sprite_id: u32 }
		instances: descriptor<{ type: Instance[], binding: 0, access: read }>;
		main: fn () -> output { value: u32 } {
			let value: u32 = instances[2].sprite_id;
			return { value };
		}
		"#,
		None,
	)
	.expect("Expected runtime buffer source to link");
	let executable = ExecutableProgram::compile(program).expect("Expected runtime buffer compilation");
	let instances_slot = ResourceSlot::new(0);
	let mut instances = Buffer::new_array(
		executable
			.buffer_layout(instances_slot)
			.expect("Expected runtime buffer layout")
			.clone(),
		2,
	)
	.expect("Expected two runtime buffer elements");
	let mut output = Buffer::new(executable.output_layout(0).expect("Expected value output").clone());
	let error = {
		let mut descriptors = DescriptorBindings::new();
		descriptors.bind_buffer(instances_slot, &mut instances);
		descriptors.bind_buffer(output_slot(0), &mut output);
		executable
			.run_main(&mut descriptors)
			.expect_err("Expected bound byte-length validation")
	};

	assert_eq!(error, VmError::BufferArrayIndexOutOfBounds { index: 2, count: 2 });
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
			true,
			false,
		)
		.into(),
		Node::binding(
			"result",
			BindingTypes::Buffer {
				members: vec![Node::member("value", u32_type).into()],
			},
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
			.buffer_layout(ResourceSlot::new(0))
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
			true,
			false,
		)
		.into(),
		Node::binding(
			"result",
			BindingTypes::Buffer {
				members: vec![Node::member("value", u32_type).into()],
			},
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
			true,
			false,
		)
		.into(),
		Node::binding(
			"result",
			BindingTypes::Buffer {
				members: vec![Node::member("value", u32_type).into()],
			},
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
			true,
			false,
		)
		.into(),
		Node::binding(
			"result",
			BindingTypes::Buffer {
				members: vec![Node::member("value", u32_type).into()],
			},
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
