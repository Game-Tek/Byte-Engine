//! UI text drawn from glyph outlines on the GPU with the Slug algorithm.
//!
//! Each character's quadratic curves are packed once, at no particular size, into two storage
//! buffers: the curves, and bands that list the curves crossing each horizontal and vertical slice
//! of the glyph. The fragment shader finds a pixel's bands and computes its exact coverage from
//! those few curves. Text stays sharp at any size, a new size uploads nothing, and the CPU never
//! rasterizes a glyph.
//!
//! The data layout follows the reference shaders by Eric Lengyel (<https://github.com/EricLengyel/Slug>),
//! with buffers instead of textures. [`UiGlyphCurves`] owns the packed data and
//! [`build_ui_slug_geometry_damaged`] emits the glyph primitives that the UI ubershader draws.

use super::*;
use crate::ui::font::GlyphOutline;

/// Elements in the GPU curve buffer. A glyph takes about one per curve, so this holds a few thousand glyphs.
pub(super) const UI_GLYPH_CURVE_CAPACITY: usize = 1 << 16;
/// Values in the GPU band buffer. A glyph takes a few per curve.
pub(super) const UI_GLYPH_BAND_CAPACITY: usize = 1 << 18;

/// Most bands per axis. Bands thinner than a glyph's curves list the same curves again and only cost memory.
const MAX_BANDS: usize = 16;
/// Overlap between neighboring bands in em units, so a sample on a band's edge finds its curves in either band.
const BAND_OVERLAP: f32 = 1.0 / 1024.0;
/// The shader's coverage reaches half a pixel past the outline, so every quad grows by this much.
const DILATION_PIXELS: f32 = 0.5;

/// The `PackedGlyph` struct tells the shader where one glyph's bands are and how to index them.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(super) struct PackedGlyph {
	/// Index of the glyph's first band header in the band buffer.
	pub(super) location: u32,
	/// Index of the last horizontal and the last vertical band.
	pub(super) last_band: [u32; 2],
	/// Maps an em space position to band indices: scale for x and y, then offset for x and y.
	pub(super) banding: [f32; 4],
}

/// Placement and bounds of one curve while its glyph is packed.
struct CurveSpan {
	location: u32,
	min: [f32; 2],
	max: [f32; 2],
}

/// The `UiGlyphCurves` struct packs glyph outlines into the curve and band data that the Slug shader reads.
///
/// Data is only ever appended, so [`Self::upload`] mirrors just the new tail. When a buffer is
/// full, [`Self::reset`] empties both and advances [`Self::generation`], which invalidates geometry
/// that referenced the old locations. Build quads with [`build_ui_slug_geometry_damaged`].
pub(super) struct UiGlyphCurves {
	curves: Vec<[f32; 4]>,
	bands: Vec<u32>,
	/// Packed glyphs by [`crate::ui::font::OutlinePlacement::index`].
	glyphs: Vec<Option<PackedGlyph>>,
	/// Curve elements and band values the GPU already holds.
	uploaded: (usize, usize),
	/// Most curve elements and band values the GPU buffers hold.
	capacity: (usize, usize),
	generation: u64,
	spans: Vec<CurveSpan>,
}

impl UiGlyphCurves {
	/// Creates an empty packing limited to `curve_capacity` curve elements and `band_capacity` band values.
	pub(super) fn new(curve_capacity: usize, band_capacity: usize) -> Self {
		Self {
			curves: Vec::new(),
			bands: Vec::new(),
			glyphs: Vec::new(),
			uploaded: (0, 0),
			capacity: (curve_capacity, band_capacity),
			generation: 0,
			spans: Vec::new(),
		}
	}

	/// Identifies the current packing; geometry built against an older generation is stale.
	pub(super) fn generation(&self) -> u64 {
		self.generation
	}

	#[cfg(test)]
	pub(super) fn curves(&self) -> &[[f32; 4]] {
		&self.curves
	}

	#[cfg(test)]
	pub(super) fn bands(&self) -> &[u32] {
		&self.bands
	}

	/// Returns where the shader finds `outline`, packing it on first use.
	///
	/// Returns `None` when the glyph does not fit in the remaining buffer space.
	pub(super) fn ensure(&mut self, index: usize, outline: &GlyphOutline) -> Option<PackedGlyph> {
		if let Some(Some(glyph)) = self.glyphs.get(index) {
			return Some(*glyph);
		}
		let glyph = self.pack(outline)?;
		if self.glyphs.len() <= index {
			self.glyphs.resize(index + 1, None);
		}
		self.glyphs[index] = Some(glyph);
		Some(glyph)
	}

