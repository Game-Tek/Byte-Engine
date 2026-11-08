//! Reads the material bindings of glTF, GLB, and FBX sidecars: the `asset.<material name>` entries that give a container
//! material an authored variant or an inline one.
//!
//! This module only reads and expands the settings. [`super::resolve_container_materials`] resolves the inline variants
//! it returns against the asset storage and bakes them.

use serde_json::{Map, Value};

/// The docs section that specifies material bindings.
pub(crate) const MATERIAL_BINDINGS_DOCS_PATH: &str = "reference/assets#material-bindings";

/// The key that binds every material of a model without a key of its own.
const ANY_MATERIAL_KEY: &str = "*";

/// The fields of an inline variant that replace the parent material's coverage settings.
const COVERAGE_FIELDS: [&str; 3] = ["alpha_mode", "alpha_cutoff", "double_sided"];

/// The `MaterialBinding` enum is what a model sidecar binds one container material to. glTF and FBX importers read it
/// with [`material_binding`] and turn it into a [`super::MaterialSource`].
pub(crate) enum MaterialBinding {
	/// The ID of an authored `.bema` variant, from `{ asset }`.
	Asset(String),
	/// A variant described in the sidecar itself.
	Inline(InlineBinding),
}

/// The `InlineBinding` struct is an inline variant with its templates expanded, before its candidate files are looked
/// up. Pass it to [`super::resolve_container_materials`] in a [`super::MaterialSource::Inline`].
#[derive(Debug, PartialEq)]
pub(crate) struct InlineBinding {
	/// The `asset` key that holds the binding, `*` or the material's name, for error messages.
	pub(crate) key: String,
	/// The parents to choose from, in order. A block without `select` has one rule that always matches.
	pub(crate) rules: Vec<ParentRule>,
	/// Each variable's name and value, in the order the sidecar lists them.
	pub(crate) variables: Vec<(String, BindingValue)>,
}

/// The `ParentRule` struct is one parent an [`InlineBinding`] may use, and the variables that must resolve for it to.
#[derive(Debug, PartialEq)]
pub(crate) struct ParentRule {
	/// Variable names that must all resolve for the rule to match. Empty matches always.
	pub(crate) requires: Vec<String>,
	/// The parent material's asset ID.
	pub(crate) parent: String,
	/// `alpha_mode`, `alpha_cutoff`, and `double_sided`: the block's, replaced field by field by the rule's. Read with
	/// the `.bema` variant rules, so a field left out keeps the parent's value.
	pub(crate) coverage: Value,
}

/// The `BindingValue` enum is one variable of an [`InlineBinding`].
#[derive(Debug, PartialEq)]
pub(crate) enum BindingValue {
	/// Asset IDs to try in order; the first that exists is the value. None existing leaves the variable unresolved.
	Candidates(Vec<String>),
	/// A number or an array of numbers, used as written. It always resolves.
	Literal(Value),
}

/// The `TemplateScope` struct holds the values of the template variables of one model material.
pub(crate) struct TemplateScope<'a> {
	/// The model's asset ID, such as `rocks/xb5ebhf/xb5ebhf_LOD0.fbx`.
	pub(crate) model_id: &'a str,
	/// The material's name, the value of `{material}`.
	pub(crate) material: &'a str,
}

impl TemplateScope<'_> {
	/// Returns the asset ID of the directory that holds the model, or `""` at the asset root.
	fn model_directory(&self) -> &str {
		self.model_id.rsplit_once('/').map_or("", |(directory, _)| directory)
	}

	/// Returns the value of the template variable `name`, or `None` for a name the templates don't define.
	fn variable(&self, name: &str) -> Option<&str> {
		match name {
			"dir" => Some(self.model_directory().rsplit('/').next().unwrap_or("")),
			"stem" => {
				let file = self.model_id.rsplit('/').next().unwrap_or(self.model_id);
				Some(file.rsplit_once('.').map_or(file, |(stem, _)| stem))
			}
			"material" => Some(self.material),
			_ => None,
		}
	}
}

/// Returns whether `spec` binds the material `name` to an authored `.bema` variant with `{ asset }`.
///
/// Such a material has no resource of its container's own, so a `materials/` fragment request never names it.
pub(crate) fn binds_variant_asset(spec: Option<&Value>, name: Option<&str>) -> bool {
	binding_entry(spec, name).is_some_and(|(_, entry)| entry.get("asset").is_some())
}

