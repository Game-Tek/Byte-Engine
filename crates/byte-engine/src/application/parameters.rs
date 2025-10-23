use crate::application::Parameter;

/// The `Parameters` trait provides methods for any entity that wnats to expose configurations.
pub trait Parameters {
	/// Returns any paramater that matches the provided full name.
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
	fn test_parse_parameter() {
		let param = parse_parameter("parameter=value").unwrap();
		assert_eq!(param.name(), "parameter");
		assert_eq!(param.value(), "value");
	}

	#[test]
	fn test_parse_parameter_no_value() {
		let param = parse_parameter("parameter").unwrap();
		assert_eq!(param.name(), "parameter");
		assert_eq!(param.value(), "");
	}

	#[test]
	fn test_parse_empty_parameter() {
		let param = parse_parameter("").unwrap();
		assert_eq!(param.name(), "");
		assert_eq!(param.value(), "");
	}

	#[test]
	fn test_parse_variable() {
		let param = parse_variable("BE_VARIABLE=value").unwrap();
		assert_eq!(param.name(), "VARIABLE");
		assert_eq!(param.value(), "value");
	}

	#[test]
	fn test_parse_variable_no_value() {
		let param = parse_variable("BE_VARIABLE").unwrap();
		assert_eq!(param.name(), "VARIABLE");
		assert_eq!(param.value(), "");
	}

	#[test]
	fn test_parse_argument() {
		let param = parse_argument("--argument=value").unwrap();
		assert_eq!(param.name(), "argument");
		assert_eq!(param.value(), "value");
	}

	#[test]
	fn test_parse_argument_no_value() {
		let param = parse_argument("--argument").unwrap();
		assert_eq!(param.name(), "argument");
		assert_eq!(param.value(), "");
	}
}
