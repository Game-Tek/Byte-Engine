//! Level asset baking.
//!
//! Baking does everything that doesn't need the application's component types: it parses the JSON5 text, checks
//! ids, parents, and references, orders entities parents first, and groups the payloads by kind as parsed tokens.
//! What's left for spawning is decoding each kind's tokens into its type and publishing it.
//!
//! A handler made with [`LevelAssetHandler::merging`] also reads one kind pair, `mesh` with `transform`: the
//! entities that place one mesh more often than its threshold become a few entities, each a cluster of neighboring
//! placements that a renderer builds one merged mesh from. [`repeated_placements`] finds those entities, the handler
//! reads how many copies of the mesh fit a cluster, and [`cluster_entities`] rewrites them. Nothing merged is stored.
//! The default handler merges nothing: a cluster picks one detail level for all its copies, which shows as popping.

use std::collections::{BTreeMap, HashMap};

use super::{
	ResourceId,
	handler::{AssetHandler, BakeContext, LoadErrors},
};
use crate::{
	ProcessedAsset,
	processors::mesh::{cluster_placements, copies_that_fit},
	resources::{
		level::{Level, LevelKind, NO_PARENT, RUN, Section, Token, TokenWriter},
		mesh::{Mesh, MeshModel},
	},
};

/// The `LevelAssetHandler` struct exists to bake `.level` files into levels that spawn without parsing text.
///
/// The default handler keeps every entity. Register one made with [`Self::merging`] to draw heavily repeated
/// placements as merged clusters.
#[derive(Default)]
pub struct LevelAssetHandler {
	/// A mesh resource that a level places more often than this bakes into merged clusters of its placements.
	merge_threshold: Option<usize>,
}

impl LevelAssetHandler {
	/// Creates a handler that bakes each mesh resource a level places more than `threshold` times into clusters of
	/// its placements, which the visibility pipeline draws as one merged mesh each.
	///
	/// Use it for scenes whose per-instance cost matters more than smooth detail changes: every copy of a cluster
	/// changes detail level together. Register it with the asset manager in place of the default handler.
	pub fn merging(threshold: usize) -> Self {
		Self {
			merge_threshold: Some(threshold),
		}
	}
}

impl AssetHandler for LevelAssetHandler {
	fn can_handle(&self, r#type: &str) -> bool {
		r#type == "level"
	}

	async fn bake<'a>(&'a self, context: BakeContext<'a>, id: ResourceId<'a>) -> Result<(), LoadErrors> {
		let (source, format) = context.resolve(id).await?;

		if format != "level" {
			return Err(LoadErrors::UnsupportedType);
		}

		let report = |cause: String| {
			log::error!("Level '{}' could not be baked. {cause}", id.as_ref());
			LoadErrors::FailedToProcess
		};
		let entities = parse(&source).map_err(report)?;

		// Each mesh placed often enough merges, as many copies to a cluster as fit one primitive of the mesh.
		let mut merges = Vec::new();
		let groups = self
			.merge_threshold
			.map_or_else(Vec::new, |threshold| repeated_placements(&entities, threshold));
		for group in groups {
			if let Some(capacity) = cluster_capacity(&context, &group).await {
				merges.push((group, capacity));
			}
		}
		let entities = cluster_entities(entities, &merges);

		let (level, payload) = lay_out(&entities).map_err(report)?;

		context.store_primary_owned(ProcessedAsset::new(id, level), payload).await
	}
}

/// Returns how many placements of `group`'s mesh one cluster holds, or `None` for a mesh that can't merge.
///
/// A mesh that can't merge is reported and left to its entities, since the level still draws without the merge.
async fn cluster_capacity(context: &BakeContext<'_>, group: &MergeGroup) -> Option<usize> {
	let leave = |cause: &str| {
		log::warn!(
			"Mesh '{}' is not merged, so its {} placements draw one by one. {cause}",
			group.mesh,
			group.entities.len()
		);
		None
	};
	const UNBAKED: &str =
		"The most likely cause is that the mesh doesn't bake. Fix the mesh's `resource` ID or the errors its bake reports.";

	// Baking the mesh as a dependency also bakes the level again when the mesh changes, and with it the clusters.
	if context.bake_dependency::<MeshModel>(&group.mesh).await.is_err() {
		return leave(UNBAKED);
	}
	let Ok(mesh) = context.resource_manager().request::<Mesh>(&group.mesh).await else {
		return leave(UNBAKED);
	};
	let primitives = &mesh.resource().primitives;
	if primitives
		.iter()
		.any(|primitive| primitive.skin.is_some() || primitive.transform_node.is_some())
	{
		return leave(
			"The most likely cause is that the mesh is skinned or animated, and merged geometry never moves. Place fewer copies, or use a static mesh.",
		);
	}
	// A cluster repeats every primitive of the mesh, so the primitive that fits the fewest copies bounds it.
	match primitives.iter().map(copies_that_fit).min() {
		Some(capacity) if capacity > 0 => Some(capacity),
		_ => leave("The most likely cause is that the mesh has no primitives. Fix the mesh, or place another one."),
	}
}

