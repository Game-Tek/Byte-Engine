//! Players' replacement bindings, as a document an application reads, edits, and saves.
//!
//! A [`Bindings`] value is plain data: action names mapped to the triggers that
//! replace their defaults. It doesn't know where it is stored. Convert it with
//! [`Bindings::from_json`] and [`Bindings::to_json`] to keep it in any settings
//! storage, or use [`Bindings::read`] and [`Bindings::write`] for a file. Give it
//! to [`InputSink::apply`](super::InputSink::apply), and take a sink's current
//! replacements back with [`InputSink::bindings`](super::InputSink::bindings).
//!
//! See [Input bindings](/docs/reference/input-bindings) for the document format.

use std::alloc::Allocator;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use math::Quaternion;
use serde_json::{Map, Value as Json};

use super::{
	ActionBindingDescription, Axis2, Axis3, DeadZone, Function, InputCollector, Invert, TriggerMode, Value, ValueMapping,
};
use crate::application::{application::resolve_application_path, parameters::Parameters};

/// The `Bindings` struct holds the bindings a player chose in place of actions' defaults.
///
/// Load it from your settings storage, pass it to
/// [`InputSink::apply`](super::InputSink::apply) at startup, and save what
/// [`InputSink::bindings`](super::InputSink::bindings) returns after the player
/// changes a control.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Bindings {
	/// The replacement bindings, by action name. An empty list unbinds the action.
	pub actions: BTreeMap<String, Vec<Binding>>,
}

/// The `Binding` struct is one replacement binding in a [`Bindings`] document, with triggers named as text.
///
/// It mirrors [`ActionBindingDescription`], which a sink builds from it once the
/// trigger names resolve against an [`InputCollector`].
#[derive(Clone, Debug, PartialEq)]
pub struct Binding {
	/// The `Class.Trigger` name whose value the binding contributes.
	pub source: String,
	/// The `Class.Trigger` name of the button that decides when the source is published, if any.
	pub triggered_by: Option<String>,
	/// When [`Self::triggered_by`] drives the binding.
	pub on: TriggerMode,
	/// How the source's value maps into the action's value.
	pub mapping: ValueMapping,
}

impl Binding {
	/// Creates a binding that follows one trigger, such as `Keyboard.Space`.
	pub fn new(source: impl Into<String>) -> Self {
		Self {
			source: source.into(),
			triggered_by: None,
			on: TriggerMode::default(),
			mapping: false.into(),
		}
	}

	/// Resolves the trigger names into a binding description for `collector`'s triggers.
	pub(super) fn description<A: Allocator + Clone>(
		&self,
		collector: &InputCollector<A>,
	) -> Result<ActionBindingDescription, String> {
		let mut description = ActionBindingDescription::new(collector.find_trigger(&self.source)?).mapped(self.mapping);
		if let Some(button) = &self.triggered_by {
			description = description.triggered_by(collector.find_trigger(button)?);
		}
		Ok(description.trigger_on(self.on))
	}

	/// Names a binding description's triggers, or returns `None` when one isn't registered with `collector`.
	pub(super) fn from_description<A: Allocator + Clone>(
		description: &ActionBindingDescription,
		collector: &InputCollector<A>,
	) -> Option<Self> {
		let triggered_by = match &description.trigger {
			Some(button) => Some(collector.trigger_name(*button)?.to_string()),
			None => None,
		};
		Some(Self {
			source: collector.trigger_name(description.input_source)?.to_string(),
			triggered_by,
			on: description.trigger_mode,
			mapping: description.mapping,
		})
	}
}

impl Bindings {
	/// Parses a bindings document.
	///
	/// Trigger names aren't checked here, because they depend on the triggers an
	/// application registers; [`InputSink::apply`](super::InputSink::apply)
	/// checks them.
	///
	/// # Errors
	///
	/// Returns a message naming the action and binding when the text isn't a valid bindings document.
	pub fn from_json(source: &str) -> Result<Self, String> {
		let document: Json = serde_json::from_str(source).map_err(|error| {
			format!(
				"Input bindings aren't valid JSON: {error}. The most likely cause is a hand edit that left a syntax error. Fix the text, or delete the file to restore the default bindings."
			)
		})?;
		let Json::Object(document) = document else {
			return Err(invalid("The document", "an object with an `actions` member"));
		};
		let mut bindings = Self::default();
		for (key, value) in document {
			if key != "actions" {
				return Err(invalid(&format!("Member '{key}'"), "absent; the document only has `actions`"));
			}
			let Json::Object(actions) = value else {
				return Err(invalid("`actions`", "an object of action names"));
			};
			for (action, list) in actions {
				let Json::Array(list) = list else {
					return Err(invalid(&format!("Action '{action}'"), "an array of bindings"));
				};
				let list = list
					.iter()
					.enumerate()
					.map(|(position, binding)| {
						parse_binding(binding).map_err(|error| format!("Action '{action}', binding {}: {error}", position + 1))
					})
					.collect::<Result<_, _>>()?;
				bindings.actions.insert(action, list);
			}
		}
		Ok(bindings)
	}

