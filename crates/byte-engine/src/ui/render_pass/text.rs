//! UI glyph atlas packing and per-glyph text geometry generation.
//!
//! Text is drawn as fractionally positioned quads that sample one shared coverage atlas.
//! Glyph bitmaps enter the atlas once per character and whole-pixel raster size and stay
//! resident, so an unchanged label costs no CPU rasterization and no upload.
//! A whole device size draws texel for texel on whole pixels; a fractional one leaves
//! the GPU a small linear-filtered scale.

use std::collections::HashMap;

use super::*;
use crate::ui::font::{Glyph, GlyphKey};

pub(super) const UI_GLYPH_ATLAS_FORMAT: ghi::Formats = ghi::Formats::R8UNORM;
pub(super) const UI_GLYPH_ATLAS_BINDING: ghi::ShaderResourceDescriptor = ghi::ShaderResourceDescriptor::single(
	ghi::ResourceSlot::new(0),
	ghi::ResourceKind::CombinedImageSampler,
	ghi::AccessPolicies::READ,
);
pub(super) const UI_GLYPH_ATLAS_INITIAL_SIZE: u32 = 512;
pub(super) const UI_GLYPH_ATLAS_MAX_SIZE: u32 = 4096;
/// Transparent texels around every glyph so linear filtering at a quad edge blends with transparent coverage.
const UI_GLYPH_ATLAS_PADDING: u32 = 1;

pub(super) const UI_TEXT_VERTEX_LAYOUT: [ghi::pipelines::VertexElement; 9] = [
	ghi::pipelines::VertexElement::new("POSITION", ghi::DataTypes::Float2, 0),
	ghi::pipelines::VertexElement::new("UV", ghi::DataTypes::Float2, 0),
	ghi::pipelines::VertexElement::new("COLOR", ghi::DataTypes::Float4, 0),
	ghi::pipelines::VertexElement::new("PIXEL_POSITION", ghi::DataTypes::Float2, 0),
	ghi::pipelines::VertexElement::new("CLIP_MASK_POSITION", ghi::DataTypes::Float2, 0),
	ghi::pipelines::VertexElement::new("CLIP_MASK_SIZE", ghi::DataTypes::Float2, 0),
	ghi::pipelines::VertexElement::new("CLIP_MASK_EDGES", ghi::DataTypes::Float4, 0),
	ghi::pipelines::VertexElement::new("CLIP_MASK_CORNER", ghi::DataTypes::Float2, 0),
	ghi::pipelines::VertexElement::new("TRANSFORM", ghi::DataTypes::Float4, 0),
];

#[repr(C)]
#[derive(Debug, Clone, Copy, Default, bytemuck::Pod, bytemuck::Zeroable)]
pub(super) struct UiTextVertex {
	pub(super) position: [f32; 2],
	pub(super) uv: [f32; 2],
	pub(super) color: [f32; 4],
	pub(super) pixel_position: [f32; 2],
	pub(super) clip_mask_position: [f32; 2],
	pub(super) clip_mask_size: [f32; 2],
	pub(super) clip_mask_edges: [f32; 4],
	pub(super) clip_mask_corner: [f32; 2],
	pub(super) transform: [f32; 4],
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct UiTextDrawBatch {
	pub(super) depth: u32,
	pub(super) order: u32,
	pub(super) index_count: u32,
	pub(super) first_index: u32,
	pub(super) vertex_offset: i32,
}

#[derive(Debug)]
pub(super) struct UiTextGeometry<'a> {
	pub(super) vertices: Vec<UiTextVertex, &'a bumpalo::Bump>,
	pub(super) indices: Vec<u16, &'a bumpalo::Bump>,
	pub(super) batches: Vec<UiTextDrawBatch, &'a bumpalo::Bump>,
	pub(super) truncated: bool,
	/// Glyphs that could not be placed in the atlas even after a reset; they are not drawn.
	pub(super) dropped_glyphs: usize,
}

/// The `AtlasRegion` struct locates one glyph bitmap inside the atlas in texels.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct AtlasRegion {
	pub(super) x: u32,
	pub(super) y: u32,
	pub(super) width: u32,
	pub(super) height: u32,
}

#[derive(Debug, Clone, Copy)]
struct Shelf {
	y: u32,
	height: u32,
	next_x: u32,
}

/// The `UiGlyphAtlas` struct packs rasterized glyph coverage into one square texture.
///
/// Packing uses shelves so insertion is cheap and deterministic. When the atlas
/// fills up it doubles in size and repacks; at the maximum size it resets and
/// keeps only the glyphs needed by the current frame. Both cases advance
/// [`Self::generation`], which invalidates any geometry that referenced old
/// regions. CPU pixels are the source of truth; [`Self::upload`] mirrors them to
/// the GPU only after a change.
pub(super) struct UiGlyphAtlas {
	runs: Vec<CachedText>,
	run_indices: HashMap<u32, usize>,
	prepared: bool,
	size: u32,
	pixels: Vec<u8>,
	shelves: Vec<Shelf>,
	next_shelf_y: u32,
	regions: HashMap<GlyphKey, AtlasRegion>,
	generation: u64,
	dirty: bool,
	full_upload: bool,
	dirty_regions: Vec<ghi::image::Region>,
	resized: bool,
}

impl UiGlyphAtlas {
	pub(super) fn new(size: u32) -> Self {
		let size = size.clamp(1, UI_GLYPH_ATLAS_MAX_SIZE);
		Self {
			runs: Vec::new(),
			run_indices: HashMap::new(),
			prepared: false,
			size,
			pixels: vec![0; (size * size) as usize],
			shelves: Vec::new(),
			next_shelf_y: 0,
			regions: HashMap::new(),
			generation: 0,
			dirty: false,
			full_upload: true,
			dirty_regions: Vec::new(),
			resized: false,
		}
	}

	#[cfg(test)]
	pub(super) fn clear_prepared_runs(&mut self) {
		self.runs.clear();
		self.run_indices.clear();
	}

	/// Releases removed labels while keeping culled labels available when they return.
	pub(super) fn retain_surfaces(&mut self, ids: &[u32]) {
		self.runs.retain(|run| ids.binary_search(&run.input.order).is_ok());
		self.run_indices.clear();
		self.run_indices
			.extend(self.runs.iter().enumerate().map(|(index, run)| (run.input.order, index)));
	}