/// The `MergeGroup` struct is one mesh that a level places often enough to merge, and where.
struct MergeGroup {
	/// The mesh resource every entity of the group draws.
	mesh: String,
	/// The file position of each entity of the group, ascending.
	entities: Vec<usize>,
	/// Where each entity places the mesh, in the order of `entities`.
	positions: Vec<[f32; 3]>,
}

/// Finds the meshes that `entities` place more than `threshold` times, with the entities that place them.
///
/// Only an entity that nothing can refer to and that holds nothing but the placement counts: no `id`, and no
/// component besides `mesh` with a `resource` source and `transform`. Entities group by mesh and parent, so a merged
/// group still belongs to one parent.
fn repeated_placements(entities: &[EntitySource], threshold: usize) -> Vec<MergeGroup> {
	let mut groups = Vec::<MergeGroup>::new();
	// Where each mesh and parent pair's group is, so groups keep the order their first entity has in the file.
	let mut indices = HashMap::<(&str, Option<&str>), usize>::new();
	for (position, entity) in entities.iter().enumerate() {
		let Some((mesh, placed)) = static_placement(entity) else {
			continue;
		};
		let index = *indices.entry((mesh, entity.parent.as_deref())).or_insert_with(|| {
			groups.push(MergeGroup {
				mesh: mesh.to_string(),
				entities: Vec::new(),
				positions: Vec::new(),
			});
			groups.len() - 1
		});
		groups[index].entities.push(position);
		groups[index].positions.push(placed);
	}
	groups.retain(|group| group.entities.len() > threshold);
	groups
}

/// Replaces the entities of each group in `merges` with one entity for each cluster of up to the paired number of
/// neighboring placements, and returns the level's entities.
///
/// A cluster entity holds a `mesh` with the `merged` source: the group's mesh and its placements' transforms. The
/// clusters take the place of the group's first entity, and belong to the group's parent.
fn cluster_entities(mut entities: Vec<EntitySource>, merges: &[(MergeGroup, usize)]) -> Vec<EntitySource> {
	// The cluster entities that take each entity's place, and whether the entity itself leaves.
	let mut clusters_at: Vec<Vec<EntitySource>> = entities.iter().map(|_| Vec::new()).collect();
	let mut merged = vec![false; entities.len()];
	for (group, capacity) in merges {
		let (order, clusters) = cluster_placements(&group.positions, *capacity);
		let first = group.entities[0];
		for cluster in clusters {
			let placements = order[cluster]
				.iter()
				.map(|&member| {
					// A placement moves its transform into the cluster. One without a transform stands at the origin.
					let entity = &mut entities[group.entities[member as usize]];
					entity
						.components
						.0
						.iter_mut()
						.find(|(kind, _)| kind == "transform")
						.map_or_else(
							|| Value::Object(Object::default()),
							|(_, transform)| std::mem::replace(transform, Value::Null),
						)
				})
				.collect();
			let source = Object(vec![
				("resource".to_string(), Value::String(group.mesh.clone())),
				("placements".to_string(), Value::Sequence(placements)),
			]);
			clusters_at[first].push(EntitySource {
				id: None,
				parent: entities[first].parent.clone(),
				components: Object(vec![(
					"mesh".to_string(),
					Value::Object(Object(vec![("merged".to_string(), Value::Object(source))])),
				)]),
			});
		}
		for &entity in &group.entities {
			merged[entity] = true;
		}
	}

	let mut result = Vec::with_capacity(entities.len());
	for ((entity, clusters), merged) in entities.into_iter().zip(clusters_at).zip(merged) {
		result.extend(clusters);
		if !merged {
			result.push(entity);
		}
	}
	result
}

/// Returns the mesh resource `entity` places and where, when the entity is nothing but that placement.
fn static_placement(entity: &EntitySource) -> Option<(&str, [f32; 3])> {
	if entity.id.is_some() {
		return None;
	}
	let (mut mesh, mut transform) = (None, None);
	for (kind, value) in &entity.components.0 {
		match kind.as_str() {
			"mesh" => mesh = Some(value),
			"transform" => transform = Some(value),
			_ => return None,
		}
	}
	let Value::Object(Object(source)) = mesh? else {
		return None;
	};
	let [(source, Value::String(resource))] = source.as_slice() else {
		return None;
	};
	(source == "resource").then_some((resource.as_str(), placed_position(transform)?))
}

