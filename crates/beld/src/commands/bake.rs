use std::{
	cell::RefCell,
	num::NonZeroUsize,
	path::{Component, Path, PathBuf},
	sync::Arc,
	time::Instant,
};

use byte_engine::{
	application::{Parameter, parameters::Parameters as _},
	rendering::pipelines::visibility::{RENDER_PARAMETERS, VisibilityFeatures},
};
use resource_management::{
	asset::{FileStorageBackend, ResourceId, intercept::BakeInterceptor, manager::AssetManager, manager::LoadMessages},
	inspect::{Base64View, ResourceDocument, inspect_payload, inspect_resource},
	resource::{
		MemoryStorageBackend, ReDBStorageBackend, ReadStorageBackend, ResourceGpuCompressionPolicy, ResourceStorageMode,
		ResourceStorageSettings,
	},
};
use serde::Serialize;
use serde_json::{Value, json};
use utils::{Glob, r#async::StreamExt, sync::Mutex};

use crate::{
	OutputFormat, Payload,
	commands::shared::{attach_trace, offload_file_operation, payload_format, plural, print_human, read_payload, write_json},
	terminal,
	utils::{get_asset_manager, globs},
};

/// Bakes selected source assets, or every discoverable asset when `ids` is empty.
///
/// `parameters` are the application's, resolved with
/// [`layer_parameters`](byte_engine::application::parameters::layer_parameters), and `configure` installs the
/// application's bake services and asset handlers for them, as its debug runtime does. Assets whose stored resource is current with its
/// source files are skipped unless `force` is set. Force a bake after changing an asset processor,
/// `texture_compression`, or a baked parameter, because source versions don't record those.
///
/// These parameters also select what bakes. The `render` scope selects the visibility features, and discovery skips
/// the engine assets only left-out features use. Discovery skips assets that match a `assets.exclude` pattern, and a
/// bake of one, or of an asset that needs one, fails. When there are `resources.roots` patterns, discovery keeps only
/// matching assets, which bake with what they need. Roots don't apply to explicit `ids`. `prune` deletes every stored
/// resource no root reaches once every asset baked; it needs roots, no `ids`, and a debug build, which records
/// references.
///
/// On a terminal, a progress line shows on standard error; install the logger with [`crate::init_logging`] so log
/// messages print above it.
///
/// Call [`crate::list`] next to inspect the resource IDs written to the destination.
#[allow(clippy::too_many_arguments)]
pub async fn bake(
	source_path: String,
	destination_path: String,
	ids: Vec<String>,
	storage_mode: Option<ResourceStorageMode>,
	texture_compression: Option<ResourceGpuCompressionPolicy>,
	memory_budget: NonZeroUsize,
	force: bool,
	parameters: &[Parameter],
	configure: impl FnOnce(&mut AssetManager, &[Parameter]),
	prune: bool,
) -> Result<(), i32> {
	let features = VisibilityFeatures::from_parameters(parameters.scope(RENDER_PARAMETERS));
	let roots = globs(parameters.scope("resources"), "roots");
	if prune {
		check_prune(&ids, &roots).map_err(|error| {
			log::error!("{error}");
			2
		})?;
	}

	let asset_storage_backend = open_asset_storage(source_path).await?;
	// The prune reopens the store after the bake closes it.
	#[cfg(debug_assertions)]
	let prune_destination = destination_path.clone();
	let destination_path = std::path::PathBuf::from(destination_path);
	let resource_storage_backend = offload_file_operation(move || match texture_compression {
		Some(compression) => ReDBStorageBackend::new_writable_with_settings(
			destination_path,
			ResourceStorageSettings::new(storage_mode.unwrap_or_default()).image_compression(compression),
		),
		None => match storage_mode {
			Some(mode) => ReDBStorageBackend::new_writable_with_mode(destination_path, mode),
			None => ReDBStorageBackend::new_writable(destination_path),
		},
	})
	.await
	.map_err(|error| {
		log::error!("Failed to bake resources. {error}");
		1
	})?;

	let mut asset_manager = get_asset_manager(asset_storage_backend, resource_storage_backend, parameters, configure);
	asset_manager.set_excluded_assets(globs(parameters.scope("assets"), "exclude"));

	// A forced run treats everything stored before it started as stale, so shared dependencies rebuild once.
	if force {
		asset_manager.rebuild_resources_baked_before(std::time::SystemTime::now());
	}

	let ids = if ids.is_empty() {
		let mut ids = asset_manager.discover().await.map_err(|error| {
			log::error!("Failed to discover assets. {error}");
			1
		})?;
		// Explicitly requested IDs still bake; only discovery skips what the project left out.
		ids.retain(|id| !features.excludes(id));
		select_discovered(ids, &roots, |id| asset_manager.is_excluded(id)).map_err(|error| {
			log::error!("{error}");
			1
		})?
	} else {
		ids
	};

	if ids.is_empty() {
		log::info!("No supported assets found to bake.");

		return Ok(());
	}

	let mut failures = bake_ids(&mut asset_manager, ids, memory_budget).await;
	failures.extend(bake_assembled(&asset_manager, &features).await);

	if !failures.is_empty() {
		if prune {
			log::error!(
				"Pruning was skipped, so no resource was deleted. The most likely cause is that {} failed to bake, and a failed root would make the resources it uses look unreachable. Fix the failed assets and run `beld bake --prune` again.",
				plural(failures.len(), "asset")
			);
		}
		return Err(1);
	}

	// `check_prune` rejects pruning in release builds before anything bakes.
	#[cfg(debug_assertions)]
	if prune {
		// The store must be closed for writing before the prune reads and then deletes from it.
		drop(asset_manager);
		return prune_unreachable(prune_destination, &roots).await;
	}

	Ok(())
}

/// Returns why `bake --prune` can't run with these `ids` and `roots`, if it can't.
fn check_prune(ids: &[String], roots: &[Glob]) -> Result<(), String> {
	if !cfg!(debug_assertions) {
		return Err("`beld bake --prune` needs a debug build of BELD. The most likely cause is that this BELD is a release build, which records no references, so every resource would look unreachable. Run a debug build of BELD to prune.".to_string());
	}
	if !ids.is_empty() {
		return Err("`beld bake --prune` can't take asset IDs. The most likely cause is that IDs were given with `--prune`, but pruning follows only a full bake. Run `beld bake --prune` without IDs.".to_string());
	}
	if roots.is_empty() {
		return Err(format!(
			"`beld bake --prune` needs roots. The most likely cause is that the configuration has no `resources.roots` patterns, so no resource is reachable and every one would be deleted. Add the IDs your application loads to `resources.roots` in `config.json`, or pass another file with `--config`. See {}.",
			utils::online_docs_url("api/beld#pruning")
		));
	}
	Ok(())
}

/// Deletes every resource in the store at `destination_path` that no root in `roots` reaches.
#[cfg(debug_assertions)]
async fn prune_unreachable(destination_path: String, roots: &[Glob]) -> Result<(), i32> {
	let ids = super::catalog::unreachable_ids(&destination_path, roots)
		.await
		.map_err(|error| {
			log::error!("Failed to prune the resource store. {error}");
			1
		})?;
	log::info!("Pruning {} that no root reaches", plural(ids.len(), "resource"));
	super::maintenance::delete(destination_path, ids).await
}

/// Returns the discovered assets a full bake starts from: those no excluded pattern matches and, when there are
/// `roots`, that a root matches.
///
/// Returns an error naming every asset that matches both a root and an excluded pattern, because the project asks to
/// bake and never bake it at once.
fn select_discovered(
	mut discovered: Vec<String>,
	roots: &[Glob],
	excluded: impl Fn(&str) -> bool,
) -> Result<Vec<String>, String> {
	let is_root = |id: &str| roots.iter().any(|root| root.matches(id));

	let conflicts = discovered
		.iter()
		.filter(|id| is_root(id) && excluded(id))
		.map(|id| format!("'{id}'"))
		.collect::<Vec<_>>();
	if !conflicts.is_empty() {
		return Err(format!(
			"The bake stopped before baking anything, because these assets match both `resources.roots` and `assets.exclude`: {}. The most likely cause is that a root pattern and an exclude pattern overlap. Remove each of these assets from one of the two lists in the configuration. See {}.",
			conflicts.join(", "),
			utils::online_docs_url("api/beld#commands")
		));
	}

	discovered.retain(|id| !excluded(id) && (roots.is_empty() || is_root(id)));
	Ok(discovered)
}

/// Bakes `ids` and every resource they depend on into memory, and writes everything produced as one document to
/// `output`, leaving the destination directory untouched.
///
/// The document lists each stored resource in [`crate::inspect`]'s shape, ordered by ID, then each requested ID that
/// failed. `parameters` and `configure` are the application's, as in [`bake`], which also apply `assets.exclude`.
/// Pass `payload` to add the decoded stream bytes, which needs [`OutputFormat::JSON`]. The resources the visibility
/// features assemble from every material bake after `ids`, from the materials `ids` reach. Without `ids` nothing bakes,
/// because a dry run doesn't discover assets. Run [`bake`] next to write the same resources to a store.
///
/// With `intercept` patterns, the document lists instead the intermediate products of the bake whose names match one
/// of them, such as `*.besl` for the BESL source of every shader; see [`resource_management::asset::intercept`] for how
/// products are named. With `intercept_out`, each product is written to a file under that directory, named after it, and
/// the human format lists the written paths instead of the products.
#[allow(clippy::too_many_arguments)]
pub async fn bake_dry_run(
	source_path: String,
	ids: Vec<String>,
	memory_budget: NonZeroUsize,
	parameters: &[Parameter],
	configure: impl FnOnce(&mut AssetManager, &[Parameter]),
	intercept: Vec<String>,
	intercept_out: Option<&Path>,
	format: OutputFormat,
	payload: Payload,
	output: &mut impl std::io::Write,
) -> Result<(), i32> {
	let payload = payload_format(payload, format)?;

	let asset_storage_backend = open_asset_storage(source_path).await?;
	// The manager owns one clone; this one reads back what it baked once it is done.
	let resource_storage_backend = MemoryStorageBackend::new();
	let mut asset_manager = get_asset_manager(asset_storage_backend, resource_storage_backend.clone(), parameters, configure);
	asset_manager.set_excluded_assets(globs(parameters.scope("assets"), "exclude"));
	let interception = (!intercept.is_empty()).then(|| {
		Arc::new(Interception {
			globs: intercept.iter().map(|pattern| Glob::new(pattern)).collect(),
			products: Mutex::new(Vec::new()),
		})
	});
	if let Some(interception) = &interception {
		asset_manager.set_interceptor(interception.clone());
	}
	let bakes_anything = !ids.is_empty();
	let mut failures = bake_ids(&mut asset_manager, ids, memory_budget).await;
	if bakes_anything {
		let features = VisibilityFeatures::from_parameters(parameters.scope(RENDER_PARAMETERS));
		failures.extend(bake_assembled(&asset_manager, &features).await);
	}
	drop(asset_manager);

	// The manager held the only other reference, so the products are now this function's alone.
	let intercepted = interception.map(|interception| {
		let mut products = Arc::into_inner(interception)
			.expect("The asset manager is dropped, so nothing else holds the interception.")
			.products
			.into_inner();
		products.sort_unstable_by(|(a, _), (b, _)| a.cmp(b));
		if products.is_empty() {
			log::warn!(
				"The dry run intercepted no product matching the `--intercept` patterns. The most likely cause is that the baked IDs build no such product, such as an image for `*.besl`, or that a pattern misses the `<resource ID>.<extension>` name. Pass `--intercept '*'` to list every product the bake builds."
			);
		}
		products
	});
	let written = match (&intercepted, intercept_out) {
		(Some(products), Some(directory)) => Some(write_product_files(directory, products)?),
		_ => None,
	};

	let mut resource_ids = resource_storage_backend
		.list()
		.await
		.map_err(|error| {
			log::error!(
				"Failed to list the baked resources. The most likely cause is an internal storage error. Error: {error}"
			);
			1
		})?
		.into_iter()
		.map(|resource| resource.id().to_owned())
		.collect::<Vec<_>>();
	resource_ids.sort_unstable();

	// Read every resource first: payload inspections borrow the resources and their bytes until the document is written.
	let mut resources = Vec::with_capacity(resource_ids.len());
	for id in &resource_ids {
		let Some((resource, reader)) = resource_storage_backend.read(ResourceId::new(id)).await else {
			continue;
		};
		let mut inspection = inspect_resource(&resource).map_err(|error| {
			log::error!("{}", error);
			1
		})?;
		attach_trace(&mut inspection.json, &resource_storage_backend, id).await?;
		let payload_bytes = match payload {
			Some(_) => Some(read_payload(&resource, reader).await?),
			None => None,
		};
		resources.push((resource, inspection.json, payload_bytes));
	}

	let mut failed = Vec::with_capacity(failures.len());
	for (id, error) in &failures {
		let mut entry = json!({ "id": id, "error": format!("{error:?}") });
		attach_trace(&mut entry, &resource_storage_backend, id).await?;
		failed.push(entry);
	}

	match format {
		// Intercepted products print as they are, so a reader can pipe one straight into a file.
		OutputFormat::Human if let Some(products) = &intercepted => {
			match &written {
				Some(paths) => write_paths(output, paths)?,
				None => write_products(output, products)?,
			}
			if !failed.is_empty() {
				print_human(&json!({ "failed": failed }));
			}
		}
		OutputFormat::Human => {
			let resources = resources.into_iter().map(|(_, json, _)| json).collect();
			print_human(&json!({ "resources": Value::Array(resources), "failed": failed }));
		}
		OutputFormat::JSON => {
			let payloads = resources
				.iter()
				.map(|(resource, _, bytes)| match (payload, bytes) {
					(Some(payload), Some(bytes)) => inspect_payload(resource, bytes, payload).map(Some),
					_ => Ok(None),
				})
				.collect::<Result<Vec<_>, _>>()
				.map_err(|error| {
					log::error!("{}", error);
					1
				})?;
			let resources = resources
				.iter()
				.zip(&payloads)
				.map(|((_, json, _), payload)| ResourceDocument {
					json,
					payload: payload.as_ref(),
				})
				.collect();
			let intercepted = intercepted.as_ref().map(|products| {
				products
					.iter()
					.map(|(name, content)| InterceptedDocument {
						name,
						content: match std::str::from_utf8(content) {
							Ok(text) => Content::Text(text),
							Err(_) => Content::Bytes(Base64View(content)),
						},
					})
					.collect()
			});
			write_json(
				output,
				&DryRunDocument {
					resources,
					failed,
					intercepted,
				},
			)?;
		}
	}

	if failures.is_empty() { Ok(()) } else { Err(1) }
}

/// The `Interception` struct collects the bake products whose names match a dry run's `--intercept` patterns.
struct Interception {
	globs: Vec<Glob>,
	/// Each received product's name and content, in the order concurrent bakes emitted them.
	products: Mutex<Vec<(String, Vec<u8>)>>,
}

impl BakeInterceptor for Interception {
	fn wants(&self, name: &str) -> bool {
		self.globs.iter().any(|glob| glob.matches(name))
	}

	fn receive(&self, name: String, content: Vec<u8>) {
		self.products.lock().push((name, content));
	}
}

/// Writes each product under a `// <name>` line, as text when it is UTF-8 and as its size when it isn't.
fn write_products(output: &mut impl std::io::Write, products: &[(String, Vec<u8>)]) -> Result<(), i32> {
	let write = |output: &mut dyn std::io::Write, name: &str, content: &[u8]| match std::str::from_utf8(content) {
		Ok(text) => writeln!(output, "// {name}\n{}\n", text.trim_end()),
		Err(_) => writeln!(output, "// {name}\n// {} of binary content\n", plural(content.len(), "byte")),
	};
	for (name, content) in products {
		write(output, name, content).map_err(|error| {
			log::error!(
				"Failed to write the intercepted products. The most likely cause is a closed output stream. Error: {error}"
			);
			1
		})?;
	}
	Ok(())
}

/// Returns where the product `name` is written under `directory`: each `/` in the name opens a directory, so
/// `mesh.gltf#shaders/<key>.metallib` lands in `mesh.gltf#shaders/`. Fails for a name with a `..` or root component,
/// which would write outside `directory`.
fn product_path(directory: &Path, name: &str) -> Result<PathBuf, String> {
	let relative = Path::new(name);
	if !relative
		.components()
		.all(|component| matches!(component, Component::Normal(_)))
	{
		return Err(format!(
			"The intercepted product '{name}' has no file path under the output directory. The most likely cause is a resource ID with a `..` or root component. Narrow `--intercept` to leave the product out, or drop `--intercept-out` to print it."
		));
	}
	Ok(directory.join(relative))
}

/// Writes every product to its [`product_path`] under `directory` and returns the paths in the products' order.
fn write_product_files(directory: &Path, products: &[(String, Vec<u8>)]) -> Result<Vec<PathBuf>, i32> {
	products
		.iter()
		.map(|(name, content)| {
			let path = product_path(directory, name).map_err(|error| {
				log::error!("{error}");
				1
			})?;
			let write = || -> std::io::Result<()> {
				if let Some(parent) = path.parent() {
					std::fs::create_dir_all(parent)?;
				}
				std::fs::write(&path, content)
			};
			write().map_err(|error| {
				log::error!(
					"Failed to write the intercepted product '{name}' to {}. The most likely cause is that the output directory isn't writable. Pass a writable `--intercept-out` directory. Error: {error}",
					path.display()
				);
				1
			})?;
			Ok(path)
		})
		.collect()
}

/// Prints each written product's path on its own line.
fn write_paths(output: &mut impl std::io::Write, paths: &[PathBuf]) -> Result<(), i32> {
	for path in paths {
		writeln!(output, "{}", path.display()).map_err(|error| {
			log::error!(
				"Failed to write the intercepted product paths. The most likely cause is a closed output stream. Error: {error}"
			);
			1
		})?;
	}
	Ok(())
}

/// One intercepted product in the JSON document, with its content as UTF-8 text when it is text and as base64
/// otherwise.
#[derive(Serialize)]
struct InterceptedDocument<'a> {
	name: &'a str,
	#[serde(flatten)]
	content: Content<'a>,
}