	/// Forgets every glyph so that a full buffer can hold the glyphs of the current frame.
	pub(super) fn reset(&mut self) {
		self.curves.clear();
		self.bands.clear();
		self.glyphs.clear();
		self.uploaded = (0, 0);
		self.generation += 1;
	}

	/// Appends one outline's curves and bands.
	///
	/// Band data starts with a (curve count, list offset) pair for every horizontal band, then for
	/// every vertical band. The lists of curve locations follow, and offsets are relative to the
	/// first header.
	fn pack(&mut self, outline: &GlyphOutline) -> Option<PackedGlyph> {
		let Self {
			curves,
			bands,
			spans,
			capacity,
			..
		} = self;
		let (curve_start, band_start) = (curves.len(), bands.len());

		// A curve stores its first two points and reads the third from the next element. Curves of
		// one contour share that point, so a contour costs one element per curve plus one to end it.
		spans.clear();
		let mut contour_end: Option<[f32; 2]> = None;
		for [p1, p2, p3] in &outline.curves {
			if let Some(end) = contour_end.filter(|end| end != p1) {
				curves.push([end[0], end[1], 0.0, 0.0]);
			}
			spans.push(CurveSpan {
				location: curves.len() as u32,
				min: [p1[0].min(p2[0]).min(p3[0]), p1[1].min(p2[1]).min(p3[1])],
				max: [p1[0].max(p2[0]).max(p3[0]), p1[1].max(p2[1]).max(p3[1])],
			});
			curves.push([p1[0], p1[1], p2[0], p2[1]]);
			contour_end = Some(*p3);
		}
		if let Some(end) = contour_end {
			curves.push([end[0], end[1], 0.0, 0.0]);
		}

		let min = [outline.bounds[0], outline.bounds[1]];
		let extent = [outline.bounds[2] - min[0], outline.bounds[3] - min[1]];
		// Every ray crosses a closed contour at least twice, so more bands than half the curves cannot thin a band further.
		let wanted = (outline.curves.len() / 2).clamp(1, MAX_BANDS);
		// Horizontal bands slice the glyph along y and hold the curves a horizontal ray can cross;
		// vertical bands slice along x. An axis without extent has nothing to slice.
		let sliced = [(1, 0), (0, 1)].map(|(across, along)| (across, along, if extent[across] > 0.0 { wanted } else { 1 }));
		bands.resize(band_start + (sliced[0].2 + sliced[1].2) * 2, 0);

		let mut header = band_start;
		for (across, along, count) in sliced {
			for band in 0..count {
				let low = min[across] + extent[across] * band as f32 / count as f32 - BAND_OVERLAP;
				let high = min[across] + extent[across] * (band + 1) as f32 / count as f32 + BAND_OVERLAP;
				let list = bands.len();
				// A line parallel to the ray never crosses it, so it is left out.
				bands.extend(spans.iter().enumerate().filter_map(|(index, span)| {
					(span.min[across] != span.max[across] && span.max[across] >= low && span.min[across] <= high)
						.then_some(index as u32)
				}));
				// The shader stops at the first curve that ends before the pixel, so the curves
				// reaching farthest along the ray come first.
				bands[list..].sort_unstable_by(|a, b| spans[*b as usize].max[along].total_cmp(&spans[*a as usize].max[along]));
				for entry in &mut bands[list..] {
					*entry = spans[*entry as usize].location;
				}
				bands[header] = (bands.len() - list) as u32;
				bands[header + 1] = (list - band_start) as u32;
				header += 2;
			}
		}

		if curves.len() > capacity.0 || bands.len() > capacity.1 {
			curves.truncate(curve_start);
			bands.truncate(band_start);
			return None;
		}

		// A position maps to its band by scale and offset. Vertical bands index x and horizontal bands index y.
		let [horizontal, vertical] = sliced.map(|(across, _, count)| {
			let scale = if extent[across] > 0.0 {
				count as f32 / extent[across]
			} else {
				0.0
			};
			(scale, -min[across] * scale, count as u32 - 1)
		});
		Some(PackedGlyph {
			location: band_start as u32,
			last_band: [horizontal.2, vertical.2],
			banding: [vertical.0, horizontal.0, vertical.1, horizontal.1],
		})
	}

