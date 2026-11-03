use besl::lexer::LexError;
use besl::vm::{Buffer, DescriptorBindings, ExecutableProgram, ResourceSlot, Value};
use besl::{CompilationError, LoadedModule, ModuleLoader, ModuleQuery, NodeReference, attach_modules, lex, parse};

/// Finds the module files `root` needs in `files`, given as path and source pairs relative to the root's directory,
/// the way a host that reads files does. Paths from the asset root are looked up under `assets/`.
pub fn modules(root: &str, files: &[(&str, &str)]) -> Result<Vec<LoadedModule>, CompilationError> {
	let mut loader = ModuleLoader::new(root)?;
	while let Some(ModuleQuery {
		files: candidates,
		from_root,
	}) = loader.query()
	{
		let read = |candidate: &String| {
			let candidate = if from_root {
				format!("assets/{candidate}")
			} else {
				candidate.clone()
			};
			files
				.iter()
				.find(|(name, _)| *name == candidate)
				.map(|(_, source)| source.to_string())
		};
		loader.answer([read(&candidates[0]), read(&candidates[1])])?;
	}
	Ok(loader.into_modules())
}

/// Parses `root` and attaches its module files from `files`. The modules are leaked so the program can borrow them.
fn load(root: &'static str, files: &[(&str, &str)]) -> Result<besl::ParserNode<'static>, CompilationError> {
	let modules: &'static [LoadedModule] = Vec::leak(modules(root, files)?);
	let mut program = parse(root)?;
	attach_modules(&mut program, modules)?;
	Ok(program)
}

/// Loads and links `root` with its module files.
fn linked(root: &'static str, files: &[(&str, &str)]) -> Result<NodeReference, CompilationError> {
	lex(load(root, files)?)
}

/// Returns the message of the rule violation `result` reports.
fn violation<T: std::fmt::Debug>(result: Result<T, CompilationError>) -> String {
	match result.expect_err("Expected the program to be rejected") {
		CompilationError::Lex(LexError::Invalid { message }) => message,
		CompilationError::Parsing(besl::parser::ParsingFailReasons::BadSyntax { message }) => message,
		error => panic!("Expected a module rule violation, got {error:?}"),
	}
}

/// Runs `program`'s `main` and returns the first element it writes to its `f32` result buffer at binding 0 of its
/// first descriptor set.
fn run(program: NodeReference) -> Value {
	run_in_set(program, 0)
}

/// Runs `program`'s `main` and returns the first element it writes to its `f32` result buffer at binding 0 of the
/// descriptor set at position `set`.
fn run_in_set(program: NodeReference, set: u32) -> Value {
	let slot = ResourceSlot::in_set(set, 0);
	let executable = ExecutableProgram::compile(program).expect("Expected the program to compile");
	let layout = executable
		.buffer_layout(slot)
		.expect("Expected a result buffer layout")
		.clone();
	let mut results = Buffer::new_array(layout, 1).expect("Expected buffer storage");
	{
		let mut descriptors = DescriptorBindings::new();
		descriptors.bind_buffer(slot, &mut results);
		executable.run_main(&mut descriptors).expect("Expected main to run");
	}
	results.read_array_element(0).expect("Expected a result")
}

/// Expands to the declaration of the result buffer [`run`] reads, for use in `concat!`.
macro_rules! results {
	() => {
		"results: descriptor<{ type: f32[], binding: 0, access: write }>;"
	};
}

const NOISE: &str = r#"
	SEED: const f32 = 3.0;

	Sample: struct {
		value: f32,
	}

	hash: fn (x: f32) -> f32 {
		return x * SEED;
	}
"#;

#[test]
fn module_files_follow_the_module_path() {
	let mut loader = ModuleLoader::new("lighting: mod { brdf: mod; }").unwrap();
	assert_eq!(
		loader.query(),
		Some(ModuleQuery {
			files: ["lighting/brdf.besl".to_string(), "lighting/brdf/mod.besl".to_string()],
			from_root: false,
		})
	);
}

#[test]
fn modules_from_the_asset_root_load_once_through_their_shortest_file() {
	let root = concat!(
		results!(),
		"use ::shared::math::double;",
		"use ::shared::math::triple;",
		"use ::shared::scale::scale;",
		"main: fn () -> void { results[0] = scale(double(1.0)) + triple(1.0); }"
	);
	// `scale` names `math` too, so the program holds `math` once and links it first.
	let files = [
		(
			"assets/shared/math.besl",
			"double: fn (x: f32) -> f32 { return x * 2.0; }\ntriple: fn (x: f32) -> f32 { return x * 3.0; }",
		),
		(
			"assets/shared/scale/mod.besl",
			"use ::shared::math::double;\nscale: fn (x: f32) -> f32 { return double(x); }",
		),
	];

	let loaded = modules(root, &files).unwrap();
	assert_eq!(
		loaded.iter().map(|module| module.file.as_str()).collect::<Vec<_>>(),
		["shared/math.besl", "shared/scale/mod.besl"]
	);
	assert_eq!(run(linked(root, &files).unwrap()), Value::F32(7.0));
}

#[test]
fn paths_reach_a_file_module_declarations() {
	let root = concat!(
		results!(),
		"noise: mod;",
		"main: fn () -> void { let s: noise::Sample = noise::Sample(noise::hash(2.0)); results[0] = s.value + noise::SEED; }"
	);

	let program = linked(root, &[("noise.besl", NOISE)]).expect("Expected the program to link");

	assert_eq!(run(program), Value::F32(9.0));
}

#[test]
fn use_brings_in_one_declaration_or_a_whole_module() {
	let root = concat!(
		results!(),
		"noise: mod;",
		"use noise::hash;",
		"main: fn () -> void { results[0] = hash(1.0); }"
	);
	assert_eq!(run(linked(root, &[("noise.besl", NOISE)]).unwrap()), Value::F32(3.0));

	let root = concat!(
		results!(),
		"noise: mod;",
		"use noise::*;",
		"main: fn () -> void { let s: Sample = Sample(SEED); results[0] = hash(s.value); }"
	);
	assert_eq!(run(linked(root, &[("noise/mod.besl", NOISE)]).unwrap()), Value::F32(9.0));
}

#[test]
fn nested_modules_find_their_files_through_inline_parents() {
	let root = concat!(
		results!(),
		"BASE: const f32 = 2.0;",
		"lighting: mod { brdf: mod; }",
		"main: fn () -> void { results[0] = lighting::brdf::ggx(5.0); }"
	);
	// `crate::`, `super::`, and `self::` all reach the same declarations from inside the nested module.
	let brdf = r#"
		scale: fn (x: f32) -> f32 { return x * super::super::BASE; }
		ggx: fn (x: f32) -> f32 { return self::scale(x) + crate::BASE; }
	"#;

	let program = linked(root, &[("lighting/brdf.besl", brdf)]).expect("Expected the program to link");

	assert_eq!(run(program), Value::F32(12.0));
}

#[test]
fn a_module_sees_the_declarations_of_the_modules_around_it() {
	let root = concat!(
		results!(),
		"BASE: const f32 = 4.0;",
		"shift: mod { offset: fn (x: f32) -> vec2f { return vec2f(x + BASE, 0.0); } }",
		"main: fn () -> void { results[0] = shift::offset(1.0).x; }"
	);

	assert_eq!(run(linked(root, &[]).unwrap()), Value::F32(5.0));
}

#[test]
fn a_module_declaration_is_not_visible_by_its_bare_name_outside_the_module() {
	let root = concat!(results!(), "noise: mod;", "main: fn () -> void { results[0] = hash(1.0); }");

	let error = linked(root, &[("noise.besl", NOISE)]).expect_err("Expected `hash` to stay inside `noise`");

	assert!(
		matches!(&error, CompilationError::Lex(LexError::UndeclaredName { name }) if name == "hash"),
		"{error:?}"
	);
}

#[test]
fn two_modules_can_not_declare_the_same_name() {
	let root = "a: mod { helper: fn () -> f32 { return 1.0; } } b: mod { helper: fn () -> f32 { return 2.0; } } main: fn () -> void {}";

	let message = violation(linked(root, &[]));

	assert!(message.contains("`helper`"), "{message}");
	assert!(message.contains("module `a`") && message.contains("module `b`"), "{message}");
}

#[test]
fn a_module_without_its_file_does_not_link() {
	let message = violation(lex(parse("noise: mod; main: fn () -> void {}").unwrap()));

	assert!(message.contains("Module `noise` has no source"), "{message}");
}

#[test]
fn only_the_shader_file_declares_main() {
	let file = violation(load("noise: mod;", &[("noise.besl", "main: fn () -> void {}")]));
	assert!(file.contains("Module `noise` declares `main`"), "{file}");

	let inline = violation(parse("a: mod { main: fn () -> void {} }"));
	assert!(inline.contains("Module `a` declares `main`"), "{inline}");
}

#[test]
fn a_module_descriptor_belongs_to_the_shader_that_declares_the_module() {
	let root = concat!("output: mod;", "main: fn () -> void { output::results[0] = 6.0; }");

	let program = linked(root, &[("output.besl", results!())]).expect("Expected the program to link");

	assert_eq!(run(program), Value::F32(6.0));
}

#[test]
fn modules_can_overload_a_function_name_with_other_parameter_types() {
	let root = concat!(
		results!(),
		"a: mod { pick: fn (x: f32) -> f32 { return x; } }",
		"b: mod { pick: fn (x: u32) -> f32 { return 2.0; } }",
		"main: fn () -> void { results[0] = a::pick(1.0) + b::pick(1); }"
	);

	assert_eq!(run(linked(root, &[]).unwrap()), Value::F32(3.0));
}

#[test]
fn declarations_and_modules_can_not_take_reserved_or_path_names() {
	assert!(parse("noise::hash: fn () -> void {}").is_err());
	assert!(parse("crate: mod;").is_err());
	assert!(parse("use: fn () -> void {}").is_err());
}

#[test]
fn modules_and_use_declarations_write_back_as_source() {
	let source = "noise: mod;\nuse noise::*;\nlighting: mod {\n\tuse noise::hash;\n}\n";

	let written = besl::write(&parse(source).unwrap());

	assert_eq!(written, besl::write(&parse(&written).unwrap()));
	assert!(
		written.contains("noise: mod;") && written.contains("use noise::*;"),
		"{written}"
	);
	assert!(
		written.contains("lighting: mod {") && written.contains("use noise::hash;"),
		"{written}"
	);
}

#[test]
fn a_program_with_modules_from_the_asset_root_writes_back_as_one_file() {
	let root = concat!(
		results!(),
		"use ::shared::math::double;",
		"main: fn () -> void { results[0] = double(2.0); }"
	);
	let files = [("assets/shared/math.besl", "double: fn (x: f32) -> f32 { return x * 2.0; }")];

	// The written program holds the module inline and needs no files to link again.
	let written = besl::write(&load(root, &files).unwrap());
	let reparsed: &'static str = written.clone().leak();

	assert!(written.contains("::shared::math: mod {"), "{written}");
	assert_eq!(run(lex(parse(reparsed).unwrap()).unwrap()), Value::F32(4.0));
}

#[test]
fn a_shader_implements_a_signature_composed_across_shared_modules() {
	// Each module declares a layer and the part of the signature that takes it. The shader names only the combined
	// signature, and its parameter types resolve in the modules that declare them.
	let files = [
		(
			"assets/shared/scene.besl",
			"Scene: layer { factors: descriptor<{ type: f32[], binding: 0, access: read }>; }\nSceneShader: fn (input: StageInput, scene: Scene) -> void;",
		),
		(
			"assets/shared/evaluation.besl",
			"use ::shared::scene::*;\nEvaluation: layer { results: descriptor<{ type: f32[], binding: 0, access: write }>; }\nEvaluationShader: SceneShader + fn (evaluation: Evaluation);",
		),
	];
	let root = "use ::shared::evaluation::EvaluationShader;\nmain: EvaluationShader { evaluation.results[0] = 3.0 + f32(input.thread_id.x); }";

	let program = linked(root, &files).expect("Expected the shader to link");

	// The scene's set comes first, as `SceneShader` takes it first, so the results are in the second set.
	assert_eq!(run_in_set(program, 1), Value::F32(3.0));
}
