use besl::lexer::LexError;
use besl::vm::{Buffer, DescriptorBindings, ExecutableProgram, ResourceSlot, Value};
use besl::{CompilationError, lex, parse, stack_layers};

/// A layer that declares one shared record and the buffer that stores it.
const SCENE_LAYER: &str = r#"
	Instance: struct {
		scale: f32,
		offset: f32,
	}

	instances: descriptor<{ type: Instance[], binding: 0, access: read }>;
"#;

/// A layer that builds on [`SCENE_LAYER`]: its helper reads the scene layer's buffer and record.
const RESULT_LAYER: &str = r#"
	results: descriptor<{ type: f32[], binding: 1, access: write }>;

	transformed: fn (index: u32, value: f32) -> f32 {
		let instance: Instance = instances[index];
		return value * instance.scale + instance.offset;
	}
"#;

/// Returns the link error message of a module stacked on `layers`.
fn stacking_error(module: &'static str, layers: &[(&'static str, &'static str)]) -> String {
	let error = stack_layers(parse(module).expect("Expected the module to parse"), layers.iter().copied())
		.expect_err("Expected stacking to reject the layers");

	match error {
		CompilationError::Lex(LexError::Invalid { message }) => message,
		error => panic!("Expected a layer rule violation, got {error:?}"),
	}
}

/// Parses `module` and stacks it on `layers`.
fn stacked(module: &'static str, layers: [(&'static str, &'static str); 2]) -> besl::ParserNode<'static> {
	stack_layers(parse(module).expect("Expected the module to parse"), layers).expect("Expected the layers to stack")
}

#[test]
fn a_module_reads_declarations_from_every_stacked_layer() {
	let module = stacked(
		"main: fn () -> void { results[0] = transformed(1, 3.0); }",
		[("scene", SCENE_LAYER), ("result", RESULT_LAYER)],
	);

	let program = lex(module).expect("Expected the module to link");
	let executable = ExecutableProgram::compile(program).expect("Expected the layered module to compile");

	let array = |slot: u32, count: usize| {
		Buffer::new_array(
			executable
				.buffer_layout(ResourceSlot::new(slot))
				.expect("Expected a layer buffer layout")
				.clone(),
			count,
		)
		.expect("Expected buffer storage")
	};
	let mut instances = array(0, 2);
	let mut results = array(1, 1);
	for (member, value) in [("scale", 2.0), ("offset", 1.0)] {
		instances
			.write_array_member(1, member, Value::F32(value))
			.expect("Expected instance write");
	}

	{
		let mut descriptors = DescriptorBindings::new();
		descriptors.bind_buffer(ResourceSlot::new(0), &mut instances);
		descriptors.bind_buffer(ResourceSlot::new(1), &mut results);
		executable
			.run_main(&mut descriptors)
			.expect("Expected layered module execution");
	}

	assert_eq!(results.read_array_element(0), Ok(Value::F32(7.0)));
}

#[test]
fn a_layer_cannot_reference_a_layer_stacked_after_it() {
	let module = stacked(
		"main: fn () -> void { results[0] = transformed(0, 1.0); }",
		[("result", RESULT_LAYER), ("scene", SCENE_LAYER)],
	);

	assert!(lex(module).is_err());
}

#[test]
fn stacking_rejects_a_name_declared_twice() {
	let message = stacking_error(
		"main: fn () -> void {}",
		&[
			("scene", SCENE_LAYER),
			("copy", "instances: descriptor<{ type: f32[], binding: 4, access: read }>;"),
		],
	);
	assert!(message.contains("`copy` redeclares `instances`") && message.contains("`scene`"));

	let message = stacking_error(
		"Instance: struct { scale: f32 }\nmain: fn () -> void {}",
		&[("scene", SCENE_LAYER)],
	);
	assert!(message.contains("shader redeclares `Instance`") && message.contains("`scene`"));

	let message = stacking_error(
		"push_constant: push_constant { index: u32 }\nmain: fn () -> void {}",
		&[("draw", "push_constant: push_constant { index: u32 }")],
	);
	assert!(message.contains("shader redeclares `push_constant`"));
}

#[test]
fn stacking_rejects_a_layer_that_is_not_a_set_of_shared_declarations() {
	let message = stacking_error("main: fn () -> void {}", &[("shader", "main: fn () -> void {}")]);
	assert!(message.contains("`shader` declares `main`"));

	let message = stacking_error("main: fn () -> void {}", &[("broken", "Frame: struct {")]);
	assert!(message.contains("`broken` did not parse"));
}

#[test]
fn a_layer_names_the_layout_of_the_descriptors_it_declares() {
	let module = stacked(
		r#"
			scratch: descriptor<{ type: f32[], binding: 2, access: write }>;
			main: fn () -> void { scratch[0] = transformed(1, 3.0); }
		"#,
		[("scene", SCENE_LAYER), ("result", RESULT_LAYER)],
	);

	let program = lex(module).expect("Expected the module to link");

	let layout = |name: &str| {
		let binding = program.get_descendant(name).expect("Expected the descriptor to link");
		let binding = binding.borrow();
		match binding.node() {
			besl::Nodes::Binding { layout, .. } => layout.clone(),
			_ => panic!("Expected `{name}` to be a binding"),
		}
	};

	// Each layer is one descriptor set layout, and the module's own descriptors form its own.
	assert_eq!(layout("instances").as_deref(), Some("scene"));
	assert_eq!(layout("results").as_deref(), Some("result"));
	assert_eq!(layout("scratch"), None);
}
