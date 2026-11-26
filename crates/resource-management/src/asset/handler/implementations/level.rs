//! Level asset baking.
//!
//! Baking does everything that doesn't need the application's component types: it parses the JSON5 text, checks
//! ids, parents, and references, orders entities parents first, and groups the payloads by kind as parsed tokens.
//! What's left for spawning is decoding each kind's tokens into its type and publishing it.
//!
//! A handler made with [`LevelAssetHandler::merging`] also reads one kind pair, `mesh` with `transform`: the
//! entities that place one mesh more often than its threshold become a few entities, each drawing a cluster mesh that
//! holds a cluster of neighboring placements. [`repeated_placements`] finds those entities, [`merge_group`] clusters
//! them and stores each cluster as a mesh of its own, and [`cluster_entities`] rewrites them to draw the cluster
//! meshes. The default handler merges nothing: a cluster is one mesh with one detail level, which shows as popping.
use std::{
	collections::{BTreeMap, HashMap},
	ops::Range,
};

use super::{
	ResourceId,
	handler::{AssetHandler, BakeContext, LoadErrors},
};
use crate::{
	ProcessedAsset,
	processors::mesh::{
		BakedPrimitive, MeshProcessorSession, Placement, RepeatedPrimitive, cluster_placements, copies_that_fit,
	},
	resource::read_target::ReadTargetsMut,
	resources::{
		level::{Level, LevelInstance, LevelKind, NO_PARENT, NamedValue, RUN, Section, Token, TokenWriter},
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
	/// its placements, each stored as one mesh that holds the cluster's copies.
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

	fn identity(&self) -> u64 {
		// The threshold decides which placements a level stores as merged clusters. No threshold is zero.
		self.merge_threshold.map_or(0, |threshold| threshold as u64 + 1)
	}

	async fn bake<'a>(&'a self, context: BakeContext<'a>, id: ResourceId<'a>) -> Result<PrimaryStored, LoadErrors> {
		let (source, format) = context.resolve(id).await?;

		if format != "level" {
			return Err(LoadErrors::UnsupportedType);
		}

		let report = |cause: String| {
			log::error!("Level '{}' could not be baked. {cause}", id.as_ref());
			LoadErrors::FailedToProcess
		};
		let LevelSource { entities, .. } = parse(&source).map_err(report)?;
		// Each mesh placed often enough merges into clusters, numbered across the level in file order.
		let mut merges = Vec::new();
		let groups = self
			.merge_threshold
			.map_or_else(Vec::new, |threshold| repeated_placements(&entities, threshold));
		let mut next_cluster = 0;
		for group in groups {
			if let Some(clusters) = merge_group(&context, id.as_ref(), &group, next_cluster).await {
				next_cluster = clusters.end;
				merges.push((group, clusters));
			}
		}
		let entities = cluster_entities(entities, id.as_ref(), &merges);
		let LaidOut { level, payload, .. } = lay_out(&entities, None).map_err(report)?;

		context.store_primary_owned(ProcessedAsset::new(id, level), payload).await
	}
}

