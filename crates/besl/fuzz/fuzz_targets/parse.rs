#![no_main]

use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
	// BESL accepts source text, so reject invalid UTF-8 without allocating a lossy copy.
	let Ok(source) = std::str::from_utf8(data) else {
		return;
	};

	// Syntax errors are expected; successful parses must retain the public root invariant.
	if let Ok(root) = besl::parse(source) {
		assert_eq!(root.name(), Some("root"));
		assert!(matches!(root.node(), besl::parser::Nodes::Scope { .. }));
	}
});
