use crate::application::Parameter;

/// The `Parameters` trait gives application components access to named configuration values.
pub trait Parameters {
	/// Returns the parameter with the specified full name, if it exists.
	fn get_parameter(&self, name: &str) -> Option<&Parameter>;
}

/// The `ParameterError` struct reports a startup parameter whose value could not be parsed for its setting.
///
/// Returned by [`Parameter::parse`]; its message names the parameter and the rejected value.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParameterError {
	pub(super) name: String,
	pub(super) value: String,
	pub(super) reason: String,
}

impl std::fmt::Display for ParameterError {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		write!(
			f,
			"Parameter `{}` is invalid. The most likely cause is that `{}` is not a valid value for it: {}",
			self.name, self.value, self.reason
		)
	}
}

impl std::error::Error for ParameterError {}

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
	fn parses_parameters_and_arguments() {
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
			(parse_argument, "--argument=value", "argument", "value"),
			(parse_argument, "--argument", "argument", ""),
		];

		for (parse, input, expected_name, expected_value) in cases {
			let parameter = parse(input).expect("test parameter should parse");

			assert_eq!(parameter.name(), expected_name, "input: {input}");
			assert_eq!(parameter.value(), expected_value, "input: {input}");
		}
	}
}
