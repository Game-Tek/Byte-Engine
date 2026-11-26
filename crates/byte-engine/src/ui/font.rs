#![expect(
	clippy::disallowed_types,
	reason = "Not given one owner yet; see \"Give the remaining shared state one owner\" in todo.md"
)]
use std::{
	fs,
	path::{Path, PathBuf},
	sync::Arc,
};

use fontdue::{Font, FontSettings};
use utils::hash::HashMap;

use super::flow::Size;

const FALLBACK_WIDTH_FACTOR: f32 = 0.6;
const FALLBACK_ASCENT_FACTOR: f32 = 0.8;
const FALLBACK_LINE_HEIGHT_FACTOR: f32 = 1.2;
const FONT_SEARCH_DEPTH: usize = 3;

/// The `LoadedFont` struct retains font data for on-demand outlines and optional bitmap rendering.
struct LoadedFont {
	/// Only the bitmap path needs the rasterizer's eagerly compiled glyph geometry.
	rasterizer: Rasterizer,
	glyph_indices: HashMap<char, u16>,
	/// The font file, kept because outlines are read from it on a character's first use.
	// Shared, not owned: every `TextSystem` of the application, one in each UI engine and one in each render pass,
	// reads the same font files, which stay loaded for the application's life. Copying them would cost megabytes
	// per window.
	data: Arc<[u8]>,
}

/// The `Rasterizer` enum records whether the bitmap rasterizer was built from the font data yet.
enum Rasterizer {
	Unbuilt,
	Ready(Font),
	/// The font data parsed for outlines but not for the rasterizer.
	Unavailable,
}

enum FontState {
	Uninitialized,
	Ready(LoadedFont),
	Unavailable,
}

/// The `GlyphKey` struct identifies one rasterized glyph by font, character, and pixel size.
///
/// The size is keyed by its bit pattern so a cache lookup never depends on float rounding.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) struct GlyphKey {
	/// The font's number in its [`TextSystem`]; see [`TextSystem::face`].
	pub(crate) face: u16,
	pub(crate) character: char,
	pub(crate) font_size_bits: u32,
}

impl GlyphKey {
	/// Keys a glyph of the default font; pick another with [`Self::in_face`].
	pub(crate) fn new(character: char, font_size: f32) -> Self {
		Self {
			face: 0,
			character,
			font_size_bits: font_size.max(1.0).to_bits(),
		}
	}

	/// Moves this key to font `face`.
	pub(crate) fn in_face(mut self, face: usize) -> Self {
		self.face = face as u16;
		self
	}
}

/// The `Glyph` struct stores one rasterized glyph and the metrics needed to place it.
///
/// The bitmap is a row-major 8-bit coverage mask of `width * height` bytes.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Glyph {
	pub(crate) width: u32,
	pub(crate) height: u32,
	/// Horizontal offset from the pen position to the bitmap's left edge.
	pub(crate) xmin: i32,
	/// Vertical offset from the baseline to the bitmap's bottom edge.
	pub(crate) ymin: i32,
	pub(crate) advance_width: f32,
	pub(crate) bitmap: Vec<u8>,
}

impl Glyph {
	pub(crate) fn is_visible(&self) -> bool {
		self.width > 0 && self.height > 0 && !self.bitmap.is_empty()
	}
}

/// The `GlyphOutline` struct stores one character's outline as quadratic Bézier curves in em units.
///
/// An outline does not depend on the font size, so one entry serves every size a character is drawn
/// at. The UI render pass packs these curves for the GPU instead of rasterizing a bitmap per size.
/// Get one from [`TextSystem::outline`] or walk a whole text with [`TextSystem::place_outlines`].
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct GlyphOutline {
	/// Control points of every curve, with the y axis pointing up. A straight line repeats its end point.
	pub(crate) curves: Vec<[[f32; 2]; 3]>,
	/// Smallest and largest control point coordinates as `[min_x, min_y, max_x, max_y]`.
	pub(crate) bounds: [f32; 4],
	/// Horizontal pen advance.
	pub(crate) advance: f32,
}

impl GlyphOutline {
	pub(crate) fn is_visible(&self) -> bool {
		!self.curves.is_empty()
	}
}

/// The `OutlinePlacement` struct locates one visible glyph outline in target pixels.
#[derive(Debug, Clone, Copy)]
pub(crate) struct OutlinePlacement<'a> {
	/// The number of the font the glyph comes from; see [`TextSystem::face`].
	pub(crate) face: usize,
	/// The font's index of this glyph. Key per-glyph data with it and [`Self::face`].
	pub(crate) index: usize,
	/// Pen position on the baseline in target pixels.
	pub(crate) pen: [f32; 2],
	pub(crate) outline: &'a GlyphOutline,
}

/// The `LineMetrics` struct stores the vertical metrics for one pixel size.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct LineMetrics {
	pub(crate) line_height: f32,
	pub(crate) ascent: f32,
	pub(crate) descent: f32,
}

/// The `GlyphPlacement` struct locates one visible glyph bitmap in target pixels.
///
/// Look the bitmap up with [`TextSystem::glyph_by_key`] and its `key`.
#[derive(Debug, Clone, Copy)]
pub(crate) struct GlyphPlacement {
	pub(crate) key: GlyphKey,
	/// Left edge of the bitmap in target pixels.
	pub(crate) x: f32,
	/// Top edge of the bitmap in target pixels.
	pub(crate) y: f32,
}

// Bound cached whole strings independently of glyphs, which remain reusable across text changes.
const MEASURE_CACHE_ENTRIES: usize = 4096;
const MEASURE_CACHE_BYTES: usize = 512 * 1024;