/// The `Content` enum writes a product's bytes in the encoding that keeps them readable.
#[derive(Serialize)]
#[serde(tag = "encoding", content = "content")]
enum Content<'a> {
	#[serde(rename = "utf-8")]
	Text(&'a str),
	#[serde(rename = "base64")]
	Bytes(Base64View<'a>),
}

/// The JSON document of a dry run. It is written straight to the output, so payload streams never become a JSON tree.
#[derive(Serialize)]
struct DryRunDocument<'a> {
	resources: Vec<ResourceDocument<'a>>,
	failed: Vec<Value>,
	/// The intercepted products, ordered by name, when the dry run has `--intercept` patterns.
	#[serde(skip_serializing_if = "Option::is_none")]
	intercepted: Option<Vec<InterceptedDocument<'a>>>,
}

/// Opens the source asset directory a bake reads from.
async fn open_asset_storage(source_path: String) -> Result<FileStorageBackend, i32> {
	let source_path = std::path::PathBuf::from(source_path);
	FileStorageBackend::open(source_path.clone()).await.map_err(|error| {
		log::error!(
			"Failed to open assets directory '{}'. The most likely cause is that the directory cannot be created or accessed. Error: {error}",
			source_path.display()
		);
		1
	})
}

/// Bakes the resources `features` assembles from every baked material, such as the uber material evaluation shaders,
/// and returns the ones that failed with their errors.
///
/// Call it after every other bake. It bakes each resource every time, because the set of materials it gathers is not
/// one of its sources, so a freshness check cannot see it change. The resources gather independently, so they bake
/// together.
async fn bake_assembled(asset_manager: &AssetManager, features: &VisibilityFeatures) -> Vec<(String, LoadMessages)> {
	let resources = features.assembled_resources();
	utils::r#async::stream::iter(resources)
		.map(|id| async move {
			log::info!("Baking resource '{id}' from every baked material");
			let error = asset_manager.bake(id).await.err()?;
			log::error!("Failed to bake '{id}'. Error: {error:#?}");
			Some((id.to_string(), error))
		})
		.buffer_unordered(resources.len().max(1))
		.filter_map(async |failure| failure)
		.collect()
		.await
}

