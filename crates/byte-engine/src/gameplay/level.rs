//! Spawning baked levels and prefabs: their entities, published on the world scope.
//!
//! A [`Spawner`] maps each kind name a level or prefab uses to the component type it publishes. Register your
//! application's kinds next to the engine's ([`Spawner::with_engine_kinds`]), create the systems that follow those
//! components, then [`Loaded::load`] a level or prefab and call [`Spawner::spawn`]. Delete the returned handle to
//! unload what spawned through the [`SceneGraph`](crate::gameplay::SceneGraph).
//!
//! A prefab spawns either directly, many times from one [`Loaded`], or from an instance: a level or prefab entity
//! that places it. Spawning expands every instance before it takes handles, so a spawn stays one run of handles that
//! publishes all or nothing.
//!
//! Spawning is built to be fast: baking already parsed the text, ordered the entities, and grouped the components by
//! kind, so a spawn takes one run of handles, decodes each kind in one pass, and publishes each kind as one batch.

mod kinds;
mod tokens;

use std::{
	borrow::Cow,
	collections::{HashMap, hash_map::Entry},
	marker::PhantomData,
	num::NonZeroU32,
	sync::atomic::{AtomicUsize, Ordering},
};

pub use resource_management::resources::level::Level;
pub use resource_management::resources::prefab::Prefab;
use resource_management::resources::{
	level::{LevelKind, NamedValue, RUN},
	prefab::ParameterValues,
};
use resource_management::{
	ResourceManager,
	asset::handler::implementations::prefab::encode_parameter_values,
	resource::read_target::{ReadTargets, ReadTargetsMut},
};
use serde::{Deserialize as _, de::DeserializeOwned};
use tokens::{Argument, Arguments, TokenDeserializer};

use crate::{
	core::{factory::Handle, machine::performance_parallelism, message_bus::MessageScope},
	gameplay::{Scene, SceneNode, Transform, transform::SerializableTransform},
};

/// The field that turns a payload object into a reference to another entity of the level.
const ENTITY_REFERENCE: &str = "$entity";

/// The kind the spawner reads itself, since it places every entity relative to its spawn.
const TRANSFORM: &str = "transform";

/// How many instances deep a spawn expands prefabs, counted from what was loaded.
pub const MAX_NESTING: u32 = 16;

/// The `Spawner` struct is the registry of everything a level or prefab can place, and the one place that spawns them.
///
/// Build it once at startup: start from [`Self::with_engine_kinds`], add your application's kinds with
/// [`Self::register`] or [`Self::register_with`], then call [`Self::spawn`] for each [`Loaded`] level or prefab.
/// Next, have each of your systems listen on its kind's factory, `world.factory::<T>().listener()`, before the spawn,
/// and give a kind behavior with [`Runtime::attach`](crate::gameplay::Runtime::attach).
pub struct Spawner {
	kinds: HashMap<Cow<'static, str>, Box<dyn Kind>>,
}

/// Returns the handle a spawn gave entity `entity` of its level, from the handle `known` it gave entity `known_index`.
///
/// One spawn takes one run of handles, and each level or prefab it expands takes consecutive handles in baked order,
/// so any entity's handle and spawn index find every other one's of the same level or prefab. Lightmaps use it to
/// find the entities they light from the entity that holds their `lightmap` kind.
pub fn level_entity_handle(known: Handle, known_index: u32, entity: u32) -> Handle {
	Handle::from_id(known.id() - known_index + entity)
}

/// Returns the handle a spawn gave entity `entity` of the level or prefab it loaded, from the spawn's root, the
/// handle [`Spawner::spawn`] returned.
///
/// The root comes first in the spawn's run of handles, so entity `entity` is `entity + 1` handles after it. Use it to
/// reach an entity you just spawned, such as to move or select it.
pub fn level_entity_handle_from_root(root: Handle, entity: u32) -> Handle {
	Handle::from_id(root.id() + 1 + entity)
}

/// The `EntityRef` struct lets a kind's payload name another entity of the same level or prefab.
///
/// Use it as a field of a payload type passed to [`Spawner::register_with`], and turn it into the entity's handle
/// with [`Self::handle`] in the converter. A level writes it as `{ "$entity": "<id>" }`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EntityRef {
	#[serde(rename = "$entity")]
	entity: NonZeroU32,
}

impl EntityRef {
	/// Returns the handle the referenced entity spawned with.
	pub fn handle(self) -> Handle {
		Handle::from_id(self.entity.get())
	}
}