/// Reads what the merged sidecar settings `spec` bind the material `scope.material` to: its own `asset.<name>` entry,
/// else `asset."*"`. `name` is `None` for a material with no name, which only `"*"` binds.
///
/// Returns `None` when neither key is present, so the importer generates the material. Returns the problem, without
/// the model or key, when the entry is malformed; the caller reports it.
///
/// Next, pass the binding to [`super::resolve_container_materials`] as a [`super::MaterialSource`].
pub(crate) fn material_binding(
	spec: Option<&Value>,
	name: Option<&str>,
	scope: &TemplateScope<'_>,
) -> Result<Option<MaterialBinding>, String> {
	let Some((key, entry)) = binding_entry(spec, name) else {
		return Ok(None);
	};

	let in_key = |problem: String| format!("`asset.\"{key}\"` {problem}");

	let Some(entry) = entry.as_object() else {
		return Err(in_key(format!(
			"is {entry}, not an object. The most likely cause is a variant ID written without `{{ asset: ... }}` around it."
		)));
	};
	// Sidecar merging keeps `null`s, so a `null` field counts as left out. That lets a deeper `.bead` drop a field it
	// inherits, such as `select`.
	let entry = without_nulls(entry);
	let entry = &entry;

	// An authored variant takes precedence over any inline fields beside it.
	if let Some(asset) = entry.get("asset") {
		return match asset.as_str() {
			Some(asset) => Ok(Some(MaterialBinding::Asset(asset.to_string()))),
			None => Err(in_key(format!(
				"has the `asset` {asset}, which is not an asset ID. The most likely cause is a value written for another field."
			))),
		};
	}

	inline_binding(key, entry, scope)
		.map(|binding| Some(MaterialBinding::Inline(binding)))
		.map_err(in_key)
}

/// Finds the entry that binds the material `name`: `asset.<name>` when present, else `asset."*"`. Returns the key with
/// it. A named entry is used on its own, never merged with `"*"`.
fn binding_entry<'a, 'b>(spec: Option<&'a Value>, name: Option<&'b str>) -> Option<(&'b str, &'a Value)> {
	let bindings = spec?.get("asset")?;

	name.and_then(|name| bindings.get(name).filter(|entry| !entry.is_null()).map(|entry| (name, entry)))
		.or_else(|| {
			bindings
				.get(ANY_MATERIAL_KEY)
				.filter(|entry| !entry.is_null())
				.map(|entry| (ANY_MATERIAL_KEY, entry))
		})
}

/// Reads an inline variant entry and expands its templates in `scope`.
fn inline_binding(key: &str, entry: &Map<String, Value>, scope: &TemplateScope<'_>) -> Result<InlineBinding, String> {
	if let Some(unknown) = entry.keys().find(|field| {
		!matches!(field.as_str(), "parent" | "select" | "variables") && !COVERAGE_FIELDS.contains(&field.as_str())
	}) {
		return Err(format!(
			"has the unknown field `{unknown}`. The most likely cause is a misspelt field. Use `asset`, `parent`, `select`, `variables`, `alpha_mode`, `alpha_cutoff`, or `double_sided`."
		));
	}

	let coverage = coverage_fields(entry, &Map::new());

	let rules = match (entry.get("select"), entry.get("parent")) {
		(Some(select), _) => {
			let rules = select
				.as_array()
				.filter(|rules| !rules.is_empty())
				.ok_or("has a `select` that is not a list of rules. The most likely cause is a single rule written without `[ ]` around it, or an empty list.")?;
			rules
				.iter()
				.enumerate()
				.map(|(index, rule)| {
					parent_rule(rule, &coverage, scope).map_err(|problem| format!("rule {index} of `select` {problem}"))
				})
				.collect::<Result<Vec<_>, _>>()?
		}
		(None, Some(parent)) => vec![ParentRule {
			requires: Vec::new(),
			parent: parent_id(parent, scope)?,
			coverage: Value::Object(coverage),
		}],
		(None, None) => {
			return Err("has neither `asset`, `parent`, nor `select`. The most likely cause is a binding that lists only `variables`. Add the `parent` material's asset ID.".to_string());
		}
	};

	let variables = match entry.get("variables") {
		None => Vec::new(),
		Some(Value::Object(variables)) => without_nulls(variables)
			.iter()
			.map(|(name, value)| {
				binding_value(value, scope)
					.map(|value| (name.clone(), value))
					.map_err(|problem| format!("variable `{name}` {problem}"))
			})
			.collect::<Result<Vec<_>, _>>()?,
		Some(variables) => {
			return Err(format!(
				"has the `variables` {variables}, which is not an object keyed by parameter name. The most likely cause is a list of `{{ name, value }}` entries copied from a `.bema` variant. Write `variables: {{ <parameter>: <value> }}`."
			));
		}
	};

	Ok(InlineBinding {
		key: key.to_string(),
		rules,
		variables,
	})
}