	pub(super) fn size(&self) -> u32 {
		self.size
	}

	pub(super) fn extent(&self) -> Extent {
		Extent::square(self.size)
	}

	/// Identifies the current packing; geometry built against an older generation is stale.
	pub(super) fn generation(&self) -> u64 {
		self.generation
	}

	pub(super) fn len(&self) -> usize {
		self.regions.len()
	}

	pub(super) fn region(&self, key: GlyphKey) -> Option<AtlasRegion> {
		self.regions.get(&key).copied()
	}

	#[cfg(test)]
	pub(super) fn pixels(&self) -> &[u8] {
		&self.pixels
	}

	/// Ensures `key` is resident, growing or resetting the atlas when it does not fit.
	///
	/// Returns `None` only when the glyph cannot fit in an empty atlas at the
	/// maximum size. Glyphs without visible coverage never occupy space.
	pub(super) fn ensure(&mut self, key: GlyphKey, text_system: &mut TextSystem) -> Option<AtlasRegion> {
		if let Some(region) = self.regions.get(&key) {
			return Some(*region);
		}
		if !text_system.glyph_by_key(key)?.is_visible() {
			return None;
		}

		loop {
			if let Some(region) = self.insert(key, text_system.glyph_by_key(key)?) {
				return Some(region);
			}
			if self.size < UI_GLYPH_ATLAS_MAX_SIZE {
				self.repack(self.size.saturating_mul(2).min(UI_GLYPH_ATLAS_MAX_SIZE), text_system);
			} else {
				// The maximum atlas is full; keep only what this frame asks for from now on.
				self.reset(self.size);
				return self.insert(key, text_system.glyph_by_key(key)?);
			}
		}
	}

	fn insert(&mut self, key: GlyphKey, glyph: &Glyph) -> Option<AtlasRegion> {
		let padded_width = glyph.width.saturating_add(UI_GLYPH_ATLAS_PADDING * 2);
		let padded_height = glyph.height.saturating_add(UI_GLYPH_ATLAS_PADDING * 2);
		let (x, y) = self.pack(padded_width, padded_height)?;
		let region = AtlasRegion {
			x: x + UI_GLYPH_ATLAS_PADDING,
			y: y + UI_GLYPH_ATLAS_PADDING,
			width: glyph.width,
			height: glyph.height,
		};
		for row in 0..glyph.height {
			let source = (row * glyph.width) as usize;
			let destination = ((region.y + row) * self.size + region.x) as usize;
			self.pixels[destination..destination + glyph.width as usize]
				.copy_from_slice(&glyph.bitmap[source..source + glyph.width as usize]);
		}
		self.regions.insert(key, region);
		self.dirty = true;
		if !self.full_upload {
			// Shelf neighbors form one upload; padding supplies transparent samples
			// for bilinear filtering without uploading the rest of the atlas.
			if let Some(dirty) = self
				.dirty_regions
				.iter_mut()
				.find(|dirty| dirty.offset[1] == y && dirty.offset[0] + dirty.size[0] == x)
			{
				dirty.size[0] += padded_width;
				dirty.size[1] = dirty.size[1].max(padded_height);
			} else {
				self.dirty_regions.push(ghi::image::Region {
					offset: [x, y],
					size: [padded_width, padded_height],
				});
			}
		}
		Some(region)
	}

	fn pack(&mut self, width: u32, height: u32) -> Option<(u32, u32)> {
		if width > self.size || height > self.size {
			return None;
		}
		// Prefer the tightest shelf so tall glyphs do not stretch short rows.
		let mut best: Option<(usize, u32)> = None;
		for (index, shelf) in self.shelves.iter().enumerate() {
			if shelf.height >= height && shelf.next_x + width <= self.size {
				let waste = shelf.height - height;
				if best.is_none_or(|(_, best_waste)| waste < best_waste) {
					best = Some((index, waste));
				}
			}
		}
		if let Some((index, _)) = best {
			let shelf = &mut self.shelves[index];
			let x = shelf.next_x;
			shelf.next_x += width;
			return Some((x, shelf.y));
		}
		if self.next_shelf_y + height > self.size {
			return None;
		}
		let y = self.next_shelf_y;
		self.shelves.push(Shelf {
			y,
			height,
			next_x: width,
		});
		self.next_shelf_y += height;
		Some((0, y))
	}

	fn reset(&mut self, size: u32) {
		self.size = size;
		self.pixels.clear();
		self.pixels.resize((size * size) as usize, 0);
		self.shelves.clear();
		self.next_shelf_y = 0;
		self.regions.clear();
		self.generation += 1;
		self.dirty = true;
		self.full_upload = true;
		self.dirty_regions.clear();
		self.resized = true;
	}

	fn repack(&mut self, size: u32, text_system: &mut TextSystem) {
		let keys: Vec<GlyphKey> = self.regions.keys().copied().collect();
		self.reset(size);
		for key in keys {
			if let Some(glyph) = text_system.glyph_by_key(key) {
				self.insert(key, glyph);
			}
		}
	}

	/// Mirrors CPU changes to the GPU image; a no-op when nothing changed since the last upload.
	pub(super) fn upload(&mut self, frame: &mut ghi::implementation::Frame, image: ghi::BaseImageHandle) {
		if self.resized {
			frame.resize_image(image, self.extent());
			self.resized = false;
		}
		if !self.dirty {
			return;
		}
		let staging = frame.get_texture_slice_mut(image);
		self.copy_dirty_pixels(staging);
		if self.full_upload {
			frame.sync_texture(image);
		} else {
			for &region in &self.dirty_regions {
				frame.sync_texture_region(image, region);
			}
		}
		self.full_upload = false;
		self.dirty_regions.clear();
		self.dirty = false;
	}

	/// Copies the pending rows into full-image staging storage without touching other texels.
	fn copy_dirty_pixels(&self, staging: &mut [u8]) {
		if self.full_upload {
			staging[..self.pixels.len()].copy_from_slice(&self.pixels);
			return;
		}
		for region in &self.dirty_regions {
			for y in region.offset[1]..region.offset[1] + region.size[1] {
				let start = (y * self.size + region.offset[0]) as usize;
				let end = start + region.size[0] as usize;
				staging[start..end].copy_from_slice(&self.pixels[start..end]);
			}
		}
	}

