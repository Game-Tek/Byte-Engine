//! Debug views: visualizations a render pass or scene pipeline can show on screen through a renderer-owned pool of
//! debug targets.
//!
//! A pass declares its views once, while it is built, with [`RenderPassBuilder::debug_views`], and writes the shown
//! ones from its shader behind `debug_view_slot(…, view)` checks. The renderer lends one pool slot to each view that
//! `render.debug.view.<pass>.<view>=true` shows, refuses a view when the pool is full, and tiles the shown slots into
//! each sink's output after everything else. Projects leave the feature out of release bakes through
//! `render.debug-views.enabled`; see [`VisibilityFeatures`].
//!
//! See the debug views design note in the Contribute documentation for the whole mechanism.
//!
//! [`RenderPassBuilder::debug_views`]: crate::rendering::render_pass::RenderPassBuilder::debug_views

use std::{
	collections::VecDeque,
	sync::{
		Arc, Mutex,
		atomic::{AtomicU32, Ordering},
	},
};

use serde::Serialize;
use smallvec::SmallVec;

use crate::configuration::{ConfigurationEventId, ConfigurationPort, ConfigurationUpdate, ConfigurationValue};
use crate::rendering::pipelines::visibility::VisibilityFeatures;

/// The most debug targets a sink's pool holds: a slot number takes four bits in [`DebugViews::shown`], and `0` means
/// hidden.
pub const MAX_DEBUG_TARGETS: u32 = 15;
/// The most views one pass declares, since each takes four bits of one `u32`.
pub const MAX_VIEWS_PER_PASS: usize = 8;
/// How many debug targets a sink's pool holds unless `render.debug.targets` says otherwise.
pub const DEFAULT_DEBUG_TARGETS: u32 = 4;
/// The name every view-writing shader declares the pool under, as an arrayed storage image.
pub const DEBUG_VIEW_TARGETS_BINDING: &str = "debug_view_targets";

/// The `DebugViewSettings` struct carries the startup choices the renderer makes about debug views to every sink
/// builder: whether the project built them in, and how large each sink's pool is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct DebugViewSettings {
	/// Whether the build has debug views; see [`VisibilityFeatures::debug_views`].
	pub(crate) enabled: bool,
	/// How many targets each sink's pool holds, from `1` to [`MAX_DEBUG_TARGETS`].
	pub(crate) pool_size: u32,
}

impl DebugViewSettings {
	/// Reads the settings from the `render` subtree of the startup parameters: `debug-views.enabled` through
	/// [`VisibilityFeatures`] and `debug.targets`, a whole number from `1` to `15`, for the pool size.
	///
	/// # Panics
	///
	/// Panics when `debug.targets` is not a whole number in range.
	pub(crate) fn from_parameters(parameters: impl crate::application::parameters::Parameters + Copy) -> Self {
		let enabled = VisibilityFeatures::from_parameters(parameters).debug_views;
		let pool_size = parameters.get_parameter("debug.targets").map_or(DEFAULT_DEBUG_TARGETS, |parameter| {
			parameter
				.parse::<u32>()
				.ok()
				.filter(|count| (1..=MAX_DEBUG_TARGETS).contains(count))
				.unwrap_or_else(|| {
					panic!(
						"Parameter `render.debug.targets` is invalid. The most likely cause is that `{}` is not a whole number from 1 to {MAX_DEBUG_TARGETS}.",
						parameter.value()
					)
				})
		});
		Self { enabled, pool_size }
	}
}

/// The `DebugViews` struct hands a pass the debug target pool and tells it, each frame, which of its views to write
/// and where.
///
/// Get one from [`RenderPassBuilder::debug_views`](crate::rendering::render_pass::RenderPassBuilder::debug_views).
/// Bind [`Self::targets`] where the shader declares `debug_view_targets`, and write [`Self::shown`] to the shader
/// member named `debug_views` every frame. The shader then asks `debug_view_slot(debug_views, view)` which slot a
/// view writes, where `view` is the index of its name in the list the pass declared.
#[derive(Clone)]
pub struct DebugViews {
	targets: SmallVec<[ghi::BaseImageHandle; MAX_DEBUG_TARGETS as usize]>,
	shown: Arc<AtomicU32>,
}