/// Returns the fields of `object` that aren't `null`.
fn without_nulls(object: &Map<String, Value>) -> Map<String, Value> {
	object
		.iter()
		.filter(|(_, value)| !value.is_null())
		.map(|(key, value)| (key.clone(), value.clone()))
		.collect()
}

/// Reads one rule of `select`, whose coverage fields replace `block`'s.
fn parent_rule(rule: &Value, block: &Map<String, Value>, scope: &TemplateScope<'_>) -> Result<ParentRule, String> {
	let rule = rule
		.as_object()
		.ok_or("is not an object. The most likely cause is a parent ID written without `{ parent: ... }` around it.")?;

	if let Some(unknown) = rule
		.keys()
		.find(|field| !matches!(field.as_str(), "requires" | "parent") && !COVERAGE_FIELDS.contains(&field.as_str()))
	{
		return Err(format!(
			"has the unknown field `{unknown}`. The most likely cause is a misspelt field. Use `requires`, `parent`, `alpha_mode`, `alpha_cutoff`, or `double_sided`."
		));
	}

	let requires = match rule.get("requires") {
		None => Vec::new(),
		Some(requires) => requires
			.as_array()
			.and_then(|names| names.iter().map(|name| name.as_str().map(ToString::to_string)).collect::<Option<Vec<_>>>())
			.ok_or("has a `requires` that is not a list of variable names. The most likely cause is a single name written without `[ ]` around it.")?,
	};

	let parent = rule.get("parent").ok_or(
		"has no `parent`. The most likely cause is a rule that lists only `requires`. Add the parent material's asset ID.",
	)?;

	Ok(ParentRule {
		requires,
		parent: parent_id(parent, scope)?,
		coverage: Value::Object(coverage_fields(rule, block)),
	})
}

/// Returns `block`'s coverage fields replaced by those `fields` holds.
fn coverage_fields(fields: &Map<String, Value>, block: &Map<String, Value>) -> Map<String, Value> {
	let mut coverage = block.clone();
	for field in COVERAGE_FIELDS {
		if let Some(value) = fields.get(field) {
			coverage.insert(field.to_string(), value.clone());
		}
	}
	coverage
}

/// Expands a `parent` template into the parent material's asset ID.
fn parent_id(parent: &Value, scope: &TemplateScope<'_>) -> Result<String, String> {
	let parent = parent.as_str().ok_or_else(|| {
		format!(
			"has the `parent` {parent}, which is not an asset ID. The most likely cause is a value written for another field."
		)
	})?;
	asset_id(parent, scope)
}

/// Reads one variable value: a template or a list of templates to try in order, or a literal.
fn binding_value(value: &Value, scope: &TemplateScope<'_>) -> Result<BindingValue, String> {
	match value {
		Value::String(template) => Ok(BindingValue::Candidates(vec![asset_id(template, scope)?])),
		Value::Array(items) if !items.is_empty() && items.iter().all(Value::is_string) => items
			.iter()
			.filter_map(Value::as_str)
			.map(|template| asset_id(template, scope))
			.collect::<Result<_, _>>()
			.map(BindingValue::Candidates),
		Value::Number(_) => Ok(BindingValue::Literal(value.clone())),
		Value::Array(items) if !items.is_empty() && items.iter().all(Value::is_number) => {
			Ok(BindingValue::Literal(value.clone()))
		}
		_ => Err(format!(
			"is {value}, which is neither an asset ID, a list of asset IDs, a number, nor a list of numbers. The most likely cause is a list that mixes IDs and numbers, or a named color, which inline variants don't accept. Write colors as arrays of numbers."
		)),
	}
}

