use std::{cell::RefCell, num::NonZeroUsize, time::Instant};

use byte_engine::rendering::pipelines::visibility::VisibilityFeatures;
use resource_management::{
	asset::{FileStorageBackend, ResourceId, manager::AssetManager, manager::LoadMessages},
	inspect::{ResourceDocument, inspect_payload, inspect_resource},
	resource::{
		MemoryStorageBackend, ReDBStorageBackend, ReadStorageBackend, ResourceGpuCompressionPolicy, ResourceStorageMode,
		ResourceStorageSettings,
	},
};
use serde::Serialize;
use serde_json::{Value, json};
use utils::{Glob, r#async::StreamExt};

use crate::{
	OutputFormat, Payload,
	commands::shared::{attach_trace, offload_file_operation, payload_format, print_human, read_payload, write_json},
	terminal,
	utils::get_asset_manager,
};

/// Bakes selected source assets, or every discoverable asset when `ids` is empty.
///
/// Assets whose stored resource is current with its source files are skipped unless `force` is set. Force a bake
/// after changing an asset processor, `texture_compression`, or `features`, because source versions don't record those.
/// `features` are the project's visibility features, usually read from its `config.json`; discovery skips the engine
/// assets only left-out features use. `excluded` are the project's `resources.exclude` patterns: discovery skips
/// matching assets, and a bake of one, or of an asset that needs one, fails. `roots` are its `resources.roots`
/// patterns: when there are any, discovery keeps only matching assets, which bake with what they need. Roots don't
/// apply to explicit `ids`. On a terminal, a progress line shows on standard error; install the logger with
/// [`crate::init_logging`] so log messages print above it.
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
	features: VisibilityFeatures,
	excluded: Vec<Glob>,
	roots: Vec<Glob>,
) -> Result<(), i32> {
	let asset_storage_backend = open_asset_storage(source_path).await?;
	let destination_path = std::path::PathBuf::from(destination_path);
	let resource_storage_backend = offload_file_operation(move || match texture_compression {
		Some(compression) => ReDBStorageBackend::new_writable_with_settings(
			destination_path,
			ResourceStorageSettings::new(storage_mode.unwrap_or_default()).image_compression(compression),
		),
		None => match storage_mode {
			Some(mode) => ReDBStorageBackend::new_writable_with_mode(destination_path, mode),
			None => Ok(ReDBStorageBackend::new_writable(destination_path)),
		},
	})
	.await
	.map_err(|error| {
		log::error!("Failed to bake resources. {error}");
		1
	})?;

	let mut asset_manager = get_asset_manager(asset_storage_backend, resource_storage_backend, features);
	asset_manager.set_excluded_assets(excluded);

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

	let failures = bake_ids(&mut asset_manager, ids, memory_budget).await;

	if failures.is_empty() { Ok(()) } else { Err(1) }
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
			"The bake stopped before baking anything, because these assets match both `resources.roots` and `resources.exclude`: {}. The most likely cause is that a root pattern and an exclude pattern overlap. Remove each of these assets from one of the two lists in the configuration. See {}.",
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
/// failed. `excluded` are the project's `resources.exclude` patterns, as in [`bake`]. Pass `payload` to add the decoded
/// stream bytes, which needs [`OutputFormat::JSON`]. Without `ids` nothing
/// bakes, because a dry run doesn't discover assets. Run [`bake`] next to write the same resources to a store.
#[allow(clippy::too_many_arguments)]
pub async fn bake_dry_run(
	source_path: String,
	ids: Vec<String>,
	memory_budget: NonZeroUsize,
	features: VisibilityFeatures,
	excluded: Vec<Glob>,
	format: OutputFormat,
	payload: Payload,
	output: &mut impl std::io::Write,
) -> Result<(), i32> {
	let payload = payload_format(payload, format)?;

	let asset_storage_backend = open_asset_storage(source_path).await?;
	// The manager owns one clone; this one reads back what it baked once it is done.
	let resource_storage_backend = MemoryStorageBackend::new();
	let mut asset_manager = get_asset_manager(asset_storage_backend, resource_storage_backend.clone(), features);
	asset_manager.set_excluded_assets(excluded);
	let failures = bake_ids(&mut asset_manager, ids, memory_budget).await;
	drop(asset_manager);

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
			write_json(output, &DryRunDocument { resources, failed })?;
		}
	}

	if failures.is_empty() { Ok(()) } else { Err(1) }
}

/// The JSON document of a dry run. It is written straight to the output, so payload streams never become a JSON tree.
#[derive(Serialize)]
struct DryRunDocument<'a> {
	resources: Vec<ResourceDocument<'a>>,
	failed: Vec<Value>,
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

/// Bakes every ID that is stale, up to 16 at once within `memory_budget`, reports progress, and returns the IDs that
/// failed with their errors.
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
		.buffer_unordered(16)
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