	#[cfg(test)]
	pub(super) fn is_dirty(&self) -> bool {
		self.dirty
	}
}

/// The `CachedText` struct retains glyph positions relative to a label's origin.
/// Visual edits invalidate quads; content and raster-size bucket changes also invalidate placement.
struct CachedText {
	/// The current draw-list slot protects callers that submit repeated surface IDs.
	source_index: usize,
	input: UiTextDrawElement,
	font_size: f32,
	viewport: (Extent, [f32; 2]),
	generation: u64,
	glyphs: Vec<PendingGlyph>,
	vertices: Vec<UiTextVertex>,
	vertices_valid: bool,
	placement_valid: bool,
	vertices_stable: bool,
}

#[derive(Debug, Clone, Copy)]
struct PendingGlyph {
	key: GlyphKey,
	x: f32,
	y: f32,
	region: Option<AtlasRegion>,
}

/// The `PreparedText` struct carries one label's frame data between atlas placement and quad emission.
struct PreparedText {
	clip: PixelClip,
	glyphs: std::ops::Range<usize>,
	cache_index: Option<usize>,
}

/// Appends local glyph positions to either retained storage or the current frame's arena.
fn place_text_glyphs<A: std::alloc::Allocator>(
	text_system: &mut TextSystem,
	text: &str,
	font_size: f32,
	glyphs: &mut Vec<PendingGlyph, A>,
) {
	text_system.place_glyphs(text, font_size, (0, 0), |placement| {
		glyphs.push(PendingGlyph {
			key: placement.key,
			x: placement.x,
			y: placement.y,
			region: None,
		});
	});
}

#[derive(Debug, Clone, Copy)]
struct PixelClip {
	x0: f32,
	y0: f32,
	x1: f32,
	y1: f32,
}

impl PixelClip {
	fn intersect(self, other: Self) -> Self {
		Self {
			x0: self.x0.max(other.x0),
			y0: self.y0.max(other.y0),
			x1: self.x1.min(other.x1),
			y1: self.y1.min(other.y1),
		}
	}

	fn is_empty(self) -> bool {
		self.x1 <= self.x0 || self.y1 <= self.y0
	}
}

// Converts a layout-space clip to viewport pixels on the same whole-pixel edges as the rectangle that owns it.
fn pixel_clip(clip: Option<DrawClip>, sx: f32, sy: f32, viewport: PixelClip) -> PixelClip {
	let Some(clip) = clip else {
		return viewport;
	};
	let [x0, y0, x1, y1] = snapped_rect(clip.position, clip.size, sx, sy);
	PixelClip { x0, y0, x1, y1 }.intersect(viewport)
}

/// Builds glyph quads with fractional transforms for every visible text run, batched by depth.
///
/// Glyph placement and atlas residency are resolved for all runs before any quad
/// is emitted, so an atlas repack in the middle of a frame can never leave
/// earlier quads pointing at stale regions.
// Keep placement, residency, clipping, and batching in one pass so every text quad follows the same rules.
#[allow(clippy::too_many_lines)]
pub(super) fn build_ui_text_geometry<'a>(
	draw_list: &UiDrawList,
	viewport: Extent,
	text_system: &mut TextSystem,
	atlas: &mut UiGlyphAtlas,
	frame_allocator: &'a bumpalo::Bump,
) -> UiTextGeometry<'a> {
	build_ui_text_geometry_damaged(draw_list, viewport, text_system, atlas, frame_allocator, None)
}