/// Clusters `group`'s placements and stores each cluster as a mesh, numbered from `first`, as cluster meshes of the
/// level `level`. Returns the numbers of the stored clusters, or `None` for a mesh that can't merge.
///
/// A mesh that can't merge is reported and left to its entities, since the level still draws without the merge.
async fn merge_group(context: &BakeContext<'_>, level: &str, group: &MergeGroup, first: usize) -> Option<Range<usize>> {
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
	let Ok(mut mesh) = context.request::<Mesh>(&group.mesh).await else {
		return leave(UNBAKED);
	};
	let Ok(payload) = mesh.load(ReadTargetsMut::backing_storage()).await else {
		return leave(UNBAKED);
	};
	let Some(payload) = payload.buffer() else {
		return leave(UNBAKED);
	};
	let mesh = mesh.resource();
	let primitives = match mesh
		.primitives
		.iter()
		.map(|primitive| BakedPrimitive::read(mesh, primitive, payload))
		.collect::<Result<Vec<_>, _>>()
	{
		Ok(primitives) => primitives,
		Err(cause) => return leave(cause),
	};
	// A cluster repeats every primitive of the mesh, so the primitive that fits the fewest copies bounds it.
	let capacity = match mesh.primitives.iter().map(copies_that_fit).min() {
		Some(capacity) if capacity > 0 => capacity,
		_ => return leave("The most likely cause is that the mesh has no primitives. Fix the mesh, or place another one."),
	};
	// A cluster bakes as the merged mesh does. Lightmap UVs only exist where the bake builds in lightmaps, so the merged
	// mesh having them says the cluster should too.
	let Ok(sidecar) = context.load_sidecar(ResourceId::new(&group.mesh)).await else {
		return leave(UNBAKED);
	};
	let lightmap_uvs = mesh
		.vertex_components
		.iter()
		.any(|component| component.semantic == crate::types::VertexSemantics::LightmapUV);
	let settings = match crate::asset::mesh_settings(sidecar.as_ref(), lightmap_uvs) {
		Ok(settings) => settings,
		Err(cause) => {
			return leave(&format!(
				"The most likely cause is that the mesh's settings are invalid: {cause}."
			));
		}
	};
	// The cluster meshes store every attribute the merged mesh has, except lightmap UVs, which the session generates.
	let layout: Vec<_> = mesh
		.vertex_components
		.iter()
		.filter(|component| component.semantic != crate::types::VertexSemantics::LightmapUV)
		.cloned()
		.collect();

	let positions: Vec<[f32; 3]> = group.placements.iter().map(|placement| placement.position).collect();
	let (order, clusters) = cluster_placements(&positions, capacity);
	// One buffer holds each cluster's placements in turn, in the order neighbors follow each other.
	let mut placements = Vec::with_capacity(capacity);
	for (number, cluster) in (first..).zip(&clusters) {
		placements.clear();
		placements.extend(order[cluster.clone()].iter().map(|&member| group.placements[member as usize]));
		let id = cluster_mesh_id(level, number);
		if let Err(cause) = bake_cluster(context, &id, &layout, settings, mesh, &primitives, &placements).await {
			return leave(&cause);
		}
	}
	Some(first..first + clusters.len())
}

/// Returns the ID of cluster mesh `number` of the level `level`.
fn cluster_mesh_id(level: &str, number: usize) -> String {
	format!("{level}#cluster/{number}")
}

/// Bakes `primitives` of `mesh` once at each of `placements` into one mesh with the vertex `layout` and `settings`,
/// and stores it as `id`. Returns the cause and its fix when the mesh doesn't bake or store.
async fn bake_cluster(
	context: &BakeContext<'_>,
	id: &str,
	layout: &[crate::types::VertexComponent],
	settings: crate::processors::mesh::MeshSettings,
	mesh: &Mesh,
	primitives: &[BakedPrimitive],
	placements: &[Placement],
) -> Result<(), String> {
	let mut session = MeshProcessorSession::new(layout.to_vec(), None, Vec::new(), settings);
	for primitive in primitives {
		session
			.push_primitive(&RepeatedPrimitive::new(primitive, placements))
			.map_err(|error| format!("The most likely cause is geometry the mesh processor rejects: {error:?}."))?;
	}
	const UNSTORED: &str = "The most likely cause is that the resource store can't be written.";
	let id = ResourceId::new(id);
	let mut transaction = context
		.begin_resource(id, session.payload_size(&mesh.materials))
		.await
		.map_err(|_| UNSTORED.to_string())?;
	let (model, streams) = session
		.finish_into_resource(&mesh.materials, &mut transaction)
		.await
		.map_err(|_| UNSTORED.to_string())?;
	context
		.commit_resource(transaction, ProcessedAsset::new(id, model).with_streams(streams))
		.await
		.map(|_| ())
		.map_err(|_| UNSTORED.to_string())
}

/// The `MergeGroup` struct is one mesh that a level places often enough to merge, and where.
struct MergeGroup {
	/// The mesh resource every entity of the group draws.
	mesh: String,
	/// The file position of each entity of the group, ascending.
	entities: Vec<usize>,
	/// Where each entity places the mesh, in the order of `entities`.
	placements: Vec<Placement>,
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
				placements: Vec::new(),
			});
			groups.len() - 1
		});
		groups[index].entities.push(position);
		groups[index].placements.push(placed);
	}
	groups.retain(|group| group.entities.len() > threshold);
	groups
}

