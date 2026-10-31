use std::fmt::Write as _;

use colored::Colorize;

pub struct LineError<'a> {
	pub column: Option<usize>,
	pub symbol: &'a str,
	pub error: &'a str,
}

pub struct Line<'a> {
	pub line_number: usize,
	pub source_code: &'a str,
	pub errors: Vec<LineError<'a>>,
}

/// Returns the compiler errors reported in `glslc` output.
pub fn process_glslc_error<'a>(shader_name: &str, source_code: &'a str, error_string: &'a str) -> Error<'a> {
	let mut unlocated = Vec::new();
	// Collect (error_line_number, error) pairs
	let errors = error_string
		.lines()
		.filter(|error| error.starts_with(shader_name))
		.filter_map(|error| {
			// `glslc` can include additional `:` characters (for example a trailing `:`).
			// Split only the fixed prefix fields and keep the remainder as the message.
			let split = error.splitn(5, ':').map(|e| e.trim()).collect::<Vec<_>>();
			// Link errors name no line, so they have no number to parse and are kept as the compiler wrote them.
			let line_number = (split.len() == 5)
				.then(|| split[1].parse::<usize>().ok()?.checked_sub(1))
				.flatten();
			let Some(line_number) = line_number else {
				unlocated.push(error);
				return None;
			};
			Some((
				line_number,
				(split[3].trim_matches('\''), split[4].trim_end_matches(':').trim()),
			))
		})
		.collect::<Vec<_>>();

	// Collect errors by line number
	let mut errors_by_line_number = std::collections::HashMap::<usize, Vec<(&'a str, &'a str)>>::new();

	for (error_line_number, error) in errors {
		if let Some(errors) = errors_by_line_number.get_mut(&error_line_number) {
			errors.push(error);
		} else {
			errors_by_line_number.insert(error_line_number, vec![error]);
		}
	}

	// Sort errors by line number
	let mut errors = errors_by_line_number.into_iter().collect::<Vec<_>>();

	errors.sort_by_key(|(line_number_a, _)| *line_number_a);

	let lines = errors
		.into_iter()
		.map(|(line_number, errors)| Line {
			line_number,
			source_code: source_code.lines().nth(line_number).unwrap_or("").trim(),
			errors: errors
				.into_iter()
				.map(|(symbol, error)| LineError {
					column: None,
					symbol,
					error,
				})
				.collect(),
		})
		.collect();
	Error { lines, unlocated }
}

/// The `Error` struct holds the errors `glslc` reported for one shader: those that name a line of it, grouped by
/// line, and those that name none, such as link errors, as the compiler wrote them.
pub struct Error<'a> {
	pub lines: Vec<Line<'a>>,
	pub unlocated: Vec<&'a str>,
}

pub fn pretty_format_glsl_error_lines(error_lines: &[Line]) -> String {
	let max_error_line_number = error_lines.iter().map(|error_line| error_line.line_number).max().unwrap_or(0);
	let max_line_number_length = max_error_line_number.to_string().len();

	let mut error_string = String::new();

	for error_line in error_lines {
		let line_errors = error_line
			.errors
			.iter()
			.map(|error| format!("'{}': {}", error.symbol, error.error))
			.collect::<Vec<_>>()
			.join(", ");

		error_string.push_str(&format!(
			"{:>width$}| {} {} {}\n",
			error_line.line_number + 1,
			error_line.source_code.bold(),
			"←".red().bold(),
			line_errors.red(),
			width = max_line_number_length
		));
	}

	error_string
}

