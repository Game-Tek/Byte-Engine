//! What the variant window edits: a material variant's parameters, as its parent material declares them, with the
//! values the variant gives them.
//!
//! These are pure functions of the parsed `.bema` sources; the editor reads the sources, opens a [`Variant`] from them,
//! and writes back the text [`Variant::set`] returns. The format is specified in
//! [Material files](/docs/reference/bema#variant-files).

use serde_json::{Value, json};

/// The `Variant` struct is a material variant open in its own window, so its parameters can be changed one at a time
/// and written back to its file.
pub(crate) struct Variant {
	/// The variant's asset ID, such as `grid_green.bema`.
	pub(crate) id: String,
	/// The asset ID of the root material it specializes.
	pub(crate) parent: String,
	/// Every parameter of the parent, in the parent's order, with the value the variant gives it.
	pub(crate) parameters: Vec<Parameter>,
	/// The variant's parsed source, which a change rewrites so the fields the window doesn't show are kept.
	source: Value,
}

/// The `Parameter` struct is one parameter the variant window shows: its name, what it holds, and its value.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Parameter {
	pub(crate) name: String,
	pub(crate) kind: Kind,
	/// The variant's value, or the parent's default for a parameter the variant leaves out.
	pub(crate) value: Value,
}

/// The `Kind` enum is what a parameter holds, by its parent's `data_type`, which decides what a typed value means.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum Kind {
	Float,
	Vec3,
	Vec4,
	Texture,
}

/// The color names a `vec3f` or `vec4f` value can be instead of numbers.
const COLORS: [&str; 6] = ["Red", "Green", "Blue", "Purple", "White", "Black"];

impl Kind {
	fn of(data_type: &str) -> Option<Self> {
		match data_type {
			"float" => Some(Self::Float),
			"vec3f" => Some(Self::Vec3),
			"vec4f" => Some(Self::Vec4),
			"Texture2D" => Some(Self::Texture),
			_ => None,
		}
	}

	/// Reads `typed` as a value of this kind: a number; three or four numbers apart by commas or spaces, or a color
	/// name; or an image's asset ID. Returns `None` for text that isn't one.
	pub(crate) fn parse(self, typed: &str) -> Option<Value> {
		let typed = typed.trim();
		let components = |count: usize| -> Option<Value> {
			if let Some(name) = COLORS.iter().find(|name| name.eq_ignore_ascii_case(typed)) {
				return Some(json!(name));
			}
			let numbers: Vec<f64> = typed
				.split(|c: char| c == ',' || c.is_whitespace())
				.filter(|part| !part.is_empty())
				.map(str::parse)
				.collect::<Result<_, _>>()
				.ok()?;
			(numbers.len() == count).then(|| json!(numbers))
		};
		match self {
			Self::Float => typed.parse::<f64>().ok().map(|number| json!(number)),
			Self::Vec3 => components(3),
			Self::Vec4 => components(4),
			Self::Texture => (!typed.is_empty()).then(|| json!(typed)),
		}
	}
}

/// How a parameter's `value` reads in its field: numbers apart by commas, or the color name or asset ID as it is.
pub(crate) fn shown(value: &Value) -> String {
	match value {
		Value::String(text) => text.clone(),
		Value::Array(items) => items.iter().map(shown).collect::<Vec<_>>().join(", "),
		Value::Null => "–".to_owned(),
		other => other.to_string(),
	}
}

impl Variant {
	/// Opens the variant `id` from its parsed source, `variant`, and its parent's, which `read` returns for the parent's
	/// asset ID. Fails when the variant has no parent, the parent can't be read, or it declares no parameters or one of
	/// a type the window can't show.
	pub(crate) fn open(id: &str, variant: Value, read: impl FnOnce(&str) -> Result<Value, String>) -> Result<Self, String> {
		let parent_id = variant.get("parent").and_then(Value::as_str).ok_or_else(|| {
			format!("'{id}' has no `parent`. The most likely cause is a root material opened as a variant. Open its graph instead.")
		})?;
		let parent = read(parent_id)?;
		let declared = parent.get("variables").and_then(Value::as_array).ok_or_else(|| {
			format!("Material '{parent_id}' has no `variables` array. The most likely cause is a material written by hand without one. Give it a `variables` array, even an empty one.")
		})?;
		let given = variant.get("variables").and_then(Value::as_array);
		let value_of = |name: &str| {
			given?
				.iter()
				.find(|variable| variable.get("name").and_then(Value::as_str) == Some(name))?
				.get("value")
				.cloned()
		};
		let parameters = declared
			.iter()
			.map(|variable| {
				let name = variable.get("name").and_then(Value::as_str).unwrap_or_default();
				let data_type = variable.get("data_type").and_then(Value::as_str).unwrap_or_default();
				let kind = Kind::of(data_type).ok_or_else(|| {
					format!("Parameter '{name}' of '{parent_id}' has the data type '{data_type}'. The most likely cause is a typo in the material. Use `float`, `vec3f`, `vec4f`, or `Texture2D`.")
				})?;
				let value = value_of(name).or_else(|| variable.get("value").cloned()).unwrap_or(Value::Null);
				Ok(Parameter { name: name.to_owned(), kind, value })
			})
			.collect::<Result<_, String>>()?;
		Ok(Self {
			id: id.to_owned(),
			parent: parent_id.to_owned(),
			parameters,
			source: variant,
		})
	}

