//! What the palette's New level, New material, and New variant commands write into the folder picked for them.
//!
//! The pure functions here build the IDs and source text; [`create`] reads what it needs from the application's
//! sources and writes the result through the inspector, which creates the folder when it doesn't exist.

use serde_json::{Value, json};

use super::palette::{self, Create};
use crate::application::graphics::GraphicsApplication;

/// Writes what `what` names into `folder`, as `/` or a folder ID such as `levels/`, under the first name no source
/// there has: `untitled.level`, `untitled.bema` with its graph `untitled.bemx`, or `<material> variant.bema`. Returns
/// what's edited next: a new material's graph, or the new variant.
pub(super) fn create(app: &GraphicsApplication, folder: &str, what: &Create) -> Option<palette::Entry> {
	let inspector = app.inspector();
	let created = match what {
		Create::Level => inspector.source_assets("level").and_then(|levels| {
			let id = format!("{}.level", free_stem(folder, "untitled", |stem| has(&levels, stem, "level")));
			inspector.write_source_asset(&id, EMPTY_LEVEL).map(|()| (id, None))
		}),
		Create::Material => inspector.source_assets("bema").and_then(|materials| {
			let graphs = inspector.source_assets("bemx")?;
			// The material and its graph share a name, so both must be free.
			let stem = free_stem(folder, "untitled", |stem| has(&materials, stem, "bema") || has(&graphs, stem, "bemx"));
			let (id, graph) = (format!("{stem}.bema"), format!("{stem}.bemx"));
			// The graph goes first, so the material's bake finds it.
			inspector.write_source_asset(&graph, EMPTY_GRAPH)?;
			let graph = palette::Entry::new(graph, palette::Kind::Graph);
			inspector.write_source_asset(&id, &material_source(&graph.label)).map(|()| (id, Some(graph)))
		}),
		Create::Variant(parent) => inspector.source_assets("bema").and_then(|materials| {
			let source = variant_source(parent, &read_material(app, parent)?)?;
			let base = format!("{} variant", palette::entity_name(parent));
			let id = format!("{}.bema", free_stem(folder, &base, |stem| has(&materials, stem, "bema")));
			let variant = palette::Entry::new(id.clone(), palette::Kind::Variant);
			inspector.write_source_asset(&id, &source).map(|()| (id, Some(variant)))
		}),
	};
	match created {
		Ok((id, next)) => {
			log::info!("The editor created '{id}'.");
			next
		}
		Err(error) => {
			log::warn!("The editor couldn't create the asset. {error}");
			None
		}
	}
}

/// Whether `ids` has `stem` with the `extension`.
fn has(ids: &[String], stem: &str, extension: &str) -> bool {
	ids.iter().any(|id| {
		id.strip_suffix(extension)
			.and_then(|id| id.strip_suffix('.'))
			.is_some_and(|id| id == stem)
	})
}

/// The root materials among the application's `.bema` sources, as their IDs with the IDs of their graphs. A source
/// that can't be read or parsed is left out, as is a variant.
pub(super) fn root_materials(app: &GraphicsApplication) -> Vec<(String, String)> {
	materials(app)
		.filter_map(|(id, material)| Some((id, root_graph(&material)?)))
		.collect()
}

/// The variants among the application's `.bema` sources: those with a `parent`. A source that can't be read or parsed
/// is left out.
pub(super) fn variants(app: &GraphicsApplication) -> Vec<String> {
	materials(app)
		.filter(|(_, material)| material.get("parent").is_some())
		.map(|(id, _)| id)
		.collect()
}

/// Every `.bema` source of the application that reads and parses, with its parsed source.
fn materials(app: &GraphicsApplication) -> impl Iterator<Item = (String, Value)> + '_ {
	let ids = app.inspector().source_assets("bema").unwrap_or_else(|error| {
		log::warn!("The palette couldn't list the game's materials. {error}");
		Vec::new()
	});
	ids.into_iter()
		.filter_map(|id| read_material(app, &id).ok().map(|material| (id, material)))
}

/// Reads and parses the `.bema` source `id`.
pub(super) fn read_material(app: &GraphicsApplication, id: &str) -> Result<Value, String> {
	let bytes = app.inspector().read_source_asset(id)?;
	let text = std::str::from_utf8(&bytes).map_err(|_| {
		format!("Material '{id}' isn't text. The most likely cause is a binary file named as a material. Save it as UTF-8 JSON5.")
	})?;
	resource_management::asset::parse_json(text).map_err(|error| {
		format!("Material '{id}' couldn't be read. The most likely cause is a syntax error in the file: {error}. Fix the file's JSON5.")
	})
}

