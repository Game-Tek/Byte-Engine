use std::fmt::Write as _;

use colored::Colorize;

/// Formats `glslc` output for `shader_name`: each error that names a line shows the source lines around it with its
/// messages on that line, then each error that names none, such as a link error, as the compiler wrote it. Returns the
/// output unchanged when it holds no error the formatter recognizes, so no message is lost.
pub fn pretty_format_glslang_error_string(error_string: &str, shader_name: &str, source_code: &str) -> String {
	// How many lines to show before and after each error line.
	const WINDOW: usize = 2;

	let mut unlocated = Vec::new();
	// The messages of each reported line, in line order.
	let mut lines = std::collections::BTreeMap::<usize, Vec<&str>>::new();
	for error in error_string.lines().filter(|error| error.starts_with(shader_name)) {
		// `glslc` can include additional `:` characters (for example a trailing `:`), so split only the fixed prefix
		// fields and keep the remainder as the message. Link errors name no line, so they are kept as written.
		let split = error.splitn(5, ':').map(str::trim).collect::<Vec<_>>();
		match (split.len() == 5)
			.then(|| split[1].parse::<usize>().ok()?.checked_sub(1))
			.flatten()
		{
			Some(line_number) => lines
				.entry(line_number)
				.or_default()
				.push(split[4].trim_end_matches(':').trim()),
			None => unlocated.push(error),
		}
	}
	if lines.is_empty() && unlocated.is_empty() {
		return error_string.to_string();
	}

	let source_lines = source_code.lines().collect::<Vec<_>>();
	let mut formatted = String::new();
	for (line_number, errors) in lines {
		let errors = errors.join(", ");
		let window = source_lines
			.iter()
			.enumerate()
			.skip(line_number.saturating_sub(WINDOW))
			.take(2 * WINDOW + 1);
		for (index, line) in window {
			if index == line_number {
				let _ = writeln!(
					formatted,
					"{}| {} {} {}",
					index + 1,
					line.bold(),
					"←".red().bold(),
					errors.red()
				);
			} else {
				let _ = writeln!(formatted, "{}| {}", index + 1, line.dimmed());
			}
		}
		// The compiler can report a line past the end of the source, such as a missing closing brace.
		if line_number >= source_lines.len() {
			let _ = writeln!(formatted, "{}| {}", line_number + 1, errors.red());
		}
	}
	for message in unlocated {
		let _ = writeln!(formatted, "{}", message.red());
	}
	formatted
}

#[cfg(test)]
mod tests {
	use super::*;

	/// Verifies each error line shows the lines around it and every message glslang reported for it, including
	/// messages that contain `:`.
	#[test]
	fn glslang_errors_show_their_line_and_messages() {
		colored::control::set_override(false);
		let source_code = "#version 450\nlayout(local_size_x = 1) in;\nvoid broken() {\n\tx = 1;\n}\nvoid main() {}\n";
		let error_string = "shader:4: error: 'x' : undeclared identifier\nshader:4: error: '>=' : can't read from writeonly object:\n1 error generated.\n";

		let formatted = pretty_format_glslang_error_string(error_string, "shader", source_code);

		assert_eq!(
			formatted,
			"2| layout(local_size_x = 1) in;\n3| void broken() {\n4| \tx = 1; ← undeclared identifier, can't read from writeonly object\n5| }\n6| void main() {}\n"
		);
	}

	/// Verifies an error that names no line of the shader, such as a link error, keeps the compiler's text, also next
	/// to errors that name a line.
	#[test]
	fn glslang_errors_without_a_line_are_kept() {
		colored::control::set_override(false);
		let link_error = "shader: error: Linking compute stage: Missing entry point: Each stage requires one entry point";

		assert_eq!(
			pretty_format_glslang_error_string(&format!("{link_error}\n"), "shader", "#version 450\n"),
			format!("{link_error}\n")
		);
		assert_eq!(
			pretty_format_glslang_error_string(
				&format!("shader:1: error: 'version' : bad profile\n{link_error}\n"),
				"shader",
				"#version 450\n"
			),
			format!("1| #version 450 ← bad profile\n{link_error}\n")
		);
	}
}