/// Returns the position a `transform` payload places its entity at, where a missing payload or field takes the
/// identity's value.
///
/// Returns `None` for a payload the `transform` kind rejects, so its entity stays and the spawn reports it, and for
/// a zero scale, which leaves no surface to merge.
fn placed_position(transform: Option<&Value>) -> Option<[f32; 3]> {
	let (mut position, mut scale, mut orientation) = ([0.0; 3], [1.0; 3], [0.0, 0.0, 0.0, 1.0]);
	match transform {
		None => {}
		Some(Value::Object(Object(fields))) => {
			for (field, value) in fields {
				match field.as_str() {
					"position" => position = numbers(value)?,
					"scale" => scale = numbers(value)?,
					"orientation" => orientation = numbers(value)?,
					_ => return None,
				}
			}
		}
		Some(_) => return None,
	}
	if scale.contains(&0.0) {
		return None;
	}
	math::Orientation::try_from_array(orientation).ok()?;
	Some(position)
}

/// Reads a sequence of exactly `N` finite numbers.
fn numbers<const N: usize>(value: &Value) -> Option<[f32; N]> {
	let Value::Sequence(values) = value else {
		return None;
	};
	let values: &[Value; N] = values.as_slice().try_into().ok()?;
	let mut numbers = [0.0; N];
	for (number, value) in numbers.iter_mut().zip(values) {
		*number = match value {
			Value::Unsigned(value) => *value as f32,
			Value::Signed(value) => *value as f32,
			Value::Float(value) => *value as f32,
			_ => return None,
		};
		if !number.is_finite() {
			return None;
		}
	}
	Some(numbers)
}

/// Parses a `.level` source, checks it, and lays it out as the level's model and payload.
///
/// Tools and tests that spawn a level from memory call it directly. It clusters no placements, which needs the baked
/// meshes: bake the level through its asset handler for that. Returns the cause and fix of the first problem found,
/// ready to follow a statement of what failed.
pub fn bake_level(source: &[u8]) -> Result<(Level, Vec<u8>), String> {
	lay_out(&parse(source)?)
}

/// Parses a `.level` source into its entities, in file order.
fn parse(source: &[u8]) -> Result<Vec<EntitySource>, String> {
	let text = std::str::from_utf8(source)
		.map_err(|_| "The most likely cause is that the file isn't UTF-8 text. Save it as UTF-8.".to_string())?;
	let LevelSource { entities, .. } = json5::from_str(text).map_err(|error| {
		format!("The most likely cause is that the file doesn't hold a level: {error}. Fix the reported field.")
	})?;
	Ok(entities)
}

/// Checks `entities` and lays them out as the level's model and payload.
fn lay_out(entities: &[EntitySource]) -> Result<(Level, Vec<u8>), String> {
	let count = u32::try_from(entities.len())
		.ok()
		.filter(|&count| count < NO_PARENT)
		.ok_or_else(|| {
			format!(
				"The most likely cause is that it places {} entities. Split it into smaller levels.",
				entities.len()
			)
		})?;

	let ids = index_ids(entities)?;
	let parents = entities
		.iter()
		.enumerate()
		.map(|(index, entity)| {
			entity
				.parent
				.as_deref()
				.map(|parent| {
					ids.get(parent).copied().ok_or_else(|| {
						format!(
							"The most likely cause is that {} names parent '{parent}', which no entity has as its id. Fix the parent or add an entity with that id.",
							describe(entities, index)
						)
					})
				})
				.transpose()
		})
		.collect::<Result<Vec<_>, _>>()?;
	let order = parents_first(&parents).map_err(|index| {
		format!(
			"The most likely cause is that the parents of {} lead back to it. Remove one parent of the cycle.",
			describe(entities, index)
		)
	})?;
	// The index each entity spawns at, by its position in the file.
	let mut spawn_index = vec![0u32; entities.len()];
	for (position, &index) in order.iter().enumerate() {
		spawn_index[index] = position as u32;
	}

	let kinds = group_kinds(entities, &order, |id| ids.get(id).map(|&index| spawn_index[index]))?;

	let mut payload = Vec::new();
	let parents = append(&mut payload, |bytes| {
		for &index in &order {
			let parent = parents[index].map_or(NO_PARENT, |parent| spawn_index[parent]);
			bytes.extend_from_slice(&parent.to_le_bytes());
		}
	});
	let ids = append(&mut payload, |bytes| {
		for &index in &order {
			let id = entities[index].id.as_deref().unwrap_or_default();
			bytes.extend_from_slice(&(id.len() as u32).to_le_bytes());
			bytes.extend_from_slice(id.as_bytes());
		}
	});
	let kinds = kinds
		.into_iter()
		.map(|(name, kind)| {
			let entities = append(&mut payload, |bytes| {
				for index in &kind.entities {
					bytes.extend_from_slice(&index.to_le_bytes());
				}
			});
			let (values, strings) = kind.values.finish();
			let strings = append(&mut payload, |bytes| bytes.extend_from_slice(&strings));
			let values = append(&mut payload, |bytes| bytes.extend_from_slice(&values));
			let runs = append(&mut payload, |bytes| {
				for start in &kind.runs {
					bytes.extend_from_slice(&start.to_le_bytes());
				}
			});
			LevelKind {
				name: name.to_string(),
				count: kind.entities.len() as u32,
				entities,
				strings,
				values,
				runs,
			}
		})
		.collect();

	Ok((
		Level {
			entities: count,
			parents,
			ids,
			kinds,
		},
		payload,
	))
}