/// Expands `template` in `scope` and makes the result an asset ID: a leading `./` is the model's directory.
fn asset_id(template: &str, scope: &TemplateScope<'_>) -> Result<String, String> {
	let expanded = expand_template(template, scope)?;

	Ok(match expanded.strip_prefix("./") {
		Some(relative) => match scope.model_directory() {
			"" => relative.to_string(),
			directory => format!("{directory}/{relative}"),
		},
		None => expanded,
	})
}

/// Expands an RFC 6570 level 1 template: each `{name}` becomes the variable's value, as it is, without percent-encoding,
/// since the result is an asset ID rather than a URI.
pub(crate) fn expand_template(template: &str, scope: &TemplateScope<'_>) -> Result<String, String> {
	let mut expanded = String::with_capacity(template.len());
	let mut rest = template;

	while let Some(open) = rest.find(['{', '}']) {
		if rest.as_bytes()[open] == b'}' {
			return Err(format!(
				"has the template '{template}' with a `}}` that closes no `{{`. The most likely cause is a misplaced brace. Write variables as `{{dir}}`, `{{stem}}`, or `{{material}}`."
			));
		}

		expanded.push_str(&rest[..open]);
		let after = &rest[open + 1..];
		let close = after.find('}').ok_or_else(|| {
			format!(
				"has the template '{template}' with a `{{` that is never closed. The most likely cause is a missing `}}`. Write variables as `{{dir}}`, `{{stem}}`, or `{{material}}`."
			)
		})?;
		let name = &after[..close];
		let value = scope.variable(name).ok_or_else(|| {
			format!(
				"has the template '{template}' with the unknown variable `{{{name}}}`. The most likely cause is a misspelt name or an RFC 6570 operator, which bindings don't support. Use `{{dir}}`, `{{stem}}`, or `{{material}}`."
			)
		})?;
		expanded.push_str(value);
		rest = &after[close + 1..];
	}

	expanded.push_str(rest);
	Ok(expanded)
}

#[cfg(test)]
mod tests {
	use super::*;