/// Bakes every ID that is stale, reports progress, and returns the IDs that failed with their errors.
///
/// Every ID starts at once, and `memory_budget` decides how many run: a fixed cap left the cores idle while the last
/// containers, which sort late, baked their images and shaders.
///
/// When standard error is a terminal, a progress line replaces the message for each asset that starts or is current.
async fn bake_ids(
	asset_manager: &mut AssetManager,
	ids: Vec<String>,
	memory_budget: NonZeroUsize,
) -> Vec<(String, LoadMessages)> {
	asset_manager.set_bake_memory_budget(memory_budget);
	let asset_manager = &*asset_manager;
	log::info!(
		"Using a {} soft memory budget for concurrent asset bakes.",
		utils::ByteSize::from(memory_budget.get())
	);

	let resource_count = ids.len();
	let progress = terminal::width(&std::io::stderr()).map(|width| {
		RefCell::new(Progress {
			width,
			total: resource_count,
			done: 0,
			failed: 0,
			baking: Vec::with_capacity(16),
			line: String::new(),
		})
	});
	// Every bake borrows the same manager; the futures only run inside `tasks.await` below, while it is still alive.
	let tasks = ids.iter().map(async |id| {
		// Every bake borrows `progress` only between awaits, and they all run on this thread.
		match &progress {
			Some(progress) => progress.borrow_mut().start(id),
			None => log::info!("Baking resource '{}'", id),
		}

		let result = asset_manager.bake_if_stale(id).await;
		if let Some(progress) = &progress {
			progress.borrow_mut().finish(id, result.is_err());
		}

		match result {
			Ok(_) => {
				// A current resource is reused rather than baked, so report the outcome both paths share.
				if progress.is_none() {
					log::info!("Resource '{}' is up to date", id);
				}

				None
			}
			Err(e) => {
				log::error!("Failed to bake '{}'. Error: {:#?}", id, e);

				Some((id.clone(), e))
			}
		}
	});

	let bake_start = Instant::now();

	let failures = utils::r#async::stream::iter(tasks)
		.buffer_unordered(resource_count.max(1))
		.filter_map(async |failure| failure)
		.collect::<Vec<_>>()
		.await;

	if progress.is_some() {
		terminal::show_status("");
	}

	log::info!(
		"Processed {} assets in {:?}: {} succeeded, {} failed",
		resource_count,
		bake_start.elapsed(),
		resource_count - failures.len(),
		failures.len()
	);

	failures
}