/// Maps each `id` to the position of its entity in the file, rejecting empty and repeated ids.
fn index_ids(entities: &[EntitySource]) -> Result<HashMap<&str, usize>, String> {
	let mut ids = HashMap::with_capacity(entities.len());
	for (index, entity) in entities.iter().enumerate() {
		let Some(id) = entity.id.as_deref() else {
			continue;
		};
		if id.is_empty() {
			return Err(format!(
				"The most likely cause is that the entity at position {index} has an empty id. Name it or remove the `id` field."
			));
		}
		if ids.insert(id, index).is_some() {
			return Err(format!(
				"The most likely cause is that two entities share the id '{id}'. Rename one of them."
			));
		}
	}
	Ok(ids)
}

/// Returns the entities' file positions ordered so each parent comes before its children, otherwise in file order.
///
/// Returns the position of an entity on a parent cycle when the parents don't form a tree.
fn parents_first(parents: &[Option<usize>]) -> Result<Vec<usize>, usize> {
	#[derive(Clone, Copy, PartialEq)]
	enum State {
		Waiting,
		/// On the chain of ancestors being walked, so meeting it again closes a cycle.
		Walking,
		Placed,
	}

	let mut states = vec![State::Waiting; parents.len()];
	let mut order = Vec::with_capacity(parents.len());
	let mut chain = Vec::new();
	for start in 0..parents.len() {
		// Walk up to the first ancestor already placed, then place the walked chain from the top down.
		let mut current = Some(start);
		while let Some(index) = current {
			match states[index] {
				State::Placed => break,
				State::Walking => return Err(index),
				State::Waiting => {
					states[index] = State::Walking;
					chain.push(index);
					current = parents[index];
				}
			}
		}
		while let Some(index) = chain.pop() {
			states[index] = State::Placed;
			order.push(index);
		}
	}
	Ok(order)
}

/// The `KindColumn` struct collects one kind's components while entities are visited in spawn order.
struct KindColumn {
	/// The spawn index of each entity that holds one, ascending.
	entities: Vec<u32>,
	/// The components' payloads, one after another.
	values: TokenWriter,
	/// Where each run of [`RUN`] payloads starts in `values`.
	runs: Vec<u32>,
}

/// Groups every component by kind, in spawn order, encoding each payload as tokens.
///
/// `entity` returns the spawn index of the entity with an id, which entity references are written as.
fn group_kinds<'s>(
	entities: &'s [EntitySource],
	order: &[usize],
	entity: impl Fn(&str) -> Option<u32>,
) -> Result<BTreeMap<&'s str, KindColumn>, String> {
	let mut kinds = BTreeMap::<&str, KindColumn>::new();
	for (spawn_index, &index) in order.iter().enumerate() {
		for (kind, value) in &entities[index].components.0 {
			if kind.is_empty() {
				return Err(format!(
					"The most likely cause is that {} has a component with an empty kind name. Name the kind.",
					describe(entities, index)
				));
			}
			let column = kinds.entry(kind).or_insert_with(|| KindColumn {
				entities: Vec::new(),
				values: TokenWriter::default(),
				runs: Vec::new(),
			});
			if column.entities.len().is_multiple_of(RUN as usize) {
				column.runs.push(column.values.offset() as u32);
			}
			column.entities.push(spawn_index as u32);
			write_value(&mut column.values, value, &entity).map_err(|cause| {
				format!(
					"The most likely cause is that the `{kind}` component of {} {cause}",
					describe(entities, index)
				)
			})?;
		}
	}
	Ok(kinds)
}

/// Appends `value` to `writer` as tokens, writing `$entity` objects as entity references.
///
/// The error completes a sentence about the component that holds the value.
fn write_value(writer: &mut TokenWriter, value: &Value, entity: &impl Fn(&str) -> Option<u32>) -> Result<(), String> {
	match value {
		Value::Null => writer.write(Token::Null),
		Value::Bool(value) => writer.write(Token::Bool(*value)),
		Value::Unsigned(value) => writer.write(Token::Unsigned(*value)),
		Value::Signed(value) => writer.write(Token::Signed(*value)),
		Value::Float(value) => writer.write(Token::Float(*value)),
		Value::String(value) => writer.write(Token::String(value)),
		Value::Sequence(values) => {
			writer.write(Token::SequenceStart);
			for value in values {
				write_value(writer, value, entity)?;
			}
			writer.write(Token::SequenceEnd);
		}
		Value::Object(Object(fields)) => {
			if fields.iter().any(|(key, _)| key == ENTITY_REFERENCE) {
				let [(_, Value::String(id))] = fields.as_slice() else {
					return Err(format!(
						"holds a malformed entity reference. Write it as an object with only `{ENTITY_REFERENCE}`, set to an entity's id."
					));
				};
				let index = entity(id).ok_or_else(|| {
					format!(
						"refers to entity '{id}', which no entity has as its id. Fix the reference or add an entity with that id."
					)
				})?;
				writer.write(Token::Entity(index));
				return Ok(());
			}
			writer.write(Token::ObjectStart);
			for (key, value) in fields {
				writer.write(Token::Key(key));
				write_value(writer, value, entity)?;
			}
			writer.write(Token::ObjectEnd);
		}
	}
	Ok(())
}

