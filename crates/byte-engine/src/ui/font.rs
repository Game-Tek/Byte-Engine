use std::{
	cell::OnceCell,
	collections::HashMap,
	fs,
	path::{Path, PathBuf},
};

use fontdue::{Font, FontSettings};

use super::flow::Size;

const FALLBACK_WIDTH_FACTOR: f32 = 0.6;
const FALLBACK_ASCENT_FACTOR: f32 = 0.8;
const FALLBACK_LINE_HEIGHT_FACTOR: f32 = 1.2;
const FONT_SEARCH_DEPTH: usize = 3;


/// The `LoadedFont` struct retains font data for on-demand outlines and optional bitmap rendering.
struct LoadedFont {
	/// Only the bitmap path needs the rasterizer's eagerly compiled glyph geometry.
	rasterizer: OnceCell<Option<Font>>,
	glyph_indices: HashMap<char, u16>,
	/// The font file, kept because outlines are read from it on a character's first use.
	data: Vec<u8>,
	path: PathBuf,
}

enum FontState {
	Uninitialized,
	Ready(LoadedFont),
	Unavailable,
}

/// The `GlyphKey` struct identifies one rasterized glyph by character and pixel size.
///
/// The size is keyed by its bit pattern so a cache lookup never depends on float rounding.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) struct GlyphKey {
	pub(crate) character: char,
	pub(crate) font_size_bits: u32,
}

impl GlyphKey {
	pub(crate) fn new(character: char, font_size: f32) -> Self {
		Self {
			character,
			font_size_bits: font_size.max(1.0).to_bits(),
		}
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
	/// The font's index of this glyph. Use it to key per-glyph data.
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
#[derive(Debug, Clone, Copy)]
pub(crate) struct GlyphPlacement<'a> {
	pub(crate) key: GlyphKey,
	/// Left edge of the bitmap in target pixels.
	pub(crate) x: f32,
	/// Top edge of the bitmap in target pixels.
	pub(crate) y: f32,
	pub(crate) glyph: &'a Glyph,
}

// Bound cached whole strings independently of glyphs, which remain reusable across text changes.
const MEASURE_CACHE_ENTRIES: usize = 4096;
const MEASURE_CACHE_BYTES: usize = 512 * 1024;

/// The `TextSystem` struct shapes UI text and supplies its glyphs as outlines or bitmaps.
///
/// Measurement and outline placement use the same size-independent advances, so layout and
/// drawing never disagree and neither rasterizes anything. Glyph bitmaps exist only for the
/// atlas renderer, which rasterizes them once per character and pixel size.
pub(crate) struct TextSystem {
	/// The font file to load instead of a system font, when the application chose one.
	font_path: Option<PathBuf>,
	font_state: FontState,
	measure_cache: HashMap<u32, HashMap<String, Size>>,
	previous_measure_cache: HashMap<u32, HashMap<String, Size>>,
	measure_cache_entries: usize,
	measure_cache_bytes: usize,
	glyph_cache: HashMap<GlyphKey, Glyph>,
	/// Outlines by the font's glyph index, read on first use.
	outlines: Vec<Option<GlyphOutline>>,
	/// Vertical metrics at one pixel per em; every size is a multiple of these.
	em_line_metrics: Option<LineMetrics>,
	reported_unavailable: bool,
}

impl TextSystem {
	pub fn new() -> Self {
		Self::with_font(None)
	}

	/// Creates a text system that loads the font at `path`, falling back to a system font
	/// when it is missing or unreadable.
	pub fn with_font(path: Option<PathBuf>) -> Self {
		Self {
			font_path: path,
			font_state: FontState::Uninitialized,
			measure_cache: HashMap::new(),
			previous_measure_cache: HashMap::new(),
			measure_cache_entries: 0,
			measure_cache_bytes: 0,
			glyph_cache: HashMap::new(),
			outlines: Vec::new(),
			em_line_metrics: None,
			reported_unavailable: false,
		}
	}

	/// Returns whether a font is available for text measurement and drawing.
	pub fn has_font(&mut self) -> bool {
		self.font().is_some()
	}