/// Builds glyph quads for the labels touching `damage`; `None` builds everything.
#[allow(clippy::too_many_lines)]
pub(super) fn build_ui_text_geometry_damaged<'a>(
	draw_list: &UiDrawList,
	viewport: Extent,
	text_system: &mut TextSystem,
	atlas: &mut UiGlyphAtlas,
	frame_allocator: &'a bumpalo::Bump,
	damage: Option<&[UiPixelRegion]>,
) -> UiTextGeometry<'a> {
	let viewport_width = viewport.width().max(1) as f32;
	let viewport_height = viewport.height().max(1) as f32;
	let sx = viewport_width / draw_list.layout_size[0].max(1.0);
	let sy = viewport_height / draw_list.layout_size[1].max(1.0);
	let font_scale = sx.min(sy);
	let viewport_clip = PixelClip {
		x0: 0.0,
		y0: 0.0,
		x1: viewport.width().max(1) as f32,
		y1: viewport.height().max(1) as f32,
	};

	// One UTF-8 byte is a cheap upper bound for one output glyph. Clamp to the
	// frame budget; this avoids repeated arena growth without reserving a maximum-sized frame.
	let glyph_capacity = draw_list
		.texts
		.iter()
		.filter(|text| should_rasterize_text(text))
		.fold(0usize, |count, text| count.saturating_add(text.text.len()))
		.min(MAX_UI_ELEMENTS);
	let mut geometry = UiTextGeometry {
		vertices: Vec::with_capacity_in(glyph_capacity * UI_VERTICES_PER_ELEMENT, frame_allocator),
		indices: Vec::with_capacity_in(glyph_capacity * UI_INDICES_PER_ELEMENT, frame_allocator),
		batches: Vec::with_capacity_in(
			draw_list
				.texts
				.len()
				.saturating_add(MAX_UI_VERTICES / MAX_UI_VERTICES_PER_DRAW)
				.min(glyph_capacity),
			frame_allocator,
		),
		truncated: false,
		dropped_glyphs: 0,
	};

	// Phase 1: place glyphs and make every needed bitmap resident.
	// First submission streams directly; caches pay for themselves on repeated work.
	let retain_runs = atlas.prepared;
	atlas.prepared = true;
	let mut pending = Vec::with_capacity_in(glyph_capacity, frame_allocator);
	let mut labels = Vec::with_capacity_in(draw_list.texts.len(), frame_allocator);
	for (text_index, text) in draw_list.texts.iter().enumerate() {
		let clip = pixel_clip(text.clip, sx, sy, viewport_clip);
		labels.push(PreparedText {
			clip,
			glyphs: pending.len()..pending.len(),
			cache_index: None,
		});
		let label = labels.last_mut().unwrap();
		if !should_rasterize_text(text) || clip.is_empty() {
			continue;
		}
		if !damage_intersects(
			damage,
			element_bounds(
				text.position,
				[text.size[0] * sx, text.size[1] * sy],
				sx,
				sy,
				UI_DAMAGE_MARGIN_PIXELS,
			),
		) {
			continue;
		}
		let font_size = raster_font_size(text.font_size * font_scale);
		if !retain_runs {
			place_text_glyphs(text_system, &text.text, font_size, &mut pending);
			label.glyphs.end = pending.len();
			continue;
		}
		let generation = atlas.generation;
		let index = *atlas.run_indices.entry(text.order).or_insert_with(|| {
			let index = atlas.runs.len();
			atlas.runs.push(CachedText {
				source_index: text_index,
				input: text.clone(),
				font_size: f32::NAN,
				viewport: (viewport, draw_list.layout_size),
				generation,
				glyphs: Vec::new(),
				vertices: Vec::new(),
				vertices_valid: false,
				placement_valid: false,
				vertices_stable: false,
			});
			index
		});
		label.cache_index = Some(index);
		let run = &mut atlas.runs[index];
		run.source_index = text_index;
		let placement_changed = run.font_size != font_size || run.input.text != text.text;
		let visual_changed = placement_changed
			|| run.input != *text
			|| run.viewport != (viewport, draw_list.layout_size)
			|| run.generation != generation;
		if !visual_changed && run.vertices_valid && run.placement_valid {
			// Resident, unchanged labels need no glyph walk. A later atlas repack
			// can rebuild their quads from retained local glyphs in phase two.
			run.vertices_stable = true;
			continue;
		}
		if placement_changed {
			run.placement_valid = false;
			// Continuously changing labels stream glyphs without retaining obsolete runs.
			place_text_glyphs(text_system, &text.text, font_size, &mut pending);
		} else {
			if !run.placement_valid {
				run.glyphs.clear();
				place_text_glyphs(text_system, &text.text, font_size, &mut run.glyphs);
				run.placement_valid = true;
			}
			for glyph in &mut run.glyphs {
				if run.generation != generation {
					glyph.region = None;
				}
				pending.push(*glyph);
			}
		}
		run.vertices_stable = !visual_changed;
		if visual_changed {
			run.vertices_valid = false;
		}
		label.glyphs.end = pending.len();
		run.input.clone_from(text);
		run.font_size = font_size;
		run.viewport = (viewport, draw_list.layout_size);
		run.generation = generation;
	}
	let generation = atlas.generation();
	for glyph in &mut pending {
		if glyph.region.is_none() {
			glyph.region = atlas.ensure(glyph.key, text_system);
		}
		if glyph.region.is_none() {
			geometry.dropped_glyphs += 1;
		}
	}
	let repacked = atlas.generation() != generation;

	// Phase 2: emit quads against the final atlas layout.
	let atlas_size = atlas.size().max(1) as f32;

	let regions = &atlas.regions;
	for (text_index, (text, label)) in draw_list.texts.iter().zip(&mut labels).enumerate() {
		if label.glyphs.is_empty() {
			let Some(index) = label.cache_index else {
				continue;
			};
			if atlas.runs[index].source_index != text_index {
				// Manually built draw lists can repeat IDs. Recover an earlier run
				// if a later occurrence replaced the entry after the fast path.
				label.glyphs.start = pending.len();
				text_system.place_glyphs(
					&text.text,
					raster_font_size(text.font_size * font_scale),
					(0, 0),
					|placement| {
						pending.push(PendingGlyph {
							key: placement.key,
							x: placement.x,
							y: placement.y,
							region: regions.get(&placement.key).copied(),
						});
					},
				);
				label.glyphs.end = pending.len();
			} else if repacked {
				// Only a repack needs to revisit a label that skipped placement.
				label.glyphs.start = pending.len();
				pending.extend_from_slice(&atlas.runs[index].glyphs);
				label.glyphs.end = pending.len();
			}
		}
		let run = label
			.cache_index
			.and_then(|index| atlas.runs.get_mut(index))
			.filter(|run| run.source_index == text_index);
		let mut local_glyphs = None;
		let mut cached_vertices = None;
		let mut valid = false;
		if let Some(run) = run {
			if run.placement_valid {
				local_glyphs = Some(&mut run.glyphs);
			}
			if run.vertices_stable {
				valid = run.vertices_valid && !repacked;
				run.vertices_valid = !repacked && geometry.dropped_glyphs == 0;
				cached_vertices = Some(&mut run.vertices);
			}
		}
		let start = geometry.vertices.len();
		let available = (MAX_UI_VERTICES - start).min((MAX_UI_INDICES - geometry.indices.len()) / 6 * 4);
		if !valid {
			if let Some(vertices) = cached_vertices.as_mut() {
				vertices.clear();
			}
			let inputs = GlyphQuadInputs::new(text, viewport, atlas_size, [sx, sy]);
			for (index, glyph) in pending[label.glyphs.clone()].iter().enumerate() {
				let region = if repacked {
					regions.get(&glyph.key).copied()
				} else {
					glyph.region
				};
				if let Some(local) = local_glyphs.as_deref_mut().and_then(|glyphs| glyphs.get_mut(index)) {
					local.region = region;
				}
				let Some(region) = region else {
					continue;
				};
				let Some(quad) = glyph_vertices(glyph, region, label.clip, &inputs) else {
					continue;
				};
				if let Some(vertices) = cached_vertices.as_mut() {
					vertices.extend_from_slice(&quad);
					// One extra quad preserves truncation reporting for oversized cached runs.
					if vertices.len() > MAX_UI_VERTICES {
						break;
					}
				} else {
					// Changing labels stream into the frame; no intermediate quad copy.
					if geometry.vertices.len() - start == available {
						geometry.truncated = true;
						break;
					}
					geometry.vertices.extend_from_slice(&quad);
				}
			}
		}
		if let Some(vertices) = cached_vertices {
			let count = vertices.len().min(available);
			geometry.vertices.extend_from_slice(&vertices[..count]);
			geometry.truncated |= count < vertices.len();
		}
		// Index the completed span, preserving depth order and each draw's u16 limit.
		let vertex_count = geometry.vertices.len() - start;
		let mut offset = 0;
		while offset < vertex_count {
			let new_batch = geometry.batches.last().is_none_or(|batch| {
				batch.depth != text.depth || batch.index_count as usize / 6 * 4 == MAX_UI_VERTICES_PER_DRAW
			});
			if new_batch {
				geometry.batches.push(UiTextDrawBatch {
					depth: text.depth,
					order: text.order,
					index_count: 0,
					first_index: geometry.indices.len() as u32,
					vertex_offset: (start + offset) as i32,
				});
			}
			let batch = geometry.batches.last_mut().unwrap();
			let base = batch.index_count as usize / 6 * 4;
			let count = (vertex_count - offset).min(MAX_UI_VERTICES_PER_DRAW - base);
			for vertex in (base..base + count).step_by(4) {
				let vertex = vertex as u16;
				geometry
					.indices
					.extend_from_slice(&[vertex, vertex + 1, vertex + 2, vertex + 2, vertex + 3, vertex]);
			}
			batch.index_count += (count / 4 * 6) as u32;
			batch.order = batch.order.min(text.order);
			offset += count;
		}

		if geometry.truncated {
			break;
		}
	}

	geometry
}