/// The field that turns an object into a reference to another entity of the level.
const ENTITY_REFERENCE: &str = "$entity";

/// Names an entity for a message: by its id, or by its position in the file.
fn describe(entities: &[EntitySource], index: usize) -> String {
	match entities[index].id.as_deref() {
		Some(id) => format!("entity '{id}'"),
		None => format!("the entity at position {index}"),
	}
}

/// Appends a section to `payload` and returns where it landed.
fn append(payload: &mut Vec<u8>, write: impl FnOnce(&mut Vec<u8>)) -> Section {
	let offset = payload.len();
	write(payload);
	Section {
		offset: offset as u64,
		size: (payload.len() - offset) as u64,
	}
}

/// The `LevelSource` struct is the authored form of a level.
#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct LevelSource {
	/// Points editors at the schema; baking ignores it.
	#[serde(rename = "$schema", default)]
	_schema: Option<serde::de::IgnoredAny>,
	entities: Vec<EntitySource>,
}

/// The `EntitySource` struct is the authored form of one entity.
#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct EntitySource {
	#[serde(default)]
	id: Option<String>,
	#[serde(default)]
	parent: Option<String>,
	#[serde(default)]
	components: Object,
}

/// The `Value` enum is one authored payload value, kept apart from serde_json so that repeated keys and non-finite
/// numbers are rejected instead of folded away, and keys keep their order.
enum Value {
	Null,
	Bool(bool),
	Unsigned(u64),
	Signed(i64),
	Float(f64),
	String(String),
	Sequence(Vec<Value>),
	Object(Object),
}

/// The `Object` struct is an authored object's fields in file order, each key once.
#[derive(Default)]
struct Object(Vec<(String, Value)>);

impl<'de> serde::Deserialize<'de> for Value {
	fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
		deserializer.deserialize_any(ValueVisitor)
	}
}

impl<'de> serde::Deserialize<'de> for Object {
	fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
		match deserializer.deserialize_map(ValueVisitor)? {
			Value::Object(object) => Ok(object),
			_ => unreachable!("visit_map is the only way to produce an object"),
		}
	}
}

/// The `ValueVisitor` struct builds a [`Value`] from whatever the JSON5 parser reads.
struct ValueVisitor;

impl<'de> serde::de::Visitor<'de> for ValueVisitor {
	type Value = Value;

	fn expecting(&self, formatter: &mut std::fmt::Formatter) -> std::fmt::Result {
		formatter.write_str("a JSON value")
	}

	fn visit_unit<E: serde::de::Error>(self) -> Result<Value, E> {
		Ok(Value::Null)
	}

	fn visit_none<E: serde::de::Error>(self) -> Result<Value, E> {
		Ok(Value::Null)
	}

	fn visit_some<D: serde::Deserializer<'de>>(self, deserializer: D) -> Result<Value, D::Error> {
		deserializer.deserialize_any(self)
	}

	fn visit_bool<E: serde::de::Error>(self, value: bool) -> Result<Value, E> {
		Ok(Value::Bool(value))
	}

	fn visit_u64<E: serde::de::Error>(self, value: u64) -> Result<Value, E> {
		Ok(Value::Unsigned(value))
	}

	fn visit_i64<E: serde::de::Error>(self, value: i64) -> Result<Value, E> {
		Ok(u64::try_from(value).map_or(Value::Signed(value), Value::Unsigned))
	}

	fn visit_f64<E: serde::de::Error>(self, value: f64) -> Result<Value, E> {
		if value.is_finite() {
			Ok(Value::Float(value))
		} else {
			Err(E::custom("NaN and Infinity have no JSON form; write a finite number"))
		}
	}

	fn visit_str<E: serde::de::Error>(self, value: &str) -> Result<Value, E> {
		Ok(Value::String(value.to_string()))
	}

	fn visit_string<E: serde::de::Error>(self, value: String) -> Result<Value, E> {
		Ok(Value::String(value))
	}

	fn visit_seq<A: serde::de::SeqAccess<'de>>(self, mut sequence: A) -> Result<Value, A::Error> {
		let mut values = Vec::with_capacity(sequence.size_hint().unwrap_or(0));
		while let Some(value) = sequence.next_element()? {
			values.push(value);
		}
		Ok(Value::Sequence(values))
	}

	fn visit_map<A: serde::de::MapAccess<'de>>(self, mut map: A) -> Result<Value, A::Error> {
		let mut fields = Vec::<(String, Value)>::with_capacity(map.size_hint().unwrap_or(0));
		while let Some(key) = map.next_key::<String>()? {
			// Objects are small, so a scan finds a repeated key without hashing.
			if fields.iter().any(|(existing, _)| *existing == key) {
				return Err(serde::de::Error::custom(format!("field `{key}` appears twice in one object")));
			}
			let value = map.next_value()?;
			fields.push((key, value));
		}
		Ok(Value::Object(Object(fields)))
	}
}

