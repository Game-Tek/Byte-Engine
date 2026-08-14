//! Run with `cargo bench -p byte-engine-besl --bench vm`.

use besl::{
	compile_to_besl,
	vm::{Buffer, DescriptorBindings, ExecutableProgram, ResourceSlot},
	BindingTypes, Node,
};
use divan::{counter::ItemsCount, Bencher};

const ARITHMETIC_SHADER: &str = r#"
main: fn () -> void {
	let first: f32 = 1.25;
	let second: f32 = 3.5;
	let product: f32 = first * second;
	let normalized: f32 = (product + second) / (first + 1.0);
	output.value = normalized * normalized - first;
}
"#;

const CALLS_SHADER: &str = r#"
add: fn (lhs: f32, rhs: f32) -> f32 {
	return lhs + rhs;
}

main: fn () -> void {
	output.value = add(2.0, 1.5);
	output.value = add(0.75, 0.5);
}
"#;

fn main() {
	divan::main();
}

/// Compiles one shader and creates the output storage that remains bound while Divan measures execution.
fn setup(script: &str) -> (ExecutableProgram, Buffer) {
	let mut root = Node::root();
	let float_type = root.get_child("f32").expect("The standard BESL scope defines f32");
	root.add_child(
		Node::binding(
			"output",
			BindingTypes::Buffer {
				members: vec![Node::member("value", float_type).into()],
			},
			0,
			true,
			true,
		)
		.into(),
	);

	let program = compile_to_besl(script, Some(root)).expect("The benchmark shader must lex successfully");
	let executable = ExecutableProgram::compile(program).expect("The benchmark shader must compile for the VM");
	let layout = executable
		.buffer_layout(ResourceSlot::new(0))
		.expect("The benchmark shader output buffer must have a VM layout")
		.clone();
	(executable, Buffer::new(layout))
}

/// Measures repeated main-frame execution after compilation, descriptor binding, and frame storage setup.
fn benchmark(bencher: Bencher, script: &str) {
	let (executable, mut output) = setup(script);
	let mut descriptors = DescriptorBindings::new();
	descriptors.bind_buffer(ResourceSlot::new(0), &mut output);

	bencher.counter(ItemsCount::new(1_usize)).bench_local(|| {
		divan::black_box(&executable)
			.run_main(&mut descriptors)
			.expect("The benchmark shader must execute successfully");
	});
}

/// Measures arithmetic, comparison-independent VM instructions, and one output-buffer store.
#[divan::bench]
fn arithmetic(bencher: Bencher) {
	benchmark(bencher, ARITHMETIC_SHADER);
}

/// Measures nested VM frame creation and return-value propagation.
#[divan::bench]
fn nested_calls(bencher: Bencher) {
	benchmark(bencher, CALLS_SHADER);
}