	/// The variant's file name, without the folders it's in.
	pub(crate) fn file(&self) -> &str {
		self.id.rsplit('/').next().unwrap_or(&self.id)
	}

	/// Gives the parameter `name` the value `value`, and returns the variant's new source text to write: its fields as
	/// they were, with the parameter's entry changed, or added when the variant left it out.
	pub(crate) fn set(&mut self, name: &str, value: Value) -> String {
		if let Some(parameter) = self.parameters.iter_mut().find(|parameter| parameter.name == name) {
			parameter.value = value.clone();
		}
		let Some(object) = self.source.as_object_mut() else {
			unreachable!("A variant opens only from an object, which has a `parent`.");
		};
		let variables = object.entry("variables").or_insert_with(|| json!([]));
		if !variables.is_array() {
			*variables = json!([]);
		}
		let variables = variables.as_array_mut().expect("`variables` was just made an array.");
		match variables
			.iter_mut()
			.find(|variable| variable.get("name").and_then(Value::as_str) == Some(name))
		{
			Some(variable) => variable["value"] = value,
			None => variables.push(json!({ "name": name, "value": value })),
		}
		super::create::pretty(&self.source)
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	fn parent() -> Value {
		json!({
			"graph": "paint.bemx",
			"variables": [
				{ "name": "albedo", "data_type": "Texture2D", "value": "rock.png" },
				{ "name": "tint", "data_type": "vec3f", "value": [1, 1, 1], "type": "Dynamic" },
				{ "name": "rough", "data_type": "float", "value": 0.5, "type": "Dynamic" },
			],
		})
	}

	#[test]
	fn a_variant_shows_every_parent_parameter_with_its_own_value_or_the_parents() {
		let variant = json!({ "parent": "paint.bema", "variables": [{ "name": "albedo", "value": "moss.png" }] });
		let variant = Variant::open("moss.bema", variant, |_| Ok(parent())).unwrap();
		assert_eq!(variant.parent, "paint.bema");
		let values: Vec<_> = variant.parameters.iter().map(|parameter| (parameter.kind, parameter.value.clone())).collect();
		assert_eq!(values, [(Kind::Texture, json!("moss.png")), (Kind::Vec3, json!([1, 1, 1])), (Kind::Float, json!(0.5))]);
		assert!(Variant::open("paint.bema", parent(), |_| Ok(parent())).is_err(), "a root material isn't a variant");
	}

	#[test]
	fn setting_a_parameter_changes_its_entry_or_adds_one_and_keeps_the_rest() {
		let source = json!({ "parent": "paint.bema", "double_sided": true, "variables": [{ "name": "albedo", "value": "moss.png" }] });
		let mut variant = Variant::open("moss.bema", source, |_| Ok(parent())).unwrap();
		variant.set("albedo", json!("bark.png"));
		let text = variant.set("rough", json!(0.25));
		let written = resource_management::asset::parse_json(&text).unwrap();
		assert_eq!(
			written,
			json!({
				"parent": "paint.bema",
				"double_sided": true,
				"variables": [{ "name": "albedo", "value": "bark.png" }, { "name": "rough", "value": 0.25 }],
			})
		);
		assert_eq!(variant.parameters[2].value, json!(0.25));
	}

	#[test]
	fn a_typed_value_reads_by_the_parameters_kind() {
		assert_eq!(Kind::Float.parse(" 0.75 "), Some(json!(0.75)));
		assert_eq!(Kind::Float.parse("half"), None);
		assert_eq!(Kind::Vec3.parse("1, 0.5 0"), Some(json!([1.0, 0.5, 0.0])));
		assert_eq!(Kind::Vec3.parse("purple"), Some(json!("Purple")));
		assert_eq!(Kind::Vec3.parse("1, 2"), None);
		assert_eq!(Kind::Vec4.parse("1 1 1 0.5"), Some(json!([1.0, 1.0, 1.0, 0.5])));
		assert_eq!(Kind::Texture.parse(" moss.png "), Some(json!("moss.png")));
		assert_eq!(Kind::Texture.parse("  "), None);
		assert_eq!(shown(&json!([1.0, 0.5, 0])), "1.0, 0.5, 0");
	}
}