/// Replaces the entities of each group in `merges` with one entity for each of the group's paired cluster meshes of
/// the level `level`, and returns the level's entities.
///
/// A cluster entity holds a `mesh` with the `resource` source that names its cluster mesh, and no transform, since the
/// cluster mesh holds its copies where they were placed. The clusters take the place of the group's first entity,
/// and belong to the group's parent.
fn cluster_entities(entities: Vec<EntitySource>, level: &str, merges: &[(MergeGroup, Range<usize>)]) -> Vec<EntitySource> {
	// The clusters that take each entity's place, and whether the entity itself leaves.
	let mut clusters_at: Vec<Option<Range<usize>>> = entities.iter().map(|_| None).collect();
	let mut merged = vec![false; entities.len()];
	for (group, clusters) in merges {
		clusters_at[group.entities[0]] = Some(clusters.clone());
		for &entity in &group.entities {
			merged[entity] = true;
		}
	}

	let mut result = Vec::with_capacity(entities.len());
	for ((entity, clusters), merged) in entities.into_iter().zip(clusters_at).zip(merged) {
		result.extend(clusters.into_iter().flatten().map(|number| {
			let source = Object(vec![("resource".to_string(), Value::String(cluster_mesh_id(level, number)))]);
			EntitySource {
				id: None,
				parent: entity.parent.clone(),
				prefab: None,
				components: Object(vec![("mesh".to_string(), Value::Object(source))]),
			}
		}));
		if !merged {
			result.push(entity);
		}
	}
	result
}