/// Rasterizes at the nearest whole pixel size, so text at a whole device size draws
/// texel for texel and a fractional zoom leaves the GPU only a few percent of scale.
/// Keep this renderer policy separate from logical text measurement.
fn raster_font_size(size: f32) -> f32 {
	size.max(1.0).round()
}

/// The `GlyphQuadInputs` struct keeps shared label transforms outside the glyph loop.
struct GlyphQuadInputs {
	origin: [f32; 2],
	residual: f32,
	inverse_residual: f32,
	inverse_atlas_size: f32,
	vertex: UiTextVertex,
}

impl GlyphQuadInputs {
	/// Resolves the requested scale and styling once for all glyphs in a label.
	fn new(text: &UiTextDrawElement, viewport: Extent, atlas_size: f32, scale: [f32; 2]) -> Self {
		let [sx, sy] = scale;
		let requested_size = (text.font_size * sx.min(sy)).max(1.0);
		let residual = requested_size / raster_font_size(requested_size);
		let mut origin = [text.position[0] * sx, text.position[1] * sy];
		// Unscaled bitmaps land on whole pixels so linear filtering returns their texels exactly.
		if residual == 1.0 {
			origin = [origin[0].round(), origin[1].round()];
		}
		let width = viewport.width().max(1) as f32;
		let height = viewport.height().max(1) as f32;
		let mask = scaled_clip_mask(text.clip_mask, sx, sy);
		Self {
			origin,
			residual,
			inverse_residual: residual.recip(),
			inverse_atlas_size: atlas_size.recip(),
			vertex: UiTextVertex {
				position: [0.0; 2],
				uv: [0.0; 2],
				pixel_position: [0.0; 2],
				color: text.color.into(),
				transform: [
					residual * 2.0 / width,
					-residual * 2.0 / height,
					origin[0] * 2.0 / width - 1.0,
					1.0 - origin[1] * 2.0 / height,
				],
				clip_mask_position: mask.position,
				clip_mask_size: mask.size,
				clip_mask_edges: mask.edges,
				clip_mask_corner: mask.corner,
			},
		}
	}
}

/// Clips one glyph against the final atlas packing and its current visual bounds.
#[inline]
fn glyph_vertices(
	glyph: &PendingGlyph,
	region: AtlasRegion,
	clip: PixelClip,
	inputs: &GlyphQuadInputs,
) -> Option<[UiTextVertex; 4]> {
	let GlyphQuadInputs {
		origin,
		residual,
		inverse_residual,
		inverse_atlas_size,
		..
	} = *inputs;
	// Include the transparent border so fractional placement does not cut off
	// coverage filtered just outside the original bitmap.
	let padding = UI_GLYPH_ATLAS_PADDING as f32;
	// Fractional pen advances would put an unscaled bitmap between pixels; snap it like its label's origin.
	let (glyph_x, glyph_y) = if residual == 1.0 {
		(glyph.x.round(), glyph.y.round())
	} else {
		(glyph.x, glyph.y)
	};
	let left = (glyph_x - padding) * residual + origin[0];
	let top = (glyph_y - padding) * residual + origin[1];
	let quad = PixelClip {
		x0: left,
		y0: top,
		x1: left + (region.width as f32 + padding * 2.0) * residual,
		y1: top + (region.height as f32 + padding * 2.0) * residual,
	}
	.intersect(clip);
	if quad.is_empty() {
		return None;
	}
	// Clip in destination pixels, then map the surviving edges back into the
	// bitmap. The vertex shader applies the remaining scale and translation.
	let local = [
		(quad.x0 - origin[0]) * inverse_residual,
		(quad.y0 - origin[1]) * inverse_residual,
		(quad.x1 - origin[0]) * inverse_residual,
		(quad.y1 - origin[1]) * inverse_residual,
	];
	let u0 = (region.x as f32 + local[0] - glyph_x) * inverse_atlas_size;
	let v0 = (region.y as f32 + local[1] - glyph_y) * inverse_atlas_size;
	let u1 = (region.x as f32 + local[2] - glyph_x) * inverse_atlas_size;
	let v1 = (region.y as f32 + local[3] - glyph_y) * inverse_atlas_size;
	let (x0, y0, x1, y1) = (quad.x0, quad.y0, quad.x1, quad.y1);
	let vertex = |pixel_position: [f32; 2], position: [f32; 2], uv: [f32; 2]| UiTextVertex {
		position,
		uv,
		pixel_position,
		..inputs.vertex
	};
	Some([
		vertex([x0, y0], [local[0], local[1]], [u0, v0]),
		vertex([x1, y0], [local[2], local[1]], [u1, v0]),
		vertex([x1, y1], [local[2], local[3]], [u1, v1]),
		vertex([x0, y1], [local[0], local[3]], [u0, v1]),
	])
}

#[cfg(test)]
mod tests {
	use utils::{Extent, RGBA};

