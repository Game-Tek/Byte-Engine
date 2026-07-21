//! Keep asset-bake messages available to development tools without creating runtime resources.

use std::collections::HashMap;

use utils::sync::Mutex;

use super::ResourceId;

/// The `ResourceTraceLevel` enum identifies the importance of one development-time bake message.
#[derive(
	Clone, Copy, Debug, Eq, PartialEq, serde::Serialize, serde::Deserialize, rkyv::Archive, rkyv::Serialize, rkyv::Deserialize,
)]
pub enum ResourceTraceLevel {
	Info,
	Warn,
	Error,
}

/// The `ResourceTraceItem` struct preserves one message emitted while a resource was baked.
#[derive(
	Clone, Debug, Eq, PartialEq, serde::Serialize, serde::Deserialize, rkyv::Archive, rkyv::Serialize, rkyv::Deserialize,
)]
pub struct ResourceTraceItem {
	level: ResourceTraceLevel,
	message: String,
}

impl ResourceTraceItem {
	/// Creates one item for trace storage and tooling snapshots.
	///
	/// Asset handlers should normally call [`crate::BakeContext::info`],
	/// [`crate::BakeContext::warn`], or [`crate::BakeContext::error`] next so the
	/// item is associated with the current bake automatically.
	pub fn new(level: ResourceTraceLevel, message: String) -> Self {
		Self { level, message }
	}

	/// Returns the importance assigned by the asset handler.
	pub fn level(&self) -> ResourceTraceLevel {
		self.level
	}

	/// Returns the developer-facing bake message.
	pub fn message(&self) -> &str {
		&self.message
	}
}

/// The `ResourceTrace` struct preserves development-time bake messages independently from successfully baked resources.
///
/// Call [`Self::items`] with a requested resource ID after a bake completes. A
/// failed bake can have trace items even though no resource with that ID exists.
#[derive(Default)]
pub struct ResourceTrace {
	items: Mutex<HashMap<String, Vec<ResourceTraceItem>>>,
}

impl ResourceTrace {
	/// Returns an ordered snapshot of the messages from the latest bake of `id`.
	pub fn items(&self, id: &str) -> Vec<ResourceTraceItem> {
		self.items.lock().get(id).cloned().unwrap_or_default()
	}

	/// Returns the resource IDs that currently have at least one trace item.
	pub fn resource_ids(&self) -> Vec<String> {
		let mut ids = self.items.lock().keys().cloned().collect::<Vec<_>>();
		ids.sort_unstable();
		ids
	}

	/// Removes messages from an earlier bake before the handler starts again.
	pub(crate) fn clear(&self, id: ResourceId<'_>) {
		self.items.lock().remove(id.as_ref());
	}

	/// Records one already-formatted item without producing a second terminal log.
	pub(crate) fn record(&self, id: ResourceId<'_>, level: ResourceTraceLevel, message: String) {
		let mut traces = self.items.lock();
		let item = ResourceTraceItem::new(level, message);
		if let Some(items) = traces.get_mut(id.as_ref()) {
			items.push(item);
		} else {
			traces.insert(id.to_string(), vec![item]);
		}
	}

	/// Returns whether the latest bake already left a handler-specific error.
	pub(crate) fn has_error(&self, id: ResourceId<'_>) -> bool {
		self.items
			.lock()
			.get(id.as_ref())
			.is_some_and(|items| items.iter().any(|item| item.level == ResourceTraceLevel::Error))
	}
}