	/// Measures text with a bounded string cache and reusable glyph metrics.
	pub fn measure(&mut self, text: &str, font_size: f32) -> Size {
		if text.is_empty() {
			return Size::new(0.0, 0.0);
		}

		let font_size = font_size.max(1.0);
		let font_size_key = font_size.to_bits();
		for cache in [&self.measure_cache, &self.previous_measure_cache] {
			if let Some(size) = cache.get(&font_size_key).and_then(|sizes| sizes.get(text)) {
				return *size;
			}
		}

		let size = if self.has_font() {
			self.measure_with_glyphs(text, font_size)
		} else {
			measure_with_fallback(text, font_size)
		};
		// Rotate whole-string measurements in two bounded generations. Recent labels remain
		// available through a rotation, while changing counters cannot retain every old value.
		if text.len() <= MEASURE_CACHE_BYTES {
			if self.measure_cache_entries == MEASURE_CACHE_ENTRIES
				|| self.measure_cache_bytes + text.len() > MEASURE_CACHE_BYTES
			{
				std::mem::swap(&mut self.measure_cache, &mut self.previous_measure_cache);
				self.measure_cache.clear();
				self.measure_cache_entries = 0;
				self.measure_cache_bytes = 0;
			}
			self.measure_cache
				.entry(font_size_key)
				.or_default()
				.insert(text.to_owned(), size);
			self.measure_cache_entries += 1;
			self.measure_cache_bytes += text.len();
			debug_assert!(
				self.measure_cache_entries <= MEASURE_CACHE_ENTRIES && self.measure_cache_bytes <= MEASURE_CACHE_BYTES
			);
		}
		size
	}

	/// Returns the vertical metrics for one pixel size, or `None` without a font.
	pub fn line_metrics(&mut self, font_size: f32) -> Option<LineMetrics> {
		let font_size = font_size.max(1.0);
		if self.em_line_metrics.is_none() {
			self.em_line_metrics = Some(font_line_metrics(self.font()?, 1.0));
		}
		// Metrics scale linearly, so a continuously animated size needs no cache entry of its own.
		let em = self.em_line_metrics?;
		Some(LineMetrics {
			line_height: em.line_height * font_size,
			ascent: em.ascent * font_size,
			descent: em.descent * font_size,
		})
	}