	use super::{AtlasRegion, PixelClip, UI_GLYPH_ATLAS_PADDING, UiGlyphAtlas, build_ui_text_geometry, pixel_clip};
	use crate::ui::{
		font::{Glyph, GlyphKey, TextSystem},
		render_pass::data::{DrawClip, UiDrawList, UiTextDrawElement},
	};

	fn glyph(width: u32, height: u32, value: u8) -> Glyph {
		Glyph {
			width,
			height,
			xmin: 0,
			ymin: 0,
			advance_width: width as f32,
			bitmap: vec![value; (width * height) as usize],
		}
	}

	fn text(content: &str, depth: u32, order: u32, position: [f32; 2], clip: Option<DrawClip>) -> UiTextDrawElement {
		UiTextDrawElement {
			depth,
			order,
			position,
			size: [80.0, 20.0],
			clip,
			clip_mask: None,
			color: RGBA::new(1.0, 0.5, 0.25, 1.0),
			font_size: 16.0,
			text: content.to_string(),
		}
	}

	fn draw_list(texts: Vec<UiTextDrawElement>) -> UiDrawList {
		UiDrawList {
			layout_size: [100.0, 100.0],
			texts,
			..UiDrawList::default()
		}
	}

	#[test]
	fn fractional_zoom_reuses_bitmaps_and_preserves_exact_positions() {
		let mut fonts = TextSystem::new();
		assert!(fonts.has_font());
		let mut atlas = UiGlyphAtlas::new(256);
		let arena = bumpalo::Bump::new();
		let mut list = draw_list(vec![text("AW", 0, 7, [10.25, 20.375], None)]);
		list.texts[0].font_size = 13.1;
		let first = build_ui_text_geometry(&list, Extent::square(100), &mut fonts, &mut atlas, &arena);
		let start = first.vertices[0].pixel_position;
		let count = atlas.len();
		atlas.dirty = false;
		for step in 1..20 {
			// Stay inside the 13 pixel bucket, which ends at 13.5.
			list.texts[0].font_size = 13.1 + step as f32 * 0.02;
			let geometry = build_ui_text_geometry(&list, Extent::square(100), &mut fonts, &mut atlas, &arena);
			assert_eq!(atlas.len(), count);
			assert!(!atlas.is_dirty(), "sizes in one bucket must reuse resident bitmaps");
			for vertex in &geometry.vertices {
				// The production vertex transform must land on the unclamped fractional destination.
				let x = (vertex.position[0] * vertex.transform[0] + vertex.transform[2] + 1.0) * 50.0;
				let y = (1.0 - vertex.position[1] * vertex.transform[1] - vertex.transform[3]) * 50.0;
				assert!((x - vertex.pixel_position[0]).abs() < 0.0001);
				assert!((y - vertex.pixel_position[1]).abs() < 0.0001);
			}
		}
		list.texts[0].font_size = 13.1;
		list.texts[0].position[0] += 0.125;
		list.texts[0].position[1] += 0.25;
		let moved = build_ui_text_geometry(&list, Extent::square(100), &mut fonts, &mut atlas, &arena);
		assert!((moved.vertices[0].pixel_position[0] - start[0] - 0.125).abs() < 0.0001);
		assert!((moved.vertices[0].pixel_position[1] - start[1] - 0.25).abs() < 0.0001);
		list.texts[0].font_size = 16.1;
		let _ = build_ui_text_geometry(&list, Extent::square(100), &mut fonts, &mut atlas, &arena);
		assert!(atlas.is_dirty());
		assert_eq!(atlas.len(), count * 2);
	}

	#[test]
	fn cached_labels_survive_later_repacking_and_repeated_ids() {
		let mut fonts = TextSystem::new();
		assert!(fonts.has_font());
		let mut atlas = UiGlyphAtlas::new(32);
		let arena = bumpalo::Bump::new();
		let mut list = draw_list(vec![text("A", 0, 1, [2.25, 3.5], None)]);
		for _ in 0..4 {
			let _ = build_ui_text_geometry(&list, Extent::square(100), &mut fonts, &mut atlas, &arena);
		}
		let generation = atlas.generation();
		list.texts.push(text("BCDEFGHIJKLMNOPQRSTUVWXYZ", 0, 2, [0.0, 25.25], None));
		let cached = build_ui_text_geometry(&list, Extent::square(100), &mut fonts, &mut atlas, &arena);
		assert!(atlas.generation() > generation);
		atlas.clear_prepared_runs();
		let fresh = build_ui_text_geometry(&list, Extent::square(100), &mut fonts, &mut atlas, &arena);
		assert_eq!(
			bytemuck::cast_slice::<_, u8>(&cached.vertices),
			bytemuck::cast_slice::<_, u8>(&fresh.vertices)
		);
		for _ in 0..4 {
			let _ = build_ui_text_geometry(&list, Extent::square(100), &mut fonts, &mut atlas, &arena);
		}
		list.texts.push(text("B", 0, 1, [10.5, 50.75], None));
		let cached = build_ui_text_geometry(&list, Extent::square(100), &mut fonts, &mut atlas, &arena);
		atlas.clear_prepared_runs();
		let fresh = build_ui_text_geometry(&list, Extent::square(100), &mut fonts, &mut atlas, &arena);
		assert_eq!(
			bytemuck::cast_slice::<_, u8>(&cached.vertices),
			bytemuck::cast_slice::<_, u8>(&fresh.vertices)
		);
	}

	#[test]
	fn atlas_updates_copy_padded_regions_and_reset_clears_old_pixels() {
		let mut atlas = UiGlyphAtlas::new(64);
		atlas.insert(GlyphKey::new('A', 16.0), &glyph(5, 8, 93)).unwrap();
		let mut staging = vec![0; 64 * 64];
		atlas.copy_dirty_pixels(&mut staging);
		assert_eq!(staging, atlas.pixels());
		// Emulate completion of the initial full upload, then add two shelf neighbors.
		atlas.full_upload = false;
		atlas.dirty = false;
		atlas.insert(GlyphKey::new('B', 16.0), &glyph(6, 8, 121)).unwrap();
		atlas.insert(GlyphKey::new('C', 16.0), &glyph(4, 7, 151)).unwrap();
		// A distant staging byte must survive a partial copy.
		*staging.last_mut().unwrap() = 231;
		atlas.copy_dirty_pixels(&mut staging);
		let mut expected = atlas.pixels().to_vec();
		*expected.last_mut().unwrap() = 231;
		assert_eq!(staging, expected);
		atlas.reset(64);
		atlas.copy_dirty_pixels(&mut staging);
		assert!(staging.iter().all(|&pixel| pixel == 0));
	}