/// The `Loaded` struct holds a baked level or prefab and every prefab its instances reach, ready to spawn.
///
/// Load it once with [`Self::load`] and spawn it as often as needed with [`Spawner::spawn`], such as one prefab for
/// every unit a player builds. Tools and tests that bake in memory build it with [`Self::level`] or [`Self::prefab`]
/// and add the prefabs it places with [`Self::with_prefab`] or [`Self::complete`].
pub struct Loaded {
	/// What was loaded first, then the prefabs its instances reach.
	templates: Vec<Template>,
	/// The index in `templates` of each held level or prefab, by ID.
	indices: HashMap<String, usize>,
}

/// The `Template` struct is one baked level or prefab and its payload.
struct Template {
	id: String,
	level: Level,
	/// The prefab's parameters with their defaults; none for a level.
	parameters: Vec<NamedValue>,
	payload: Payload,
}

/// The `Payload` enum holds a template's payload: bytes baked in memory, or bytes read from the resource store.
enum Payload {
	Owned(Vec<u8>),
	Read(ReadTargets<'static>),
}

impl Payload {
	fn bytes(&self) -> Option<&[u8]> {
		match self {
			Payload::Owned(bytes) => Some(bytes),
			Payload::Read(targets) => targets.buffer(),
		}
	}
}

impl Loaded {
	/// Holds `level`, baked in memory as `id`, with `payload`, such as what `bake_level` returns.
	pub fn level(id: impl Into<String>, level: Level, payload: Vec<u8>) -> Self {
		Self::from_template(Template {
			id: id.into(),
			level,
			parameters: Vec::new(),
			payload: Payload::Owned(payload),
		})
	}

	/// Holds `prefab`, baked in memory as `id`, with `payload`, such as what `bake_prefab` returns.
	pub fn prefab(id: impl Into<String>, prefab: Prefab, payload: Vec<u8>) -> Self {
		Self::from_template(prefab_template(id.into(), prefab, Payload::Owned(payload)))
	}

	/// Adds `prefab`, baked in memory as `id`, for the instances that place it. An ID already held is kept.
	pub fn with_prefab(mut self, id: impl Into<String>, prefab: Prefab, payload: Vec<u8>) -> Self {
		self.insert(prefab_template(id.into(), prefab, Payload::Owned(payload)));
		self
	}

	/// The spawn index of the entity of the loaded level or prefab that placed the spawned entity `entity`, counted
	/// from the first handle after the root: the entity itself when it's one of the level's own, or the instance whose
	/// prefab spawned it, at any depth. `None` past the last entity a spawn of it adds.
	///
	/// Use it to trace a handle a query found back to what the level's file placed, such as the entity an editor's
	/// click lands on. It follows the handle order [`Spawner::spawn`] gives; see
	/// [spawning](/docs/reference/levels#spawning).
	pub fn placer(&self, entity: u32) -> Option<u32> {
		let root = &self.templates.first()?.level;
		if entity < root.entities {
			return Some(entity);
		}
		// Each instance's prefab, with every prefab nested in it, takes the consecutive handles after the last one's.
		let mut first = root.entities;
		for instance in &root.instances {
			first = first.checked_add(self.spawned(&instance.resource, 0)?)?;
			if entity < first {
				return Some(instance.entity);
			}
		}
		None
	}

	/// How many entities a spawn of the held prefab `id` adds, its nested prefabs' included, or `None` when a prefab
	/// it reaches isn't held or the nesting passes [`MAX_NESTING`].
	fn spawned(&self, id: &str, depth: u32) -> Option<u32> {
		if depth > MAX_NESTING {
			return None;
		}
		let level = &self.templates[*self.indices.get(id)?].level;
		level.instances.iter().try_fold(level.entities, |count, instance| {
			count.checked_add(self.spawned(&instance.resource, depth + 1)?)
		})
	}

	/// Requests the level or prefab `id` from `resources`, and every prefab its instances reach.
	///
	/// An ID that ends in `.prefab` loads a prefab; any other loads a level. Payloads are read in place where the store
	/// maps its files, so they aren't copied. Await it on the thread that owns the world, for example with
	/// `build_single_threaded_async_runtime().block_on(..)` at startup, then spawn with [`Spawner::spawn`].
	pub async fn load(resources: &ResourceManager, id: &str) -> Result<Self, String> {
		let template = if id.ends_with(".prefab") {
			let (prefab, payload) = request::<Prefab>(resources, id).await?;
			prefab_template(id.to_string(), prefab, payload)
		} else {
			let (level, payload) = request::<Level>(resources, id).await?;
			Template {
				id: id.to_string(),
				level,
				parameters: Vec::new(),
				payload,
			}
		};
		Self::from_template(template).complete(resources).await
	}