	/// Writes this document as indented JSON text, writing a binding that only has a source as a string.
	pub fn to_json(&self) -> String {
		let actions = self
			.actions
			.iter()
			.map(|(action, list)| (action.clone(), Json::Array(list.iter().map(write_binding).collect())))
			.collect::<Map<_, _>>();
		let mut document = Map::new();
		document.insert("actions".to_string(), Json::Object(actions));
		serde_json::to_string_pretty(&Json::Object(document)).expect("JSON values always serialize")
	}

	/// Reads a bindings file, or returns an empty document when the file doesn't exist.
	///
	/// Find the application's file with [`Self::path`].
	///
	/// # Errors
	///
	/// Returns a message when the file exists but can't be read or isn't a valid document.
	pub fn read(path: &Path) -> Result<Self, String> {
		match std::fs::read_to_string(path) {
			Ok(source) => Self::from_json(&source).map_err(|error| format!("{error} File: '{}'.", path.display())),
			Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(Self::default()),
			Err(error) => Err(format!(
				"Input bindings file '{}' couldn't be read: {error}. The most likely cause is missing read permission. Give the application read access to the file, or set `input.bindings` to another path.",
				path.display()
			)),
		}
	}

	/// Replaces a bindings file with this document.
	///
	/// The document goes to a file beside `path` first and then takes its place,
	/// so an interrupted write never leaves part of a document behind.
	///
	/// # Errors
	///
	/// Returns a message when the file can't be written.
	pub fn write(&self, path: &Path) -> Result<(), String> {
		let staging = path.with_extension("json.partial");
		std::fs::write(&staging, self.to_json())
			.and_then(|()| std::fs::rename(&staging, path))
			.map_err(|error| {
				format!(
					"Input bindings file '{}' couldn't be written: {error}. The most likely cause is a directory without write permission. Set `input.bindings` to a path the application can write.",
					path.display()
				)
			})
	}

	/// Returns where the application keeps its bindings file: the `input.bindings`
	/// parameter, or `bindings.json` in the application directory.
	pub fn path(parameters: &impl Parameters) -> PathBuf {
		resolve_application_path(parameters.get_parameter("input.bindings"), "bindings.json")
	}
}

/// Describes a document member whose JSON type is wrong.
fn invalid(member: &str, expected: &str) -> String {
	format!(
		"{member} in the input bindings has the wrong form. It must be {expected}. Correct it as the input bindings reference describes."
	)
}

/// Parses one binding: a trigger name, or an object with `source` and optional members.
fn parse_binding(binding: &Json) -> Result<Binding, String> {
	let object = match binding {
		Json::String(source) => return Ok(Binding::new(source.clone())),
		Json::Object(object) => object,
		_ => return Err(invalid("The binding", "a trigger name or an object")),
	};
	let text = |key: &str| match object.get(key) {
		None => Ok(None),
		Some(Json::String(text)) => Ok(Some(text.clone())),
		Some(_) => Err(invalid(&format!("`{key}`"), "a string")),
	};
	let mut parsed = Binding::new(text("source")?.ok_or_else(|| invalid("`source`", "present"))?);
	parsed.triggered_by = text("triggered_by")?;
	parsed.on = match text("on")?.as_deref() {
		None | Some("release") => TriggerMode::Release,
		Some("press") => TriggerMode::Press,
		Some("drag") => TriggerMode::Drag,
		Some("toggle") => TriggerMode::Toggle,
		Some(_) => return Err(invalid("`on`", "`release`, `press`, `drag`, or `toggle`")),
	};
	let function = match text("function")?.as_deref() {
		None | Some("linear") => Function::Linear,
		Some("sphere") => Function::Sphere,
		Some(_) => return Err(invalid("`function`", "`linear` or `sphere`")),
	};
	let value = match object.get("value") {
		None => Value::Bool(false),
		Some(value) => parse_value(value)?,
	};
	parsed.mapping = ValueMapping::new(function, value);
	if let Some(dead_zone) = object.get("dead_zone") {
		let expected = "an array of 2 numbers, `[inner, outer]`, with 0 ≤ inner < outer ≤ 1";
		let bounds = match dead_zone.as_array().map(Vec::as_slice) {
			Some([inner, outer]) => inner.as_f64().zip(outer.as_f64()),
			_ => None,
		};
		let dead_zone = bounds
			.and_then(|(inner, outer)| DeadZone::new(inner as f32, outer as f32).ok())
			.ok_or_else(|| invalid("`dead_zone`", expected))?;
		parsed.mapping = parsed.mapping.dead_zone(dead_zone);
	}
	if let Some(curve) = object.get("curve") {
		let exponent = curve
			.as_f64()
			.map(|exponent| exponent as f32)
			.filter(|exponent| exponent.is_finite() && *exponent > 0.0)
			.ok_or_else(|| invalid("`curve`", "a number above 0"))?;
		parsed.mapping = parsed.mapping.response_curve(exponent);
	}
	if let Some(axes) = object.get("invert") {
		let expected = "an array of `\"x\"`, `\"y\"`, or both";
		let mut invert = Invert::default();
		for axis in axes.as_array().ok_or_else(|| invalid("`invert`", expected))? {
			match axis.as_str() {
				Some("x") => invert.x = true,
				Some("y") => invert.y = true,
				_ => return Err(invalid("`invert`", expected)),
			}
		}
		parsed.mapping = parsed.mapping.invert(invert);
	}
	if let Some(key) = object.keys().find(|key| {
		!matches!(
			key.as_str(),
			"source" | "triggered_by" | "on" | "function" | "value" | "dead_zone" | "curve" | "invert"
		)
	}) {
		return Err(invalid(
			&format!("Member '{key}'"),
			"absent; a binding only has `source`, `triggered_by`, `on`, `function`, `value`, `dead_zone`, `curve`, and `invert`",
		));
	}
	Ok(parsed)
}

