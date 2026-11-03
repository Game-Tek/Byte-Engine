//! Covers how linking places descriptors in a program's descriptor sets.

use besl::lexer::LexError;
use besl::{CompilationError, NodeReference, Nodes, Placement, compile_to_besl};

const SOURCE: &str = r#"
	Data: struct { value: f32, }

	Textures: layer {
		textures: descriptor<{ type: Texture2D, binding: 0, access: read, count: 4 }>;
		data: descriptor<{ type: Data, binding: 1, access: read }>;
	}

	Unused: layer {
		ignored: descriptor<{ type: Data, binding: 0, access: read }>;
	}

	Results: layer {
		results: descriptor<{ type: f32[], binding: 0, access: write }>;
	}

	own: descriptor<{ type: Data, binding: 0, access: read }>;

	main: fn (input: StageInput, results: Results, textures: Textures) -> void {
		results.results[0] = textures.data.value + own.value;
	}
"#;

/// Returns the placement of the descriptor `name`, declared at the program root or in one of its layers.
fn placement(program: &NodeReference, name: &str) -> Option<Placement> {
	fn find(node: &NodeReference, name: &str) -> Option<Option<Placement>> {
		let borrowed = node.borrow();
		match borrowed.node() {
			Nodes::Binding {
				name: binding,
				placement,
				..
			} if binding == name => Some(*placement),
			Nodes::Scope { children, .. } | Nodes::Layer { members: children, .. } => {
				children.iter().find_map(|child| find(child, name))
			}
			_ => None,
		}
	}
	find(program, name).unwrap_or_else(|| panic!("Expected a descriptor named `{name}`"))
}

/// Returns the message of the rule violation linking `source` reports.
fn violation(source: &str) -> String {
	match compile_to_besl(source, None).expect_err("Expected the program to be rejected") {
		CompilationError::Lex(LexError::Invalid { message }) => message,
		error => panic!("Expected a rule violation, got {error:?}"),
	}
}

#[test]
fn layers_take_sets_in_parameter_order_and_the_shaders_own_descriptors_come_last() {
	let program = compile_to_besl(SOURCE, None).expect("Expected the program to link");

	assert_eq!(placement(&program, "results").map(|placement| placement.set), Some(0));
	assert_eq!(placement(&program, "textures").map(|placement| placement.set), Some(1));
	assert_eq!(placement(&program, "own").map(|placement| placement.set), Some(2));
}

#[test]
fn an_array_is_one_binding_whose_descriptors_come_before_the_next_binding() {
	let program = compile_to_besl(SOURCE, None).expect("Expected the program to link");

	assert_eq!(
		placement(&program, "textures"),
		Some(Placement {
			set: 1,
			resource: 0,
			sampler: 0
		})
	);
	// The four textures each take a descriptor and a sampler, so the buffer after them takes the fifth descriptor.
	assert_eq!(
		placement(&program, "data"),
		Some(Placement {
			set: 1,
			resource: 4,
			sampler: 4
		})
	);
}

#[test]
fn a_layer_main_does_not_take_forms_no_set() {
	let program = compile_to_besl(SOURCE, None).expect("Expected the program to link");

	assert_eq!(placement(&program, "ignored"), None);
}

#[test]
fn a_taken_layer_keeps_its_set_when_main_reads_none_of_it() {
	let program = compile_to_besl(
		r#"
		First: layer { first: descriptor<{ type: f32[], binding: 0, access: read }>; }
		Second: layer { second: descriptor<{ type: f32[], binding: 0, access: write }>; }
		main: fn (input: StageInput, first: First, second: Second) -> void {
			second.second[0] = 1.0;
		}
		"#,
		None,
	)
	.expect("Expected the program to link");

	assert_eq!(placement(&program, "second").map(|placement| placement.set), Some(1));
}

#[test]
fn bindings_of_a_set_number_from_zero_without_gaps() {
	let message = violation(
		r#"
		first: descriptor<{ type: f32[], binding: 0, access: read }>;
		third: descriptor<{ type: f32[], binding: 2, access: read }>;
		main: fn () -> void {}
		"#,
	);

	assert!(message.contains("`third`"), "{message}");
	assert!(message.contains("from 0 without gaps or repeats"), "{message}");
}

#[test]
fn bindings_of_a_set_take_distinct_numbers() {
	let message = violation(
		r#"
		first: descriptor<{ type: f32[], binding: 0, access: read }>;
		second: descriptor<{ type: f32[], binding: 0, access: read }>;
		main: fn () -> void {}
		"#,
	);

	assert!(message.contains("from 0 without gaps or repeats"), "{message}");
}