	/// Returns one character's glyph index and outline, reading the outline from the font on first use.
	///
	/// Returns `None` without a font. Characters the font draws alike share a glyph index, and a
	/// character without contours, such as a space, has an outline that only advances the pen.
	pub fn outline(&mut self, character: char) -> Option<(usize, &GlyphOutline)> {
		if matches!(self.font_state, FontState::Uninitialized) {
			self.font()?;
		}
		let FontState::Ready(font) = &mut self.font_state else {
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
		if self.outlines.len() <= index as usize {
			self.outlines.resize(index as usize + 1, None);
		}
		let outline = self.outlines[index as usize].get_or_insert_with(|| read_outline(font, index));
		Some((index as usize, outline))
	}

	/// Visits every visible glyph outline of `text` with its pen position relative to the text's top left corner.
	///
	/// Lines advance by the font's line height and advances stay fractional, exactly as
	/// [`Self::measure`] sums them. Returns `false` when no font is available.
	pub fn place_outlines(&mut self, text: &str, font_size: f32, mut visit: impl FnMut(OutlinePlacement<'_>)) -> bool {
		if text.is_empty() {
			return false;
		}
		let font_size = font_size.max(1.0);
		let Some(line) = self.line_metrics(font_size) else {
			return false;
		};

		let mut pen = [0.0, line.ascent.max(font_size * FALLBACK_ASCENT_FACTOR)];
		for character in text.chars() {
			if character == '\n' {
				pen = [0.0, pen[1] + line.line_height];
				continue;
			}
			let Some((index, outline)) = self.outline(character) else {
				return false;
			};
			if outline.is_visible() {
				visit(OutlinePlacement { index, pen, outline });
			}
			pen[0] += outline.advance * font_size;
		}

		true
	}

	/// Returns the cached glyph for one character and pixel size, rasterizing it on first use.
	pub fn glyph(&mut self, character: char, font_size: f32) -> Option<&Glyph> {
		self.glyph_by_key(GlyphKey::new(character, font_size))
	}

	/// Returns the cached glyph for one key, rasterizing it on first use.
	pub fn glyph_by_key(&mut self, key: GlyphKey) -> Option<&Glyph> {
		if matches!(self.font_state, FontState::Uninitialized) {
			self.font()?;
		}
		let FontState::Ready(font) = &self.font_state else {
			return None;
		};
		// Bitmap consumers initialize the rasterizer once; layout and GPU outlines never need it.
		let rasterizer = font
			.rasterizer
			.get_or_init(|| Font::from_bytes(font.data.as_slice(), FontSettings::default()).ok())
			.as_ref()?;
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

	/// Places every visible glyph of `text` in target pixels, starting at `origin`.
	///
	/// Lines advance by the font's line height. Fractional advances are preserved
	/// so rendering can scale and translate the run without pixel snapping. Returns
	/// `false` when no font is available.
	pub fn place_glyphs(
		&mut self,
		text: &str,
		font_size: f32,
		origin: (i32, i32),
		mut visit: impl FnMut(GlyphPlacement<'_>),
	) -> bool {
		if text.is_empty() {
			return false;
		}
		let font_size = font_size.max(1.0);
		let Some(line) = self.line_metrics(font_size) else {
			return false;
		};

		let mut baseline_y = origin.1 as f32 + line.ascent.max(font_size * FALLBACK_ASCENT_FACTOR);
		let mut pen_x = origin.0 as f32;

		for character in text.chars() {
			if character == '\n' {
				pen_x = origin.0 as f32;
				baseline_y += line.line_height;
				continue;
			}

			let key = GlyphKey::new(character, font_size);
			let Some(glyph) = self.glyph_by_key(key) else {
				return false;
			};
			if glyph.is_visible() {
				visit(GlyphPlacement {
					key,
					x: pen_x + glyph.xmin as f32,
					y: baseline_y - glyph.height as f32 - glyph.ymin as f32,
					glyph,
				});
			}
			pen_x += glyph.advance_width;
		}

		true
	}

	fn measure_with_glyphs(&mut self, text: &str, font_size: f32) -> Size {
		let Some(line) = self.line_metrics(font_size) else {
			return measure_with_fallback(text, font_size);
		};
		let mut max_width: f32 = 0.0;
		let mut current_width: f32 = 0.0;
		let mut line_count = 1u32;

		for character in text.chars() {
			if character == '\n' {
				max_width = max_width.max(current_width);
				current_width = 0.0;
				line_count += 1;
				continue;
			}

			// Advances come from the outline, so measuring never rasterizes a glyph.
			current_width += self
				.outline(character)
				.map_or(0.0, |(_, outline)| outline.advance * font_size);
		}

		max_width = max_width.max(current_width);

		let line_box_height = (line.ascent - line.descent).max(font_size);
		let height = line_box_height + (line_count.saturating_sub(1) as f32 * line.line_height);

		Size::new(max_width.max(0.0), height.max(0.0))
	}

	/// Loads font tables on first use without preparing every glyph for CPU rasterization.
	fn font(&mut self) -> Option<&LoadedFont> {
		if matches!(self.font_state, FontState::Uninitialized) {
			self.font_state = match load_font(self.font_path.as_deref()) {
				Ok(font) => {
					log::debug!("Loaded UI font from '{}'.", font.path.display());
					FontState::Ready(font)
				}
				Err(error) => {
					if !self.reported_unavailable {
						log::warn!("{error}");
						self.reported_unavailable = true;
					}

					FontState::Unavailable
				}
			};
		}

		match &self.font_state {
			FontState::Ready(font) => Some(font),
			_ => None,
		}
	}
}

fn measure_with_fallback(text: &str, font_size: f32) -> Size {
	let lines = text.lines().collect::<Vec<_>>();
	let line_count = lines.len().max(1) as f32;
	let max_width = lines
		.iter()
		.map(|line| line.chars().count() as f32 * font_size * FALLBACK_WIDTH_FACTOR)
		.fold(0.0, f32::max);
	let height = line_count * font_size * FALLBACK_LINE_HEIGHT_FACTOR;

	Size::new(max_width.max(0.0), height.max(0.0))
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

/// Largest distance, in em units, between a cubic outline segment and the quadratic curves that replace it.
const CUBIC_TOLERANCE: f32 = 1.0 / 4096.0;

/// The `OutlineCollector` struct turns a font's outline segments into quadratic curves in em units.
struct OutlineCollector {
	curves: Vec<[[f32; 2]; 3]>,
	/// Em units per font unit.
	scale: f32,
	start: [f32; 2],
	last: [f32; 2],
}

impl OutlineCollector {
	/// A line repeats its end point as the control point, which keeps the curve's polynomial quadratic.
	fn line(&mut self, end: [f32; 2]) {
		if end != self.last {
			self.curves.push([self.last, end, end]);
			self.last = end;
		}
	}
}

impl ttf_parser::OutlineBuilder for OutlineCollector {
	fn move_to(&mut self, x: f32, y: f32) {
		self.start = [x * self.scale, y * self.scale];
		self.last = self.start;
	}

	fn line_to(&mut self, x: f32, y: f32) {
		self.line([x * self.scale, y * self.scale]);
	}

	fn quad_to(&mut self, x1: f32, y1: f32, x: f32, y: f32) {
		let control = [x1 * self.scale, y1 * self.scale];
		let end = [x * self.scale, y * self.scale];
		if control != self.last || end != self.last {
			self.curves.push([self.last, control, end]);
			self.last = end;
		}
	}

	/// Splits a cubic segment into enough quadratic curves to stay within [`CUBIC_TOLERANCE`].
	fn curve_to(&mut self, x1: f32, y1: f32, x2: f32, y2: f32, x: f32, y: f32) {
		let [p0, p1, p2, p3] = [
			self.last,
			[x1 * self.scale, y1 * self.scale],
			[x2 * self.scale, y2 * self.scale],
			[x * self.scale, y * self.scale],
		];
		// The cubic and its closest quadratic differ by the third difference times t (t - 1/2) (t - 1),
		// which peaks at sqrt(3) / 36. Splitting into n pieces divides the third difference by n cubed.
		let third_difference = (0..2)
			.map(|axis| (p3[axis] - 3.0 * p2[axis] + 3.0 * p1[axis] - p0[axis]).powi(2))
			.sum::<f32>()
			.sqrt();
		let pieces = (third_difference * 3f32.sqrt() / 36.0 / CUBIC_TOLERANCE)
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
		for piece in 0..pieces as usize {
			let (t0, t1) = (piece as f32 / pieces, (piece + 1) as f32 / pieces);
			// The last piece ends on the segment's own end point so that contours stay closed.
			let end = if t1 >= 1.0 { p3 } else { point(t1) };
			let (from, to) = (derivative(t0), derivative(t1));
			// The piece's cubic controls are start + from * h and end - to * h with h = (t1 - t0) / 3.
			// The closest quadratic control is (3 * (c1 + c2) - (start + end)) / 4.
			let h = (t1 - t0) / 3.0;
			let control = [0, 1].map(|axis| {
				(3.0 * (self.last[axis] + from[axis] * h + end[axis] - to[axis] * h) - (self.last[axis] + end[axis])) / 4.0
			});
			self.curves.push([self.last, control, end]);
			self.last = end;
		}
	}

	fn close(&mut self) {
		self.line(self.start);
	}
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

	let mut collector = OutlineCollector {
		curves: Vec::new(),
		scale,
		start: [0.0; 2],
		last: [0.0; 2],
	};
	face.outline_glyph(glyph, &mut collector);
	outline.curves = collector.curves;
	if let Some(first) = outline.curves.first() {
		outline.bounds = [first[0][0], first[0][1], first[0][0], first[0][1]];
		for point in outline.curves.iter().flatten() {
			outline.bounds = [
				outline.bounds[0].min(point[0]),
				outline.bounds[1].min(point[1]),
				outline.bounds[2].max(point[0]),
				outline.bounds[3].max(point[1]),
			];
		}
	}
	outline
}

/// Finds a readable font and validates its tables before retaining its bytes.
fn load_font(preferred: Option<&Path>) -> Result<LoadedFont, String> {
	if let Some(path) = preferred {
		match fs::read(path) {
			Ok(bytes) if ttf_parser::Face::parse(&bytes, 0).is_ok() => {
				return Ok(LoadedFont {
					rasterizer: OnceCell::new(),
					glyph_indices: HashMap::new(),
					data: bytes,
					path: path.to_path_buf(),
				});
			}
			_ => log::warn!(
				"The UI font at '{}' could not be used; falling back to a system font. The most likely cause is a missing or corrupt font file.",
				path.display()
			),
		}
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

		return Ok(LoadedFont {
			rasterizer: OnceCell::new(),
			glyph_indices: HashMap::new(),
			data: bytes,
			path,
		});
	}

	Err(
		"Failed to load a system UI font. The most likely cause is that no readable TrueType or OpenType font was found in the supported OS font directories."
			.into(),
	)
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
	use super::{Glyph, GlyphKey, TextSystem};

	#[test]
	fn outline_and_bitmap_text_keep_the_same_metrics() {
		let mut text = TextSystem::new();
		if !text.has_font() {
			return;
		}
		for character in "AV café ΩЖ中🙂\u{10ffff}".chars() {
			let advance = text.outline(character).unwrap().1.advance;
			for size in [12.0, 19.5, 32.0] {
				let bitmap = text.glyph(character, size).unwrap();
				assert!((advance * size - bitmap.advance_width).abs() < 0.0001);
			}
		}
		let mut outlines = Vec::new();
		text.place_outlines("A B\nC", 19.5, |glyph| outlines.push(glyph.pen));
		let mut bitmaps = Vec::new();
		text.place_glyphs("A B\nC", 19.5, (0, 0), |glyph| {
			bitmaps.push([
				glyph.x - glyph.glyph.xmin as f32,
				glyph.y + glyph.glyph.height as f32 + glyph.glyph.ymin as f32,
			]);
		});
		assert_eq!(outlines.len(), bitmaps.len());
		for (outline, bitmap) in outlines.into_iter().zip(bitmaps) {
			assert!((outline[0] - bitmap[0]).abs() < 0.0001);
			assert!((outline[1] - bitmap[1]).abs() < 0.0001);
		}
	}

	#[test]
	fn measurements_stay_consistent_after_many_text_changes() {
		let mut text_system = TextSystem::new();
		let labels = [("Score: 123", 16.0), ("First line\nSecond line", 24.0), ("áβ中", 12.0)];
		let expected = labels.map(|(text, size)| text_system.measure(text, size));
		for value in 0..3 * super::MEASURE_CACHE_ENTRIES {
			text_system.measure(&format!("Changing score: {value}"), 16.0);
		}
		for ((text, size), expected) in labels.into_iter().zip(expected) {
			assert_eq!(text_system.measure(text, size), expected);
		}
	}

	#[test]
	fn measure_reuses_cached_text_size_for_same_font_size() {
		let mut text_system = TextSystem::new();

		let first = text_system.measure("Cached", 16.0);
		let cache_entries = text_system
			.measure_cache
			.get(&16.0f32.to_bits())
			.map(|entries| entries.len())
			.unwrap_or_default();

		let second = text_system.measure("Cached", 16.0);
		let second_cache_entries = text_system
			.measure_cache
			.get(&16.0f32.to_bits())
			.map(|entries| entries.len())
			.unwrap_or_default();

		assert_eq!(second, first);
		assert_eq!(second_cache_entries, cache_entries);
	}

	#[test]
	fn glyphs_are_rasterized_once_per_character_and_size() {
		let mut text_system = TextSystem::new();
		if !text_system.has_font() {
			return;
		}

		let small = text_system.glyph('A', 16.0).unwrap().clone();
		let large = text_system.glyph('A', 24.0).unwrap().clone();
		assert_ne!(small, large);
		assert!(small.is_visible());
		assert_eq!(text_system.glyph_cache.len(), 2);

		// Repeated lookups hit the same entry instead of rasterizing again.
		let first: *const Glyph = text_system.glyph('A', 16.0).unwrap();
		let second: *const Glyph = text_system.glyph('A', 16.0).unwrap();
		assert_eq!(first, second);
		assert_eq!(text_system.glyph_cache.len(), 2);
	}

	#[test]
	fn glyph_key_normalizes_sizes_below_one_pixel() {
		assert_eq!(GlyphKey::new('a', 0.25), GlyphKey::new('a', 1.0));
		assert_ne!(GlyphKey::new('a', 2.0), GlyphKey::new('a', 1.0));
	}

	#[test]
	fn placement_advances_pen_and_wraps_lines_from_cached_metrics() {
		let mut text_system = TextSystem::new();
		if !text_system.has_font() {
			return;
		}

		// Placements borrow the cache, so copy what the assertions need.
		let mut placements = Vec::new();
		let placed = text_system.place_glyphs("AB\nC", 20.0, (10, 5), |placement| {
			placements.push((placement.x, placement.y, placement.glyph.xmin, placement.glyph.advance_width));
		});
		assert!(placed);
		assert_eq!(placements.len(), 3);

		let [a, b, c] = placements[..] else { unreachable!() };
		assert_eq!(a.0, 10.0 + a.2 as f32);
		assert_eq!(b.0, 10.0 + a.3 + b.2 as f32);
		assert_eq!(c.0, 10.0 + c.2 as f32);
		assert!(c.1 > a.1, "second line must sit below the first");
		let line = text_system.line_metrics(20.0).unwrap();
		assert!((c.1 - a.1 - line.line_height).abs() <= 2.0);

		// Measurement uses the same advances as placement.
		let measured = text_system.measure("AB", 20.0);
		assert!((measured.x() - (a.3 + b.3)).abs() < 0.001);
	}

	#[test]
	fn outlines_are_closed_contours_inside_their_bounds() {
		let mut text_system = TextSystem::new();
		if !text_system.has_font() {
			return;
		}

		let (index, outline) = text_system
			.outline('B')
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

		assert_eq!(text_system.outline('B').unwrap().0, index);
		let (_, space) = text_system.outline(' ').unwrap();
		assert!(!space.is_visible());
		assert!(space.advance > 0.0);
	}

	#[test]
	fn outline_placement_advances_like_measurement_and_wraps_lines() {
		let mut text_system = TextSystem::new();
		if !text_system.has_font() {
			return;
		}

		let mut placements = Vec::new();
		assert!(text_system.place_outlines("A B\nC", 20.0, |placement| {
			placements.push((placement.pen, placement.outline.advance));
		}));
		let [a, b, c] = placements[..] else {
			panic!("Expected three visible glyphs, found {}", placements.len());
		};
		let space = text_system.outline(' ').unwrap().1.advance;
		assert_eq!(a.0[0], 0.0);
		assert!((b.0[0] - (a.1 + space) * 20.0).abs() < 0.001);
		assert_eq!(c.0[0], 0.0);
		let line = text_system.line_metrics(20.0).unwrap();
		assert!((c.0[1] - a.0[1] - line.line_height).abs() < 0.001);

		// Layout measures with the same advances that drawing places with.
		let measured = text_system.measure("A B", 20.0);
		assert!((measured.x() - (a.1 + space + b.1) * 20.0).abs() < 0.001);
	}

	#[test]
	fn cubic_segments_become_quadratic_curves_within_tolerance() {
		use ttf_parser::OutlineBuilder as _;

		// A quarter circle of 800 font units in a 1000 unit em, the usual cubic approximation.
		let cubic = [[800.0f32, 0.0], [800.0, 441.6], [441.6, 800.0], [0.0, 800.0]];
		let mut collector = super::OutlineCollector {
			curves: Vec::new(),
			scale: 1.0 / 1000.0,
			start: [0.0; 2],
			last: [0.0; 2],
		};
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
	fn whitespace_produces_no_visible_placement_but_advances() {
		let mut text_system = TextSystem::new();
		if !text_system.has_font() {
			return;
		}

		let mut placements = Vec::new();
		assert!(text_system.place_glyphs(" A", 16.0, (0, 0), |placement| {
			placements.push((placement.x, placement.glyph.xmin))
		}));
		assert_eq!(placements.len(), 1);
		let space = text_system.glyph(' ', 16.0).unwrap();
		assert_eq!(placements[0].0, space.advance_width + placements[0].1 as f32);
	}
}