/// Formats each error that names a line with the source lines around it and its message on that line, then each error
/// that names none.
pub fn pretty_format_glslang_errors(error: &Error, source_code: &str) -> String {
	// How many lines to show before and after each error line.
	const WINDOW: usize = 2;
	let source_lines = source_code.lines().collect::<Vec<_>>();

	let mut error_string = String::new();
	for error_line in &error.lines {
		let line_errors = error_line.errors.iter().map(|e| e.error).collect::<Vec<_>>().join(", ");
		let first = error_line.line_number.saturating_sub(WINDOW);
		let window = source_lines.iter().enumerate().skip(first).take(2 * WINDOW + 1);
		for (index, line) in window {
			if index == error_line.line_number {
				let _ = writeln!(
					error_string,
					"{}| {} {} {}",
					index + 1,
					line.bold(),
					"←".red().bold(),
					line_errors.red()
				);
			} else {
				let _ = writeln!(error_string, "{}| {}", index + 1, line.dimmed());
			}
		}
		// The compiler can report a line past the end of the source, such as a missing closing brace.
		if error_line.line_number >= source_lines.len() {
			let _ = writeln!(error_string, "{}| {}", error_line.line_number + 1, line_errors.red());
		}
	}
	for message in &error.unlocated {
		let _ = writeln!(error_string, "{}", message.red());
	}

	error_string
}

/// Formats `glslc` output for `shader_name`, or returns it unchanged when it holds no error the formatter recognizes,
/// so no message is lost.
pub fn pretty_format_glslang_error_string(error_string: &str, shader_name: &str, source_code: &str) -> String {
	let error = process_glslc_error(shader_name, source_code, error_string);
	if error.lines.is_empty() && error.unlocated.is_empty() {
		return error_string.to_string();
	}
	pretty_format_glslang_errors(&error, source_code)
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn test_format_glslang_error() {
		let shader_name = "shaders/fragment.besl";
		let error_string = "shaders/fragment.besl:3: error: 'fresnel_schlick' : no matching overloaded function found
shaders/fragment.besl:3: error: '=' :  cannot convert from ' const float' to ' temp highp 3-component vector of float'
shaders/fragment.besl:3: error: 'distribution_ggx' : no matching overloaded function found
shaders/fragment.besl:3: error: 'geometry_smith' : no matching overloaded function found
shaders/fragment.besl:3: error: 'PI' : undeclared identifier";
		let source_code = "#version 450\nlayout(local_size_x = 1) in;\nvoid main() {}\n";

		let error = process_glslc_error(shader_name, source_code, error_string).lines;

		assert_eq!(error.len(), 1);
		assert_eq!(error[0].line_number, 2);
		assert_eq!(error[0].source_code, "void main() {}");
		assert_eq!(error[0].errors.len(), 5);
		assert_eq!(error[0].errors[0].symbol, "fresnel_schlick");
		assert_eq!(error[0].errors[0].error, "no matching overloaded function found");

		println!("{}", pretty_format_glsl_error_lines(&error));
	}

	/// Verifies each error shows the source line glslang reported, the lines around it, and the error text.
	#[test]
	fn glslang_errors_show_their_line_and_message() {
		colored::control::set_override(false);
		let source_code = "#version 450\nlayout(local_size_x = 1) in;\nvoid broken() {\n\tx = 1;\n}\nvoid main() {}\n";
		let error_string = "shader:4: error: 'x' : undeclared identifier\n1 error generated.\n";

		let formatted = pretty_format_glslang_error_string(error_string, "shader", source_code);

		assert_eq!(
			formatted,
			"2| layout(local_size_x = 1) in;\n3| void broken() {\n4| \tx = 1; ← undeclared identifier\n5| }\n6| void main() {}\n"
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

	#[test]
	fn test_other_glsl_error() {
		let shader_name = "shader";
		let error_string = "2 compilation errors:\nshader:145: error: '>=' : can't read from writeonly object:\nshader:147: error: 'initializer' : can't read from writeonly object:\n";
		let source_code = "";

		let error = process_glslc_error(shader_name, source_code, error_string).lines;

		assert_eq!(error.len(), 2);
		assert_eq!(error[0].line_number, 144);
		assert_eq!(error[0].source_code, "");
		assert_eq!(error[0].errors.len(), 1);
		assert_eq!(error[0].errors[0].symbol, ">=");
		assert_eq!(error[0].errors[0].error, "can't read from writeonly object");
		assert_eq!(error[1].line_number, 146);
		assert_eq!(error[1].source_code, "");
		assert_eq!(error[1].errors.len(), 1);
		assert_eq!(error[1].errors[0].symbol, "initializer");
		assert_eq!(error[1].errors[0].error, "can't read from writeonly object");

		println!("{}", pretty_format_glsl_error_lines(&error));
	}
}