	/// Requests from `resources` every prefab that the held levels' and prefabs' instances place and that isn't held.
	///
	/// Each prefab loads once, however many instances place it.
	pub async fn complete(mut self, resources: &ResourceManager) -> Result<Self, String> {
		// Templates are visited in the order they were added, so a newly loaded prefab's instances are visited too.
		let mut next = 0;
		while next < self.templates.len() {
			let missing = self.templates[next]
				.level
				.instances
				.iter()
				.map(|instance| instance.resource.clone())
				.filter(|id| !self.indices.contains_key(id))
				.collect::<Vec<_>>();
			for id in missing {
				if self.indices.contains_key(&id) {
					continue;
				}
				let (prefab, payload) = request::<Prefab>(resources, &id).await?;
				self.insert(prefab_template(id, prefab, payload));
			}
			next += 1;
		}
		Ok(self)
	}

	fn from_template(template: Template) -> Self {
		let mut loaded = Self {
			templates: Vec::new(),
			indices: HashMap::new(),
		};
		loaded.insert(template);
		loaded
	}

	fn insert(&mut self, template: Template) {
		if let Entry::Vacant(entry) = self.indices.entry(template.id.clone()) {
			entry.insert(self.templates.len());
			self.templates.push(template);
		}
	}
}

/// Turns a baked prefab into a template.
fn prefab_template(id: String, prefab: Prefab, payload: Payload) -> Template {
	Template {
		id,
		level: prefab.level,
		parameters: prefab.parameters,
		payload,
	}
}

/// Requests the resource `id` and reads its payload in place.
async fn request<T>(resources: &ResourceManager, id: &str) -> Result<(T, Payload), String>
where
	T: resource_management::Resource + 'static,
	T::Model: resource_management::StoredModel<Resource = T>,
{
	let unavailable = |cause: String| {
		format!(
			"'{id}' could not be loaded. The most likely cause is that {cause}. Check that the ID is the level or prefab file's path under the assets directory, and bake it."
		)
	};
	let mut reference = resources
		.request::<T>(id)
		.await
		.map_err(|error| unavailable(format!("it isn't baked or its ID is wrong ({error})")))?;
	let payload = reference
		.load(ReadTargetsMut::backing_storage())
		.await
		.map_err(|_| unavailable("its payload couldn't be read".to_string()))?;
	Ok((reference.into_resource(), Payload::Read(payload)))
}

/// The `Placement` struct says where one spawn places its entities and what it sets the spawned prefab's parameters to.
///
/// The default places a level as it was authored: at the identity transform, as its own scene. Build one with
/// [`Self::at`], then chain [`Self::under`] and [`Self::with_parameters`] as needed, and pass it to
/// [`Spawner::spawn`].
#[derive(Default)]
pub struct Placement {
	transform: Transform,
	parent: Option<Handle>,
	parameters: ParameterValues,
}

impl Placement {
	/// Places the spawn's entities relative to `transform`.
	pub fn at(transform: Transform) -> Self {
		Self {
			transform,
			..Self::default()
		}
	}

	/// Makes the spawn's root a member of `parent`, so deleting `parent` deletes the spawn.
	pub fn under(self, parent: Handle) -> Self {
		Self {
			parent: Some(parent),
			..self
		}
	}

	/// Sets the spawned prefab's parameters from `values`, a struct or map from parameter name to value.
	///
	/// A parameter `values` doesn't name takes its default. Returns the cause and fix when `values` doesn't serialize
	/// to an object of plain values.
	pub fn with_parameters(self, values: &impl serde::Serialize) -> Result<Self, String> {
		let failed = |cause: String| format!("The parameter values could not be encoded. {cause}");
		let values = serde_json::to_value(values).map_err(|error| {
			failed(format!(
				"The most likely cause is that they don't serialize to JSON ({error}). Pass plain data."
			))
		})?;
		Ok(Self {
			parameters: encode_parameter_values(values).map_err(failed)?,
			..self
		})
	}
}

impl Default for Spawner {
	fn default() -> Self {
		Self::new()
	}
}

impl Spawner {
	/// Creates a spawner that knows no kinds but `transform`, which every spawner reads itself to place entities.
	/// Most applications start from [`Self::with_engine_kinds`] instead.
	pub fn new() -> Self {
		Self { kinds: HashMap::new() }
	}

