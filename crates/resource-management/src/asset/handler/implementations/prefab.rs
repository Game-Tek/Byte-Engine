//! Prefab asset baking.
//!
//! A `.prefab` file is a level's entities plus parameters, so it bakes through the level's layout: the same checks,
//! order, and tokens, with `$parameter` references allowed and each parameter's default stored with the values.

use super::{
	ResourceId,
	handler::{AssetHandler, BakeContext, LoadErrors},
	level::{EntitySource, LaidOut, Object, Value, lay_out, parse, write_plain_value},
};
use crate::{
	ProcessedAsset,
	resources::{
		level::{NamedValue, TokenWriter},
		prefab::{ParameterValues, Prefab},
	},
};

/// The `PrefabAssetHandler` struct exists to bake `.prefab` files into prefabs that spawn without parsing text.
///
/// Register it with the asset manager next to the level handler; the engine's default baking does. Next, spawn a
/// prefab through the engine's spawner, directly or from a level instance.
#[derive(Default)]
pub struct PrefabAssetHandler;

impl AssetHandler for PrefabAssetHandler {
	fn can_handle(&self, r#type: &str) -> bool {
		r#type == "prefab"
	}

	fn identity(&self) -> u64 {
		0
	}

	async fn bake<'a>(&'a self, context: BakeContext<'a>, id: ResourceId<'a>) -> Result<PrimaryStored, LoadErrors> {
		let source = context.resolve(id).await?;
		let (prefab, payload) = bake_prefab(&source).map_err(|cause| {
			log::error!("Prefab '{}' could not be baked. {cause}", id.as_ref());
			LoadErrors::FailedToProcess
		})?;
		context.store_primary_owned(ProcessedAsset::new(id, prefab), payload).await
	}
}

/// Parses a `.prefab` source, checks it, and lays it out as the prefab's model and payload.
///
/// Tools and tests that spawn a prefab from memory call it directly. Returns the cause and fix of the first problem
/// found, ready to follow a statement of what failed.
pub fn bake_prefab(source: &[u8]) -> Result<(Prefab, Vec<u8>), String> {
	let PrefabSource {
		parameters, entities, ..
	} = parse(source)?;
	let LaidOut {
		level,
		payload,
		defaults: parameters,
		..
	} = lay_out(&entities, Some(&parameters))?;
	Ok((Prefab { level, parameters }, payload))
}

/// Encodes `values`, an object from parameter name to value, as the parameter values of one spawn.
///
/// Use it when code spawns a prefab with its own values, such as a unit's team. Returns the cause and fix when
/// `values` isn't an object or holds a number JSON can't represent.
pub fn encode_parameter_values(values: serde_json::Value) -> Result<ParameterValues, String> {
	let serde_json::Value::Object(fields) = values else {
		return Err(
			"The most likely cause is that the values aren't an object from parameter name to value. Pass a struct or a map."
				.to_string(),
		);
	};
	let mut writer = TokenWriter::default();
	let mut names = Vec::with_capacity(fields.len());
	for (name, value) in fields {
		names.push(NamedValue {
			name: name.clone(),
			value: writer.offset() as u32,
		});
		let value =
			Value::try_from(value).map_err(|cause| format!("The most likely cause is that parameter '{name}' {cause}"))?;
		write_plain_value(&mut writer, &value)
			.map_err(|cause| format!("The most likely cause is that parameter '{name}' {cause}"))?;
	}
	let (values, strings) = writer.finish();
	Ok(ParameterValues { names, values, strings })
}

/// The `PrefabSource` struct is the authored form of a prefab.
#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct PrefabSource {
	/// Points editors at the schema; baking ignores it.
	#[serde(rename = "$schema", default)]
	_schema: Option<serde::de::IgnoredAny>,
	#[serde(default)]
	parameters: Object,
	entities: Vec<EntitySource>,
}

#[cfg(test)]
mod tests {
	use super::bake_prefab;
	use crate::resources::level::{Token, Tokens};

