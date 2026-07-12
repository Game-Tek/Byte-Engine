#![no_main]

use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
	let source = byte_engine_besl_fuzz::generate_program(data);
	let program = besl::compile_to_besl(&source, None)
		.unwrap_or_else(|error| panic!("Grammar-generated BESL failed source compilation: {error:?}\n\n{source}"));

	besl::vm::ExecutableProgram::compile(program)
		.unwrap_or_else(|error| panic!("Grammar-generated BESL failed VM compilation: {error}\n\n{source}"));
});