	/// Registers `kind`, whose payloads decode straight into `T` and are published as they are.
	///
	/// Use it for plain data components that derive `serde::Deserialize`; deny unknown fields so a misspelled field
	/// fails the spawn. When the payload needs checking or turning into another type, such as one holding a
	/// [`Handle`], use [`Self::register_with`]. To start a task for every entity that gets one, call
	/// [`Runtime::attach`](crate::gameplay::Runtime::attach) for `T`.
	pub fn register<T>(&mut self, kind: impl Into<Cow<'static, str>>) -> Result<(), String>
	where
		T: DeserializeOwned + Clone + Send + Sync + 'static,
	{
		self.register_with(kind, |component: T| Ok(component))
	}

	/// Registers `kind`, whose payloads decode into `D` and are published as the `T` that `convert` returns.
	///
	/// `convert` runs once per component before anything of the spawn is published, so an `Err` stops the whole
	/// spawn. Return the cause and the fix, as a sentence that completes "the component doesn't fit its kind:".
	/// Large spawns convert on several threads at once, so keep `convert` a pure function of its payload.
	pub fn register_with<D, T>(
		&mut self,
		kind: impl Into<Cow<'static, str>>,
		convert: impl Fn(D) -> Result<T, String> + Send + Sync + 'static,
	) -> Result<(), String>
	where
		D: DeserializeOwned + 'static,
		T: Clone + Send + Sync + 'static,
	{
		self.insert(
			kind.into(),
			Box::new(Converted {
				convert,
				types: PhantomData,
			}),
		)
	}

	/// Registers `kind` as one this build skips: its components aren't decoded or published.
	///
	/// Use it for kinds that only some builds follow, such as rendering kinds on a headless server, so one level
	/// spawns in every build.
	pub fn ignore(&mut self, kind: impl Into<Cow<'static, str>>) -> Result<(), String> {
		self.insert(kind.into(), Box::new(Ignored))
	}

	/// Adds `kind` to the registry unless its name is empty, taken, or `transform`.
	fn insert(&mut self, kind: Cow<'static, str>, decoder: Box<dyn Kind>) -> Result<(), String> {
		if kind.is_empty() {
			return Err(
				"Level kind could not be registered. The most likely cause is that its name is empty. Give the kind a name."
					.to_string(),
			);
		}
		if kind == TRANSFORM {
			return Err(
				"Level kind could not be registered. The most likely cause is that `transform` is the spawner's own kind, which places every entity. Pick another name."
					.to_string(),
			);
		}
		match self.kinds.entry(kind) {
			Entry::Occupied(entry) => Err(format!(
				"Level kind could not be registered. The most likely cause is that '{}' is already registered. Register each kind once, or pick another name.",
				entry.key()
			)),
			Entry::Vacant(entry) => {
				entry.insert(decoder);
				Ok(())
			}
		}
	}

	/// Publishes every entity of `loaded`, and of every prefab its instances place, on `world`, and returns the
	/// spawn's root, which they all belong to.
	///
	/// `placement` places the entities and sets a loaded prefab's parameters. Every component is decoded before
	/// anything is published, so an error leaves the world untouched. Systems created after the spawn don't see it,
	/// so create them first.
	pub fn spawn(&self, world: &MessageScope, loaded: &Loaded, placement: Placement) -> Result<Handle, String> {
		let root_id = &loaded.templates[0].id;
		let failed = |cause: String| format!("'{root_id}' could not be spawned. The most likely cause is that {cause}");

		// Every template's payload, value strings, and kinds are checked once, however often it's expanded.
		let mut arguments = Arguments::default();
		let mut templates = Vec::with_capacity(loaded.templates.len());
		for template in &loaded.templates {
			let payload = template.payload.bytes().ok_or_else(malformed)?;
			arguments
				.strings
				.push(template.level.value_strings(payload).ok_or_else(malformed)?);
			templates.push(self.check(template, payload).map_err(&failed)?);
		}
		// Values from code take the string table after the templates'.
		let code_strings = arguments.strings.len();
		arguments
			.strings
			.push(read_strings(&placement.parameters.strings).ok_or_else(malformed)?);

		// The loaded template is expansion 0; each instance adds the expansion of its prefab, depth first.
		let mut planner = Planner {
			loaded,
			templates: &templates,
			expansions: Vec::new(),
			arguments: Vec::new(),
			entities: 0,
		};
		let root_arguments = resolve(
			&loaded.templates[0],
			placement.parameters.names.iter().map(|value| {
				(
					value,
					Argument {
						tokens: placement.parameters.values.get(value.value as usize..).unwrap_or_default(),
						strings: code_strings,
						scope: None,
					},
				)
			}),
			templates[0].values,
			0,
		)
		.map_err(&failed)?;
		planner.expand(0, None, root_arguments, 0).map_err(&failed)?;
		let Planner {
			expansions,
			arguments: expansion_arguments,
			entities,
			..
		} = planner;
		arguments.expansions = expansion_arguments;

		let root = Handle::run(entities.checked_add(1).ok_or_else(|| failed(TOO_MANY.to_string()))?);
		let first_entity = root.id() + 1;
		// Entity `n` of an expansion spawns as the `first + n`-th handle after the root.
		let handle = |expansion: usize, entity: u32| Handle::from_id(first_entity + expansions[expansion].first + entity);

		// Each run of each kind of each expansion decodes on its own, so they spread over the fast cores.
		let runs = expansions
			.iter()
			.enumerate()
			.flat_map(|(expansion, planned)| {
				templates[planned.template]
					.columns
					.iter()
					.enumerate()
					.flat_map(move |(column, checked)| (0..checked.column.run_count()).map(move |run| (expansion, column, run)))
			})
			.collect::<Vec<_>>();
		let decoded = decode_all(runs.len(), |index| {
			let (expansion, column_index, run) = runs[index];
			let planned = &expansions[expansion];
			let template = &templates[planned.template];
			let column = &template.columns[column_index];
			let values = column
				.column
				.run(template.payload, &column.strings, run)
				.ok_or_else(malformed)?;
			let values = TokenDeserializer::new(values, first_entity + planned.first, Some(expansion), &arguments);
			let count = run_length(column.column, run);
			match column.kind {
				Some(kind) => kind.decode(values, count).map(Run::Components),
				None => decode_transforms(values, count).map(Run::Transforms),
			}
			.map_err(|Failure { component, cause }| {
				let entity = column
					.column
					.entities(template.payload)
					.and_then(|mut entities| entities.nth((run * RUN + component) as usize));
				failed(format!(
					"the `{}` component of {} doesn't fit its kind: {cause}. Fix the component to match the kind.",
					column.column.name,
					describe(&loaded.templates[planned.template], template.payload, entity)
				))
			})
		})?;

		// Transforms compose with their expansion's placement in expansion order, so an instance's own world transform
		// is known before its prefab is placed at it.
		let mut decoded = decoded.into_iter().map(Some).collect::<Vec<_>>();
		let mut transforms = vec![Vec::<(u32, Transform)>::new(); expansions.len()];
		let mut placements = Vec::<Transform>::with_capacity(expansions.len());
		for (expansion, planned) in expansions.iter().enumerate() {
			let at = match planned.owner {
				None => placement.transform.clone(),
				Some((owner, entity)) => {
					let owner_transforms: &Vec<(u32, Transform)> = &transforms[owner];
					match owner_transforms.binary_search_by_key(&entity, |(index, _)| *index) {
						Ok(found) => owner_transforms[found].1.clone(),
						Err(_) => placements[owner].clone(),
					}
				}
			};
			let template = &templates[planned.template];
			if let Some(column_index) = template.transform {
				let column = &template.columns[column_index];
				let mut entities = column.column.entities(template.payload).ok_or_else(malformed)?;
				for (index, &(run_expansion, run_column, _)) in runs.iter().enumerate() {
					if run_expansion != expansion || run_column != column_index {
						continue;
					}
					let Some(Run::Transforms(locals)) = decoded[index].take() else {
						unreachable!("The transform column decodes into transforms.");
					};
					for local in locals {
						let entity = entities.next().ok_or_else(malformed)?;
						transforms[expansion].push((entity, at.compose(&local)));
					}
				}
			}
			placements.push(at);
		}

		match placement.parent {
			Some(parent) => world.factory::<SceneNode>().derive(root, SceneNode::under(parent)),
			None => world.factory::<Scene>().derive(root, Scene),
		}
		for (expansion, planned) in expansions.iter().enumerate() {
			let template = &templates[planned.template];
			let owner = planned.owner.map_or(root, |(owner, entity)| handle(owner, entity));
			world
				.factory::<SceneNode>()
				.derive_many(template.parents.iter().enumerate().map(|(entity, parent)| {
					(
						handle(expansion, entity as u32),
						SceneNode::under(parent.map_or(owner, |parent| handle(expansion, parent))),
					)
				}));
		}
		// Runs are in expansion order, then kind order, then entity order, so each kind's entities are taken run by run.
		let mut entities = None;
		let mut current = None;
		for (&(expansion, column_index, run), components) in runs.iter().zip(decoded) {
			let template = &templates[expansions[expansion].template];
			let column = &template.columns[column_index];
			if current != Some((expansion, column_index)) {
				current = Some((expansion, column_index));
				entities = column.column.entities(template.payload);
				if Some(column_index) == template.transform {
					world.factory::<Transform>().derive_many(
						std::mem::take(&mut transforms[expansion])
							.into_iter()
							.map(|(entity, transform)| (handle(expansion, entity), transform)),
					);
				}
			}
			let Some(Run::Components(components)) = components else {
				continue;
			};
			let entities = entities.as_mut().ok_or_else(malformed)?;
			let length = run_length(column.column, run) as usize;
			components.publish(
				world,
				&mut entities.by_ref().take(length).map(|entity| handle(expansion, entity)),
			);
		}
		Ok(root)
	}

	/// Checks `template`'s sections and finds a decoder for each of its kinds, before any handle is taken.
	fn check<'t>(&'t self, template: &'t Template, payload: &'t [u8]) -> Result<Checked<'t>, String> {
		let level = &template.level;
		let parents = level
			.parents(payload)
			.filter(|parents| parents.len() == level.entities as usize)
			.ok_or_else(malformed)?
			.collect();
		let mut columns = Vec::with_capacity(level.kinds.len());
		let mut transform = None;
		for column in &level.kinds {
			let kind = if column.name == TRANSFORM {
				transform = Some(columns.len());
				None
			} else {
				Some(self.kinds.get(column.name.as_str()).ok_or_else(|| {
					format!(
						"its kind '{}', used by '{}', isn't registered with the spawner. Register the kind before spawning.",
						column.name, template.id
					)
				})?)
			};
			if column
				.entities(payload)
				.is_none_or(|entities| entities.len() != column.count as usize)
			{
				return Err(malformed());
			}
			let strings = column.strings(payload).ok_or_else(malformed)?;
			columns.push(Column {
				column,
				kind: kind.map(|kind| &**kind),
				strings,
			});
		}
		Ok(Checked {
			payload,
			parents,
			columns,
			transform,
			values: level.values.of(payload).ok_or_else(malformed)?,
		})
	}
}