impl DebugViews {
	/// Creates the handle a pass keeps, over `targets`, the sink's pool in slot order, and the cell the registry
	/// packs the pass's shown views into.
	pub(crate) fn new(targets: &[ghi::BaseImageHandle], shown: Arc<AtomicU32>) -> Self {
		Self {
			targets: targets.iter().copied().collect(),
			shown,
		}
	}

	/// The handle of a build without debug views: no targets and no view ever shown.
	pub(crate) fn disabled() -> Self {
		Self::new(&[], Arc::new(AtomicU32::new(0)))
	}

	/// The sink's pool, in slot order, to bind as the `debug_view_targets` array. Empty when the build has no debug
	/// views.
	pub fn targets(&self) -> &[ghi::BaseImageHandle] {
		&self.targets
	}

	/// Which of this pass's views are shown this frame and in which slot, packed as the shader reads it: four bits
	/// per view in declaration order, holding the slot number plus one, or `0` while the view is hidden.
	pub fn shown(&self) -> u32 {
		self.shown.load(Ordering::Relaxed)
	}
}

/// The `DebugViewCatalog` struct publishes what the registry knows to threads that cannot reach the renderer, such
/// as the inspector's `GET /debug-views`.
#[derive(Default)]
pub struct DebugViewCatalog {
	state: Mutex<DebugViewCatalogState>,
}

/// The `DebugViewCatalogState` struct is one snapshot of the registry for the inspector.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize)]
pub struct DebugViewCatalogState {
	/// Whether this build can show debug views.
	pub enabled: bool,
	/// How many debug targets each sink's pool holds.
	pub targets: u32,
	/// Every declared view, in registration order.
	pub views: Vec<DebugViewEntry>,
}

/// The `DebugViewEntry` struct describes one declared view and the pool slot it holds while shown.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct DebugViewEntry {
	/// The view as `render.debug.view.<pass>.<view>` names it, without the prefix.
	pub name: String,
	/// The pool slot the view holds, or `None` while it is hidden.
	pub slot: Option<u32>,
}

impl DebugViewCatalog {
	/// Returns the latest snapshot.
	pub fn state(&self) -> DebugViewCatalogState {
		self.state.lock().unwrap_or_else(|poisoned| poisoned.into_inner()).clone()
	}

	fn publish(&self, state: DebugViewCatalogState) {
		*self.state.lock().unwrap_or_else(|poisoned| poisoned.into_inner()) = state;
	}
}

/// One pass that declared views, with the cell of every sink's instance.
struct RegisteredPass {
	name: String,
	views: &'static [&'static str],
	cells: Vec<Arc<AtomicU32>>,
}

/// The `DebugViewRegistry` struct owns which views exist, which are shown, and which pool slot each shown view holds.
///
/// The renderer registers each pass's views as it builds the pass, applies `render.debug.view.*` updates through
/// [`apply_debug_view_configuration`], and reads [`Self::taken`] to size the pool and tile the output.
pub(crate) struct DebugViewRegistry {
	settings: DebugViewSettings,
	passes: Vec<RegisteredPass>,
	/// The pass and view holding each pool slot.
	slots: SmallVec<[Option<(usize, usize)>; MAX_DEBUG_TARGETS as usize]>,
	catalog: Arc<DebugViewCatalog>,
}

impl DebugViewRegistry {
	/// Creates the registry of a renderer that `settings` describe, publishing to `catalog`.
	pub(crate) fn new(settings: DebugViewSettings, catalog: Arc<DebugViewCatalog>) -> Self {
		let registry = Self {
			settings,
			passes: Vec::new(),
			slots: (0..settings.pool_size).map(|_| None).collect(),
			catalog,
		};
		registry.publish();
		registry
	}

