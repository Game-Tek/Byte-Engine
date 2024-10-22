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

/// Process the output of glslc and return a list of errors.
pub fn process_glslc_error<'a>(shader_name: &str, source_code: &'a str, error_string: &'a str) -> Error<'a> {
	// Collect (error_line_number, error) pairs
	let errors = error_string.lines().filter(|error|
		error.starts_with(shader_name)).filter_map(|error| {
			let split = error.split(':').map(|e| e.trim()).collect::<Vec<_>>();
			if split.len() == 5 {
				Some((split[1], (split[3].trim_matches('\''), split[4])))
			} else {
				None
			}
		}).map(|(error_line_number_string, error)|
			(error_line_number_string.trim().parse::<usize>().unwrap() - 1, error)
	).collect::<Vec<_>>();
	
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
	
	errors.sort_by(|(line_number_a, _), (line_number_b, _)| line_number_a.cmp(line_number_b));

	errors.into_iter().map(|(line_number, errors)| Line {
		line_number,
		source_code: source_code.lines().nth(line_number).unwrap_or("").trim(),
		errors: errors.into_iter().map(|(symbol, error)| {
			LineError {
				column: None,
				symbol,
				error,
			}
		}).collect()
	}).collect()
}

pub type Error<'a> = Vec<Line<'a>>;

pub fn pretty_print(error_lines: &[Line]) -> String {
	let max_error_line_number = error_lines.iter().map(|error_line| error_line.line_number).max().unwrap_or(0);
	let max_line_number_length = max_error_line_number.to_string().len();

	let mut error_string = String::new();

	for error_line in error_lines {
		let line_errors = error_line.errors.iter().map(|error| {
			format!("'{}': {}", error.symbol, error.error)
		}).collect::<Vec<_>>().join(", ");

		error_string.push_str(&format!("{:>width$}| {} {} {}\n", error_line.line_number + 1, error_line.source_code.bold(), "←".red().bold(), line_errors.red(), width = max_line_number_length));
	}

	error_string
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

		let error = process_glslc_error(shader_name, source_code, error_string);

		assert_eq!(error.len(), 1);
		assert_eq!(error[0].line_number, 2);
		assert_eq!(error[0].source_code, "void main() {}");
		assert_eq!(error[0].errors.len(), 5);
		assert_eq!(error[0].errors[0].symbol, "fresnel_schlick");
		assert_eq!(error[0].errors[0].error, "no matching overloaded function found");		

		println!("{}", pretty_print(&error));
	}
}