	/// Reads the tokens of the first payload of kind `kind`.
	fn first_payload(source: &str, kind: &str) -> Vec<String> {
		let (prefab, payload) = bake_prefab(source.as_bytes()).expect("the prefab should bake");
		let column = prefab
			.level
			.kinds
			.iter()
			.find(|column| column.name == kind)
			.expect("the kind");
		let strings = column.strings(&payload).unwrap();
		let tokens = column.run(&payload, &strings, 0).unwrap();
		collect_value(tokens)
	}

	/// Reads one whole value from `tokens`, as debug strings.
	fn collect_value(tokens: Tokens<'_, '_>) -> Vec<String> {
		let mut depth = 0i32;
		let mut read = Vec::new();
		for token in tokens {
			let token = token.unwrap();
			match token {
				Token::SequenceStart | Token::ObjectStart => depth += 1,
				Token::SequenceEnd | Token::ObjectEnd => depth -= 1,
				_ => {}
			}
			read.push(format!("{token:?}"));
			if depth == 0 && !matches!(token, Token::Key(_)) {
				break;
			}
		}
		read
	}

	#[test]
	fn parameter_references_bake_to_the_parameter_index_and_defaults_are_stored() {
		let source = r#"{
			parameters: { speed: 3.5, team: "red" },
			entities: [{ components: { patrol: { team: { "$parameter": "team" } } } }],
		}"#;
		assert_eq!(
			first_payload(source, "patrol"),
			["ObjectStart", "Key(\"team\")", "Parameter(1)", "ObjectEnd"]
		);

		let (prefab, payload) = bake_prefab(source.as_bytes()).unwrap();
		let names = prefab
			.parameters
			.iter()
			.map(|parameter| parameter.name.as_str())
			.collect::<Vec<_>>();
		assert_eq!(names, ["speed", "team"]);
		let strings = prefab.level.value_strings(&payload).unwrap();
		let default = prefab.level.value(&payload, &strings, prefab.parameters[1].value).unwrap();
		assert_eq!(collect_value(default), ["String(\"red\")"]);
	}

	#[test]
	fn instances_store_their_prefab_and_parameter_values() {
		let source = r#"{
			parameters: { damage: 4 },
			entities: [
				{ id: "hand" },
				{ parent: "hand", prefab: { resource: "spear.prefab", parameters: { damage: { "$parameter": "damage" } } } },
			],
		}"#;
		let (prefab, payload) = bake_prefab(source.as_bytes()).unwrap();
		let [instance] = prefab.level.instances.as_slice() else {
			panic!("one instance");
		};
		assert_eq!((instance.entity, instance.resource.as_str()), (1, "spear.prefab"));
		let strings = prefab.level.value_strings(&payload).unwrap();
		let value = prefab.level.value(&payload, &strings, instance.parameters[0].value).unwrap();
		assert_eq!(collect_value(value), ["Parameter(0)"]);
	}

	#[test]
	fn prefabs_that_break_the_parameter_rules_are_rejected() {
		let rejected = [
			// A parameter the prefab doesn't declare.
			r#"{ entities: [{ components: { patrol: { "$parameter": "speed" } } }] }"#,
			// A default can't refer to a parameter or an entity.
			r#"{ parameters: { a: 1, b: { "$parameter": "a" } }, entities: [] }"#,
			r#"{ parameters: { a: { "$entity": "x" } }, entities: [{ id: "x" }] }"#,
			// A malformed reference.
			r#"{ parameters: { a: 1 }, entities: [{ components: { patrol: { "$parameter": "a", extra: 1 } } }] }"#,
			// An instance without a resource, and an empty parameter name.
			r#"{ entities: [{ prefab: { parameters: {} } }] }"#,
			r#"{ parameters: { "": 1 }, entities: [] }"#,
		];
		for source in rejected {
			assert!(bake_prefab(source.as_bytes()).is_err(), "{source} should be rejected");
		}
	}
}

use crate::asset::handler::PrimaryStored;
