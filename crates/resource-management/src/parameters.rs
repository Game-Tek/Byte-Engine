//! Named configuration values that an application and its BELD read the same way.
//!
//! Start with [`layer_parameters`] to resolve the values from code, `config.json`, `BE_*` variables, and arguments, then
//! hand each system its subtree with [`Parameters::scope`].

use smallvec::SmallVec;

/// The [`Parameter`] struct represents one application configuration value from
/// code, the environment, or command-line arguments.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Parameter {
	name: String,
	value: String,
}

impl Parameter {
	/// Creates a parameter from borrowed name and value strings.
	pub fn new(name: &str, value: &str) -> Self {
		Parameter {
			name: name.into(),
			value: value.into(),
		}
	}

	/// Creates a parameter from owned name and value strings.
	pub fn new_string(name: String, value: String) -> Self {
		Parameter { name, value }
	}

	/// Returns whether this parameter has the requested name.
	pub fn is(&self, name: &str) -> bool {
		self.name == name
	}

	/// Parses the parameter's value as a bool.
	///
	/// Returns `Some(true)` for `true`, `TRUE`, or `1`. Returns `Some(false)` for
	/// `false`, `FALSE`, or `0`. Returns `None` for all other values.
	pub fn as_bool(&self) -> Option<bool> {
		match self.value.as_str() {
			"true" | "TRUE" | "1" => Some(true),
			"false" | "FALSE" | "0" => Some(false),
			_ => None,
		}
	}

	/// Parses the parameter as a Boolean value and returns `false` if parsing fails.
	///
	/// Use [`Self::as_bool`] when the caller must distinguish `false` from an
	/// invalid value.
	pub fn as_bool_simple(&self) -> bool {
		self.as_bool().unwrap_or(false)
	}

	/// Parses the parameter's value for a setting of type `T`.
	///
	/// Each caller chooses its own policy for an invalid value: ignore it with `.ok()`, or stop with
	/// `.unwrap_or_else(|error| panic!("{error}"))`.
	///
	/// # Errors
	///
	/// Returns [`ParameterError`] naming the parameter and the rejected value when `T` cannot parse it.
	pub fn parse<T: std::str::FromStr>(&self) -> Result<T, ParameterError>
	where
		T::Err: std::fmt::Display,
	{
		self.value.parse::<T>().map_err(|error| ParameterError {
			name: self.name.clone(),
			value: self.value.clone(),
			reason: error.to_string(),
		})
	}

	/// Returns the parameter name used by application configuration lookup.
	pub fn name(&self) -> &str {
		&self.name
	}

	/// Returns the raw parameter value before caller-specific parsing.
	pub fn value(&self) -> &str {
		&self.value
	}
}

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
	name: String,
	value: String,
	reason: String,
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
/// The engine's application reads it at startup, and BELD reads it to choose what to bake.
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

/// Returns `defaults` overridden by `configuration`, then by `environment`, and then by the `--name=value` entries
/// among `arguments`, so the most dynamic source wins.
///
/// The application and BELD both resolve their parameters this way, so a bake reads the settings the application
/// starts with. Pass [`read_configuration_file`]'s result as `configuration`, [`std::env::vars`] as `environment`,
/// and [`std::env::args`] as `arguments`, or nothing when the arguments are commands. In a `BE_` variable's name, `_`
/// stands for `-` and `__` for `.`, so `BE_PHYSICS__STEP_LANES` sets `physics.step-lanes`. `RUST_LOG` sets
/// `log.level` unless `BE_LOG__LEVEL` is set too.
pub fn layer_parameters(
	defaults: &[Parameter],
	configuration: Vec<Parameter>,
	environment: impl IntoIterator<Item = (String, String)>,
	arguments: impl IntoIterator<Item = String>,
) -> Vec<Parameter> {
	let mut parameters = defaults.to_vec();
	for parameter in configuration {
		upsert_parameter(&mut parameters, parameter);
	}
	let environment = environment.into_iter().collect::<Vec<_>>();
	// Variables come in no particular order, so `RUST_LOG` goes first and the engine's own name wins.
	if let Some((_, filter)) = environment.iter().find(|(key, _)| key == "RUST_LOG") {
		upsert_parameter(
			&mut parameters,
			Parameter::new_string("log.level".to_string(), filter.clone()),
		);
	}
	for (key, value) in environment {
		let Some(name) = key.strip_prefix("BE_") else {
			continue;
		};
		let name = name.replace("__", ".").replace('_', "-").to_lowercase();
		upsert_parameter(&mut parameters, Parameter::new_string(name, value));
	}
	for argument in arguments.into_iter().filter(|argument| argument.starts_with("--")) {
		if let Ok(parameter) = parse_argument(&argument) {
			upsert_parameter(&mut parameters, parameter);
		}
	}
	parameters
}

/// Replaces the parameter with the same name as `parameter`, or appends it when there is none.
fn upsert_parameter(parameters: &mut Vec<Parameter>, parameter: Parameter) {
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
	fn later_sources_override_earlier_ones() {
		let parameters = layer_parameters(
			&[
				Parameter::new("render.gtao.enabled", "false"),
				Parameter::new("physics.step-lanes", "2"),
				Parameter::new("inspector.port", "6681"),
				Parameter::new("messages.capacity", "64"),
			],
			parameters_from_json(r#"{ "render.gtao.enabled": true, "physics.step-lanes": 4, "inspector.port": 6682 }"#)
				.expect("test configuration should parse"),
			[
				("BE_PHYSICS__STEP_LANES".to_string(), "8".to_string()),
				("BE_INSPECTOR__PORT".to_string(), "7000".to_string()),
				("PATH".to_string(), "/usr/bin".to_string()),
			],
			["program", "--physics.step-lanes=16", "positional"].map(String::from),
		);

		for (name, value) in [
			("messages.capacity", Some("64")),
			("render.gtao.enabled", Some("true")),
			("inspector.port", Some("7000")),
			("physics.step-lanes", Some("16")),
			("path", None),
			("program", None),
			("positional", None),
		] {
			assert_eq!(
				parameters[..].get_parameter(name).map(Parameter::value),
				value,
				"name: {name}"
			);
		}
	}

	#[test]
	fn log_level_follows_the_most_dynamic_source() {
		// `BE_LOG__LEVEL` comes first in the environment to show that its order there does not matter.
		let rust_log = ("RUST_LOG", "info,byte_engine::physics=trace");
		let be_log_level = ("BE_LOG__LEVEL", "error");
		let cases: [(&[(&str, &str)], &[&str], &str); 4] = [
			(&[], &[], "warn"),
			(&[rust_log], &[], "info,byte_engine::physics=trace"),
			(&[be_log_level, rust_log], &[], "error"),
			(&[be_log_level, rust_log], &["--log.level=off"], "off"),
		];

		for (environment, arguments, expected) in cases {
			let parameters = layer_parameters(
				&[Parameter::new("log.level", "debug")],
				parameters_from_json(r#"{ "log.level": "warn" }"#).expect("test configuration should parse"),
				environment.iter().map(|&(key, value)| (key.to_string(), value.to_string())),
				arguments.iter().map(|argument| argument.to_string()),
			);
			assert_eq!(
				parameters[..].get_parameter("log.level").map(Parameter::value),
				Some(expected),
				"environment: {environment:?}, arguments: {arguments:?}"
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