	#[test]
	fn insert_pads_glyphs_and_copies_coverage() {
		let mut atlas = UiGlyphAtlas::new(16);
		let region = atlas.insert(GlyphKey::new('a', 8.0), &glyph(3, 2, 200)).unwrap();

		assert_eq!(
			region,
			AtlasRegion {
				x: UI_GLYPH_ATLAS_PADDING,
				y: UI_GLYPH_ATLAS_PADDING,
				width: 3,
				height: 2
			}
		);
		assert!(atlas.is_dirty());
		let size = atlas.size();
		for row in 0..2 {
			for column in 0..3 {
				assert_eq!(atlas.pixels()[((region.y + row) * size + region.x + column) as usize], 200);
			}
		}
		// Padding stays transparent so linear filtering blends with empty coverage.
		assert_eq!(atlas.pixels()[0], 0);
		assert_eq!(atlas.pixels()[(region.y * size + region.x + 3) as usize], 0);
	}

	#[test]
	fn packing_reuses_the_tightest_shelf_and_reports_exhaustion() {
		let mut atlas = UiGlyphAtlas::new(16);
		// Two rows: one 4 high, one 8 high (padded sizes 6 and 10).
		let short = atlas.insert(GlyphKey::new('a', 8.0), &glyph(2, 4, 1)).unwrap();
		let tall = atlas.insert(GlyphKey::new('b', 8.0), &glyph(2, 8, 1)).unwrap();
		assert_ne!(short.y, tall.y);
		// A second short glyph lands on the short shelf, not the tall one.
		let short_again = atlas.insert(GlyphKey::new('c', 8.0), &glyph(2, 4, 1)).unwrap();
		assert_eq!(short_again.y, short.y);
		assert_eq!(short_again.x, short.x + 2 + UI_GLYPH_ATLAS_PADDING * 2);
		// Nothing 16 high fits below the two shelves.
		assert!(atlas.insert(GlyphKey::new('d', 8.0), &glyph(2, 14, 1)).is_none());
		assert_eq!(atlas.len(), 3);
	}

	#[test]
	fn reset_clears_regions_and_advances_generation() {
		let mut atlas = UiGlyphAtlas::new(16);
		atlas.insert(GlyphKey::new('a', 8.0), &glyph(2, 2, 255)).unwrap();
		let generation = atlas.generation();

		atlas.reset(32);

		assert_eq!(atlas.len(), 0);
		assert_eq!(atlas.size(), 32);
		assert_eq!(atlas.extent(), Extent::square(32));
		assert_eq!(atlas.generation(), generation + 1);
		assert!(atlas.pixels().iter().all(|value| *value == 0));
		assert!(atlas.is_dirty());
		assert!(atlas.resized);
	}

	#[test]
	fn ensure_grows_and_repacks_when_the_atlas_is_full() {
		let mut text_system = TextSystem::new();
		if !text_system.has_font() {
			return;
		}
		let mut atlas = UiGlyphAtlas::new(16);
		let generation = atlas.generation();

		let mut regions = Vec::new();
		for character in "abcdefghijklmnop".chars() {
			regions.push(atlas.ensure(GlyphKey::new(character, 24.0), &mut text_system));
		}

		assert!(regions.iter().all(Option::is_some));
		assert!(atlas.size() > 16);
		assert!(atlas.generation() > generation);
		assert_eq!(atlas.len(), 16);
		// A resident glyph is returned without touching the atlas again.
		let before = atlas.generation();
		let first = atlas.ensure(GlyphKey::new('a', 24.0), &mut text_system);
		assert_eq!(first, atlas.region(GlyphKey::new('a', 24.0)));
		assert_eq!(atlas.generation(), before);
	}

	#[test]
	fn invisible_glyphs_never_occupy_atlas_space() {
		let mut text_system = TextSystem::new();
		if !text_system.has_font() {
			return;
		}
		let mut atlas = UiGlyphAtlas::new(64);
		assert!(atlas.ensure(GlyphKey::new(' ', 16.0), &mut text_system).is_none());
		assert_eq!(atlas.len(), 0);
		assert!(!atlas.is_dirty());
	}

	#[test]
	fn pixel_clip_snaps_edges_to_whole_pixels_inside_the_viewport() {
		let viewport = PixelClip {
			x0: 0.0,
			y0: 0.0,
			x1: 100.0,
			y1: 50.0,
		};
		let clip = pixel_clip(
			Some(DrawClip {
				position: [10.4, -5.0],
				size: [20.2, 100.0],
			}),
			2.0,
			1.0,
			viewport,
		);
		assert_eq!((clip.x0, clip.y0, clip.x1, clip.y1), (21.0, 0.0, 61.0, 50.0));
		assert!(!clip.is_empty());
		assert_eq!(pixel_clip(None, 1.0, 1.0, viewport).x1, 100.0);
	}

	#[test]
	fn text_geometry_emits_scaled_quads_inside_the_atlas() {
		let mut text_system = TextSystem::new();
		if !text_system.has_font() {
			return;
		}
		let frame_allocator = bumpalo::Bump::new();
		let mut atlas = UiGlyphAtlas::new(256);
		let list = draw_list(vec![text("Hi", 3, 7, [10.0, 20.0], None)]);

		let geometry = build_ui_text_geometry(&list, Extent::square(200), &mut text_system, &mut atlas, &frame_allocator);

		assert!(!geometry.truncated);
		assert_eq!(geometry.dropped_glyphs, 0);
		assert_eq!(geometry.batches.len(), 1);
		assert_eq!(geometry.batches[0].depth, 3);
		assert_eq!(geometry.batches[0].order, 7);
		assert_eq!(geometry.vertices.len(), 8);
		assert_eq!(geometry.indices.len(), 12);
		assert_eq!(atlas.len(), 2);
		let atlas_size = atlas.size() as f32;
		for vertex in &geometry.vertices {
			assert!((0.0..=1.0).contains(&vertex.uv[0]) && (0.0..=1.0).contains(&vertex.uv[1]));
			assert_eq!(vertex.color, [1.0, 0.5, 0.25, 1.0]);
		}
		// Each quad covers exactly its glyph's region in the atlas at 1:1 texel scale.
		let quad = &geometry.vertices[0..4];
		let width = quad[1].pixel_position[0] - quad[0].pixel_position[0];
		let height = quad[2].pixel_position[1] - quad[1].pixel_position[1];
		assert!(((quad[1].uv[0] - quad[0].uv[0]) * atlas_size - width).abs() < 0.001);
		assert!(((quad[2].uv[1] - quad[1].uv[1]) * atlas_size - height).abs() < 0.001);
		// Glyphs scale with the viewport: the run starts at the scaled origin.
		assert!(quad[0].pixel_position[0] >= 20.0 - 2.0 && quad[0].pixel_position[1] >= 40.0);
	}

