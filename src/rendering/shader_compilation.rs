use colored::Colorize;

pub fn format_glslang_error(shader_name: &str, error_string: &str, source_code: &str) -> Option<String> {
	let errors = error_string.lines().filter(|error|
		error.starts_with(shader_name)).map(|error|
			(error.split(':').nth(1).unwrap(), error.split(':').nth(4).unwrap())).map(|(error_line_number_string, error)|
				(error_line_number_string.trim().parse::<usize>().unwrap() - 1, error.trim())
	).collect::<Vec<_>>();

	let mut error_string = String::new();

	for (error_line_index, error) in errors {
		let previous_previous_line = format!("{}| {}", error_line_index - 2, source_code.lines().nth(error_line_index - 2).unwrap_or("").dimmed());
		let previous_line = format!("{}| {}", error_line_index - 1, source_code.lines().nth(error_line_index - 1).unwrap_or("").dimmed());
		let current_line = format!("{}| {} {} {}", error_line_index, source_code.lines().nth(error_line_index).unwrap_or("").bold(), "←".red().bold(), error.red());
		let next_line = format!("{}| {}", error_line_index + 1, source_code.lines().nth(error_line_index + 1).unwrap_or("").dimmed());
		let next_next_line = format!("{}| {}", error_line_index + 2, source_code.lines().nth(error_line_index + 2).unwrap_or("").dimmed());

		error_string.push_str(&format!("{}\n{}\n{}\n{}\n{}\n", previous_previous_line, previous_line, current_line, next_line, next_next_line));
	}

	Some(error_string)
}