	/// Mirrors the data appended since the last upload to the GPU; a no-op when nothing was added.
	pub(super) fn upload(
		&mut self,
		frame: &mut ghi::implementation::Frame,
		curve_buffer: ghi::BufferHandle<[[f32; 4]; UI_GLYPH_CURVE_CAPACITY]>,
		band_buffer: ghi::BufferHandle<[u32; UI_GLYPH_BAND_CAPACITY]>,
	) {
		let (curves, bands) = self.uploaded;
		if curves != self.curves.len() {
			frame.get_mut_buffer_slice(curve_buffer)[curves..self.curves.len()].copy_from_slice(&self.curves[curves..]);
			frame.sync_buffer(curve_buffer);
		}
		if bands != self.bands.len() {
			frame.get_mut_buffer_slice(band_buffer)[bands..self.bands.len()].copy_from_slice(&self.bands[bands..]);
			frame.sync_buffer(band_buffer);
		}
		self.uploaded = (self.curves.len(), self.bands.len());
	}
}

/// Builds one primitive per visible glyph of every label.
#[cfg(test)]
pub(super) fn build_ui_slug_geometry<'a>(
	draw_list: &UiDrawList,
	viewport: Extent,
	text_system: &mut TextSystem,
	glyphs: &mut UiGlyphCurves,
	masks: &mut UiMaskTable,
	frame_allocator: &'a bumpalo::Bump,
) -> UiTextGeometry<'a> {
	build_ui_slug_geometry_damaged(draw_list, viewport, text_system, glyphs, masks, frame_allocator, None)
}

/// Builds glyph primitives for the labels touching `damage`; `None` builds everything.
///
/// Every primitive carries its glyph's band data location. When the buffers fill up partway through a
/// frame, they are reset and the frame is built again, so no primitive keeps a location from before the reset.
pub(super) fn build_ui_slug_geometry_damaged<'a>(
	draw_list: &UiDrawList,
	viewport: Extent,
	text_system: &mut TextSystem,
	glyphs: &mut UiGlyphCurves,
	masks: &mut UiMaskTable,
	frame_allocator: &'a bumpalo::Bump,
	damage: Option<&[UiPixelRegion]>,
) -> UiTextGeometry<'a> {
	let width = viewport.width().max(1) as f32;
	let height = viewport.height().max(1) as f32;
	let sx = width / draw_list.layout_size[0].max(1.0);
	let sy = height / draw_list.layout_size[1].max(1.0);
	let font_scale = sx.min(sy);
	let viewport_clip = PixelClip {
		x0: 0.0,
		y0: 0.0,
		x1: width,
		y1: height,
	};

	// One UTF-8 byte is a cheap upper bound for one output glyph.
	let glyph_capacity = draw_list
		.texts
		.iter()
		.filter(|text| should_rasterize_text(text))
		.fold(0usize, |count, text| count.saturating_add(text.text.len()))
		.min(MAX_UI_PRIMITIVES);
	let mut geometry = UiTextGeometry {
		primitives: Vec::with_capacity_in(glyph_capacity, frame_allocator),
		labels: Vec::with_capacity_in(draw_list.texts.len(), frame_allocator),
		truncated: false,
		dropped_glyphs: 0,
	};

	let mut reset = false;
	loop {
		for text in &draw_list.texts {
			let start = geometry.primitives.len();
			geometry.labels.push(start..start);
			let clip = pixel_clip(text.clip, sx, sy, viewport_clip);
			if geometry.truncated || !should_rasterize_text(text) || clip.is_empty() {
				continue;
			}
			let bounds = turned_bounds(
				element_bounds(
					text.position,
					[text.size[0] * sx, text.size[1] * sy],
					sx,
					sy,
					UI_DAMAGE_MARGIN_PIXELS,
				),
				text.clip_mask,
				sx,
				sy,
			);
			if !damage_intersects(damage, bounds) {
				continue;
			}

			let size = (text.font_size * font_scale).max(1.0);
			let origin = [text.position[0] * sx, text.position[1] * sy];
			let label = UiPrimitive {
				color: text.color.into(),
				kind: UI_KIND_SLUG_GLYPH,
				mask: masks.index(None, text.clip_mask, sx, sy),
				..UiPrimitive::default()
			};
			text_system.place_outlines(&text.text, size, |placement| {
				let Some(glyph) = glyphs.ensure(placement.index, placement.outline) else {
					geometry.dropped_glyphs += 1;
					return;
				};
				if geometry.primitives.len() == MAX_UI_PRIMITIVES {
					geometry.truncated = true;
					return;
				}
				// A baseline on a whole pixel keeps horizontal strokes crisp. Horizontal positions stay
				// fractional because the shader resolves them exactly.
				let pen = [origin[0] + placement.pen[0], (origin[1] + placement.pen[1]).round()];
				let [min_x, min_y, max_x, max_y] = placement.outline.bounds;
				// The shader's coverage reaches half a pixel past the outline. UI text is axis aligned,
				// so growing the quad by that constant replaces the reference shader's dynamic dilation.
				let quad = PixelClip {
					x0: pen[0] + min_x * size - DILATION_PIXELS,
					y0: pen[1] - max_y * size - DILATION_PIXELS,
					x1: pen[0] + max_x * size + DILATION_PIXELS,
					y1: pen[1] - min_y * size + DILATION_PIXELS,
				}
				.intersect(clip);
				if quad.is_empty() {
					return;
				}
				// The shader derives the em space sample from the pixel, the pen, and the size, which
				// accounts for the dilation and the clip at once.
				geometry.primitives.push(UiPrimitive {
					bounds: [quad.x0, quad.y0, quad.x1, quad.y1],
					a: [pen[0], pen[1], size, 0.0],
					b: glyph.banding,
					data0: glyph.location,
					data1: glyph.last_band[0] | glyph.last_band[1] << 16,
					..label
				});
			});
			geometry.labels.last_mut().unwrap().end = geometry.primitives.len();
		}

		if geometry.dropped_glyphs == 0 || reset {
			break;
		}
		// The buffers filled up. Keep only what this frame draws, and build it against the new locations.
		glyphs.reset();
		reset = true;
		geometry.primitives.clear();
		geometry.labels.clear();
		geometry.truncated = false;
		geometry.dropped_glyphs = 0;
	}

	geometry
}

