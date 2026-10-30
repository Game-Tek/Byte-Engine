use smallvec::SmallVec;

use crate::application::Parameter;

/// The `Parameters` trait gives application components access to named configuration values.
///
/// Hand a system the subtree it owns with [`Self::scope`], so it reads its settings by their own names and the
/// caller decides where they live in the whole tree.
pub trait Parameters {
	/// Returns the parameter with the specified full name, if it exists.
	fn get_parameter(&self, name: &str) -> Option<&Parameter>;

	/// Returns the elements of the array parameter `name`, in order.
	///
	/// An array in `config.json` sets `name.0`, `name.1`, and so on, so this reads from index 0 and stops at the
	/// first index without a value. A name that isn't set returns no elements.
	fn get_elements<'s>(&'s self, name: &'s str) -> impl Iterator<Item = &'s Parameter> + 's {
		(0..).map_while(move |index| self.get_parameter(&format!("{name}.{index}")))
	}

	/// Returns the parameters under `prefix`, looked up by the rest of their names.
	///
	/// `app.parameters().scope("physics").get_parameter("step-lanes")` reads `physics.step-lanes`. Scopes nest,
	/// so a system can hand a part of its subtree on. Call it on a reference, which is what a slice or an
	/// application gives; a scope is `Copy`.
	fn scope<'a>(self, prefix: &'a str) -> ScopedParameters<'a, Self>
	where
		Self: Sized,
	{
		ScopedParameters {
			parameters: self,
			prefix,
		}
	}
}

/// A reference reads the parameters it points to, so a scope or a borrowed slice can be passed by value.
impl<P: Parameters + ?Sized> Parameters for &P {
	fn get_parameter(&self, name: &str) -> Option<&Parameter> {
		(**self).get_parameter(name)
	}
}

/// The `ScopedParameters` struct lets a system read the subtree of parameters its caller routed to it by their
/// own names.
///
/// Create it with [`Parameters::scope`]. It is `Copy` when made from a reference, so pass it by value.
#[derive(Clone, Copy)]
pub struct ScopedParameters<'a, P> {
	parameters: P,
	prefix: &'a str,
}

impl<P: Parameters> Parameters for ScopedParameters<'_, P> {
	fn get_parameter(&self, name: &str) -> Option<&Parameter> {
		// A full name fits the inline buffer, so a lookup allocates nothing.
		let mut full_name: SmallVec<[u8; 96]> = SmallVec::with_capacity(self.prefix.len() + 1 + name.len());
		full_name.extend_from_slice(self.prefix.as_bytes());
		full_name.push(b'.');
		full_name.extend_from_slice(name.as_bytes());
		let full_name = std::str::from_utf8(&full_name).expect("Two strings joined by a dot are UTF-8.");
		self.parameters.get_parameter(full_name)
	}
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

/// Returns `defaults` overridden by `configuration` and then by the `BE_*` variables among `environment`.
///
/// The application and BELD both resolve their parameters this way, so a bake reads the settings the application
/// starts with. Pass the application's own parameters as `defaults`, the result of [`read_configuration_file`] as
/// `configuration`, and [`std::env::vars`] as `environment`. In a variable's name, `_` stands for `-` and `__` for `.`,
/// so `BE_PHYSICS__STEP_LANES` sets `physics.step-lanes`.
pub fn layer_parameters(
	defaults: &[Parameter],
	configuration: Vec<Parameter>,
	environment: impl IntoIterator<Item = (String, String)>,
) -> Vec<Parameter> {
	let mut parameters = defaults.to_vec();
	for parameter in configuration {
		upsert_parameter(&mut parameters, parameter);
	}
	for (key, value) in environment {
		let Some(name) = key.strip_prefix("BE_") else {
			continue;
		};
		let name = name.replace("__", ".").replace('_', "-").to_lowercase();
		upsert_parameter(&mut parameters, Parameter::new_string(name, value));
	}
	parameters
}

/// Replaces the parameter with the same name as `parameter`, or appends it when there is none.
pub(crate) fn upsert_parameter(parameters: &mut Vec<Parameter>, parameter: Parameter) {
	if let Some(existing) = parameters.iter_mut().find(|existing| existing.name == parameter.name) {
		*existing = parameter;
	} else {
		parameters.push(parameter);
	}
}