/// Reads a mapping value, whose JSON type selects the input value type.
fn parse_value(value: &Json) -> Result<Value, String> {
	let expected = "a Boolean, a number, or an array of 2, 3, or 4 numbers";
	let numbers = |elements: &[Json]| {
		elements
			.iter()
			.map(|element| element.as_f64().map(|number| number as f32))
			.collect::<Option<Vec<_>>>()
			.ok_or_else(|| invalid("`value`", expected))
	};
	Ok(match value {
		Json::Bool(value) => Value::Bool(*value),
		Json::Number(number) => Value::Float(number.as_f64().unwrap_or_default() as f32),
		Json::Array(elements) => match numbers(elements)?.as_slice() {
			&[x, y] => Value::Vector2(Axis2::new(x, y)),
			&[x, y, z] => Value::Vector3(Axis3::new(x, y, z)),
			&[x, y, z, w] => Value::Quaternion(Quaternion::new(x, y, z, w)),
			_ => return Err(invalid("`value`", expected)),
		},
		_ => return Err(invalid("`value`", expected)),
	})
}

/// Writes one binding, as its source's name alone when every other member has its default.
fn write_binding(binding: &Binding) -> Json {
	let default_mapping = ValueMapping::from(false);
	let toggle = binding.on == TriggerMode::Toggle;
	if binding.triggered_by.is_none() && !toggle && binding.mapping == default_mapping {
		return Json::String(binding.source.clone());
	}
	let mut object = Map::new();
	object.insert("source".into(), binding.source.clone().into());
	if let Some(button) = &binding.triggered_by {
		object.insert("triggered_by".into(), button.clone().into());
	}
	// Without a button, only a toggle's mode means anything.
	let on = match binding.on {
		TriggerMode::Toggle => Some("toggle"),
		_ if binding.triggered_by.is_none() => None,
		TriggerMode::Release => None,
		TriggerMode::Press => Some("press"),
		TriggerMode::Drag => Some("drag"),
	};
	if let Some(on) = on {
		object.insert("on".into(), on.into());
	}
	if let Some(dead_zone) = binding.mapping.dead_zone {
		let (inner, outer) = dead_zone.bounds();
		object.insert("dead_zone".into(), Json::Array(vec![inner.into(), outer.into()]));
	}
	if let Some(exponent) = binding.mapping.curve {
		object.insert("curve".into(), exponent.into());
	}
	let Invert { x, y } = binding.mapping.invert;
	if x || y {
		let axes = [(x, "x"), (y, "y")].into_iter().filter(|(inverted, _)| *inverted);
		object.insert("invert".into(), axes.map(|(_, axis)| Json::from(axis)).collect());
	}
	if binding.mapping.function == Function::Sphere {
		object.insert("function".into(), "sphere".into());
	}
	if binding.mapping.value != Value::Bool(false) {
		object.insert("value".into(), write_value(binding.mapping.value));
	}
	Json::Object(object)
}

/// Writes a mapping value; types a document can't hold become the number a Float action reads from them.
fn write_value(value: Value) -> Json {
	let numbers = |elements: &[f32]| Json::Array(elements.iter().map(|&number| Json::from(number)).collect());
	match value {
		Value::Bool(value) => value.into(),
		Value::Float(value) => value.into(),
		Value::Int(value) => (value as f32).into(),
		Value::Rgba(value) => value.r.into(),
		Value::Unicode(_) => 0.0f32.into(),
		Value::Vector2(value) => numbers(&[value.x, value.y]),
		Value::Vector3(value) => numbers(&[value.x, value.y, value.z]),
		Value::Quaternion(value) => numbers(&[value.x, value.y, value.z, value.w]),
	}
}