	/// Registers the views one instance of the pass named `name` declared, with the cell it reads its shown views
	/// from. Every instance of a pass declares the same list, so a second instance only adds its cell.
	///
	/// # Panics
	///
	/// Panics when an instance declares a different list than the first instance of its name did.
	pub(crate) fn register(&mut self, name: &str, views: &'static [&'static str], cell: Arc<AtomicU32>) {
		match self.passes.iter_mut().find(|pass| pass.name == name) {
			Some(pass) => {
				assert!(
					pass.views == views,
					"Render pass `{name}` declared the debug views {views:?} after another instance declared {:?}. The most likely cause is a pass whose view list depends on the sink.",
					pass.views
				);
				cell.store(
					pass.cells.first().map_or(0, |first| first.load(Ordering::Relaxed)),
					Ordering::Relaxed,
				);
				pass.cells.push(cell);
			}
			None => self.passes.push(RegisteredPass {
				name: name.to_string(),
				views,
				cells: vec![cell],
			}),
		}
		self.publish();
	}

	/// Shows `view` of `pass` in the first free pool slot, or hides it and frees its slot. Hiding a hidden view or
	/// showing a shown one changes nothing.
	///
	/// Returns the reason when the pass has no such view, when every slot is taken, or when the build has no debug
	/// views. Returns `Ok(false)` when no pass named `pass` has registered yet, so the caller can keep the request.
	pub(crate) fn set_view(&mut self, pass: &str, view: &str, shown: bool) -> Result<bool, String> {
		if !self.settings.enabled {
			return Err(
				"Debug view was not set. The most likely cause is that this build has no debug views; set `render.debug-views.enabled` to `true` and bake again."
					.to_string(),
			);
		}
		let Some(pass_index) = self.passes.iter().position(|registered| registered.name == pass) else {
			return Ok(false);
		};
		let registered = &self.passes[pass_index];
		let Some(view_index) = registered.views.iter().position(|name| *name == view) else {
			return Err(format!(
				"Debug view was not set. The most likely cause is that render pass `{pass}` declares no view named `{view}`; it declares {:?}.",
				registered.views
			));
		};
		let held = self.slots.iter().position(|slot| *slot == Some((pass_index, view_index)));
		match (shown, held) {
			(true, Some(_)) | (false, None) => {}
			(true, None) => {
				let Some(free) = self.slots.iter().position(Option::is_none) else {
					return Err(format!(
						"Debug view was not set. The most likely cause is that every one of the {} debug targets is taken; hide a view or raise `render.debug.targets`.",
						self.slots.len()
					));
				};
				self.slots[free] = Some((pass_index, view_index));
			}
			(false, Some(slot)) => self.slots[slot] = None,
		}
		self.pack(pass_index);
		self.publish();
		Ok(true)
	}

	/// Writes the packed slots of every view of one pass into each of its instances' cells.
	fn pack(&self, pass_index: usize) {
		let mut packed = 0u32;
		for (slot, held) in self.slots.iter().enumerate() {
			if let Some((pass, view)) = held
				&& *pass == pass_index
			{
				packed |= (slot as u32 + 1) << (4 * *view as u32);
			}
		}
		for cell in &self.passes[pass_index].cells {
			cell.store(packed, Ordering::Relaxed);
		}
	}

	/// One bit per pool slot, set while a view holds it.
	pub(crate) fn taken(&self) -> u32 {
		self.slots
			.iter()
			.enumerate()
			.filter(|(_, slot)| slot.is_some())
			.fold(0, |mask, (index, _)| mask | (1 << index))
	}

	fn publish(&self) {
		let views = self
			.passes
			.iter()
			.enumerate()
			.flat_map(|(pass_index, pass)| {
				pass.views.iter().enumerate().map(move |(view_index, view)| DebugViewEntry {
					name: format!("{}.{view}", pass.name),
					slot: self
						.slots
						.iter()
						.position(|slot| *slot == Some((pass_index, view_index)))
						.map(|slot| slot as u32),
				})
			})
			.collect();
		self.catalog.publish(DebugViewCatalogState {
			enabled: self.settings.enabled,
			targets: self.settings.pool_size,
			views,
		});
	}
}

