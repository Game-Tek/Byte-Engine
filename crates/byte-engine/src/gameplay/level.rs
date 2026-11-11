//! Spawning baked levels: the entities of a `.level` file, published on the world scope.
//!
//! A [`Spawner`] maps each kind name a level uses to the component type it publishes. Register your application's
//! kinds next to the engine's ([`Spawner::with_engine_kinds`]), create the systems that follow those components,
//! then call [`Spawner::spawn_resource`], which loads and spawns the level off the frame. Delete the returned handle to
//! unload the level through the [`SceneGraph`](crate::gameplay::SceneGraph).
//!
//! Spawning is built to be fast: baking already parsed the text, ordered the entities, and grouped the components by
//! kind, so a spawn takes one run of handles, decodes each kind in one pass, and publishes each kind as one batch.

mod kinds;
mod tokens;

use std::{
	borrow::Cow,
	collections::{HashMap, hash_map::Entry},
	marker::PhantomData,
	sync::Arc,
	num::NonZeroU32,
	sync::atomic::{AtomicUsize, Ordering},
};

pub use resource_management::resources::level::Level;
use resource_management::resources::level::{LevelKind, RUN};
use resource_management::{ResourceManager, resource::read_target::ReadTargetsMut};
use serde::de::DeserializeOwned;
use tokens::TokenDeserializer;

use crate::{
	core::{EntityHandle, factory::Handle, machine::performance_parallelism, message_bus::MessageScope},
	gameplay::{Scene, SceneNode},
};

/// The field that turns a payload object into a reference to another entity of the level.
const ENTITY_REFERENCE: &str = "$entity";

/// The `Spawner` struct is the registry of everything a level can place, and the one place that spawns levels.
///
/// Build it once at startup: start from [`Self::with_engine_kinds`], add your application's kinds with
/// [`Self::register`] or [`Self::register_with`], then call [`Self::spawn_resource`] for each level. Next, have each
/// of your systems listen on its kind's factory, `world.factory::<T>().listener()`, before the spawn.
///
/// Cloning a spawner shares its kinds, which is how a spawn takes them to its own thread. Register every kind before
/// the first clone or spawn.
#[derive(Clone)]
pub struct Spawner {
	kinds: Arc<HashMap<Cow<'static, str>, Box<dyn Kind>>>,
}

/// Returns the handle a spawn gave entity `entity` of its level, from the handle `known` it gave entity `known_index`.
///
/// One spawn takes one run of handles, the level's root first and then each entity in baked order, so any entity's
/// handle and spawn index find every other one's. Lightmaps use it to find the entities they light from the entity
/// that holds their `lightmap` kind.
pub fn level_entity_handle(known: Handle, known_index: u32, entity: u32) -> Handle {
	Handle::from_id(known.id() - known_index + entity)
}

/// The `EntityRef` struct lets a kind's payload name another entity of the same level.
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

impl Default for Spawner {
	fn default() -> Self {
		Self::new()
	}
}

impl Spawner {
	/// Creates a spawner that knows no kinds. Most applications start from [`Self::with_engine_kinds`] instead.
	pub fn new() -> Self {
		Self {
			kinds: Arc::new(HashMap::new()),
		}
	}