/// The `Progress` struct holds what a bake's progress line shows, so each finished asset redraws it without
/// allocating.
struct Progress<'a> {
	width: usize,
	total: usize,
	done: usize,
	failed: usize,
	/// The IDs baking now, in the order they started.
	baking: Vec<&'a str>,
	/// The text of the line, reused for every draw.
	line: String,
}

impl<'a> Progress<'a> {
	/// Adds `id` to the IDs baking now.
	fn start(&mut self, id: &'a str) {
		self.baking.push(id);
		self.draw();
	}

	/// Counts `id` as processed, and as failed when `failed` is set.
	fn finish(&mut self, id: &str, failed: bool) {
		self.baking.retain(|baking| !std::ptr::eq(*baking, id));
		self.done += 1;
		self.failed += usize::from(failed);
		self.draw();
	}

	/// Writes the line, `[done/total]`, the failures, then the IDs baking now, cut to fit the terminal, and shows it.
	fn draw(&mut self) {
		use std::fmt::Write as _;

		self.line.clear();
		let _ = write!(self.line, "[{}/{}]", self.done, self.total);
		if self.failed > 0 {
			let _ = write!(self.line, " {} failed", self.failed);
		}
		for (index, id) in self.baking.iter().enumerate() {
			self.line.push_str(if index == 0 { " · " } else { ", " });
			self.line.push_str(id);
		}
		// The last column stays empty, because a terminal wraps a line that fills it, and a wrapped line can't be erased.
		if let Some((end, _)) = self.line.char_indices().nth(self.width.saturating_sub(1)) {
			self.line.truncate(end);
		}
		terminal::show_status(&self.line);
	}
}