/// Returns the mesh resource `entity` places and how, when the entity is nothing but that placement.
fn static_placement(entity: &EntitySource) -> Option<(&str, Placement)> {
	if entity.id.is_some() || entity.prefab.is_some() {
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
	(source == "resource").then_some((resource.as_str(), placement(transform)?))
}

/// Returns how a `transform` payload places its entity, where a missing payload or field takes the identity's value.
///
/// Returns `None` for a payload the `transform` kind rejects, so its entity stays and the spawn reports it, and for
/// a zero scale, which leaves no surface to merge.
fn placement(transform: Option<&Value>) -> Option<Placement> {
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
	Some(Placement {
		position,
		orientation: math::Orientation::try_from_array(orientation).ok()?,
		scale,
	})
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
///
/// Also returns where each entity sits in the file, by spawn index, so a tool can find the source of a spawned
/// entity, such as the one an editor's click lands on.
pub fn bake_level(source: &[u8]) -> Result<(Level, Vec<u8>, Vec<usize>), String> {
	let LevelSource { entities, .. } = parse(source)?;
	let laid = lay_out(&entities, None)?;
	Ok((laid.level, laid.payload, laid.order))
}

/// Parses a JSON5 source into `T`, a level's or a prefab's authored form.
pub(super) fn parse<T: serde::de::DeserializeOwned>(source: &[u8]) -> Result<T, String> {
	let text = std::str::from_utf8(source)
		.map_err(|_| "The most likely cause is that the file isn't UTF-8 text. Save it as UTF-8.".to_string())?;
	json5::from_str(text).map_err(|error| {
		format!("The most likely cause is that the file doesn't hold its format's fields: {error}. Fix the reported field.")
	})
}

/// The `LaidOut` struct is a level or prefab file laid out for storage, with what its bakers need besides.
pub(super) struct LaidOut {
	pub(super) level: Level,
	pub(super) payload: Vec<u8>,
	/// Each parameter's name and where its default starts in the values; none for a level.
	pub(super) defaults: Vec<NamedValue>,
	/// The file position of each entity, by spawn index.
	pub(super) order: Vec<usize>,
}

/// Checks `entities` and lays them out as a level's model and payload.
///
/// `parameters` are a prefab's parameters with their defaults, which its payloads may refer to; a level has none and
/// rejects every parameter reference.
pub(super) fn lay_out(entities: &[EntitySource], parameters: Option<&Object>) -> Result<LaidOut, String> {
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

	let entity = |id: &str| ids.get(id).map(|&index| spawn_index[index]);
	let names = parameter_indices(parameters)?;
	let references = References {
		entity: Some(&entity),
		parameters: names.as_ref(),
	};
	let kinds = group_kinds(entities, &order, &references)?;

	// Instances' parameters and the prefab's defaults share one token stream, so a value is an offset into it.
	let mut values = TokenWriter::default();
	let defaults = match parameters {
		Some(Object(fields)) => fields
			.iter()
			.map(|(name, value)| {
				let offset = values.offset() as u32;
				write_value(&mut values, value, &References::default())
					.map_err(|cause| format!("The most likely cause is that the default of parameter '{name}' {cause}"))?;
				Ok(NamedValue {
					name: name.clone(),
					value: offset,
				})
			})
			.collect::<Result<Vec<_>, String>>()?,
		None => Vec::new(),
	};
	let mut instances = Vec::new();
	for (spawn, &index) in order.iter().enumerate() {
		let Some(instance) = &entities[index].prefab else {
			continue;
		};
		if instance.resource.is_empty() {
			return Err(format!(
				"The most likely cause is that the `prefab` of {} has an empty `resource`. Set it to a prefab's ID.",
				describe(entities, index)
			));
		}
		let parameters = instance
			.parameters
			.0
			.iter()
			.map(|(name, value)| {
				let offset = values.offset() as u32;
				// An instance's value can pass on the enclosing prefab's parameters, but names no entity.
				let references = References {
					entity: None,
					parameters: names.as_ref(),
				};
				write_value(&mut values, value, &references).map_err(|cause| {
					format!(
						"The most likely cause is that parameter '{name}' of the `prefab` of {} {cause}",
						describe(entities, index)
					)
				})?;
				Ok(NamedValue {
					name: name.clone(),
					value: offset,
				})
			})
			.collect::<Result<Vec<_>, String>>()?;
		instances.push(LevelInstance {
			entity: spawn as u32,
			resource: instance.resource.clone(),
			parameters,
		});
	}

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

	let (values, value_strings) = values.finish();
	let value_strings = append(&mut payload, |bytes| bytes.extend_from_slice(&value_strings));
	let values = append(&mut payload, |bytes| bytes.extend_from_slice(&values));

	Ok(LaidOut {
		level: Level {
			entities: count,
			parents,
			ids,
			kinds,
			instances,
			values,
			value_strings,
		},
		payload,
		defaults,
		order,
	})
}

/// Maps each parameter of a prefab to its index, rejecting empty names. A level, which has no parameters, maps none.
fn parameter_indices(parameters: Option<&Object>) -> Result<Option<HashMap<&str, u32>>, String> {
	let Some(Object(fields)) = parameters else {
		return Ok(None);
	};
	let mut names = HashMap::with_capacity(fields.len());
	for (index, (name, _)) in fields.iter().enumerate() {
		if name.is_empty() {
			return Err("The most likely cause is that a parameter has an empty name. Name the parameter.".to_string());
		}
		// The authored object already rejects a repeated key, so each name is new here.
		names.insert(name.as_str(), index as u32);
	}
	Ok(Some(names))
}

/// The `References` struct says which references a value may hold where it's written, and what they resolve to.
#[derive(Default)]
struct References<'a> {
	/// Returns the spawn index of the entity with an id; `None` where no entity can be referred to.
	entity: Option<&'a dyn Fn(&str) -> Option<u32>>,
	/// The index of each parameter; `None` outside a prefab.
	parameters: Option<&'a HashMap<&'a str, u32>>,
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
/// `references` resolves the entity and parameter references the payloads hold.
fn group_kinds<'s>(
	entities: &'s [EntitySource],
	order: &[usize],
	references: &References<'_>,
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
			write_value(&mut column.values, value, references).map_err(|cause| {
				format!(
					"The most likely cause is that the `{kind}` component of {} {cause}",
					describe(entities, index)
				)
			})?;
		}
	}
	Ok(kinds)
}