/// The `PendingDebugView` struct keeps one validated `render.debug.view.<pass>.<view>` update until its pass exists.
pub(crate) struct PendingDebugView {
	event: ConfigurationEventId,
	pass: String,
	view: String,
	shown: bool,
}

impl PendingDebugView {
	/// Validates the parameter and value of one update before it is retained. The parameter is `<pass>.<view>`, split
	/// at its last dot since view names carry none, and the value is `true` or `false`.
	fn from_update(update: ConfigurationUpdate) -> Result<Self, (ConfigurationEventId, String)> {
		let event = update.id();
		let Some((pass, view)) = update
			.parameter()
			.rsplit_once('.')
			.filter(|(pass, view)| !pass.is_empty() && !view.is_empty())
		else {
			return Err((
				event,
				"Debug view was not set. The most likely cause is that the parameter is not `<pass>.<view>`.".to_string(),
			));
		};
		let Some(shown) = update.value().to_bool() else {
			return Err((
				event,
				"Debug view was not set. The most likely cause is that the value is neither `true` nor `false`.".to_string(),
			));
		};
		Ok(Self {
			event,
			pass: pass.to_string(),
			view: view.to_string(),
			shown,
		})
	}
}

/// Applies the queued `render.debug.view.*` updates to `registry`, keeping the ones whose pass has not registered
/// yet. Returns whether any view changed.
pub(crate) fn apply_debug_view_configuration(
	configuration: &ConfigurationPort,
	pending: &mut VecDeque<PendingDebugView>,
	registry: &mut DebugViewRegistry,
) -> bool {
	while let Some(update) = configuration.read() {
		match PendingDebugView::from_update(update) {
			Ok(update) => pending.push_back(update),
			Err((id, reason)) => configuration.not_set(id, reason),
		}
	}

	let mut changed = false;
	// Try each retained update once per call. A pass that has not registered yet keeps the event pending.
	let pending_count = pending.len();
	for _ in 0..pending_count {
		let update = pending.pop_front().expect("pending configuration count changed");
		match registry.set_view(&update.pass, &update.view, update.shown) {
			Ok(true) => {
				changed = true;
				configuration.set(update.event, ConfigurationValue::Bool(update.shown));
			}
			Ok(false) => pending.push_back(update),
			Err(reason) => configuration.not_set(update.event, reason),
		}
	}
	changed
}

#[cfg(test)]
mod tests {
	use super::*;
	use crate::configuration::{Configuration, ConfigurationUpdateState};

	fn registry(pool_size: u32) -> DebugViewRegistry {
		DebugViewRegistry::new(
			DebugViewSettings {
				enabled: true,
				pool_size,
			},
			Arc::new(DebugViewCatalog::default()),
		)
	}

	fn cell() -> Arc<AtomicU32> {
		Arc::new(AtomicU32::new(0))
	}

	fn value(cell: &Arc<AtomicU32>) -> u32 {
		cell.load(Ordering::Relaxed)
	}

	#[test]
	fn showing_a_view_takes_the_first_free_slot_and_packs_it_into_every_instance() {
		let mut registry = registry(2);
		let (first_sink, second_sink, other) = (cell(), cell(), cell());
		registry.register("blur", &["velocity", "taps"], first_sink.clone());
		registry.register("blur", &["velocity", "taps"], second_sink.clone());
		registry.register("scene", &["normals"], other.clone());

		assert_eq!(registry.set_view("blur", "taps", true), Ok(true));
		assert_eq!(registry.set_view("scene", "normals", true), Ok(true));

		// Taps holds slot 0 as its second view (bits 4..8); normals holds slot 1.
		assert_eq!(value(&first_sink), 1 << 4);
		assert_eq!(value(&second_sink), 1 << 4);
		assert_eq!(value(&other), 2);
		assert_eq!(registry.taken(), 0b11);
		assert_eq!(DebugViews::new(&[], first_sink.clone()).shown(), 1 << 4);

		// Two views of one pass pack side by side once a slot frees up.
		assert_eq!(registry.set_view("scene", "normals", false), Ok(true));
		assert_eq!(registry.set_view("blur", "velocity", true), Ok(true));
		assert_eq!(value(&first_sink), (1 << 4) | 2);
		assert_eq!(value(&other), 0);
	}