/// The graph of `material`, a parsed `.bema` source, when it's a root material: one without a `parent`.
fn root_graph(material: &Value) -> Option<String> {
	if material.get("parent").is_some() {
		return None;
	}
	material.get("graph")?.as_str().map(str::to_owned)
}

/// The first of `base`, `base 2`, `base 3`, and so on, in `folder`, for which `taken` is false. The asset directory's
/// own folder, `/`, adds nothing to the ID.
fn free_stem(folder: &str, base: &str, taken: impl Fn(&str) -> bool) -> String {
	let folder = folder.strip_prefix('/').unwrap_or(folder);
	(1..)
		.map(|n| match n {
			1 => format!("{folder}{base}"),
			n => format!("{folder}{base} {n}"),
		})
		.find(|stem| !taken(stem))
		.expect("An unbounded count always finds a free name.")
}

/// The source of a level with no entities, which a new level starts as.
const EMPTY_LEVEL: &str = "{\n\t\"entities\": []\n}\n";

/// The source of a graph with only its surface node, whose inputs keep their defaults, which a new material's graph
/// starts as.
const EMPTY_GRAPH: &str = "{\n\tnodes: [\n\t\t{ name: \"surface\", node: \"surface\" },\n\t],\n}\n";

/// The source of a root material in the `World` domain, with no parameters, whose surface the graph `graph` computes.
fn material_source(graph: &str) -> String {
	pretty(&json!({ "domain": "World", "graph": graph, "variables": [] }))
}

/// The source of a variant of the root material `parent`, whose parsed source is `material`, that gives each of the
/// material's parameters the material's own value, so it bakes as the material does until a value changes.
fn variant_source(parent: &str, material: &Value) -> Result<String, String> {
	let variables = material.get("variables").and_then(Value::as_array).ok_or_else(|| {
		format!("Material '{parent}' has no `variables` array. The most likely cause is a material written by hand without one. Give it a `variables` array, even an empty one.")
	})?;
	let variables: Vec<Value> = variables
		.iter()
		.map(|variable| json!({ "name": variable.get("name"), "value": variable.get("value") }))
		.collect();
	Ok(pretty(&json!({ "parent": parent, "variables": variables })))
}

/// `value` as JSON indented with tabs, as the engine's sources are, which `.bema` files, being JSON5, read.
pub(super) fn pretty(value: &Value) -> String {
	use serde::Serialize;
	let mut text = Vec::new();
	let formatter = serde_json::ser::PrettyFormatter::with_indent(b"\t");
	value
		.serialize(&mut serde_json::Serializer::with_formatter(&mut text, formatter))
		.expect("A JSON value always serializes.");
	text.push(b'\n');
	String::from_utf8(text).expect("Serialized JSON is UTF-8.")
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn a_new_asset_takes_the_first_name_free_in_its_folder() {
		assert_eq!(free_stem("/", "untitled", |_| false), "untitled");
		let taken = ["levels/untitled", "levels/untitled 2", "untitled 3"];
		assert_eq!(free_stem("levels/", "untitled", |stem| taken.contains(&stem)), "levels/untitled 3");
	}

	#[test]
	fn a_new_material_names_its_graph_and_declares_no_parameters() {
		let material = resource_management::asset::parse_json(&material_source("m/untitled.bemx")).unwrap();
		assert_eq!(material, json!({ "domain": "World", "graph": "m/untitled.bemx", "variables": [] }));
		assert_eq!(root_graph(&material).as_deref(), Some("m/untitled.bemx"));
		resource_management::bemx::json::parse(EMPTY_GRAPH).expect("the new graph is valid");
	}

	#[test]
	fn a_variant_starts_with_its_materials_values_and_isnt_a_root() {
		let material = json!({
			"domain": "World",
			"graph": "paint.bemx",
			"variables": [
				{ "name": "color", "data_type": "vec3f", "value": [0.5, 0.5, 0.5], "type": "Dynamic" },
				{ "name": "albedo", "data_type": "Texture2D", "value": "rock.png" },
			],
		});
		let variant = resource_management::asset::parse_json(&variant_source("paint.bema", &material).unwrap()).unwrap();
		assert_eq!(
			variant,
			json!({
				"parent": "paint.bema",
				"variables": [{ "name": "color", "value": [0.5, 0.5, 0.5] }, { "name": "albedo", "value": "rock.png" }],
			})
		);
		assert_eq!(root_graph(&variant), None);
		assert!(variant_source("paint.bema", &json!({ "graph": "paint.bemx" })).is_err());
	}
}