#[cfg(test)]
mod tests {
	use utils::{Extent, RGBA};

	use super::{UI_GLYPH_BAND_CAPACITY, UI_GLYPH_CURVE_CAPACITY, UiGlyphCurves, build_ui_slug_geometry};
	use crate::ui::{
		font::TextSystem,
		render_pass::data::{DrawClip, UI_KIND_SLUG_GLYPH, UiDrawList, UiMaskTable, UiPrimitive, UiTextDrawElement},
	};

	fn text(content: &str, depth: u32, order: u32, position: [f32; 2], clip: Option<DrawClip>) -> UiTextDrawElement {
		UiTextDrawElement {
			depth,
			order,
			position,
			size: [90.0, 20.0],
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

	fn glyph_curves() -> UiGlyphCurves {
		UiGlyphCurves::new(UI_GLYPH_CURVE_CAPACITY, UI_GLYPH_BAND_CAPACITY)
	}

	/// The em space sample the shader derives for a pixel of a glyph primitive.
	fn em_position(glyph: &UiPrimitive, pixel: [f32; 2]) -> [f32; 2] {
		[(pixel[0] - glyph.a[0]) / glyph.a[2], (glyph.a[1] - pixel[1]) / glyph.a[2]]
	}

	#[test]
	fn geometry_emits_one_primitive_per_visible_glyph_in_em_space() {
		let mut fonts = TextSystem::new();
		if !fonts.has_font() {
			return;
		}
		let arena = bumpalo::Bump::new();
		let mut glyphs = glyph_curves();
		let mut masks = UiMaskTable::default();
		let list = draw_list(vec![text("H i", 3, 7, [10.0, 20.0], None)]);

		let geometry = build_ui_slug_geometry(&list, Extent::square(200), &mut fonts, &mut glyphs, &mut masks, &arena);

		assert!(!geometry.truncated);
		assert_eq!(geometry.dropped_glyphs, 0);
		// The space advances the pen without a primitive.
		assert_eq!(geometry.primitives.len(), 2);
		assert_eq!(geometry.labels.as_slice(), std::slice::from_ref(&(0..2)));

		// The viewport doubles the layout, so the label draws at 32 pixels per em.
		let (_, outline) = fonts.outline('H').unwrap();
		let bounds = outline.bounds;
		let glyph = &geometry.primitives[0];
		assert_eq!(glyph.kind, UI_KIND_SLUG_GLYPH);
		assert_eq!(glyph.mask, 0);
		assert_eq!(glyph.color, [1.0, 0.5, 0.25, 1.0]);
		assert_eq!(glyph.a[2], 32.0);
		// Half a pixel on every side lets the shader's partial edge coverage reach the screen.
		let width = glyph.bounds[2] - glyph.bounds[0];
		assert!((width - ((bounds[2] - bounds[0]) * 32.0 + 1.0)).abs() < 0.001);
		// Em space maps to pixels by the font size alone, with y pointing up.
		let top_left = em_position(glyph, [glyph.bounds[0], glyph.bounds[1]]);
		assert!((top_left[0] - (bounds[0] - 0.5 / 32.0)).abs() < 0.0001);
		assert!((top_left[1] - (bounds[3] + 0.5 / 32.0)).abs() < 0.0001);
		// The label starts at its scaled origin.
		assert!(glyph.bounds[0] >= 20.0 - 2.0 && glyph.bounds[1] >= 40.0 - 1.0);
	}

	#[test]
	fn new_sizes_and_positions_reuse_packed_glyphs() {
		let mut fonts = TextSystem::new();
		if !fonts.has_font() {
			return;
		}
		let arena = bumpalo::Bump::new();
		let mut glyphs = glyph_curves();
		let mut masks = UiMaskTable::default();
		let mut list = draw_list(vec![text("Resize", 0, 0, [4.0, 4.0], None)]);
		let first = build_ui_slug_geometry(&list, Extent::square(100), &mut fonts, &mut glyphs, &mut masks, &arena);
		let packed = (glyphs.curves().len(), glyphs.bands().len(), glyphs.generation());
		assert!(packed.0 > 0 && packed.1 > 0);

		for font_size in [9.5, 17.25, 64.0] {
			list.texts[0].font_size = font_size;
			list.texts[0].position[0] += 0.375;
			let geometry = build_ui_slug_geometry(&list, Extent::square(100), &mut fonts, &mut glyphs, &mut masks, &arena);
			// Outlines have no size, so nothing new reaches the GPU and primitives keep their band data.
			assert_eq!((glyphs.curves().len(), glyphs.bands().len(), glyphs.generation()), packed);
			for (glyph, original) in geometry.primitives.iter().zip(&first.primitives) {
				assert_eq!(glyph.b, original.b);
				assert_eq!((glyph.data0, glyph.data1), (original.data0, original.data1));
				assert_eq!(glyph.a[2], font_size);
			}
		}
	}

	#[test]
	fn fractional_positions_move_quads_exactly_but_keep_the_baseline_on_a_pixel() {
		let mut fonts = TextSystem::new();
		if !fonts.has_font() {
			return;
		}
		let arena = bumpalo::Bump::new();
		let mut glyphs = glyph_curves();
		let mut masks = UiMaskTable::default();
		let mut list = draw_list(vec![text("A", 0, 0, [10.0, 20.0], None)]);
		let start =
			build_ui_slug_geometry(&list, Extent::square(100), &mut fonts, &mut glyphs, &mut masks, &arena).primitives[0];

		for offset in [0.125, 0.375, 0.75] {
			list.texts[0].position = [10.0 + offset, 20.0 + offset];
			let moved =
				build_ui_slug_geometry(&list, Extent::square(100), &mut fonts, &mut glyphs, &mut masks, &arena).primitives[0];

			assert!((moved.bounds[0] - start.bounds[0] - offset).abs() < 0.0001);
			// The pen's y is the baseline.
			assert_eq!(moved.a[1], moved.a[1].round());
			let (moved_em, start_em) = (
				em_position(&moved, [moved.bounds[0], moved.bounds[1]]),
				em_position(&start, [start.bounds[0], start.bounds[1]]),
			);
			assert!((moved_em[0] - start_em[0]).abs() < 0.0001 && (moved_em[1] - start_em[1]).abs() < 0.0001);
		}
	}

	#[test]
	fn geometry_trims_quads_to_the_clip_without_moving_the_outline() {
		let mut fonts = TextSystem::new();
		if !fonts.has_font() {
			return;
		}
		let arena = bumpalo::Bump::new();
		let mut glyphs = glyph_curves();
		let mut masks = UiMaskTable::default();
		let unclipped = draw_list(vec![text("W", 0, 0, [0.0, 0.0], None)]);
		let full = build_ui_slug_geometry(&unclipped, Extent::square(100), &mut fonts, &mut glyphs, &mut masks, &arena);
		let x0 = full.primitives[0].bounds[0];
		assert!(
			full.primitives[0].bounds[2] - x0 >= 6.0,
			"test glyph must be wide enough to trim"
		);

		let clip_right = x0.ceil() + 3.0;
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
		let trimmed = build_ui_slug_geometry(&clipped, Extent::square(100), &mut fonts, &mut glyphs, &mut masks, &arena);

		assert_eq!(trimmed.primitives.len(), 1);
		assert_eq!(trimmed.primitives[0].bounds[2], clip_right);
		// The trimmed quad samples the outline where the clip cut it, because the pen did not move.
		assert_eq!(trimmed.primitives[0].a, full.primitives[0].a);

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
		let empty = build_ui_slug_geometry(&hidden, Extent::square(100), &mut fonts, &mut glyphs, &mut masks, &arena);
		assert!(empty.primitives.is_empty());
		assert_eq!(empty.labels.as_slice(), std::slice::from_ref(&(0..0)));
	}

	#[test]
	fn labels_locate_each_text_so_the_frame_can_merge_them_in_painter_order() {
		let mut fonts = TextSystem::new();
		if !fonts.has_font() {
			return;
		}
		let arena = bumpalo::Bump::new();
		let mut glyphs = glyph_curves();
		let mut masks = UiMaskTable::default();
		let list = draw_list(vec![
			text("ab", 1, 9, [0.0, 0.0], None),
			text("", 1, 4, [0.0, 30.0], None),
			text("c", 2, 6, [0.0, 60.0], None),
		]);

		let geometry = build_ui_slug_geometry(&list, Extent::square(100), &mut fonts, &mut glyphs, &mut masks, &arena);

		assert_eq!(geometry.labels.as_slice(), [0..2, 2..2, 2..3]);
	}

	#[test]
	fn full_buffers_reset_for_the_current_frame_and_report_what_still_does_not_fit() {
		let mut fonts = TextSystem::new();
		if !fonts.has_font() {
			return;
		}
		let arena = bumpalo::Bump::new();
		let mut masks = UiMaskTable::default();
		// Measure two labels' data, then allow only a little more than the larger one.
		let first = draw_list(vec![text("abcdefg", 0, 0, [0.0, 0.0], None)]);
		let second = draw_list(vec![text("STUVWXYZ", 0, 0, [0.0, 0.0], None)]);
		let mut measured = Vec::new();
		for list in [&first, &second] {
			let mut glyphs = glyph_curves();
			build_ui_slug_geometry(list, Extent::square(100), &mut fonts, &mut glyphs, &mut masks, &arena);
			measured.push((glyphs.curves().len(), glyphs.bands().len()));
		}
		let capacity = (measured[0].0.max(measured[1].0) + 8, measured[0].1.max(measured[1].1) + 8);
		let mut glyphs = UiGlyphCurves::new(capacity.0, capacity.1);

		let geometry = build_ui_slug_geometry(&first, Extent::square(100), &mut fonts, &mut glyphs, &mut masks, &arena);
		assert_eq!((geometry.primitives.len(), geometry.dropped_glyphs), (7, 0));
		let generation = glyphs.generation();

		// The second label does not fit next to the first, so the first label's glyphs make room.
		let geometry = build_ui_slug_geometry(&second, Extent::square(100), &mut fonts, &mut glyphs, &mut masks, &arena);
		assert_eq!((geometry.primitives.len(), geometry.dropped_glyphs), (8, 0));
		assert!(glyphs.generation() > generation);
		assert_eq!((glyphs.curves().len(), glyphs.bands().len()), measured[1]);

		// One frame that needs more than the buffers hold draws what fits and reports the rest.
		let both = draw_list(vec![first.texts[0].clone(), second.texts[0].clone()]);
		let geometry = build_ui_slug_geometry(&both, Extent::square(100), &mut fonts, &mut glyphs, &mut masks, &arena);
		assert!(geometry.dropped_glyphs > 0);
		assert_eq!(geometry.primitives.len() + geometry.dropped_glyphs, 15);
		assert!(glyphs.curves().len() <= capacity.0 && glyphs.bands().len() <= capacity.1);
		assert_eq!(geometry.labels.len(), 2);
	}
}
