//! UI glyph atlas packing and per-glyph text geometry generation.
//!
//! Text is drawn as pixel-aligned quads that sample one shared coverage atlas.
//! Glyph bitmaps enter the atlas once per character and pixel size and stay
//! resident, so an unchanged label costs no CPU rasterization and no upload.
//! A moving or restyled label only regenerates its quads.

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
/// Transparent texels around every glyph so nearest sampling at a quad edge never reads a neighbor.
const UI_GLYPH_ATLAS_PADDING: u32 = 1;

pub(super) const UI_TEXT_VERTEX_LAYOUT: [ghi::pipelines::VertexElement; 8] = [
	ghi::pipelines::VertexElement::new("POSITION", ghi::DataTypes::Float2, 0),
	ghi::pipelines::VertexElement::new("UV", ghi::DataTypes::Float2, 0),
	ghi::pipelines::VertexElement::new("COLOR", ghi::DataTypes::Float4, 0),
	ghi::pipelines::VertexElement::new("PIXEL_POSITION", ghi::DataTypes::Float2, 0),
	ghi::pipelines::VertexElement::new("FEATHER_MASK_POSITION", ghi::DataTypes::Float2, 0),
	ghi::pipelines::VertexElement::new("FEATHER_MASK_SIZE", ghi::DataTypes::Float2, 0),
	ghi::pipelines::VertexElement::new("FEATHER_MASK_EDGES", ghi::DataTypes::Float4, 0),
	ghi::pipelines::VertexElement::new("FEATHER_MASK_CORNER", ghi::DataTypes::Float2, 0),
];

#[repr(C)]
#[derive(Debug, Clone, Copy, Default, bytemuck::Pod, bytemuck::Zeroable)]
pub(super) struct UiTextVertex {
	pub(super) position: [f32; 2],
	pub(super) uv: [f32; 2],
	pub(super) color: [f32; 4],
	pub(super) pixel_position: [f32; 2],
	pub(super) feather_mask_position: [f32; 2],
	pub(super) feather_mask_size: [f32; 2],
	pub(super) feather_mask_edges: [f32; 4],
	pub(super) feather_mask_corner: [f32; 2],
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
	size: u32,
	pixels: Vec<u8>,
	shelves: Vec<Shelf>,
	next_shelf_y: u32,
	regions: HashMap<GlyphKey, AtlasRegion>,
	generation: u64,
	dirty: bool,
	resized: bool,
}

impl UiGlyphAtlas {
	pub(super) fn new(size: u32) -> Self {
		let size = size.clamp(1, UI_GLYPH_ATLAS_MAX_SIZE);
		Self {
			size,
			pixels: vec![0; (size * size) as usize],
			shelves: Vec::new(),
			next_shelf_y: 0,
			regions: HashMap::new(),
			generation: 0,
			dirty: false,
			resized: false,
		}
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
		staging[..self.pixels.len()].copy_from_slice(&self.pixels);
		frame.sync_texture(image);
		self.dirty = false;
	}

	#[cfg(test)]
	pub(super) fn is_dirty(&self) -> bool {
		self.dirty
	}
}

#[derive(Debug, Clone, Copy)]
struct PendingGlyph {
	text_index: usize,
	key: GlyphKey,
	x: i32,
	y: i32,
	width: u32,
	height: u32,
}

#[derive(Debug, Clone, Copy)]
struct PixelClip {
	x0: i32,
	y0: i32,
	x1: i32,
	y1: i32,
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

// Converts a layout-space clip to whole viewport pixels so glyph trimming matches pixel-snapped glyph placement.
fn pixel_clip(clip: Option<DrawClip>, sx: f32, sy: f32, viewport: PixelClip) -> PixelClip {
	let Some(clip) = clip else {
		return viewport;
	};
	let x0 = (clip.position[0] * sx).round() as i32;
	let y0 = (clip.position[1] * sy).round() as i32;
	let x1 = ((clip.position[0] + clip.size[0]) * sx).round() as i32;
	let y1 = ((clip.position[1] + clip.size[1]) * sy).round() as i32;
	PixelClip { x0, y0, x1, y1 }.intersect(viewport)
}

/// Builds pixel-aligned glyph quads for every visible text run, batched by depth.
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
	let viewport_width = viewport.width().max(1) as f32;
	let viewport_height = viewport.height().max(1) as f32;
	let sx = viewport_width / draw_list.layout_size[0].max(1.0);
	let sy = viewport_height / draw_list.layout_size[1].max(1.0);
	let font_scale = sx.min(sy);
	let viewport_clip = PixelClip {
		x0: 0,
		y0: 0,
		x1: viewport.width().max(1) as i32,
		y1: viewport.height().max(1) as i32,
	};

	let mut geometry = UiTextGeometry {
		vertices: Vec::new_in(frame_allocator),
		indices: Vec::new_in(frame_allocator),
		batches: Vec::new_in(frame_allocator),
		truncated: false,
		dropped_glyphs: 0,
	};

	// Phase 1: place glyphs and make every needed bitmap resident.
	let mut pending = Vec::new_in(frame_allocator);
	let mut clips = Vec::with_capacity_in(draw_list.texts.len(), frame_allocator);
	for (text_index, text) in draw_list.texts.iter().enumerate() {
		let clip = pixel_clip(text.clip, sx, sy, viewport_clip);
		clips.push(clip);
		if !should_rasterize_text(text) || clip.is_empty() {
			continue;
		}
		let origin = ((text.position[0] * sx).round() as i32, (text.position[1] * sy).round() as i32);
		let font_size = (text.font_size * font_scale).max(1.0);
		text_system.place_glyphs(&text.text, font_size, origin, |placement| {
			pending.push(PendingGlyph {
				text_index,
				key: placement.key,
				x: placement.x,
				y: placement.y,
				width: placement.glyph.width,
				height: placement.glyph.height,
			});
		});
	}
	for glyph in &pending {
		if atlas.ensure(glyph.key, text_system).is_none() {
			geometry.dropped_glyphs += 1;
		}
	}