/// Appends `value` to `writer` as tokens, writing `$entity` and `$parameter` objects as references.
///
/// The error completes a sentence about the value.
fn write_value(writer: &mut TokenWriter, value: &Value, references: &References<'_>) -> Result<(), String> {
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
				write_value(writer, value, references)?;
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
				let entity = references
					.entity
					.ok_or_else(|| "holds an entity reference where none can stand. Write the value itself.".to_string())?;
				let index = entity(id).ok_or_else(|| {
					format!(
						"refers to entity '{id}', which no entity has as its id. Fix the reference or add an entity with that id."
					)
				})?;
				writer.write(Token::Entity(index));
				return Ok(());
			}
			if fields.iter().any(|(key, _)| key == PARAMETER_REFERENCE) {
				let [(_, Value::String(name))] = fields.as_slice() else {
					return Err(format!(
						"holds a malformed parameter reference. Write it as an object with only `{PARAMETER_REFERENCE}`, set to a parameter's name."
					));
				};
				let parameters = references.parameters.ok_or_else(|| {
					format!(
						"refers to parameter '{name}' where no parameter can stand. Only a prefab's components and instances refer to parameters; write the value itself."
					)
				})?;
				let index = parameters.get(name.as_str()).ok_or_else(|| {
					format!(
						"refers to parameter '{name}', which the prefab doesn't declare. Fix the reference or add the parameter to `parameters`."
					)
				})?;
				writer.write(Token::Parameter(*index));
				return Ok(());
			}
			writer.write(Token::ObjectStart);
			for (key, value) in fields {
				writer.write(Token::Key(key));
				write_value(writer, value, references)?;
			}
			writer.write(Token::ObjectEnd);
		}
	}
	Ok(())
}

/// Appends `value` to `writer` as tokens, where no entity or parameter can be referred to.
pub(super) fn write_plain_value(writer: &mut TokenWriter, value: &Value) -> Result<(), String> {
	write_value(writer, value, &References::default())
}

/// The field that turns an object into a reference to another entity of the level.
const ENTITY_REFERENCE: &str = "$entity";

/// The field that turns an object into a reference to a parameter of the prefab.
const PARAMETER_REFERENCE: &str = "$parameter";

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
pub(super) struct LevelSource {
	/// Points editors at the schema; baking ignores it.
	#[serde(rename = "$schema", default)]
	_schema: Option<serde::de::IgnoredAny>,
	pub(super) entities: Vec<EntitySource>,
}

/// The `EntitySource` struct is the authored form of one entity.
#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct EntitySource {
	#[serde(default)]
	id: Option<String>,
	#[serde(default)]
	parent: Option<String>,
	#[serde(default)]
	prefab: Option<InstanceSource>,
	#[serde(default)]
	components: Object,
}

/// The `InstanceSource` struct is the authored form of an entity's `prefab` field: the prefab it places and the
/// parameters it sets.
#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct InstanceSource {
	resource: String,
	#[serde(default)]
	parameters: Object,
}

/// The `Value` enum is one authored payload value, kept apart from serde_json so that repeated keys and non-finite
/// numbers are rejected instead of folded away, and keys keep their order.
pub(super) enum Value {
	Null,
	Bool(bool),
	Unsigned(u64),
	Signed(i64),
	Float(f64),
	String(String),
	Sequence(Vec<Value>),
	Object(Object),
}

impl TryFrom<serde_json::Value> for Value {
	type Error = String;

	/// Converts a JSON value from code, keeping its fields' order.
	fn try_from(value: serde_json::Value) -> Result<Self, String> {
		Ok(match value {
			serde_json::Value::Null => Value::Null,
			serde_json::Value::Bool(value) => Value::Bool(value),
			serde_json::Value::Number(number) => match (number.as_u64(), number.as_i64(), number.as_f64()) {
				(Some(value), ..) => Value::Unsigned(value),
				(None, Some(value), _) => Value::Signed(value),
				(None, None, Some(value)) => Value::Float(value),
				_ => return Err("holds a number JSON can't represent. Pass a finite number.".to_string()),
			},
			serde_json::Value::String(value) => Value::String(value),
			serde_json::Value::Array(values) => {
				Value::Sequence(values.into_iter().map(Value::try_from).collect::<Result<_, _>>()?)
			}
			serde_json::Value::Object(fields) => Value::Object(Object(
				fields
					.into_iter()
					.map(|(key, value)| Ok((key, Value::try_from(value)?)))
					.collect::<Result<_, String>>()?,
			)),
		})
	}
}