#[cfg(test)]
mod tests {
	use super::LevelAssetHandler;
	use crate::{
		asset::{self, manager::AssetManager},
		r#async, resource,
		resource::{read_target::ReadTargetsMut, resource_manager::ResourceManager},
		resources::level::{Level, Token, Tokens},
	};

	/// The threshold the fixture handler merges past.
	const MERGE_THRESHOLD: usize = 128;

	fn asset_manager(
		assets: asset::storage_backend::tests::TestStorageBackend,
		resources: resource::storage_backend::MemoryStorageBackend,
	) -> AssetManager {
		let mut asset_manager = AssetManager::new(assets, resources);
		asset_manager.add_asset_handler(LevelAssetHandler::merging(MERGE_THRESHOLD));
		asset_manager
	}

	/// Bakes `source` as `arena.level` and returns the level and its payload.
	async fn bake(source: &str) -> Result<(Level, Vec<u8>), ()> {
		let assets = asset::storage_backend::tests::TestStorageBackend::new();
		let resources = resource::storage_backend::MemoryStorageBackend::new();
		assets.add_file("arena.level", source.as_bytes());
		asset_manager(assets, resources.clone())
			.bake("arena.level")
			.await
			.map_err(|_| ())?;
		let mut level = ResourceManager::new(resources)
			.request::<Level>("arena.level")
			.await
			.expect("a baked level should be requested");
		let payload = level
			.load(ReadTargetsMut::backing_storage())
			.await
			.expect("a baked level's payload should load")
			.buffer()
			.expect("a level payload is one buffer")
			.to_vec();
		Ok((level.into_resource(), payload))
	}

	/// The triangles of the mesh a `.leaf` file bakes into. None share a vertex, so 36 copies fit one primitive.
	const LEAF_TRIANGLES: u32 = 600;

	/// The `LeafHandler` struct bakes any `.leaf` file into a mesh of separate triangles, for levels to place.
	struct LeafHandler;

	impl asset::handler::AssetHandler for LeafHandler {
		fn can_handle(&self, r#type: &str) -> bool {
			r#type == "leaf"
		}

		async fn bake<'a>(
			&'a self,
			context: asset::handler::BakeContext<'a>,
			id: asset::ResourceId<'a>,
		) -> Result<(), asset::handler::LoadErrors> {
			use crate::processors::mesh::{MeshPrimitiveSource, MeshProcessorSession, MeshSettings};
			use crate::types::{VertexComponent, VertexSemantics};

			struct Triangle;
			impl MeshPrimitiveSource for Triangle {
				type Error = std::convert::Infallible;

				fn material_slot(&self) -> usize {
					0
				}

				fn indices(&self) -> Result<impl ExactSizeIterator<Item = Result<u32, Self::Error>> + '_, Self::Error> {
					Ok((0..LEAF_TRIANGLES * 3).map(Ok))
				}

				fn positions(
					&self,
				) -> Result<
					impl ExactSizeIterator<Item = Result<math::Point<crate::resources::ModelSpace>, Self::Error>> + '_,
					Self::Error,
				> {
					Ok((0..LEAF_TRIANGLES * 3).map(|vertex| {
						// Each triangle stands a little higher than the last.
						let [x, y] = [[0.0, 0.0], [1.0, 0.0], [0.0, 1.0]][vertex as usize % 3];
						Ok(math::Point::new(x, y, (vertex / 3) as f32 * 0.01))
					}))
				}
			}

