use besl::lexer::LexError;
use besl::vm::{Buffer, DescriptorBindings, ExecutableProgram, ResourceSlot, Value};
use besl::{CompilationError, NodeReference, Nodes, compile_to_besl};

/// A layer whose one buffer the tests write their result to, and the signature of a compute shader that takes it.
const RESULTS: &str = r#"
	Results: layer {
		values: descriptor<{ type: f32[], binding: 0, access: write }>;
	}

	Compute: fn (input: StageInput, results: Results) -> void;
"#;

/// Links `source` after [`RESULTS`].
fn linked(source: &str) -> Result<NodeReference, CompilationError> {
	compile_to_besl(&format!("{RESULTS}\n{source}"), None)
}

/// Returns the message of the rule violation `result` reports.
fn violation(result: Result<NodeReference, CompilationError>) -> String {
	match result.expect_err("Expected the program to be rejected") {
		CompilationError::Lex(LexError::Invalid { message }) => message,
		error => panic!("Expected a rule violation, got {error:?}"),
	}
}

/// Runs `program`'s `main` and returns the first element of the result buffer at binding 0.
fn run(program: NodeReference) -> Value {
	let executable = ExecutableProgram::compile(program).expect("Expected the program to compile");
	let layout = executable
		.buffer_layout(ResourceSlot::new(0))
		.expect("Expected a result buffer layout")
		.clone();
	let mut results = Buffer::new_array(layout, 1).expect("Expected buffer storage");
	{
		let mut descriptors = DescriptorBindings::new();
		descriptors.bind_buffer(ResourceSlot::new(0), &mut results);
		executable.run_main(&mut descriptors).expect("Expected main to run");
	}
	results.read_array_element(0).expect("Expected a result")
}

/// Returns the parameter names of the function `name` in `program`.
fn parameters(program: &NodeReference, name: &str) -> Vec<String> {
	let function = program.borrow().get_child(name).expect("Expected the function");
	let function = function.borrow();
	let Nodes::Function { params, .. } = function.node() else {
		panic!("Expected `{name}` to be a function");
	};
	params
		.iter()
		.filter_map(|parameter| parameter.borrow().get_name().map(str::to_string))
		.collect()
}

#[test]
fn main_implementing_a_signature_reaches_its_layer_members() {
	let program = linked("main: Compute { results.values[0] = 4.0; }").expect("Expected the program to link");

	assert_eq!(run(program), Value::F32(4.0));
}

#[test]
fn a_helper_takes_a_layer_that_generated_code_no_longer_passes() {
	let program = linked(
		"store: fn (results: Results, value: f32) -> void { results.values[0] = value; }\nmain: Compute { store(results, 5.0); }",
	)
	.expect("Expected the program to link");

	assert_eq!(parameters(&program, "store"), ["value"]);
	assert!(parameters(&program, "main").is_empty());
	assert_eq!(run(program), Value::F32(5.0));
}

#[test]
fn combined_signatures_take_each_parameter_once() {
	let program = linked(
		r#"
		Scale: layer { factors: descriptor<{ type: f32[], binding: 0, access: read }>; }
		Scaled: fn (input: StageInput, scale: Scale);
		Shader: Compute + Scaled;
		main: Shader { results.values[0] = 2.0 + f32(input.thread_id.x); }
		"#,
	)
	.expect("Expected the program to link");

	assert_eq!(run(program), Value::F32(2.0));
}

#[test]
fn combined_signatures_reject_a_parameter_with_two_types() {
	let message = violation(linked(
		"Other: fn (results: u32);\nShader: Compute + Other;\nmain: Shader { }",
	));

	assert!(
		message.contains("parameter `results` as both `Results` and `u32`"),
		"{message}"
	);
}

#[test]
fn combined_signatures_reject_two_return_types() {
	let message = violation(linked(
		"A: fn () -> f32;\nB: fn () -> u32;\nShader: A + B;\nhelper: Shader { return 1.0; }\nmain: Compute { }",
	));

	assert!(message.contains("return both `f32` and `u32`"), "{message}");
}

#[test]
fn an_implementation_must_name_a_signature() {
	let message = violation(linked("main: Results { }"));

	assert!(message.contains("`Results` isn't a signature"), "{message}");
}

#[test]
fn a_layer_member_is_reached_only_through_a_layer_value() {
	let error = linked("main: Compute { values[0] = 1.0; }").expect_err("Expected `values` to stay inside the layer");

	assert!(
		matches!(&error, CompilationError::Lex(LexError::UndeclaredName { name }) if name == "values"),
		"{error:?}"
	);
}

#[test]
fn a_shader_takes_at_most_one_push_constant_block() {
	let layers = r#"
		Exposure: layer { constants: push_constant { scale: f32 }; }
		Frame: layer { constants: push_constant { index: u32 }; }
	"#;

	let one = format!(
		"{layers}\nShader: Compute + fn (exposure: Exposure);\nmain: Shader {{ results.values[0] = exposure.constants.scale; }}"
	);
	linked(&one).expect("Expected one push constant block to link");

	let two = format!("{layers}\nShader: Compute + fn (exposure: Exposure, frame: Frame);\nmain: Shader {{ }}");
	let message = violation(linked(&two));
	assert!(message.contains("two push constant blocks"), "{message}");
}

#[test]
fn a_layer_holds_only_descriptors_a_push_constant_and_tile_targets() {
	compile_to_besl("Targets: layer { color: output<vec4f, 0>; }\nmain: fn () -> void { }", None)
		.expect("Expected tile targets in a layer to link");

	let error = compile_to_besl("Bad: layer { value: const f32 = 1.0; }", None).expect_err("Expected the layer to be rejected");
	assert!(
		format!("{error:?}").contains("Invalid member `value` of layer Bad"),
		"{error:?}"
	);
}

#[test]
fn layers_and_signatures_write_back_as_source() {
	let source = "Results: layer {\n\tvalues: descriptor<{ type: f32[], binding: 0, access: write }>;\n\tconstants: push_constant {\n\t\tscale: f32,\n\t}\n}\nCompute: fn (results: Results) -> void + Other;\nmain: Compute {\n}\n";

	let written = besl::write(&besl::parse(source).unwrap());

	assert_eq!(written, besl::write(&besl::parse(&written).unwrap()));
	assert!(
		written.contains("Results: layer {") && written.contains("constants: push_constant {"),
		"{written}"
	);
	assert!(
		written.contains("Compute: fn (results: Results) -> void + Other;"),
		"{written}"
	);
	assert!(written.contains("main: Compute {"), "{written}");
}
