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

impl Parameters for [Parameter] {
	fn get_parameter(&self, name: &str) -> Option<&Parameter> {
		self.iter().find(|parameter| parameter.is(name))
	}
}

/// The name of the project configuration file that both the application and BELD read.
pub const CONFIGURATION_FILE_NAME: &str = "config.json";

/// Reads the project's configuration file at `path`, or returns no parameters when the file does not exist.
///
/// The application reads it in [`crate::application::BaseApplication::new`], and BELD reads it to choose what to bake.
///
/// # Errors
///
/// Returns a message when the file exists but cannot be read or is not valid configuration.
pub fn read_configuration_file(path: &std::path::Path) -> Result<Vec<Parameter>, String> {
	match std::fs::read_to_string(path) {
		Ok(source) => parameters_from_json(&source).map_err(|error| format!("{error} File: '{}'.", path.display())),
		Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(Vec::new()),
		Err(error) => Err(format!(
			"Configuration file '{}' could not be read. The most likely cause is missing read permission: {error}",
			path.display()
		)),
	}
}

/// Converts the contents of a project's `config.json` into parameters.
///
/// Nested objects become dotted names, so `{"render": {"gtao": {"enabled": false}}}` and
/// `{"render.gtao.enabled": false}` both set `render.gtao.enabled`. Strings, numbers, and Booleans become parameter
/// values.
///
/// # Errors
///
/// Returns a message when the file is not a JSON object or holds an array or `null`.
pub fn parameters_from_json(source: &str) -> Result<Vec<Parameter>, String> {
	/// Appends every value under `object`, prefixing its names with the path of enclosing keys.
	fn flatten(
		prefix: &str,
		object: &serde_json::Map<String, serde_json::Value>,
		parameters: &mut Vec<Parameter>,
	) -> Result<(), String> {
		for (key, value) in object {
			let name = if prefix.is_empty() {
				key.clone()
			} else {
				format!("{prefix}.{key}")
			};
			let value = match value {
				serde_json::Value::Object(object) => {
					flatten(&name, object, parameters)?;
					continue;
				}
				serde_json::Value::String(value) => value.clone(),
				serde_json::Value::Bool(value) => value.to_string(),
				serde_json::Value::Number(value) => value.to_string(),
				serde_json::Value::Array(_) | serde_json::Value::Null => {
					return Err(format!(
						"Configuration value `{name}` is invalid. The most likely cause is that it is an array or `null` instead of a string, number, Boolean, or object."
					));
				}
			};
			parameters.push(Parameter::new_string(name, value));
		}
		Ok(())
	}

	let root = serde_json::from_str::<serde_json::Value>(source).map_err(|error| {
		format!("Configuration file could not be parsed. The most likely cause is invalid JSON syntax: {error}")
	})?;
	let serde_json::Value::Object(root) = root else {
		return Err(
			"Configuration file could not be read. The most likely cause is that its top-level value is not a JSON object."
				.to_string(),
		);
	};
	let mut parameters = Vec::new();
	flatten("", &root, &mut parameters)?;
	Ok(parameters)
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

	#[test]
	fn configuration_file_nesting_becomes_dotted_names() {
		let parameters = parameters_from_json(
			r#"{ "render": { "gtao": { "enabled": false }, "debug": true }, "kill-after": 3, "render.ssgi.enabled": "false" }"#,
		)
		.expect("test configuration should parse");

		for (name, value) in [
			("render.gtao.enabled", "false"),
			("render.debug", "true"),
			("kill-after", "3"),
			("render.ssgi.enabled", "false"),
		] {
			assert_eq!(
				parameters[..].get_parameter(name).map(Parameter::value),
				Some(value),
				"name: {name}"
			);
		}
	}

	#[test]
	fn configuration_file_rejects_values_without_a_parameter_form() {
		for source in [
			"[]",
			"{ invalid",
			r#"{ "render": { "passes": [] } }"#,
			r#"{ "log.level": null }"#,
		] {
			assert!(parameters_from_json(source).is_err(), "source: {source}");
		}
	}
}