			let layout = vec![VertexComponent::canonical(VertexSemantics::Position)];
			let mut session = MeshProcessorSession::new(layout, None, Vec::new(), MeshSettings::default())
				.map_err(|_| asset::handler::LoadErrors::FailedToProcess)?;
			session
				.push_primitive(&Triangle)
				.map_err(|_| asset::handler::LoadErrors::FailedToProcess)?;
			let material = crate::ReferenceModel::new_serialized(
				"leaf.variant",
				0,
				0,
				crate::to_vec(&crate::resources::material::VariantModel {
					material: crate::ReferenceModel::new_serialized("leaf.material", 0, 0, Vec::new(), None),
					variables: Vec::new(),
					alpha_mode: crate::types::AlphaMode::Mask(0.5),
					double_sided: true,
				})
				.expect("variant should serialize"),
				None,
			);
			asset::commit_mesh(context, id, session, &[material]).await
		}
	}

	/// Bakes a level that places `leaf.leaf` `count` times along x, one unit apart, beside one entity with an `id`
	/// and one with a `name`, which also place it. Returns the level, its payload, and the baked resources.
	async fn bake_placements(count: usize) -> (Level, Vec<u8>, ResourceManager) {
		let assets = asset::storage_backend::tests::TestStorageBackend::new();
		let resources = resource::storage_backend::MemoryStorageBackend::new();
		let mut source = String::from("{ entities: [\n");
		for index in 0..count {
			source += &format!(
				"{{ components: {{ mesh: {{ resource: \"leaf.leaf\" }}, transform: {{ position: [{index}, 0, 0] }} }} }},\n"
			);
		}
		source += "{ id: \"marked\", components: { mesh: { resource: \"leaf.leaf\" } } },\n";
		source += "{ components: { mesh: { resource: \"leaf.leaf\" }, name: \"named\" } },\n] }";
		assets.add_file("arena.level", source.as_bytes());
		assets.add_file("leaf.leaf", b"");
		let mut asset_manager = asset_manager(assets, resources.clone());
		asset_manager.add_asset_handler(LeafHandler);
		asset_manager.bake("arena.level").await.expect("the level should bake");
		let resources = ResourceManager::new(resources);
		let mut level = resources
			.request::<Level>("arena.level")
			.await
			.expect("a baked level should be requested");
		let payload = level
			.load(ReadTargetsMut::backing_storage())
			.await
			.expect("a baked level's payload should load")
			.buffer()
			.expect("a level payload is one buffer")
			.to_vec();
		(level.into_resource(), payload, resources)
	}

	/// Returns the tokens of each `mesh` component of `level`, in entity order.
	fn mesh_payloads<'p>(level: &Level, payload: &'p [u8]) -> Vec<Vec<Token<'p>>> {
		let kind = level
			.kinds
			.iter()
			.find(|kind| kind.name == "mesh")
			.expect("the level places meshes");
		let strings = kind.strings(payload).expect("the kind's strings should read");
		let tokens = (0..kind.run_count())
			.flat_map(|run| read_all(kind.run(payload, &strings, run).expect("a run should read")))
			.take_while({
				// A run's tokens continue past it, so stop after the kind's last payload.
				let (mut depth, mut payloads) = (0, 0);
				move |token| {
					let more = payloads < kind.count;
					match token {
						Token::ObjectStart | Token::SequenceStart => depth += 1,
						Token::ObjectEnd | Token::SequenceEnd => depth -= 1,
						_ => {}
					}
					if depth == 0 {
						payloads += 1;
					}
					more
				}
			});
		// Each payload is one object, so it ends where the nesting closes.
		let mut payloads = vec![Vec::new()];
		let mut depth = 0;
		for token in tokens {
			match token {
				Token::ObjectStart | Token::SequenceStart => depth += 1,
				Token::ObjectEnd | Token::SequenceEnd => depth -= 1,
				_ => {}
			}
			payloads.last_mut().expect("a payload is open").push(token);
			if depth == 0 {
				payloads.push(Vec::new());
			}
		}
		payloads.pop();
		payloads
	}

	/// Returns how many placements a `mesh` payload with the `merged` source holds, or `None` for another source.
	fn merged_placements(payload: &[Token<'_>]) -> Option<usize> {
		let [
			Token::ObjectStart,
			Token::Key("merged"),
			Token::ObjectStart,
			Token::Key("resource"),
			Token::String("leaf.leaf"),
			Token::Key("placements"),
			Token::SequenceStart,
			placements @ ..,
		] = payload
		else {
			return None;
		};
		// Every placement of the test level names its position.
		Some(placements.iter().filter(|token| **token == Token::Key("position")).count())
	}

	#[r#async::test]
	async fn a_mesh_placed_past_the_threshold_bakes_into_clusters_of_its_placements() {
		let (level, payload, _) = bake_placements(MERGE_THRESHOLD + 1).await;

		let payloads = mesh_payloads(&level, &payload);
		// The clusters, then the two entities that hold more than a placement.
		let (others, clusters): (Vec<_>, Vec<_>) = payloads.iter().partition(|payload| merged_placements(payload).is_none());
		assert_eq!(others.len(), 2);
		assert_eq!(level.entities as usize, payloads.len());
		let sizes: Vec<usize> = clusters.iter().map(|cluster| merged_placements(cluster).unwrap()).collect();
		// 36 copies of the leaf fit one primitive, so 129 placements need four clusters.
		assert_eq!(sizes.len(), 4);
		assert!(
			sizes.iter().all(|&size| size <= 36),
			"A cluster holds more copies than fit one primitive: {sizes:?}."
		);
		assert_eq!(sizes.iter().sum::<usize>(), MERGE_THRESHOLD + 1);
		// The placements moved into the clusters, and the other two entities never had a transform.
		assert!(level.kinds.iter().all(|kind| kind.name != "transform"));
	}

	#[r#async::test]
	async fn a_mesh_placed_up_to_the_threshold_keeps_its_entities() {
		let (level, payload, _) = bake_placements(MERGE_THRESHOLD).await;

		assert_eq!(level.entities as usize, MERGE_THRESHOLD + 2);
		assert!(
			mesh_payloads(&level, &payload)
				.iter()
				.all(|payload| merged_placements(payload).is_none())
		);
	}

	/// Reads every token of `tokens`.
	fn read_all<'p>(tokens: Tokens<'p, '_>) -> Vec<Token<'p>> {
		tokens.collect::<Result<_, _>>().expect("baked tokens should be well formed")
	}

	#[r#async::test]
	async fn entities_spawn_parents_first_and_otherwise_in_file_order() {
		let (level, payload) = bake(
			r#"{
				// The door comes first in the file but spawns after its frame.
				entities: [
					{ id: "door", parent: "frame" },
					{ id: "frame" },
					{},
				],
			}"#,
		)
		.await
		.expect("the level should bake");

		assert_eq!(level.entities, 3);
		let ids = (0..3).map(|entity| level.id(&payload, entity)).collect::<Vec<_>>();
		assert_eq!(ids, [Some("frame"), Some("door"), None]);
		let parents = level.parents(&payload).expect("parents should load").collect::<Vec<_>>();
		assert_eq!(parents, [None, Some(0), None]);
	}

	#[r#async::test]
	async fn components_are_grouped_by_kind_in_entity_order_with_references_resolved() {
		let (level, payload) = bake(
			r#"{
				"$schema": "byte_engine/schemas/level.schema.json",
				entities: [
					{ id: "a", components: { tag: "first", hinge: { other: { "$entity": "b" }, axis: [0, 1.5, -2] } } },
					{ id: "b", components: { tag: "second" } },
				],
			}"#,
		)
		.await
		.expect("the level should bake");

		let names = level.kinds.iter().map(|kind| kind.name.as_str()).collect::<Vec<_>>();
		assert_eq!(names, ["hinge", "tag"]);

		let hinge = &level.kinds[0];
		assert_eq!(hinge.entities(&payload).unwrap().collect::<Vec<_>>(), [0]);
		let strings = hinge.strings(&payload).unwrap();
		assert_eq!(
			read_all(hinge.run(&payload, &strings, 0).unwrap()),
			[
				Token::ObjectStart,
				Token::Key("other"),
				Token::Entity(1),
				Token::Key("axis"),
				Token::SequenceStart,
				Token::Unsigned(0),
				Token::Float(1.5),
				Token::Signed(-2),
				Token::SequenceEnd,
				Token::ObjectEnd,
			]
		);

		let tag = &level.kinds[1];
		assert_eq!(tag.count, 2);
		assert_eq!(tag.entities(&payload).unwrap().collect::<Vec<_>>(), [0, 1]);
		let strings = tag.strings(&payload).unwrap();
		assert_eq!(
			read_all(tag.run(&payload, &strings, 0).unwrap()),
			[Token::String("first"), Token::String("second")]
		);
	}

	#[r#async::test]
	async fn levels_that_break_the_format_are_rejected() {
		for (problem, source) in [
			("unknown file field", r#"{ entities: [], extra: 1 }"#),
			("unknown entity field", r#"{ entities: [{ name: "a" }] }"#),
			("empty id", r#"{ entities: [{ id: "" }] }"#),
			("repeated id", r#"{ entities: [{ id: "a" }, { id: "a" }] }"#),
			("unknown parent", r#"{ entities: [{ parent: "a" }] }"#),
			("own parent", r#"{ entities: [{ id: "a", parent: "a" }] }"#),
			(
				"parent cycle",
				r#"{ entities: [{ id: "a", parent: "b" }, { id: "b", parent: "a" }] }"#,
			),
			("repeated kind", r#"{ entities: [{ components: { mesh: 1, mesh: 2 } }] }"#),
			(
				"repeated payload field",
				r#"{ entities: [{ components: { mesh: { a: 1, a: 2 } } }] }"#,
			),
			("empty kind", r#"{ entities: [{ components: { "": 1 } }] }"#),
			(
				"unknown reference",
				r#"{ entities: [{ components: { follow: { "$entity": "a" } } }] }"#,
			),
			(
				"reference with another field",
				r#"{ entities: [{ id: "a", components: { follow: { "$entity": "a", b: 1 } } }] }"#,
			),
			(
				"reference that isn't a string",
				r#"{ entities: [{ components: { follow: { "$entity": 0 } } }] }"#,
			),
			("non-finite number", r#"{ entities: [{ components: { scale: NaN } }] }"#),
			("not JSON5", r#"{ entities: [ "#),
		] {
			assert!(bake(source).await.is_err(), "a level with a {problem} should fail to bake");
		}
	}
}