	#[test]
	fn a_full_pool_refuses_and_keeps_the_shown_views() {
		let mut registry = registry(1);
		let shown = cell();
		registry.register("blur", &["velocity", "taps"], shown.clone());
		assert_eq!(registry.set_view("blur", "velocity", true), Ok(true));

		let refused = registry.set_view("blur", "taps", true).unwrap_err();

		assert!(refused.contains("taken"), "{refused}");
		assert_eq!(value(&shown), 1);
		assert_eq!(registry.taken(), 1);
		// Showing it again is not a second slot.
		assert_eq!(registry.set_view("blur", "velocity", true), Ok(true));
		assert_eq!(registry.taken(), 1);
	}

	#[test]
	fn unknown_views_and_passes_are_told_apart() {
		let mut registry = registry(2);
		registry.register("blur", &["velocity"], cell());

		assert_eq!(registry.set_view("bloom", "velocity", true), Ok(false));
		let refused = registry.set_view("blur", "taps", true).unwrap_err();
		assert!(refused.contains("velocity"), "{refused}");
	}

	#[test]
	fn a_build_without_debug_views_refuses_every_view() {
		let mut registry = DebugViewRegistry::new(
			DebugViewSettings {
				enabled: false,
				pool_size: 0,
			},
			Arc::new(DebugViewCatalog::default()),
		);
		registry.register("blur", &["velocity"], cell());

		assert!(registry.set_view("blur", "velocity", true).is_err());
		assert!(!registry.catalog.state().enabled);
	}

	#[test]
	fn the_catalog_lists_every_view_with_its_slot() {
		let catalog = Arc::new(DebugViewCatalog::default());
		let mut registry = DebugViewRegistry::new(
			DebugViewSettings {
				enabled: true,
				pool_size: 3,
			},
			catalog.clone(),
		);
		registry.register("blur", &["velocity", "taps"], cell());
		registry.set_view("blur", "taps", true).unwrap();

		assert_eq!(
			catalog.state(),
			DebugViewCatalogState {
				enabled: true,
				targets: 3,
				views: vec![
					DebugViewEntry {
						name: "blur.velocity".into(),
						slot: None
					},
					DebugViewEntry {
						name: "blur.taps".into(),
						slot: Some(0)
					},
				],
			}
		);
	}

	#[test]
	fn configuration_shows_views_and_reports_refusals() {
		let configuration = Configuration::new();
		let port = configuration.register("render.debug.view.");
		let shown = configuration.update("render.debug.view.blur.velocity", "true");
		let unknown = configuration.update("render.debug.view.blur.radius", "true");
		let malformed = configuration.update("render.debug.view.blur", "true");
		let later = configuration.update("render.debug.view.bloom.levels", ConfigurationValue::Bool(true));
		let mut pending = VecDeque::new();
		let mut registry = registry(2);
		let cell = cell();
		registry.register("blur", &["velocity"], cell.clone());

		assert!(apply_debug_view_configuration(&port, &mut pending, &mut registry));

		assert_eq!(value(&cell), 1);
		assert_eq!(
			configuration.event(shown).unwrap().state(),
			&ConfigurationUpdateState::Set {
				value: ConfigurationValue::Bool(true)
			}
		);
		assert!(matches!(
			configuration.event(unknown).unwrap().state(),
			ConfigurationUpdateState::NotSet { .. }
		));
		assert!(matches!(
			configuration.event(malformed).unwrap().state(),
			ConfigurationUpdateState::NotSet { .. }
		));
		assert_eq!(
			configuration.event(later).unwrap().state(),
			&ConfigurationUpdateState::Pending
		);
		assert_eq!(pending.len(), 1);

		registry.register("bloom", &["levels"], Arc::new(AtomicU32::new(0)));
		assert!(apply_debug_view_configuration(&port, &mut pending, &mut registry));
		assert!(pending.is_empty());
		assert_eq!(registry.taken(), 0b11);
	}
}
