use besl::lexer::LexError;
use besl::vm::{Buffer, DescriptorBindings, ExecutableProgram, ResourceSlot, Value};
use besl::{CompilationError, Nodes, Program, compile_to_besl};

/// A layer whose one buffer the tests write their result to, and the signature of a compute shader that takes it.
const RESULTS: &str = r#"
	Results: layer {
		values: descriptor<{ type: f32[], binding: 0, access: write }>;
	}

	Compute: fn (input: StageInput, results: Results) -> void;
"#;

/// Links `source` after [`RESULTS`].
fn linked<'b>(builtins: &'b besl::Builtins, source: &str) -> Result<Program<'b>, CompilationError> {
	compile_to_besl(&format!("{RESULTS}\n{source}"), besl::Program::new(builtins))
}

/// Returns the message of the rule violation `result` reports.
fn violation(result: Result<Program, CompilationError>) -> String {
	match result.expect_err("Expected the program to be rejected") {
		CompilationError::Lex(LexError::Invalid { message }) => message,
		error => panic!("Expected a rule violation, got {error:?}"),
	}
}

/// Runs `program`'s `main` and returns the first element of the result buffer at binding 0.
fn run(program: Program) -> Value {
	let executable = ExecutableProgram::compile(&program).expect("Expected the program to compile");
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
fn parameters(program: &Program, name: &str) -> Vec<String> {
	let function = program.get_child(program.root(), name).expect("Expected the function");
	let Nodes::Function { params, .. } = program[function].node() else {
		panic!("Expected `{name}` to be a function");
	};
	params
		.iter()
		.filter_map(|parameter| program.name(*parameter).map(str::to_string))
		.collect()
}

#[test]
fn main_implementing_a_signature_reaches_its_layer_members() {
	let builtins = besl::Builtins::new();
	let program = linked(&builtins, "main: Compute { results.values[0] = 4.0; }").expect("Expected the program to link");

	assert_eq!(run(program), Value::F32(4.0));
}

#[test]
fn a_helper_takes_a_layer_that_generated_code_no_longer_passes() {
	let builtins = besl::Builtins::new();
	let program = linked(
		&builtins,
		"store: fn (results: Results, value: f32) -> void { results.values[0] = value; }\nmain: Compute { store(results, 5.0); }",
	)
	.expect("Expected the program to link");

	assert_eq!(parameters(&program, "store"), ["value"]);
	assert!(parameters(&program, "main").is_empty());
	assert_eq!(run(program), Value::F32(5.0));
}

#[test]
fn combined_signatures_take_each_parameter_once() {
	let builtins = besl::Builtins::new();
	let program = linked(
		&builtins,
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
	let builtins = besl::Builtins::new();
	let message = violation(linked(
		&builtins,
		"Other: fn (results: u32);\nShader: Compute + Other;\nmain: Shader { }",
	));

	assert!(
		message.contains("parameter `results` as both `Results` and `u32`"),
		"{message}"
	);
}

#[test]
fn combined_signatures_reject_two_return_types() {
	let builtins = besl::Builtins::new();
	let message = violation(linked(
		&builtins,
		"A: fn () -> f32;\nB: fn () -> u32;\nShader: A + B;\nhelper: Shader { return 1.0; }\nmain: Compute { }",
	));

	assert!(message.contains("return both `f32` and `u32`"), "{message}");
}

#[test]
fn an_implementation_must_name_a_signature() {
	let builtins = besl::Builtins::new();
	let message = violation(linked(&builtins, "main: Results { }"));

	assert!(message.contains("`Results` isn't a signature"), "{message}");
}

#[test]
fn a_layer_member_is_reached_only_through_a_layer_value() {
	let builtins = besl::Builtins::new();
	let error =
		linked(&builtins, "main: Compute { values[0] = 1.0; }").expect_err("Expected `values` to stay inside the layer");

	assert!(
		matches!(&error, CompilationError::Lex(LexError::UndeclaredName { name }) if name == "values"),
		"{error:?}"
	);
}

#[test]
fn a_shader_takes_at_most_one_push_constant_block() {
	let builtins = besl::Builtins::new();
	let layers = r#"
		Exposure: layer { constants: push_constant { scale: f32 }; }
		Frame: layer { constants: push_constant { index: u32 }; }
	"#;

	let one = format!(
		"{layers}\nShader: Compute + fn (exposure: Exposure);\nmain: Shader {{ results.values[0] = exposure.constants.scale; }}"
	);
	linked(&builtins, &one).expect("Expected one push constant block to link");

	let two = format!("{layers}\nShader: Compute + fn (exposure: Exposure, frame: Frame);\nmain: Shader {{ }}");
	let message = violation(linked(&builtins, &two));
	assert!(message.contains("two push constant blocks"), "{message}");
}

#[test]
fn a_layer_holds_only_descriptors_a_push_constant_and_tile_targets() {
	let builtins = besl::Builtins::new();
	compile_to_besl(
		"Targets: layer { color: output<vec4f, 0>; }\nmain: fn () -> void { }",
		besl::Program::new(&builtins),
	)
	.expect("Expected tile targets in a layer to link");

	let error = compile_to_besl("Bad: layer { value: const f32 = 1.0; }", besl::Program::new(&builtins))
		.expect_err("Expected the layer to be rejected");
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
