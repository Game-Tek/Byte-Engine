//! Surface caches retain prepared data across edits elsewhere in the UI.

use super::*;

/// The `SurfaceCache` struct keeps preparation local to each stable surface ID.
/// Consecutive layers share an ID and use their layer offset to remain distinct.
pub(super) struct SurfaceCache<K, V> {
	entries: HashMap<(u32, usize), (K, V, bool)>,
	viewport: Option<(Extent, [f32; 2])>,
	previous: Option<u32>,
	layer: usize,
}

impl<K, V> Default for SurfaceCache<K, V> {
	fn default() -> Self {
		Self {
			entries: HashMap::new(),
			viewport: None,
			previous: None,
			layer: 0,
		}
	}
}

impl<K: PartialEq + Clone, V: Copy> SurfaceCache<K, V> {
	/// Releases removed surfaces while preserving geometry outside the current clip.
	pub(super) fn retain_surfaces(&mut self, ids: &[u32]) {
		self.entries.retain(|(id, _), _| ids.binary_search(id).is_ok());
	}

	/// Starts a preparation pass, invalidating pixel geometry after a viewport resize.
	pub(super) fn begin(&mut self, viewport: Extent, layout: [f32; 2]) {
		if self.viewport != Some((viewport, layout)) {
			self.entries.clear();
			self.viewport = Some((viewport, layout));
		}
		self.previous = None;
		self.layer = 0;
	}

	/// Returns prepared data, refreshing only the surface whose inputs changed.
	pub(super) fn get(&mut self, id: u32, input: &K, prepare: impl FnOnce() -> V) -> V {
		self.layer = if self.previous == Some(id) { self.layer + 1 } else { 0 };
		self.previous = Some(id);
		match self.entries.entry((id, self.layer)) {
			std::collections::hash_map::Entry::Occupied(mut entry) => {
				let (key, output, valid) = entry.get_mut();
				if *key != *input {
					key.clone_from(input);
					*valid = false;
					// Moving surfaces consume fresh geometry directly. Retain their
					// quads once they settle instead of rewriting a cache every frame.
					prepare()
				} else {
					if !*valid {
						*output = prepare();
						*valid = true;
					}
					*output
				}
			}
			std::collections::hash_map::Entry::Vacant(entry) => {
				let output = prepare();
				entry.insert((input.clone(), output, true));
				output
			}
		}
	}
}

/// Image pixels and source indices do not affect a quad's shape or UV coordinates.
pub(super) type ImageGeometryCache =
	SurfaceCache<([f32; 2], [f32; 2], Option<DrawClip>, Option<DrawFeatherMask>, f32), Option<[UiImageVertex; 4]>>;

/// The `CurveGeometryCache` struct retains local tessellation and the last visible quads.
#[derive(Default)]
pub(super) struct CurveGeometryCache {
	/// Previous visible output sizes the next frame without keeping a peak allocation alive.
	pub(super) previous_span_count: usize,
	pub(super) surfaces: HashMap<(u32, usize), CachedCurve>,
}

/// The `CachedCurve` struct separates scale-sensitive points from clipped geometry.
#[derive(Default)]
pub(super) struct CachedCurve {
	pub(super) input: Option<UiCurveDrawElement>,
	pub(super) valid: bool,
	pub(super) viewport: Option<(Extent, [f32; 2])>,
	pub(super) flattened: crate::ui::components::curve::FlattenedCurve,
	pub(super) vertices: Vec<UiCurveVertex>,
	pub(super) indices: Vec<u16>,
}