/// Converts the contents of a project's `config.json` into parameters.
///
/// Nested objects become dotted names, so `{"render": {"gtao": {"enabled": false}}}` and
/// `{"render.gtao.enabled": false}` both set `render.gtao.enabled`. Arrays set one name per element by index, so
/// `{"roots": ["a", "b"]}` sets `roots.0` and `roots.1`; read them with [`Parameters::get_elements`]. Strings,
/// numbers, and Booleans become parameter values.
///
/// # Errors
///
/// Returns a message when the file is not a JSON object or holds `null`.
pub fn parameters_from_json(source: &str) -> Result<Vec<Parameter>, String> {
	/// Appends `value` under `name`, or every value inside it with `name` as the prefix of their names.
	fn flatten(name: String, value: &serde_json::Value, parameters: &mut Vec<Parameter>) -> Result<(), String> {
		let value = match value {
			serde_json::Value::Object(object) => {
				for (key, value) in object {
					let name = if name.is_empty() {
						key.clone()
					} else {
						format!("{name}.{key}")
					};
					flatten(name, value, parameters)?;
				}
				return Ok(());
			}
			serde_json::Value::Array(elements) => {
				for (index, value) in elements.iter().enumerate() {
					flatten(format!("{name}.{index}"), value, parameters)?;
				}
				return Ok(());
			}
			serde_json::Value::String(value) => value.clone(),
			serde_json::Value::Bool(value) => value.to_string(),
			serde_json::Value::Number(value) => value.to_string(),
			serde_json::Value::Null => {
				return Err(format!(
					"Configuration value `{name}` is invalid. The most likely cause is that it is `null` instead of a string, number, Boolean, array, or object. Remove the key or give it a value."
				));
			}
		};
		parameters.push(Parameter::new_string(name, value));
		Ok(())
	}

	let root = serde_json::from_str::<serde_json::Value>(source).map_err(|error| {
		format!("Configuration file could not be parsed. The most likely cause is invalid JSON syntax: {error}")
	})?;
	if !root.is_object() {
		return Err(
			"Configuration file could not be read. The most likely cause is that its top-level value is not a JSON object."
				.to_string(),
		);
	}
	let mut parameters = Vec::new();
	flatten(String::new(), &root, &mut parameters)?;
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
	fn configuration_file_arrays_become_indexed_elements() {
		let parameters = parameters_from_json(r#"{ "resources": { "roots": ["scenes/*", "music.wav"], "empty": [] } }"#)
			.expect("test configuration should parse");

		let roots = parameters[..]
			.get_elements("resources.roots")
			.map(Parameter::value)
			.collect::<Vec<_>>();
		assert_eq!(roots, ["scenes/*", "music.wav"]);
		assert_eq!(parameters[..].get_elements("resources.empty").count(), 0);
		assert_eq!(
			parameters.as_slice().scope("resources").get_elements("roots").count(),
			2,
			"a scope reads the same elements"
		);
	}

	#[test]
	fn configuration_overrides_defaults_and_environment_overrides_both() {
		let parameters = layer_parameters(
			&[
				Parameter::new("render.gtao.enabled", "false"),
				Parameter::new("physics.step-lanes", "2"),
				Parameter::new("log.level", "info"),
			],
			parameters_from_json(r#"{ "render.gtao.enabled": true, "physics.step-lanes": 4 }"#)
				.expect("test configuration should parse"),
			[
				("BE_PHYSICS__STEP_LANES".to_string(), "8".to_string()),
				("PATH".to_string(), "/usr/bin".to_string()),
			],
		);

		for (name, value) in [
			("render.gtao.enabled", Some("true")),
			("physics.step-lanes", Some("8")),
			("log.level", Some("info")),
			("path", None),
		] {
			assert_eq!(
				parameters[..].get_parameter(name).map(Parameter::value),
				value,
				"name: {name}"
			);
		}
	}

	#[test]
	fn configuration_file_rejects_values_without_a_parameter_form() {
		for source in ["[]", "{ invalid", r#"{ "log.level": null }"#] {
			assert!(parameters_from_json(source).is_err(), "source: {source}");
		}
	}
}