/// The `Fonts` type names the fonts a UI draws text with, as pairs of a name and a font file.
///
/// Pass the same list to [`crate::ui::Engine::with_fonts`] and to
/// [`crate::application::graphics::setup_ui_render_pass`], so layout and drawing agree. The first font is the
/// default; text picks another by name with [`crate::ui::Properties::font`]. Name each font by its asset ID, such as
/// `"Inter-SemiBold.ttf"`, and read its file with [`load_font`]. An empty list draws every text with a system font.
// Shared, not owned: every `TextSystem` of the application reads the same font files; see `LoadedFont::data`.
pub type Fonts<'a> = &'a [(&'static str, Arc<[u8]>)];

/// The `Face` struct is one font of a [`TextSystem`], with the outlines and metrics read from it so far.
struct Face {
	/// The name text picks this font by; `None` for the system font used when the application gives none.
	name: Option<&'static str>,
	/// The application's font file until the font opens, which moves it into [`LoadedFont::data`], or `None` to find
	/// a system font.
	data: Option<Arc<[u8]>>,
	state: FontState,
	/// Outlines by the font's glyph index, read on first use.
	outlines: Vec<Option<GlyphOutline>>,
	/// Vertical metrics at one pixel per em; every size is a multiple of these.
	em_line_metrics: Option<LineMetrics>,
}

impl Face {
	fn new(name: Option<&'static str>, data: Option<Arc<[u8]>>) -> Self {
		Self {
			name,
			data,
			state: FontState::Uninitialized,
			outlines: Vec::new(),
			em_line_metrics: None,
		}
	}
}

/// The `TextSystem` struct shapes UI text and supplies its glyphs as outlines or bitmaps.
///
/// It draws with one or more fonts, numbered in the order the application gave them; font `0` is the default. A
/// text picks a font by name, which [`Self::face`] turns into that number.
///
/// Measurement and outline placement use the same size-independent advances, so layout and
/// drawing never disagree and neither rasterizes anything. Glyph bitmaps exist only for the
/// atlas renderer, which rasterizes them once per font, character, and pixel size.
pub(crate) struct TextSystem {
	/// Never empty: without application fonts, it holds one face that finds a system font.
	faces: Vec<Face>,
	/// Font names text asked for that no face has, so each is reported once.
	reported_names: Vec<&'static str>,
	/// Measurements by font number and font size, packed into one key.
	/// Measurements by face and font size, then wrap width, then text.
	measure_cache: HashMap<(u64, u32), HashMap<String, Size>>,
	previous_measure_cache: HashMap<(u64, u32), HashMap<String, Size>>,
	measure_cache_entries: usize,
	measure_cache_bytes: usize,
	/// Emptied keys of the generation that rotated out, which new measurements reuse instead of allocating.
	spare_measure_keys: Vec<String>,
	/// Emptied size groups of the generation that rotated out, which new font sizes reuse instead of allocating.
	spare_measure_groups: Vec<HashMap<String, Size>>,
	glyph_cache: HashMap<GlyphKey, Glyph>,
	reported_unavailable: bool,
}

impl TextSystem {
	pub fn new() -> Self {
		Self::with_fonts(&[])
	}

	/// Creates a text system that draws with `fonts`, the first being the default. A font that doesn't parse falls
	/// back to a system font, and so does an empty list.
	pub fn with_fonts(fonts: Fonts<'_>) -> Self {
		let mut faces: Vec<Face> = fonts
			.iter()
			.map(|(name, data)| Face::new(Some(*name), Some(data.clone())))
			.collect();
		if faces.is_empty() {
			faces.push(Face::new(None, None));
		}
		Self {
			faces,
			reported_names: Vec::new(),
			measure_cache: HashMap::default(),
			previous_measure_cache: HashMap::default(),
			measure_cache_entries: 0,
			measure_cache_bytes: 0,
			spare_measure_keys: Vec::new(),
			spare_measure_groups: Vec::new(),
			glyph_cache: HashMap::default(),
			reported_unavailable: false,
		}
	}

	/// Returns the number of the font named `name`, or of the default font for `None` or a name no font has.
	pub fn face(&mut self, name: Option<&'static str>) -> usize {
		let Some(name) = name else {
			return 0;
		};
		if let Some(face) = self.faces.iter().position(|face| face.name == Some(name)) {
			return face;
		}
		if !self.reported_names.contains(&name) {
			self.reported_names.push(name);
			log::warn!(
				"UI font '{name}' isn't loaded, so its text uses the default font. The most likely cause is a font missing from the list given to `Engine::with_fonts` and `setup_ui_render_pass`. Add `(\"{name}\", load_font(..., \"{name}\"))` to that list."
			);
		}
		0
	}

	/// Returns whether a font is available for text measurement and drawing.
	pub fn has_font(&mut self) -> bool {
		self.font(0).is_some()
	}

	/// Measures text in the font named `font` with a bounded string cache and reusable glyph metrics.
	///
	/// `wrap` is the widest a line may be, in multiples of the font size, or `None` to break lines only at newlines;
	/// see [`Lines`].
	pub fn measure(&mut self, text: &str, font: Option<&'static str>, font_size: f32, wrap: Option<f32>) -> Size {
		if text.is_empty() {
			return Size::new(0.0, 0.0);
		}

		let face = self.face(font);
		let font_size = font_size.max(1.0);
		let key = (
			((face as u64) << 32) | font_size.to_bits() as u64,
			wrap.map_or(u32::MAX, f32::to_bits),
		);
		for cache in [&self.measure_cache, &self.previous_measure_cache] {
			if let Some(size) = cache.get(&key).and_then(|sizes| sizes.get(text)) {
				return *size;
			}
		}

		let size = if self.font(face).is_some() {
			self.measure_with_glyphs(text, face, font_size, wrap)
		} else {
			measure_with_fallback(text, font_size, wrap)
		};
		// Rotate whole-string measurements in two bounded generations. Recent labels remain
		// available through a rotation, while changing counters cannot retain every old value.
		if text.len() <= MEASURE_CACHE_BYTES {
			if self.measure_cache_entries == MEASURE_CACHE_ENTRIES
				|| self.measure_cache_bytes + text.len() > MEASURE_CACHE_BYTES
			{
				std::mem::swap(&mut self.measure_cache, &mut self.previous_measure_cache);
				// The rotated-out generation leaves its key storage to the measurements that replace it.
				// The size groups leave the map too, keeping their buckets for the sizes measured next. Leaving
				// them in place would keep one empty group for every size ever used, such as an animated font size.
				for (_, mut sizes) in self.measure_cache.drain() {
					self.spare_measure_keys.extend(sizes.drain().map(|(mut key, _)| {
						key.clear();
						key
					}));
					self.spare_measure_groups.push(sizes);
				}
				self.measure_cache_entries = 0;
				self.measure_cache_bytes = 0;
			}
			let mut text_key = self.spare_measure_keys.pop().unwrap_or_default();
			text_key.push_str(text);
			let spare_groups = &mut self.spare_measure_groups;
			self.measure_cache
				.entry(key)
				.or_insert_with(|| spare_groups.pop().unwrap_or_default())
				.insert(text_key, size);
			self.measure_cache_entries += 1;
			self.measure_cache_bytes += text.len();
			// Every size group holds at least one measurement, so groups are bounded by entries.
			debug_assert!(
				self.measure_cache_entries <= MEASURE_CACHE_ENTRIES
					&& self.measure_cache_bytes <= MEASURE_CACHE_BYTES
					&& self.measure_cache.len() <= self.measure_cache_entries
			);
		}
		size
	}

	/// Returns the vertical metrics of font `face` for one pixel size, or `None` without a font.
	pub fn line_metrics(&mut self, face: usize, font_size: f32) -> Option<LineMetrics> {
		let font_size = font_size.max(1.0);
		if self.faces[face].em_line_metrics.is_none() {
			let metrics = font_line_metrics(self.font(face)?, 1.0);
			self.faces[face].em_line_metrics = Some(metrics);
		}
		// Metrics scale linearly, so a continuously animated size needs no cache entry of its own.
		let em = self.faces[face].em_line_metrics?;
		Some(LineMetrics {
			line_height: em.line_height * font_size,
			ascent: em.ascent * font_size,
			descent: em.descent * font_size,
		})
	}

	/// Returns one character's glyph index in font `face` and its outline, reading the outline from the font on
	/// first use.
	///
	/// Returns `None` without a font. Characters the font draws alike share a glyph index, and a
	/// character without contours, such as a space, has an outline that only advances the pen.
	pub fn outline(&mut self, face: usize, character: char) -> Option<(usize, &GlyphOutline)> {
		self.font(face)?;
		let Face { state, outlines, .. } = &mut self.faces[face];
		let FontState::Ready(font) = state else {
			return None;
		};
		let index = *font.glyph_indices.entry(character).or_insert_with(|| {
			let face = ttf_parser::Face::parse(&font.data, 0)
				.expect("Loaded font is invalid. The most likely cause is that validated font data changed after loading.");
			// Match fontdue's last nonzero cmap mapping, including fonts with several character maps.
			face.tables()
				.cmap
				.and_then(|table| {
					table
						.subtables
						.into_iter()
						.filter_map(|subtable| subtable.glyph_index(character as u32))
						.filter(|glyph| glyph.0 != 0)
						.last()
				})
				.map_or(0, |glyph| glyph.0)
		});
		if outlines.len() <= index as usize {
			outlines.resize(index as usize + 1, None);
		}
		let outline = outlines[index as usize].get_or_insert_with(|| read_outline(font, index));
		Some((index as usize, outline))
	}

	/// Visits every visible glyph outline of `text`, in the font named `font`, with its pen position relative to the
	/// text's top left corner.
	///
	/// Lines advance by the font's line height and advances stay fractional, exactly as
	/// [`Self::measure`] sums them. Returns `false` when no font is available.
	pub fn place_outlines(
		&mut self,
		text: &str,
		font: Option<&'static str>,
		font_size: f32,
		wrap: Option<f32>,
		mut visit: impl FnMut(OutlinePlacement<'_>),
	) -> bool {
		if text.is_empty() {
			return false;
		}
		let face = self.face(font);
		let font_size = font_size.max(1.0);
		let Some(line) = self.line_metrics(face, font_size) else {
			return false;
		};

		let mut pen = [0.0, line.ascent.max(font_size * FALLBACK_ASCENT_FACTOR)];
		let mut lines = Lines::new(text);
		while let Some(text_line) = lines.next(wrap, &mut |character| self.em_advance(face, character)) {
			for character in text_line.chars() {
				let Some((index, outline)) = self.outline(face, character) else {
					return false;
				};
				if outline.is_visible() {
					visit(OutlinePlacement {
						face,
						index,
						pen,
						outline,
					});
				}
				pen[0] += outline.advance * font_size;
			}
			pen = [0.0, pen[1] + line.line_height];
		}

		true
	}

	/// Returns the cached glyph for one key, rasterizing it on first use.
	pub fn glyph_by_key(&mut self, key: GlyphKey) -> Option<&Glyph> {
		let face = key.face as usize;
		self.font(face)?;
		let FontState::Ready(font) = &mut self.faces[face].state else {
			return None;
		};
		// Bitmap consumers build the rasterizer once; layout and GPU outlines never need it.
		if matches!(font.rasterizer, Rasterizer::Unbuilt) {
			font.rasterizer = match Font::from_bytes(&*font.data, FontSettings::default()) {
				Ok(rasterizer) => Rasterizer::Ready(rasterizer),
				Err(_) => Rasterizer::Unavailable,
			};
		}
		let Rasterizer::Ready(rasterizer) = &font.rasterizer else {
			return None;
		};
		// Borrow the loaded font separately so a cache hit needs only one lookup.
		Some(self.glyph_cache.entry(key).or_insert_with(|| {
			let (metrics, bitmap) = rasterizer.rasterize(key.character, f32::from_bits(key.font_size_bits));
			Glyph {
				width: metrics.width as u32,
				height: metrics.height as u32,
				xmin: metrics.xmin,
				ymin: metrics.ymin,
				advance_width: metrics.advance_width,
				bitmap,
			}
		}))
	}

	/// Places every visible glyph of `text`, in the font named `font`, in target pixels, starting at `origin`.
	///
	/// Lines advance by the font's line height. Fractional advances are preserved
	/// so rendering can scale and translate the run without pixel snapping. Returns
	/// `false` when no font is available.
	pub fn place_glyphs(
		&mut self,
		text: &str,
		font: Option<&'static str>,
		font_size: f32,
		wrap: Option<f32>,
		origin: (i32, i32),
		mut visit: impl FnMut(GlyphPlacement),
	) -> bool {
		if text.is_empty() {
			return false;
		}
		let face = self.face(font);
		let font_size = font_size.max(1.0);
		let Some(line) = self.line_metrics(face, font_size) else {
			return false;
		};

		let mut baseline_y = origin.1 as f32 + line.ascent.max(font_size * FALLBACK_ASCENT_FACTOR);
		let mut pen_x = origin.0 as f32;

		let mut lines = Lines::new(text);
		while let Some(text_line) = lines.next(wrap, &mut |character| self.em_advance(face, character)) {
			for character in text_line.chars() {
				let key = GlyphKey::new(character, font_size).in_face(face);
				let Some(glyph) = self.glyph_by_key(key) else {
					return false;
				};
				if glyph.is_visible() {
					visit(GlyphPlacement {
						key,
						x: pen_x + glyph.xmin as f32,
						y: baseline_y - glyph.height as f32 - glyph.ymin as f32,
					});
				}
				pen_x += glyph.advance_width;
			}
			pen_x = origin.0 as f32;
			baseline_y += line.line_height;
		}

		true
	}

	/// The advance of `character` in font `face`, in multiples of the font size, which [`Lines`] breaks against.
	fn em_advance(&mut self, face: usize, character: char) -> f32 {
		self.outline(face, character).map_or(0.0, |(_, outline)| outline.advance)
	}

	fn measure_with_glyphs(&mut self, text: &str, face: usize, font_size: f32, wrap: Option<f32>) -> Size {
		let Some(line) = self.line_metrics(face, font_size) else {
			return measure_with_fallback(text, font_size, wrap);
		};
		let mut max_width: f32 = 0.0;
		let mut line_count = 0u32;
		let mut lines = Lines::new(text);
		while let Some(text_line) = lines.next(wrap, &mut |character| self.em_advance(face, character)) {
			// Advances come from the outline, so measuring never rasterizes a glyph.
			let width = text_line
				.chars()
				.map(|character| self.em_advance(face, character) * font_size)
				.sum::<f32>();
			max_width = max_width.max(width);
			line_count += 1;
		}

		let line_box_height = (line.ascent - line.descent).max(font_size);
		let height = line_box_height + (line_count.saturating_sub(1) as f32 * line.line_height);

		Size::new(max_width.max(0.0), height.max(0.0))
	}

	/// Loads font `face`'s tables on first use without preparing every glyph for CPU rasterization.
	fn font(&mut self, face: usize) -> Option<&LoadedFont> {
		let entry = &mut self.faces[face];
		if matches!(entry.state, FontState::Uninitialized) {
			entry.state = match open_font(entry.data.take()) {
				Ok(font) => FontState::Ready(font),
				Err(error) => {
					if !self.reported_unavailable {
						log::warn!("{error}");
						self.reported_unavailable = true;
					}

					FontState::Unavailable
				}
			};
		}

		match &self.faces[face].state {
			FontState::Ready(font) => Some(font),
			_ => None,
		}
	}
}

fn measure_with_fallback(text: &str, font_size: f32, wrap: Option<f32>) -> Size {
	let (mut max_width, mut line_count) = (0.0f32, 0u32);
	let mut lines = Lines::new(text);
	while let Some(line) = lines.next(wrap, &mut |_| FALLBACK_WIDTH_FACTOR) {
		max_width = max_width.max(line.chars().count() as f32 * font_size * FALLBACK_WIDTH_FACTOR);
		line_count += 1;
	}
	let height = line_count.max(1) as f32 * font_size * FALLBACK_LINE_HEIGHT_FACTOR;

	Size::new(max_width.max(0.0), height.max(0.0))
}

/// The `Lines` struct breaks a text into the lines it's measured and drawn in, so measuring and both drawing paths
/// break it at the same places.
///
/// A newline always breaks. With a wrap width, lines also break greedily at spaces: each line takes as many whole
/// words as fit, a word wider than the width takes a line of its own, and the space a line breaks at is dropped. Widths
/// are in multiples of the font size, so the breaks don't depend on the size a text is drawn at. See
/// [Wrapping](/docs/reference/gui#wrapping).
pub(crate) struct Lines<'t> {
	/// What's left to break; `None` once the last line was returned.
	rest: Option<&'t str>,
}

impl<'t> Lines<'t> {
	pub(crate) fn new(text: &'t str) -> Self {
		Self { rest: Some(text) }
	}

	/// Returns the next line, breaking it to `wrap` with `advance`, each character's advance in multiples of the font
	/// size, or `None` after the last line.
	pub(crate) fn next(&mut self, wrap: Option<f32>, advance: &mut impl FnMut(char) -> f32) -> Option<&'t str> {
		let rest = self.rest?;
		let wrap = wrap.unwrap_or(f32::INFINITY);
		let mut width = 0.0;
		// The last space a break could drop, by byte index.
		let mut space = None;
		for (index, character) in rest.char_indices() {
			if character == '\n' {
				self.rest = Some(&rest[index + 1..]);
				return Some(&rest[..index]);
			}
			let step = advance(character);
			if character == ' ' {
				space = Some(index);
			} else if width + step > wrap
				&& let Some(space) = space
			{
				self.rest = Some(&rest[space + 1..]);
				return Some(&rest[..space]);
			}
			width += step;
		}
		self.rest = None;
		Some(rest)
	}
}

/// Reads the same horizontal line metrics used by the bitmap rasterizer.
fn font_line_metrics(font: &LoadedFont, font_size: f32) -> LineMetrics {
	let face = ttf_parser::Face::parse(&font.data, 0)
		.expect("Loaded font is invalid. The most likely cause is that validated font data changed after loading.");
	let scale = font_size / face.units_per_em() as f32;
	let ascent = face.ascender() as i32;
	let descent = face.descender() as i32;
	LineMetrics {
		line_height: (ascent - descent + face.line_gap() as i32) as f32 * scale,
		ascent: ascent as f32 * scale,
		descent: descent as f32 * scale,
	}
}

/// Largest distance between a cubic segment and the quadratic curves that replace it: a fraction of an em for glyphs,
/// or of a path's extent for filled paths.
pub(crate) const CUBIC_TOLERANCE: f32 = 1.0 / 4096.0;

/// The `QuadraticCollector` struct turns outline segments into the closed quadratic contours that Slug coverage reads.
///
/// Glyph outlines drive it through [`ttf_parser::OutlineBuilder`] in font units. The UI render pass drives it with
/// path segments through [`Self::start_contour`], [`Self::line`], [`Self::quad`], and [`Self::cubic`]. Read the
/// result with [`Self::finish`].
pub(crate) struct QuadraticCollector {
	/// Every contour's curves, one after another. A straight line repeats its end point.
	curves: Vec<[[f32; 2]; 3]>,
	/// Output units per input unit of the [`ttf_parser::OutlineBuilder`] calls.
	scale: f32,
	/// Largest distance between a cubic and its quadratic replacements, in output units.
	tolerance: f32,
	start: [f32; 2],
	/// The open contour's current point, or `None` between contours.
	last: Option<[f32; 2]>,
}

impl QuadraticCollector {
	/// Creates an empty collector. `scale` applies only to font outline input; `tolerance` is in output units.
	pub(crate) fn new(scale: f32, tolerance: f32) -> Self {
		Self {
			curves: Vec::new(),
			scale,
			tolerance,
			start: [0.0; 2],
			last: None,
		}
	}

	/// Returns the open contour's current point, or `None` between contours.
	pub(crate) fn last(&self) -> Option<[f32; 2]> {
		self.last
	}

	/// Returns how many curves the collector holds. After [`Self::close_contour`] this is where the closed contour
	/// ends, which the even-odd path fill records to reorient contours.
	pub(crate) fn curve_count(&self) -> usize {
		self.curves.len()
	}

	/// Closes the open contour and starts a new one at `point`.
	pub(crate) fn start_contour(&mut self, point: [f32; 2]) {
		self.close_contour();
		self.start = point;
		self.last = Some(point);
	}

	/// A line repeats its end point as the control point, which keeps the curve's polynomial quadratic.
	pub(crate) fn line(&mut self, end: [f32; 2]) {
		let Some(last) = self.last else { return };
		if end != last {
			self.curves.push([last, end, end]);
			self.last = Some(end);
		}
	}

	pub(crate) fn quad(&mut self, control: [f32; 2], end: [f32; 2]) {
		let Some(last) = self.last else { return };
		if control != last || end != last {
			self.curves.push([last, control, end]);
			self.last = Some(end);
		}
	}

	/// Splits a cubic into enough quadratics to stay within the tolerance.
	pub(crate) fn cubic(&mut self, p1: [f32; 2], p2: [f32; 2], p3: [f32; 2]) {
		let Some(p0) = self.last else { return };
		// The cubic and its closest quadratic differ by the third difference times t (t - 1/2) (t - 1),
		// which peaks at sqrt(3) / 36. Splitting into n pieces divides the third difference by n cubed.
		let third_difference = (0..2)
			.map(|axis| (p3[axis] - 3.0 * p2[axis] + 3.0 * p1[axis] - p0[axis]).powi(2))
			.sum::<f32>()
			.sqrt();
		let pieces = (third_difference * 3f32.sqrt() / 36.0 / self.tolerance)
			.cbrt()
			.ceil()
			.clamp(1.0, 16.0);
		let point = |t: f32| {
			let s = 1.0 - t;
			[0, 1].map(|axis| {
				s * s * s * p0[axis] + 3.0 * s * s * t * p1[axis] + 3.0 * s * t * t * p2[axis] + t * t * t * p3[axis]
			})
		};
		let derivative = |t: f32| {
			let s = 1.0 - t;
			[0, 1].map(|axis| {
				3.0 * s * s * (p1[axis] - p0[axis]) + 6.0 * s * t * (p2[axis] - p1[axis]) + 3.0 * t * t * (p3[axis] - p2[axis])
			})
		};
		let mut last = p0;
		for piece in 0..pieces as usize {
			let (t0, t1) = (piece as f32 / pieces, (piece + 1) as f32 / pieces);
			// The last piece ends on the segment's own end point so that contours stay closed.
			let end = if t1 >= 1.0 { p3 } else { point(t1) };
			let (from, to) = (derivative(t0), derivative(t1));
			// The piece's cubic controls are start + from * h and end - to * h with h = (t1 - t0) / 3.
			// The closest quadratic control is (3 * (c1 + c2) - (start + end)) / 4.
			let h = (t1 - t0) / 3.0;
			let control = [0, 1]
				.map(|axis| (3.0 * (last[axis] + from[axis] * h + end[axis] - to[axis] * h) - (last[axis] + end[axis])) / 4.0);
			self.curves.push([last, control, end]);
			last = end;
		}
		self.last = Some(last);
	}

	/// Ends the open contour with a line back to its start, as a fill closes every subpath.
	pub(crate) fn close_contour(&mut self) {
		if self.last.is_some() {
			self.line(self.start);
		}
		self.last = None;
	}

	/// Closes the open contour and returns every curve, contour after contour.
	pub(crate) fn finish(mut self) -> Vec<[[f32; 2]; 3]> {
		self.close_contour();
		self.curves
	}
}

impl ttf_parser::OutlineBuilder for QuadraticCollector {
	fn move_to(&mut self, x: f32, y: f32) {
		self.start_contour([x * self.scale, y * self.scale]);
	}

	fn line_to(&mut self, x: f32, y: f32) {
		self.line([x * self.scale, y * self.scale]);
	}

	fn quad_to(&mut self, x1: f32, y1: f32, x: f32, y: f32) {
		self.quad([x1 * self.scale, y1 * self.scale], [x * self.scale, y * self.scale]);
	}

	fn curve_to(&mut self, x1: f32, y1: f32, x2: f32, y2: f32, x: f32, y: f32) {
		self.cubic(
			[x1 * self.scale, y1 * self.scale],
			[x2 * self.scale, y2 * self.scale],
			[x * self.scale, y * self.scale],
		);
	}

	fn close(&mut self) {
		self.close_contour();
	}
}

/// Returns the smallest and largest control point coordinates as `[min_x, min_y, max_x, max_y]`, or zeros without curves.
pub(crate) fn curve_bounds(curves: &[[[f32; 2]; 3]]) -> [f32; 4] {
	let Some(first) = curves.first() else {
		return [0.0; 4];
	};
	let mut bounds = [first[0][0], first[0][1], first[0][0], first[0][1]];
	for point in curves.iter().flatten() {
		bounds = [
			bounds[0].min(point[0]),
			bounds[1].min(point[1]),
			bounds[2].max(point[0]),
			bounds[3].max(point[1]),
		];
	}
	bounds
}

/// Reads one glyph's outline and advance from the font file in em units.
fn read_outline(font: &LoadedFont, glyph: u16) -> GlyphOutline {
	let mut outline = GlyphOutline {
		curves: Vec::new(),
		bounds: [0.0; 4],
		advance: 0.0,
	};
	let Ok(face) = ttf_parser::Face::parse(&font.data, 0) else {
		return outline;
	};
	let scale = 1.0 / face.units_per_em() as f32;
	let glyph = ttf_parser::GlyphId(glyph);
	outline.advance = face.glyph_hor_advance(glyph).unwrap_or(0) as f32 * scale;

	let mut collector = QuadraticCollector::new(scale, CUBIC_TOLERANCE);
	face.outline_glyph(glyph, &mut collector);
	outline.curves = collector.finish();
	outline.bounds = curve_bounds(&outline.curves);
	outline
}

/// Uses the application's font when it parses, else finds a readable system font, and validates its tables before
/// retaining its bytes.
fn open_font(preferred: Option<Arc<[u8]>>) -> Result<LoadedFont, String> {
	if let Some(data) = preferred {
		if ttf_parser::Face::parse(&data, 0).is_ok() {
			return Ok(LoadedFont {
				rasterizer: Rasterizer::Unbuilt,
				glyph_indices: HashMap::default(),
				data,
			});
		}
		log::warn!(
			"The application's UI font could not be parsed, so text uses a system font. The most likely cause is font data that isn't a TrueType or OpenType file. Pass the payload of a `Font` resource, as `load_font` returns it."
		);
	}
	for path in explicit_font_candidates().into_iter().chain(
		font_search_roots()
			.into_iter()
			.flat_map(|path| collect_font_files(&path, FONT_SEARCH_DEPTH)),
	) {
		if !path.is_file() {
			continue;
		}

		let Ok(bytes) = fs::read(&path) else {
			continue;
		};

		let Ok(_) = ttf_parser::Face::parse(&bytes, 0) else {
			continue;
		};

		log::debug!("Loaded the UI system font from '{}'.", path.display());
		return Ok(LoadedFont {
			rasterizer: Rasterizer::Unbuilt,
			glyph_indices: HashMap::default(),
			data: bytes.into(),
		});
	}

	Err(
		"Failed to load a system UI font. The most likely cause is that no readable TrueType or OpenType font was found in the supported OS font directories."
			.into(),
	)
}

/// Reads the payload of the `Font` resource `id`, the font file the UI draws text with.
///
/// Call it once at startup for each font, then name the results in the [`Fonts`] list you pass to
/// [`crate::ui::Engine::with_fonts`] and to [`crate::application::graphics::setup_ui_render_pass`], so layout and
/// drawing use the same fonts. Cloning the
/// result shares the bytes instead of copying them.
pub async fn load_font(resource_manager: &resource_management::ResourceManager, id: &str) -> Result<Arc<[u8]>, String> {
	use resource_management::resource::ReadTargetsMut;

	let mut reference: resource_management::Reference<resource_management::resources::font::Font> =
		resource_manager.request(id).await.map_err(|error| {
			format!(
				"Font '{id}' could not be loaded. The most likely cause is a missing or unbaked `.ttf` or `.otf` asset. Check the font's asset ID: {error}"
			)
		})?;
	let read_target = ReadTargetsMut::Box {
		buffer: vec![0_u8; reference.size].into_boxed_slice(),
		offset: 0,
		size: None,
	};
	let read = reference.load(read_target).await.map_err(|_| {
		format!(
			"Font '{id}' could not be read. The most likely cause is a missing or unreadable payload in the resource store. Rebake the font."
		)
	})?;

	read.buffer().map(Arc::from).ok_or_else(|| {
		format!(
			"Font '{id}' was stored as streams. The most likely cause is a resource store baked by a different engine version. Rebake the font."
		)
	})
}

fn collect_font_files(path: &Path, depth: usize) -> Vec<PathBuf> {
	let mut fonts = Vec::new();

	if depth == 0 {
		return fonts;
	}

	let Ok(entries) = fs::read_dir(path) else {
		return fonts;
	};

	for entry in entries.flatten() {
		let path = entry.path();

		if path.is_dir() {
			fonts.extend(collect_font_files(&path, depth - 1));
			continue;
		}

		let Some(extension) = path.extension().and_then(|extension| extension.to_str()) else {
			continue;
		};

		if matches!(extension, "ttf" | "otf" | "TTF" | "OTF") {
			fonts.push(path);
		}
	}

	fonts
}

fn font_search_roots() -> Vec<PathBuf> {
	let mut roots = Vec::new();

	if let Some(home) = std::env::var_os("HOME") {
		let home = PathBuf::from(home);
		roots.push(home.join("Library/Fonts"));
		roots.push(home.join(".fonts"));
		roots.push(home.join(".local/share/fonts"));
	}

	#[cfg(target_os = "macos")]
	{
		roots.push(PathBuf::from("/System/Library/Fonts"));
		roots.push(PathBuf::from("/System/Library/Fonts/Supplemental"));
		roots.push(PathBuf::from("/Library/Fonts"));
	}

	#[cfg(target_os = "linux")]
	{
		roots.push(PathBuf::from("/usr/share/fonts"));
		roots.push(PathBuf::from("/usr/local/share/fonts"));
	}

	#[cfg(target_os = "windows")]
	{
		if let Some(windir) = std::env::var_os("WINDIR") {
			roots.push(PathBuf::from(windir).join("Fonts"));
		}
	}

	roots
}

fn explicit_font_candidates() -> Vec<PathBuf> {
	let mut candidates = Vec::new();

	#[cfg(target_os = "macos")]
	{
		candidates.extend(
			[
				"/System/Library/Fonts/SFNS.ttf",
				"/System/Library/Fonts/SFNSMono.ttf",
				"/System/Library/Fonts/NewYork.ttf",
				"/System/Library/Fonts/Geneva.ttf",
				"/System/Library/Fonts/Supplemental/Arial.ttf",
				"/System/Library/Fonts/Supplemental/Arial Unicode.ttf",
				"/Library/Fonts/Arial.ttf",
			]
			.into_iter()
			.map(PathBuf::from),
		);
	}

	#[cfg(target_os = "linux")]
	{
		candidates.extend(
			[
				"/usr/share/fonts/truetype/dejavu/DejaVuSans.ttf",
				"/usr/share/fonts/truetype/liberation2/LiberationSans-Regular.ttf",
				"/usr/share/fonts/opentype/noto/NotoSans-Regular.otf",
			]
			.into_iter()
			.map(PathBuf::from),
		);
	}

	#[cfg(target_os = "windows")]
	{
		if let Some(windir) = std::env::var_os("WINDIR") {
			let fonts = PathBuf::from(windir).join("Fonts");
			candidates.extend([fonts.join("segoeui.ttf"), fonts.join("arial.ttf"), fonts.join("calibri.ttf")]);
		}
	}

	candidates
}

#[cfg(test)]
mod tests {
	use super::{GlyphKey, TextSystem};

	#[test]
	fn outline_and_bitmap_text_keep_the_same_metrics() {
		let mut text = TextSystem::new();
		if !text.has_font() {
			return;
		}
		for character in "AV café ΩЖ中🙂\u{10ffff}".chars() {
			let advance = text.outline(0, character).unwrap().1.advance;
			for size in [12.0, 19.5, 32.0] {
				let bitmap = text.glyph_by_key(GlyphKey::new(character, size)).unwrap();
				assert!((advance * size - bitmap.advance_width).abs() < 0.0001);
			}
		}
		let mut outlines = Vec::new();
		text.place_outlines("A B\nC", None, 19.5, None, |glyph| outlines.push(glyph.pen));
		let mut placements = Vec::new();
		text.place_glyphs("A B\nC", None, 19.5, None, (0, 0), |glyph| placements.push(glyph));
		let bitmaps = placements.into_iter().map(|placement| {
			let bitmap = text.glyph_by_key(placement.key).unwrap();
			[
				placement.x - bitmap.xmin as f32,
				placement.y + bitmap.height as f32 + bitmap.ymin as f32,
			]
		});
		let bitmaps = bitmaps.collect::<Vec<_>>();
		assert_eq!(outlines.len(), bitmaps.len());
		for (outline, bitmap) in outlines.into_iter().zip(bitmaps) {
			assert!((outline[0] - bitmap[0]).abs() < 0.0001);
			assert!((outline[1] - bitmap[1]).abs() < 0.0001);
		}
	}

	#[test]
	fn text_measures_with_the_font_it_names_and_falls_back_to_the_first() {
		// Two different readable fonts from the machine; without them there is nothing to tell apart.
		let mut files = super::explicit_font_candidates()
			.into_iter()
			.filter_map(|path| std::fs::read(path).ok())
			.filter(|bytes| ttf_parser::Face::parse(bytes, 0).is_ok());
		let (Some(first), Some(second)) = (files.next(), files.next()) else {
			return;
		};
		let fonts = [("first", first.into()), ("second", second.into())];
		let mut text = TextSystem::with_fonts(&fonts);
		let mut only_first = TextSystem::with_fonts(&fonts[..1]);
		let mut only_second = TextSystem::with_fonts(&fonts[1..]);
		let sample = "Wide glyphs, narrow ones";

		assert_eq!(
			text.measure(sample, None, 16.0, None),
			only_first.measure(sample, None, 16.0, None)
		);
		assert_eq!(
			text.measure(sample, Some("second"), 16.0, None),
			only_second.measure(sample, None, 16.0, None)
		);
		assert_eq!(
			text.measure(sample, Some("missing"), 16.0, None),
			only_first.measure(sample, None, 16.0, None)
		);
	}

	#[test]
	fn measurements_stay_consistent_after_many_text_changes() {
		let mut text_system = TextSystem::new();
		let labels = [("Score: 123", 16.0), ("First line\nSecond line", 24.0), ("áβ中", 12.0)];
		let expected = labels.map(|(text, size)| text_system.measure(text, None, size, None));
		// Changing text, and a size no other measurement uses, as an animated font size would.
		for value in 0..3 * super::MEASURE_CACHE_ENTRIES {
			text_system.measure(&format!("Changing score: {value}"), None, 16.0, None);
			text_system.measure("Score: 123", None, 16.0 + value as f32 / 1024.0, None);
		}
		for ((text, size), expected) in labels.into_iter().zip(expected) {
			assert_eq!(text_system.measure(text, None, size, None), expected);
		}
	}

	#[test]
	fn placement_advances_pen_and_wraps_lines_from_cached_metrics() {
		let mut text_system = TextSystem::new();
		if !text_system.has_font() {
			return;
		}

		let mut placements = Vec::new();
		let placed = text_system.place_glyphs("AB\nC", None, 20.0, None, (10, 5), |placement| placements.push(placement));
		assert!(placed);
		let placements = placements
			.into_iter()
			.map(|placement| {
				let glyph = text_system.glyph_by_key(placement.key).unwrap();
				(placement.x, placement.y, glyph.xmin, glyph.advance_width)
			})
			.collect::<Vec<_>>();
		assert_eq!(placements.len(), 3);

		let [a, b, c] = placements[..] else { unreachable!() };
		assert_eq!(a.0, 10.0 + a.2 as f32);
		assert_eq!(b.0, 10.0 + a.3 + b.2 as f32);
		assert_eq!(c.0, 10.0 + c.2 as f32);
		assert!(c.1 > a.1, "second line must sit below the first");
		let line = text_system.line_metrics(0, 20.0).unwrap();
		assert!((c.1 - a.1 - line.line_height).abs() <= 2.0);

		// Measurement uses the same advances as placement.
		let measured = text_system.measure("AB", None, 20.0, None);
		assert!((measured.x() - (a.3 + b.3)).abs() < 0.001);
	}

	#[test]
	fn outlines_are_closed_contours_inside_their_bounds() {
		let mut text_system = TextSystem::new();
		if !text_system.has_font() {
			return;
		}

		let (index, outline) = text_system
			.outline(0, 'B')
			.map(|(index, outline)| (index, outline.clone()))
			.unwrap();
		assert!(outline.is_visible());
		// Every contour returns to where it started; an open contour would leak coverage along a ray.
		let mut start = outline.curves[0][0];
		for pair in outline.curves.windows(2) {
			if pair[0][2] != pair[1][0] {
				assert_eq!(pair[0][2], start);
				start = pair[1][0];
			}
		}
		assert_eq!(outline.curves.last().unwrap()[2], start);
		for point in outline.curves.iter().flatten() {
			assert!(point[0] >= outline.bounds[0] && point[0] <= outline.bounds[2]);
			assert!(point[1] >= outline.bounds[1] && point[1] <= outline.bounds[3]);
		}
		// An upright capital stands on the baseline and is about as tall as most of an em.
		assert!(outline.bounds[1].abs() < 0.05 && outline.bounds[3] > 0.5 && outline.bounds[3] < 1.0);

		assert_eq!(text_system.outline(0, 'B').unwrap().0, index);
		let (_, space) = text_system.outline(0, ' ').unwrap();
		assert!(!space.is_visible());
		assert!(space.advance > 0.0);
	}

	#[test]
	fn cubic_segments_become_quadratic_curves_within_tolerance() {
		use ttf_parser::OutlineBuilder as _;

		// A quarter circle of 800 font units in a 1000 unit em, the usual cubic approximation.
		let cubic = [[800.0f32, 0.0], [800.0, 441.6], [441.6, 800.0], [0.0, 800.0]];
		let mut collector = super::QuadraticCollector::new(1.0 / 1000.0, super::CUBIC_TOLERANCE);
		collector.move_to(cubic[0][0], cubic[0][1]);
		collector.curve_to(cubic[1][0], cubic[1][1], cubic[2][0], cubic[2][1], cubic[3][0], cubic[3][1]);

		assert!(
			collector.curves.len() > 1,
			"one quadratic cannot follow a quarter circle closely"
		);
		assert_eq!(collector.curves[0][0], [0.8, 0.0]);
		assert_eq!(collector.curves.last().unwrap()[2], [0.0, 0.8]);
		let on_cubic = |t: f32| {
			let s = 1.0 - t;
			[0, 1].map(|axis| {
				(s * s * s * cubic[0][axis]
					+ 3.0 * s * s * t * cubic[1][axis]
					+ 3.0 * s * t * t * cubic[2][axis]
					+ t * t * t * cubic[3][axis])
					/ 1000.0
			})
		};
		for (index, pair) in collector.curves.windows(2).enumerate() {
			assert_eq!(pair[0][2], pair[1][0], "curve {index} does not continue into the next one");
		}
		let pieces = collector.curves.len() as f32;
		for (piece, [p1, p2, p3]) in collector.curves.iter().enumerate() {
			for step in 0..=8 {
				let t = step as f32 / 8.0;
				let s = 1.0 - t;
				let point = [0, 1].map(|axis| s * s * p1[axis] + 2.0 * s * t * p2[axis] + t * t * p3[axis]);
				// Each piece covers an equal share of the cubic's parameter range.
				let cubic = on_cubic((piece as f32 + t) / pieces);
				let distance = ((cubic[0] - point[0]).powi(2) + (cubic[1] - point[1]).powi(2)).sqrt();
				assert!(
					distance <= super::CUBIC_TOLERANCE,
					"quadratic strays {distance} em from the cubic"
				);
			}
		}
	}

	#[test]
	fn wrapping_breaks_greedily_at_spaces_like_explicit_newlines() {
		let mut text = TextSystem::new();
		let em = |text: &mut TextSystem, sample: &str| text.measure(sample, None, 10.0, None).x() / 10.0;
		// Just wide enough for "aa bb", so "cc" starts a second line and the space before it is dropped.
		let wrap = em(&mut text, "aa bb") + 0.01;
		assert_eq!(
			text.measure("aa bb cc", None, 10.0, Some(wrap)),
			text.measure("aa bb\ncc", None, 10.0, None)
		);
		// A word wider than the line keeps a line of its own, and a newline still breaks.
		assert_eq!(
			text.measure("abcdefgh ij\nk", None, 10.0, Some(wrap)),
			text.measure("abcdefgh\nij\nk", None, 10.0, None)
		);
		// Breaks are in multiples of the font size, so the same wrap breaks a larger text at the same places.
		assert_eq!(
			text.measure("aa bb cc", None, 30.0, Some(wrap)),
			text.measure("aa bb\ncc", None, 30.0, None)
		);

		if !text.has_font() {
			return;
		}
		let mut wrapped = Vec::new();
		text.place_outlines("aa bb cc", None, 10.0, Some(wrap), |glyph| wrapped.push(glyph.pen));
		let mut broken = Vec::new();
		text.place_outlines("aa bb\ncc", None, 10.0, None, |glyph| broken.push(glyph.pen));
		assert_eq!(wrapped, broken, "drawing breaks where measuring does");
	}
}