/// The `Checked` struct is one template's payload with its sections checked and its kinds found.
struct Checked<'t> {
	payload: &'t [u8],
	/// The parent of each entity in spawn order.
	parents: Vec<Option<u32>>,
	columns: Vec<Column<'t>>,
	/// The index in `columns` of the `transform` kind, which the spawner decodes and places itself.
	transform: Option<usize>,
	/// The tokens of the template's values: its instances' parameters and its defaults.
	values: &'t [u8],
}

/// The `Column` struct is one kind of a template, with its decoder and its strings.
struct Column<'t> {
	column: &'t LevelKind,
	/// The registered kind, or `None` for `transform`.
	kind: Option<&'t dyn Kind>,
	strings: Vec<&'t str>,
}

/// The `Expansion` struct is one level or prefab a spawn places, and where its handles start.
struct Expansion {
	/// The index of its template in the loaded templates.
	template: usize,
	/// How many handles after the root's first entity its entity 0 is.
	first: u32,
	/// The expansion and spawn index of the instance entity it spawns under; `None` for what was loaded.
	owner: Option<(usize, u32)>,
}

/// The `Planner` struct lays out every expansion of one spawn before any handle is taken.
struct Planner<'l, 't, 'p> {
	loaded: &'l Loaded,
	templates: &'t [Checked<'p>],
	expansions: Vec<Expansion>,
	/// The value of each parameter, for each expansion.
	arguments: Vec<Vec<Argument<'p>>>,
	/// How many entities the expansions so far place.
	entities: u32,
}