	const SCOPE: TemplateScope<'static> = TemplateScope {
		model_id: "quarry/xb5ebhf/uddmcgbia_VarA_LOD0.fbx",
		material: "MatID_1",
	};

	fn binding(spec: Value, name: Option<&str>) -> Result<Option<MaterialBinding>, String> {
		material_binding(Some(&spec), name, &SCOPE)
	}

	fn inline(spec: Value, name: Option<&str>) -> InlineBinding {
		match binding(spec, name) {
			Ok(Some(MaterialBinding::Inline(binding))) => binding,
			Ok(Some(MaterialBinding::Asset(asset))) => panic!("expected an inline binding, got the asset '{asset}'"),
			Ok(None) => panic!("expected an inline binding, got none"),
			Err(problem) => panic!("expected an inline binding, got: {problem}"),
		}
	}

	#[test]
	fn templates_expand_level_one_variables() {
		assert_eq!(
			expand_template("{dir}/{stem}_{material}.jpg", &SCOPE).unwrap(),
			"xb5ebhf/uddmcgbia_VarA_LOD0_MatID_1.jpg"
		);
		assert_eq!(expand_template("no variables", &SCOPE).unwrap(), "no variables");
	}

	#[test]
	fn malformed_templates_fail() {
		for template in ["{size}", "{+dir}", "{dir", "dir}"] {
			assert!(expand_template(template, &SCOPE).is_err(), "'{template}' expanded");
		}
	}

	#[test]
	fn dot_slash_is_the_models_directory() {
		assert_eq!(
			asset_id("./{dir}_Normal.jpg", &SCOPE).unwrap(),
			"quarry/xb5ebhf/xb5ebhf_Normal.jpg"
		);
		assert_eq!(asset_id("materials/a.bema", &SCOPE).unwrap(), "materials/a.bema");

		let root = TemplateScope {
			model_id: "rock.glb",
			material: "rock",
		};
		assert_eq!(asset_id("./{dir}{stem}.png", &root).unwrap(), "rock.png");
	}

	#[test]
	fn a_named_key_replaces_the_wildcard() {
		let spec = serde_json::json!({ "asset": {
			"*": { "parent": "any.bema", "variables": { "a": "./a.png" } },
			"Billboard": { "parent": "billboard.bema" },
		}});

		let named = inline(spec.clone(), Some("Billboard"));
		assert_eq!(named.key, "Billboard");
		assert_eq!(named.rules[0].parent, "billboard.bema");
		assert!(named.variables.is_empty(), "a named key isn't merged with `*`");

		let other = inline(spec.clone(), Some("MatID_1"));
		assert_eq!(other.key, "*");
		assert_eq!(
			other.variables,
			vec![(
				"a".to_string(),
				BindingValue::Candidates(vec!["quarry/xb5ebhf/a.png".to_string()])
			)]
		);

		assert_eq!(inline(spec, None).key, "*", "an unnamed material takes `*`");
	}

	#[test]
	fn an_asset_takes_precedence_over_inline_fields() {
		let spec = serde_json::json!({ "asset": { "MatID_1": { "asset": "rock.bema", "parent": "ignored.bema" } } });
		assert!(
			matches!(binding(spec.clone(), Some("MatID_1")), Ok(Some(MaterialBinding::Asset(asset))) if asset == "rock.bema")
		);
		assert!(binds_variant_asset(Some(&spec), Some("MatID_1")));
		assert!(!binds_variant_asset(Some(&spec), Some("Atlas")));
	}

	#[test]
	fn no_entry_means_generated() {
		assert!(matches!(
			binding(
				serde_json::json!({ "asset": { "Atlas": { "parent": "a.bema" } } }),
				Some("MatID_1")
			),
			Ok(None)
		));
		assert!(matches!(binding(serde_json::json!({}), Some("MatID_1")), Ok(None)));
	}

	#[test]
	fn select_rules_inherit_and_replace_the_blocks_coverage() {
		let spec = serde_json::json!({ "asset": { "*": {
			"double_sided": true,
			"select": [
				{ "requires": ["opacity"], "parent": "masked.bema", "alpha_cutoff": 0.3 },
				{ "parent": "{material}.bema", "double_sided": false },
			],
			"variables": { "opacity": ["./{dir}_Opacity.jpg", "./Opacity.png"], "tint": [1.0, 0.5, 0.5] },
		}}});

		let binding = inline(spec, Some("MatID_1"));
		assert_eq!(
			binding.rules,
			vec![
				ParentRule {
					requires: vec!["opacity".to_string()],
					parent: "masked.bema".to_string(),
					coverage: serde_json::json!({ "double_sided": true, "alpha_cutoff": 0.3 }),
				},
				ParentRule {
					requires: Vec::new(),
					parent: "MatID_1.bema".to_string(),
					coverage: serde_json::json!({ "double_sided": false }),
				},
			]
		);
		assert_eq!(
			binding.variables,
			vec![
				(
					"opacity".to_string(),
					BindingValue::Candidates(vec![
						"quarry/xb5ebhf/xb5ebhf_Opacity.jpg".to_string(),
						"quarry/xb5ebhf/Opacity.png".to_string()
					])
				),
				("tint".to_string(), BindingValue::Literal(serde_json::json!([1.0, 0.5, 0.5]))),
			]
		);
	}

	#[test]
	fn malformed_entries_name_the_key() {
		for entry in [
			serde_json::json!("rock.bema"),
			serde_json::json!({ "variables": {} }),
			serde_json::json!({ "parent": "a.bema", "variables": [{ "name": "a", "value": 1 }] }),
			serde_json::json!({ "parent": "a.bema", "variables": { "a": ["./a.png", 1] } }),
			serde_json::json!({ "parent": "a.bema", "variables": { "a": "{unknown}" } }),
			serde_json::json!({ "parent": "a.bema", "colour": "Red" }),
			serde_json::json!({ "select": [{ "requires": "opacity", "parent": "a.bema" }] }),
		] {
			let problem = match binding(serde_json::json!({ "asset": { "MatID_1": entry } }), Some("MatID_1")) {
				Err(problem) => problem,
				Ok(_) => panic!("{entry} was accepted"),
			};
			assert!(problem.starts_with("`asset.\"MatID_1\"`"), "{problem}");
		}
	}
}