	#[test]
	fn text_geometry_trims_quads_and_uvs_to_the_clip() {
		let mut text_system = TextSystem::new();
		if !text_system.has_font() {
			return;
		}
		let frame_allocator = bumpalo::Bump::new();
		let mut atlas = UiGlyphAtlas::new(256);
		let unclipped = draw_list(vec![text("W", 0, 0, [0.0, 0.0], None)]);
		let full = build_ui_text_geometry(
			&unclipped,
			Extent::square(100),
			&mut text_system,
			&mut atlas,
			&frame_allocator,
		);
		let x0 = full.vertices[0].pixel_position[0];
		let x1 = full.vertices[1].pixel_position[0];
		assert!(x1 - x0 >= 4.0, "test glyph must be wide enough to trim");

		let clip_right = x0 + 2.0;
		let clipped = draw_list(vec![text(
			"W",
			0,
			0,
			[0.0, 0.0],
			Some(DrawClip {
				position: [0.0, 0.0],
				size: [clip_right, 100.0],
			}),
		)]);
		let trimmed = build_ui_text_geometry(&clipped, Extent::square(100), &mut text_system, &mut atlas, &frame_allocator);

		assert_eq!(trimmed.vertices.len(), 4);
		assert_eq!(trimmed.vertices[1].pixel_position[0], clip_right);
		assert_eq!(trimmed.vertices[0].uv[0], full.vertices[0].uv[0]);
		assert!(trimmed.vertices[1].uv[0] < full.vertices[1].uv[0]);
		let atlas_size = atlas.size() as f32;
		assert!(((trimmed.vertices[1].uv[0] - trimmed.vertices[0].uv[0]) * atlas_size - 2.0).abs() < 0.001);

		let hidden = draw_list(vec![text(
			"W",
			0,
			0,
			[0.0, 0.0],
			Some(DrawClip {
				position: [0.0, 0.0],
				size: [0.0, 0.0],
			}),
		)]);
		let empty = build_ui_text_geometry(&hidden, Extent::square(100), &mut text_system, &mut atlas, &frame_allocator);
		assert!(empty.vertices.is_empty());
		assert!(empty.batches.is_empty());
	}

	#[test]
	fn text_geometry_batches_by_depth_and_keeps_the_lowest_order() {
		let mut text_system = TextSystem::new();
		if !text_system.has_font() {
			return;
		}
		let frame_allocator = bumpalo::Bump::new();
		let mut atlas = UiGlyphAtlas::new(256);
		let list = draw_list(vec![
			text("a", 1, 9, [0.0, 0.0], None),
			text("b", 1, 4, [0.0, 30.0], None),
			text("c", 2, 6, [0.0, 60.0], None),
		]);

		let geometry = build_ui_text_geometry(&list, Extent::square(100), &mut text_system, &mut atlas, &frame_allocator);

		assert_eq!(geometry.batches.len(), 2);
		assert_eq!((geometry.batches[0].depth, geometry.batches[0].order), (1, 4));
		assert_eq!((geometry.batches[1].depth, geometry.batches[1].order), (2, 6));
		assert_eq!(geometry.batches[0].index_count, 12);
		assert_eq!(geometry.batches[1].first_index, 12);
		assert_eq!(geometry.batches[1].vertex_offset, 8);
	}

	#[test]
	fn text_geometry_matches_after_atlas_growth_and_warm_reuse() {
		let mut text_system = TextSystem::new();
		if !text_system.has_font() {
			return;
		}
		// A sixteen-pixel atlas cannot hold this alphabet; early glyphs move as it grows.
		let mut atlas = UiGlyphAtlas::new(16);
		let arena = bumpalo::Bump::new();
		let list = draw_list(vec![text("ABCDEFGHIJKLMNOPQRSTUVWXYZ", 0, 0, [0.0, 0.0], None)]);
		let first = build_ui_text_geometry(&list, Extent::square(100), &mut text_system, &mut atlas, &arena);
		let again = build_ui_text_geometry(&list, Extent::square(100), &mut text_system, &mut atlas, &arena);
		assert!(!first.vertices.is_empty());
		assert_eq!(first.dropped_glyphs, 0);
		assert_eq!(
			bytemuck::cast_slice::<_, u8>(&first.vertices),
			bytemuck::cast_slice::<_, u8>(&again.vertices),
		);
		assert_eq!(first.indices, again.indices);
		assert_eq!(first.batches, again.batches);
	}

	#[test]
	fn unchanged_text_reuses_resident_glyphs_without_dirtying_the_atlas() {
		let mut text_system = TextSystem::new();
		if !text_system.has_font() {
			return;
		}
		let frame_allocator = bumpalo::Bump::new();
		let mut atlas = UiGlyphAtlas::new(256);
		let list = draw_list(vec![text("Idle", 0, 0, [5.0, 5.0], None)]);

		build_ui_text_geometry(&list, Extent::square(100), &mut text_system, &mut atlas, &frame_allocator);
		assert!(atlas.is_dirty());
		atlas.dirty = false;
		let generation = atlas.generation();

		let again = build_ui_text_geometry(&list, Extent::square(100), &mut text_system, &mut atlas, &frame_allocator);

		assert!(!atlas.is_dirty());
		assert_eq!(atlas.generation(), generation);
		assert_eq!(again.vertices.len(), 16);
	}
}