	/// Registers `kind`, whose payloads decode straight into `T` and are published as they are.
	///
	/// Use it for plain data components that derive `serde::Deserialize`; deny unknown fields so a misspelled field
	/// fails the spawn. When the payload needs checking or turning into another type, such as one holding a
	/// [`Handle`], use [`Self::register_with`].
	pub fn register<T>(&mut self, kind: impl Into<Cow<'static, str>>) -> Result<(), String>
	where
		T: DeserializeOwned + Clone + Send + Sync + 'static,
	{
		self.register_with(kind, |component: T| Ok(component))
	}

	/// Registers `kind`, whose payloads decode into `D` and are published as the `T` that `convert` returns.
	///
	/// `convert` runs once per component before anything of the level is published, so an `Err` stops the whole
	/// spawn. Return the cause and the fix, as a sentence that completes "the component doesn't fit its kind:".
	/// Large levels convert on several threads at once, so keep `convert` a pure function of its payload.
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

	/// Adds `kind` to the registry unless its name is empty or taken.
	fn insert(&mut self, kind: Cow<'static, str>, decoder: Box<dyn Kind>) -> Result<(), String> {
		if kind.is_empty() {
			return Err(
				"Level kind could not be registered. The most likely cause is that its name is empty. Give the kind a name."
					.to_string(),
			);
		}
		let kinds = Arc::get_mut(&mut self.kinds).ok_or_else(|| {
			format!(
				"Level kind '{kind}' could not be registered. The most likely cause is that the spawner was cloned or already spawns a level, which shares its kinds. Register every kind before the first spawn."
			)
		})?;
		match kinds.entry(kind) {
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

	/// Loads the level `id` from `resources` and spawns it on a thread of its own, so no frame waits for it.
	///
	/// Reading the baked level, decoding its components, and publishing them all happen off the calling thread. The
	/// level's entities reach the systems that listen for them on a later frame, the same way loaded meshes and
	/// textures do. Keep the returned [`LevelSpawn`] to learn the level's root handle, or why it couldn't spawn, with
	/// [`LevelSpawn::poll`]; dropping it lets the spawn finish anyway.
	///
	/// Create the systems and listeners that follow the level's kinds before you call it, since they only see what is
	/// published after they exist. Next, read their listeners each frame as usual.
	pub fn spawn_resource(&self, world: &MessageScope, resources: EntityHandle<ResourceManager>, id: &str) -> LevelSpawn {
		let (sender, receiver) = std::sync::mpsc::channel();
		let spawner = self.clone();
		let world = world.clone();
		let id = id.to_owned();
		let spawned = std::thread::Builder::new().name(format!("Level spawn {id}")).spawn(move || {
			let result = match compio::runtime::Runtime::new() {
				Ok(runtime) => runtime.block_on(spawner.load_and_spawn(&world, &resources, &id)),
				Err(error) => Err(format!(
					"Level '{id}' could not be loaded. The most likely cause is that the system ran out of file descriptors or threads ({error}). Close other applications, or raise the open file limit."
				)),
			};
			// The caller may have dropped its LevelSpawn; the level spawned anyway.
			let _ = sender.send(result);
		});
		// The closure that held the sender is gone when the thread didn't start, so the failure is kept up front.
		let result = spawned.err().map(|error| {
			Err(format!(
				"Level spawn could not start. The most likely cause is that the system ran out of threads ({error}). Close other applications and try again."
			))
		});
		LevelSpawn { receiver, result }
	}

	/// Requests the level `id` from `resources`, loads its payload, and spawns it with [`Self::spawn`].
	///
	/// The payload is read in place where the store maps its files, so it isn't copied.
	async fn load_and_spawn(&self, world: &MessageScope, resources: &ResourceManager, id: &str) -> Result<Handle, String> {
		let unavailable = |cause: String| {
			format!(
				"Level '{id}' could not be loaded. The most likely cause is that {cause}. Check that the ID is the level file's path under the assets directory, and bake it."
			)
		};
		let mut level = resources
			.request::<Level>(id)
			.await
			.map_err(|error| unavailable(format!("it isn't baked or its ID is wrong ({error})")))?;
		let payload = level
			.load(ReadTargetsMut::backing_storage())
			.await
			.map_err(|_| unavailable("its payload couldn't be read".to_string()))?;
		let payload = payload.buffer().ok_or_else(malformed)?;
		self.spawn(world, level.resource(), payload)
	}

	/// Publishes every entity of `level` on `world` and returns the level's root, a [`Scene`] they all belong to.
	///
	/// `payload` is the level's loaded payload. Every component is decoded before anything is published, so an
	/// error leaves the world untouched. Systems created after the spawn don't see it, so create them first.
	pub fn spawn(&self, world: &MessageScope, level: &Level, payload: &[u8]) -> Result<Handle, String> {
		let parents = level
			.parents(payload)
			.filter(|parents| parents.len() == level.entities as usize)
			.ok_or_else(malformed)?;
		// Every kind is found and every section is checked before handles are taken.
		let mut columns = Vec::with_capacity(level.kinds.len());
		for column in &level.kinds {
			let kind = self.kinds.get(column.name.as_str()).ok_or_else(|| {
				format!(
					"Level could not be spawned. The most likely cause is that its kind '{}' isn't registered with the spawner. Register the kind before spawning.",
					column.name
				)
			})?;
			let entities = column
				.entities(payload)
				.filter(|entities| entities.len() == column.count as usize)
				.ok_or_else(malformed)?;
			let strings = column.strings(payload).ok_or_else(malformed)?;
			columns.push((column, kind, entities, strings));
		}

		let root = Handle::run(level.entities.checked_add(1).ok_or_else(malformed)?);
		let first_entity = root.id() + 1;
		// Entity `n` of the level spawns as the `n`-th handle after the root.
		let handle = |entity: u32| {
			debug_assert!(entity < level.entities, "A baked level names an entity it doesn't place.");
			Handle::from_id(first_entity + entity)
		};

		// Each run of a kind's payloads decodes on its own, so the runs of every kind spread over the fast cores.
		let runs = columns
			.iter()
			.enumerate()
			.flat_map(|(index, (column, ..))| (0..column.run_count()).map(move |run| (index, run)))
			.collect::<Vec<_>>();
		let decoded = decode_all(runs.len(), |index| {
			let (column_index, run) = runs[index];
			let (column, kind, _, strings) = &columns[column_index];
			let first = run * RUN;
			let values = column.run(payload, strings, run).ok_or_else(malformed)?;
			kind.decode(TokenDeserializer::new(values, first_entity), run_length(column, run))
				.map_err(|Failure { component, cause }| {
					let entity = column
						.entities(payload)
						.and_then(|mut entities| entities.nth((first + component) as usize));
					format!(
						"Level could not be spawned. The most likely cause is that the `{}` component of {} doesn't fit its kind: {cause}. Fix the component to match the kind.",
						column.name,
						describe(level, payload, entity)
					)
				})
		})?;

		world.factory::<Scene>().derive(root, Scene);
		world.factory::<SceneNode>().derive_many(
			parents
				.enumerate()
				.map(|(entity, parent)| (handle(entity as u32), SceneNode::under(parent.map_or(root, handle)))),
		);
		// Runs are in kind order, then in entity order, so each kind's entities are taken run by run.
		for (&(column_index, run), components) in runs.iter().zip(decoded) {
			let (column, _, entities, _) = &mut columns[column_index];
			let length = run_length(column, run) as usize;
			components.publish(world, &mut entities.by_ref().take(length).map(handle));
		}
		Ok(root)
	}
}

/// The `LevelSpawn` struct follows one level that [`Spawner::spawn_resource`] loads and spawns off the frame.
///
/// Poll it once a frame to learn when the level's entities were published, and under which root handle. Delete that
/// handle to unload the level.
pub struct LevelSpawn {
	receiver: std::sync::mpsc::Receiver<Result<Handle, String>>,
	result: Option<Result<Handle, String>>,
}

impl LevelSpawn {
	/// Returns the level's root handle once every entity was published, the reason it couldn't spawn, or `None` while
	/// it still loads. It never blocks.
	pub fn poll(&mut self) -> Option<&Result<Handle, String>> {
		if self.result.is_none() {
			self.result = match self.receiver.try_recv() {
				Ok(result) => Some(result),
				Err(std::sync::mpsc::TryRecvError::Empty) => None,
				Err(std::sync::mpsc::TryRecvError::Disconnected) => Some(Err(
					"Level spawn stopped before it finished. The most likely cause is a panic while decoding the level, which the log shows. Fix the level or the kind that panicked, and bake again.".to_string(),
				)),
			};
		}
		self.result.as_ref()
	}

	/// Blocks until the level spawned and returns its root handle, or the reason it couldn't spawn.
	///
	/// Use it in tools and tests that have no frames to keep running. An application calls [`Self::poll`] instead.
	pub fn wait(mut self) -> Result<Handle, String> {
		if let Some(result) = self.result.take() {
			return result;
		}
		self.receiver.recv().unwrap_or_else(|_| {
			Err("Level spawn stopped before it finished. The most likely cause is a panic while decoding the level, which the log shows. Fix the level or the kind that panicked, and bake again.".to_string())
		})
	}
}

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

/// Names an entity for a message: by its id, or by its spawn index when it has none.
fn describe(level: &Level, payload: &[u8], entity: Option<u32>) -> String {
	match entity {
		Some(entity) => match level.id(payload, entity) {
			Some(id) => format!("entity '{id}'"),
			None => format!("the entity without an id at spawn index {entity}"),
		},
		None => "an entity".to_string(),
	}
}

/// Reports a payload that doesn't hold what its level describes.
fn malformed() -> String {
	"Level could not be spawned. The most likely cause is that its payload doesn't match its description, as when the level was baked by another engine version. Bake the level again."
		.to_string()
}

#[cfg(test)]
mod tests;