	// Phase 2: emit quads against the final atlas layout.
	let atlas_size = atlas.size().max(1) as f32;
	let to_clip_x = |pixel_x: f32| (pixel_x / viewport_width) * 2.0 - 1.0;
	let to_clip_y = |pixel_y: f32| 1.0 - (pixel_y / viewport_height) * 2.0;

	let mut batch_first_index = 0usize;
	let mut batch_vertex_offset = 0usize;
	let mut batch_vertex_count = 0usize;
	let mut batch_index_count = 0usize;
	let mut batch_depth = 0u32;
	let mut batch_order = 0u32;

	for glyph in &pending {
		let text = &draw_list.texts[glyph.text_index];
		let Some(region) = atlas.region(glyph.key) else {
			continue;
		};
		let quad = PixelClip {
			x0: glyph.x,
			y0: glyph.y,
			x1: glyph.x + glyph.width as i32,
			y1: glyph.y + glyph.height as i32,
		}
		.intersect(clips[glyph.text_index]);
		if quad.is_empty() {
			continue;
		}

		if geometry.vertices.len() + UI_VERTICES_PER_ELEMENT > MAX_UI_VERTICES
			|| geometry.indices.len() + UI_INDICES_PER_ELEMENT > MAX_UI_INDICES
		{
			geometry.truncated = true;
			break;
		}

		if batch_index_count > 0
			&& (batch_vertex_count + UI_VERTICES_PER_ELEMENT > MAX_UI_VERTICES_PER_DRAW || batch_depth != text.depth)
		{
			geometry.batches.push(UiTextDrawBatch {
				depth: batch_depth,
				order: batch_order,
				index_count: batch_index_count as u32,
				first_index: batch_first_index as u32,
				vertex_offset: batch_vertex_offset as i32,
			});
			batch_first_index = geometry.indices.len();
			batch_vertex_offset = geometry.vertices.len();
			batch_vertex_count = 0;
			batch_index_count = 0;
		}
		if batch_index_count == 0 {
			batch_depth = text.depth;
			batch_order = text.order;
		} else {
			batch_order = batch_order.min(text.order);
		}

		// Trimmed quads keep their texel alignment by trimming the atlas window identically.
		let u0 = (region.x as f32 + (quad.x0 - glyph.x) as f32) / atlas_size;
		let v0 = (region.y as f32 + (quad.y0 - glyph.y) as f32) / atlas_size;
		let u1 = (region.x as f32 + (quad.x1 - glyph.x) as f32) / atlas_size;
		let v1 = (region.y as f32 + (quad.y1 - glyph.y) as f32) / atlas_size;
		let (x0, y0, x1, y1) = (quad.x0 as f32, quad.y0 as f32, quad.x1 as f32, quad.y1 as f32);
		let color: [f32; 4] = text.color.into();
		let feather_mask = scaled_feather_mask(text.feather_mask, sx, sy);
		let vertex = |position: [f32; 2], uv: [f32; 2]| UiTextVertex {
			position: [to_clip_x(position[0]), to_clip_y(position[1])],
			uv,
			color,
			pixel_position: position,
			feather_mask_position: feather_mask.position,
			feather_mask_size: feather_mask.size,
			feather_mask_edges: feather_mask.edges,
			feather_mask_corner: feather_mask.corner,
		};
		geometry.vertices.extend_from_slice(&[
			vertex([x0, y0], [u0, v0]),
			vertex([x1, y0], [u1, v0]),
			vertex([x1, y1], [u1, v1]),
			vertex([x0, y1], [u0, v1]),
		]);
		let base_vertex = batch_vertex_count as u16;
		geometry.indices.extend_from_slice(&[
			base_vertex,
			base_vertex + 1,
			base_vertex + 2,
			base_vertex + 2,
			base_vertex + 3,
			base_vertex,
		]);
		batch_vertex_count += UI_VERTICES_PER_ELEMENT;
		batch_index_count += UI_INDICES_PER_ELEMENT;
	}

	if batch_index_count > 0 {
		geometry.batches.push(UiTextDrawBatch {
			depth: batch_depth,
			order: batch_order,
			index_count: batch_index_count as u32,
			first_index: batch_first_index as u32,
			vertex_offset: batch_vertex_offset as i32,
		});
	}

	geometry
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
			feather_mask: None,
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
		// Padding stays transparent so nearest sampling never bleeds into a neighbor.
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
	fn pixel_clip_rounds_to_whole_pixels_and_stays_inside_the_viewport() {
		let viewport = PixelClip {
			x0: 0,
			y0: 0,
			x1: 100,
			y1: 50,
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
		assert_eq!((clip.x0, clip.y0, clip.x1, clip.y1), (21, 0, 61, 50));
		assert!(!clip.is_empty());
		assert_eq!(pixel_clip(None, 1.0, 1.0, viewport).x1, 100);
	}

	#[test]
	fn text_geometry_emits_pixel_aligned_quads_inside_the_atlas() {
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
			assert_eq!(vertex.pixel_position[0].fract(), 0.0);
			assert_eq!(vertex.pixel_position[1].fract(), 0.0);
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