impl<'p> Planner<'_, '_, 'p> {
	/// Adds the expansion of template `template` under `owner`, then the expansions of its instances, depth first.
	fn expand(
		&mut self,
		template: usize,
		owner: Option<(usize, u32)>,
		arguments: Vec<Argument<'p>>,
		depth: u32,
	) -> Result<(), String> {
		let loaded = self.loaded;
		let source = &loaded.templates[template];
		if depth > MAX_NESTING {
			return Err(format!(
				"its instances nest more than {MAX_NESTING} deep, at '{}', which happens when a prefab places itself through its instances. Remove the instance that closes the loop.",
				source.id
			));
		}
		let index = self.expansions.len();
		let first = self.entities;
		self.entities = self
			.entities
			.checked_add(source.level.entities)
			.ok_or_else(|| TOO_MANY.to_string())?;
		self.expansions.push(Expansion { template, first, owner });
		self.arguments.push(arguments);

		for instance in &source.level.instances {
			let child = *loaded.indices.get(&instance.resource).ok_or_else(|| {
				format!(
					"prefab '{}', placed by '{}', isn't loaded. Load with `Loaded::load` or `Loaded::complete`, which load every prefab instances place.",
					instance.resource, source.id
				)
			})?;
			let values = self.templates[template].values;
			let arguments = resolve(
				&loaded.templates[child],
				instance.parameters.iter().map(|value| {
					(
						value,
						Argument {
							tokens: values.get(value.value as usize..).unwrap_or_default(),
							strings: template,
							scope: Some(index),
						},
					)
				}),
				self.templates[child].values,
				child,
			)?;
			self.expand(child, Some((index, instance.entity)), arguments, depth + 1)?;
		}
		Ok(())
	}
}

