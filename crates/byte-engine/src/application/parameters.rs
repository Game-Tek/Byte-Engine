use crate::application::Parameter;

/// The `Parameters` trait gives application components access to named configuration values.
pub trait Parameters {
	/// Returns the parameter with the specified full name, if it exists.
	fn get_parameter(&self, name: &str) -> Option<&Parameter>;
}

pub fn parse_variable(value: &str) -> Result<Parameter, ()> {
	let value = value.trim_start_matches("BE_");
	parse_parameter(value)
}

pub fn parse_argument(value: &str) -> Result<Parameter, ()> {
	parse_parameter(value.trim_start_matches("--"))
}

pub fn parse_parameter(value: &str) -> Result<Parameter, ()> {
	let mut split = value.split('=');
	let name = split.next().ok_or(())?;
	let value = split.next().unwrap_or("");
	Ok(Parameter::new(name, value))
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn parses_parameters_variables_and_arguments() {
		// Cover each input source with and without an explicit value.
		let cases = [
			(
				parse_parameter as fn(&str) -> Result<Parameter, ()>,
				"parameter=value",
				"parameter",
				"value",
			),
			(parse_parameter, "parameter", "parameter", ""),
			(parse_parameter, "", "", ""),
			(parse_variable, "BE_VARIABLE=value", "VARIABLE", "value"),
			(parse_variable, "BE_VARIABLE", "VARIABLE", ""),
			(parse_argument, "--argument=value", "argument", "value"),
			(parse_argument, "--argument", "argument", ""),
		];

		for (parse, input, expected_name, expected_value) in cases {
			let parameter = parse(input).unwrap();

			assert_eq!(parameter.name(), expected_name, "input: {input}");
			assert_eq!(parameter.value(), expected_value, "input: {input}");
		}
	}
}
