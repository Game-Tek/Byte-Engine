use colored::Colorize;

pub fn format_glslang_error(shader_name: &str, error_string: &str, source_code: &str) -> Option<String> {
	let errors = error_string.lines().filter(|error|
		error.starts_with(shader_name)).filter_map(|error| {
			let split = error.split(':').map(|e| e.trim()).collect::<Vec<_>>();
			if split.len() >= 5 {
				Some((split[1], [split[3], split[4]].join(" ")))
			} else {
				None
			}
		}).map(|(error_line_number_string, error)|
			(error_line_number_string.trim().parse::<usize>().unwrap() - 1, error)
	).collect::<Vec<_>>();

	// Collect errors by line number
	let mut errors_by_line_number = std::collections::HashMap::<usize, Vec<String>>::new();

	for (error_line_number, error) in errors {
		if let Some(errors) = errors_by_line_number.get_mut(&error_line_number) {
			errors.push(error);
		} else {
			errors_by_line_number.insert(error_line_number, vec![error]);
		}
	}

	// Sort errors by line number
	let mut errors = errors_by_line_number.into_iter().collect::<Vec<_>>();

	errors.sort_by(|(line_number_a, _), (line_number_b, _)| line_number_a.cmp(line_number_b));

	let mut error_string = String::new();

	for (error_line_index, line_errors) in errors {
		let error = line_errors.join(", ");

		let previous_previous_line = format!("{}| {}", error_line_index - 2 + 1, source_code.lines().nth(error_line_index - 2).unwrap_or("").dimmed());
		let previous_line = format!("{}| {}", error_line_index - 1 + 1, source_code.lines().nth(error_line_index - 1).unwrap_or("").dimmed());
		let current_line = format!("{}| {} {} {}", error_line_index + 1, source_code.lines().nth(error_line_index).unwrap_or("").bold(), "←".red().bold(), error.red());
		let next_line = format!("{}| {}", error_line_index + 1 + 1, source_code.lines().nth(error_line_index + 1).unwrap_or("").dimmed());
		let next_next_line = format!("{}| {}", error_line_index + 2 + 1, source_code.lines().nth(error_line_index + 2).unwrap_or("").dimmed());

		error_string.push_str(&format!("{}\n{}\n{}\n{}\n{}\n", previous_previous_line, previous_line, current_line, next_line, next_next_line));
	}

	Some(error_string)
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

		let error = format_glslang_error(shader_name, error_string, source_code).unwrap();

		println!("{}", &error);

		assert_ne!(error, "");
	}
}