/// Returns the value of each parameter of `template`: the one `set` names, otherwise the parameter's default.
///
/// `defaults` are the template's values and `strings` the index of their string table. Rejects a value for a
/// parameter the template doesn't declare.
fn resolve<'v, 'p>(
	template: &Template,
	set: impl Iterator<Item = (&'v NamedValue, Argument<'p>)>,
	defaults: &'p [u8],
	strings: usize,
) -> Result<Vec<Argument<'p>>, String> {
	let mut arguments = template
		.parameters
		.iter()
		.map(|parameter| Argument {
			tokens: defaults.get(parameter.value as usize..).unwrap_or_default(),
			strings,
			scope: None,
		})
		.collect::<Vec<_>>();
	for (value, argument) in set {
		let index = template
			.parameters
			.iter()
			.position(|parameter| parameter.name == value.name)
			.ok_or_else(|| {
				format!(
					"a value is set for parameter '{}', which '{}' doesn't declare. Remove the value or add the parameter to the prefab.",
					value.name, template.id
				)
			})?;
		arguments[index] = argument;
	}
	Ok(arguments)
}

/// The `Run` enum is one decoded run: transforms, which the spawner places, or a registered kind's components.
enum Run {
	Transforms(Vec<Transform>),
	Components(Box<dyn Decoded + Send>),
}

/// Decodes the `count` transform payloads of one run, relative to their spawn's placement.
fn decode_transforms(mut values: TokenDeserializer<'_, '_>, count: u32) -> Result<Vec<Transform>, Failure> {
	(0..count)
		.map(|component| {
			let fail = |cause: String| Failure { component, cause };
			let transform = SerializableTransform::deserialize(&mut values).map_err(|error| fail(error.to_string()))?;
			Transform::try_from(transform).map_err(fail)
		})
		.collect()
}

/// Reads a strings section, or returns `None` when a string isn't UTF-8.
fn read_strings(mut bytes: &[u8]) -> Option<Vec<&str>> {
	let mut strings = Vec::new();
	while let Some((length, rest)) = bytes.split_first_chunk::<4>() {
		let (text, rest) = rest.split_at_checked(u32::from_le_bytes(*length) as usize)?;
		strings.push(std::str::from_utf8(text).ok()?);
		bytes = rest;
	}
	Some(strings)
}

/// The cause of a spawn whose expanded prefabs place more entities than one run of handles holds.
const TOO_MANY: &str = "its instances place more entities than one spawn holds. Place fewer instances per spawn.";

/// The `Kind` trait decodes one kind's components, keeping their type out of the spawner.
///
/// A spawn decodes several runs of one kind at once, on separate threads.
trait Kind: Send + Sync {
	/// Decodes and converts the `count` components of one run, or reports the first that fails.
	fn decode(&self, values: TokenDeserializer<'_, '_>, count: u32) -> Result<Box<dyn Decoded + Send>, Failure>;
}

/// The `Decoded` trait publishes one kind's components once every kind of the level decoded.
trait Decoded {
	/// Publishes each component under the handle `handles` yields for it, as one batch.
	fn publish(self: Box<Self>, world: &MessageScope, handles: &mut dyn ExactSizeIterator<Item = Handle>);
}