/// The `Object` struct is an authored object's fields in file order, each key once.
#[derive(Default)]
pub(super) struct Object(Vec<(String, Value)>);

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
		resources::{
			level::{Level, Token, Tokens},
			mesh::Mesh,
		},
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
		let manager = asset_manager(assets, resources);
		manager.bake("arena.level").await.map_err(|_| ())?;
		let mut level = ResourceManager::from_boxed(manager.into_storage().1)
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
		) -> Result<asset::handler::PrimaryStored, asset::handler::LoadErrors> {
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
			let mut session = MeshProcessorSession::new(layout, None, Vec::new(), MeshSettings::default());
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
		let mut asset_manager = asset_manager(assets, resources);
		asset_manager.add_asset_handler(LeafHandler);
		asset_manager.bake("arena.level").await.expect("the level should bake");
		let resources = ResourceManager::from_boxed(asset_manager.into_storage().1);
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

	/// Returns the cluster mesh a `mesh` payload draws, or `None` for a payload that draws anything else.
	fn cluster_mesh<'p>(payload: &[Token<'p>]) -> Option<&'p str> {
		let [
			Token::ObjectStart,
			Token::Key("resource"),
			Token::String(id),
			Token::ObjectEnd,
		] = payload
		else {
			return None;
		};
		id.contains("#cluster/").then_some(*id)
	}

	#[r#async::test]
	async fn a_mesh_placed_past_the_threshold_bakes_into_clusters_of_its_placements() {
		let (level, payload, resources) = bake_placements(MERGE_THRESHOLD + 1).await;

		let payloads = mesh_payloads(&level, &payload);
		// The clusters, then the two entities that hold more than a placement.
		let (others, clusters): (Vec<_>, Vec<_>) = payloads.iter().partition(|payload| cluster_mesh(payload).is_none());
		assert_eq!(others.len(), 2);
		assert_eq!(level.entities as usize, payloads.len());
		let ids: Vec<&str> = clusters.iter().map(|cluster| cluster_mesh(cluster).unwrap()).collect();
		// 36 copies of the leaf fit one primitive, so 129 placements need four clusters, numbered in file order.
		assert_eq!(
			ids,
			[
				"arena.level#cluster/0",
				"arena.level#cluster/1",
				"arena.level#cluster/2",
				"arena.level#cluster/3"
			]
		);
		let leaf = resources
			.request::<Mesh>("leaf.leaf")
			.await
			.expect("the leaf should be baked");
		let leaf = &leaf.resource().primitives[0];
		let mut copies = 0;
		for id in ids {
			let cluster = resources
				.request::<Mesh>(id)
				.await
				.expect("each cluster mesh should be stored");
			let [primitive] = cluster.resource().primitives.as_slice() else {
				panic!("A cluster mesh has one primitive for each primitive of the leaf.");
			};
			let count = primitive.vertex_count / leaf.vertex_count;
			assert_eq!(
				count * leaf.vertex_count,
				primitive.vertex_count,
				"a cluster holds whole copies"
			);
			assert!(count <= 36, "A cluster holds more copies than fit one primitive: {count}.");
			copies += count;
			// The copies are unscaled, so the largest copy is as wide as the leaf.
			assert_eq!(primitive.caster_radius, Some(leaf.bounding_sphere[3]));
			assert!(primitive.bounding_sphere[3] > leaf.bounding_sphere[3]);
		}
		assert_eq!(copies as usize, MERGE_THRESHOLD + 1);
		// The placements are baked into the cluster meshes, and the other two entities never had a transform.
		assert!(level.kinds.iter().all(|kind| kind.name != "transform"));
	}

	#[r#async::test]
	async fn a_mesh_placed_up_to_the_threshold_keeps_its_entities() {
		let (level, payload, _) = bake_placements(MERGE_THRESHOLD).await;

		assert_eq!(level.entities as usize, MERGE_THRESHOLD + 2);
		assert!(
			mesh_payloads(&level, &payload)
				.iter()
				.all(|payload| cluster_mesh(payload).is_none())
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

use crate::asset::handler::PrimaryStored;
