#![no_main]

use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
	// BESL accepts source text, so reject invalid UTF-8 without allocating a lossy copy.
	let Ok(source) = std::str::from_utf8(data) else {
		return;
	};

	let Ok(program) = besl::compile_to_besl(source, None) else {
		return;
	};

	// Source compilation always produces a scope, even for an empty program.
	{
		let program = program.borrow();
		assert!(matches!(program.node(), besl::Nodes::Scope { .. }));
	}

	// Unsupported VM programs are valid fuzz outcomes; internal panics are not.
	let _ = besl::vm::ExecutableProgram::compile(program);
});