/// The `Failure` struct names the component of a kind that failed to decode or convert, and why.
struct Failure {
	component: u32,
	cause: String,
}

/// The `Converted` struct is a registered kind: its payload type `D`, its component type `T`, and the conversion.
struct Converted<D, T, F> {
	convert: F,
	types: PhantomData<fn(D) -> T>,
}

impl<D, T, F> Kind for Converted<D, T, F>
where
	D: DeserializeOwned,
	T: Clone + Send + Sync + 'static,
	F: Fn(D) -> Result<T, String> + Send + Sync,
{
	fn decode(&self, mut values: TokenDeserializer<'_, '_>, count: u32) -> Result<Box<dyn Decoded + Send>, Failure> {
		// Each payload decodes through `D`'s own derived code, straight from the tokens.
		let mut components = Vec::with_capacity(count as usize);
		for component in 0..count {
			let payload = D::deserialize(&mut values).map_err(|error| Failure {
				component,
				cause: error.to_string(),
			})?;
			components.push((self.convert)(payload).map_err(|cause| Failure { component, cause })?);
		}
		Ok(Box::new(Components(components)))
	}
}

/// The `Ignored` struct is a kind this build skips, so its components are neither decoded nor published.
struct Ignored;

impl Kind for Ignored {
	fn decode(&self, _: TokenDeserializer<'_, '_>, _: u32) -> Result<Box<dyn Decoded + Send>, Failure> {
		Ok(Box::new(Ignored))
	}
}

impl Decoded for Ignored {
	fn publish(self: Box<Self>, _: &MessageScope, _: &mut dyn ExactSizeIterator<Item = Handle>) {}
}

/// The `Components` struct holds one kind's decoded components until the level publishes.
struct Components<T>(Vec<T>);

impl<T: Clone + Send + Sync + 'static> Decoded for Components<T> {
	fn publish(self: Box<Self>, world: &MessageScope, handles: &mut dyn ExactSizeIterator<Item = Handle>) {
		world.factory::<T>().derive_many(handles.zip(self.0));
	}
}

/// Returns how many payloads run `run` of `column` holds: [`RUN`], or fewer for the last.
fn run_length(column: &LevelKind, run: u32) -> u32 {
	(column.count - run * RUN).min(RUN)
}

/// Calls `decode` for every index below `count`, spread over the machine's fast cores, and returns the results in
/// index order, or the error of the lowest index that failed.
///
/// One index or one core decodes on the calling thread, so small levels start no threads.
fn decode_all<T: Send>(count: usize, decode: impl Fn(usize) -> Result<T, String> + Sync) -> Result<Vec<T>, String> {
	let threads = performance_parallelism().min(count);
	if threads <= 1 {
		return (0..count).map(decode).collect();
	}

	let next = AtomicUsize::new(0);
	// Each thread takes the next undecoded index until none are left, so uneven runs balance themselves.
	let work = || {
		let mut done = Vec::new();
		loop {
			let index = next.fetch_add(1, Ordering::Relaxed);
			if index >= count {
				return done;
			}
			done.push((index, decode(index)));
		}
	};
	let mut results = std::iter::repeat_with(|| None).take(count).collect::<Vec<_>>();
	std::thread::scope(|scope| {
		let helpers = (1..threads).map(|_| scope.spawn(work)).collect::<Vec<_>>();
		let mine = work();
		let theirs = helpers
			.into_iter()
			.flat_map(|helper| helper.join().unwrap_or_else(|panic| std::panic::resume_unwind(panic)));
		for (index, result) in mine.into_iter().chain(theirs) {
			results[index] = Some(result);
		}
	});
	results
		.into_iter()
		.map(|result| result.expect("Every index below the count is decoded once."))
		.collect()
}

/// Names an entity of `template` for a message: by its id, or by its spawn index when it has none.
fn describe(template: &Template, payload: &[u8], entity: Option<u32>) -> String {
	let id = &template.id;
	match entity {
		Some(entity) => match template.level.id(payload, entity) {
			Some(entity) => format!("entity '{entity}' of '{id}'"),
			None => format!("the entity without an id at spawn index {entity} of '{id}'"),
		},
		None => format!("an entity of '{id}'"),
	}
}

/// Reports a payload that doesn't hold what its level describes.
fn malformed() -> String {
	"Level or prefab could not be spawned. The most likely cause is that its payload doesn't match its description, as when it was baked by another engine version. Bake it again."
		.to_string()
}

#[cfg(test)]
mod tests;
