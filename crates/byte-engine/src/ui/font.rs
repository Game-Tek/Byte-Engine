use std::{
	collections::HashMap,
	fs,
	path::{Path, PathBuf},
};

use fontdue::{Font, FontSettings};

use super::flow::Size;

const FALLBACK_WIDTH_FACTOR: f32 = 0.6;
const FALLBACK_ASCENT_FACTOR: f32 = 0.8;
const FALLBACK_DESCENT_FACTOR: f32 = 0.2;
const FALLBACK_LINE_HEIGHT_FACTOR: f32 = 1.2;
const FONT_SEARCH_DEPTH: usize = 3;

struct LoadedFont {
	font: Font,
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
	pub(crate) x: i32,
	/// Top edge of the bitmap in target pixels.
	pub(crate) y: i32,
	pub(crate) glyph: &'a Glyph,
}

// Bound cached whole strings independently of glyphs, which remain reusable across text changes.
const MEASURE_CACHE_ENTRIES: usize = 4096;
const MEASURE_CACHE_BYTES: usize = 512 * 1024;

/// The `TextSystem` struct shapes and rasterizes UI text through one shared glyph cache.
///
/// Measurement and rendering use the same cached glyph metrics, so layout and draw
/// placement can never disagree. Glyph bitmaps are rasterized once per character
/// and pixel size and reused for the life of the system.
pub(crate) struct TextSystem {
	font_state: FontState,
	measure_cache: HashMap<u32, HashMap<String, Size>>,
	previous_measure_cache: HashMap<u32, HashMap<String, Size>>,
	measure_cache_entries: usize,
	measure_cache_bytes: usize,
	glyph_cache: HashMap<GlyphKey, Glyph>,
	line_metrics_cache: HashMap<u32, LineMetrics>,
	reported_unavailable: bool,
}

impl TextSystem {
	pub fn new() -> Self {
		Self {
			font_state: FontState::Uninitialized,
			measure_cache: HashMap::new(),
			previous_measure_cache: HashMap::new(),
			measure_cache_entries: 0,
			measure_cache_bytes: 0,
			glyph_cache: HashMap::new(),
			line_metrics_cache: HashMap::new(),
			reported_unavailable: false,
		}
	}

	/// Returns whether a font is available for glyph rasterization.
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
		let key = font_size.to_bits();
		if let Some(metrics) = self.line_metrics_cache.get(&key) {
			return Some(*metrics);
		}
		let metrics = font_line_metrics(self.font()?, font_size);
		self.line_metrics_cache.insert(key, metrics);
		Some(metrics)
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
		// Borrow the loaded font separately so a cache hit needs only one lookup.
		Some(self.glyph_cache.entry(key).or_insert_with(|| {
			let (metrics, bitmap) = font.font.rasterize(key.character, f32::from_bits(key.font_size_bits));
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
	/// Lines advance by the font's line height and each glyph is snapped to whole
	/// pixels, so the same run always maps to the same bitmap positions. Returns
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
					x: pen_x.round() as i32 + glyph.xmin,
					y: baseline_y.round() as i32 - glyph.height as i32 - glyph.ymin,
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

			current_width += self.glyph(character, font_size).map_or(0.0, |glyph| glyph.advance_width);
		}

		max_width = max_width.max(current_width);

		let line_box_height = (line.ascent - line.descent).max(font_size);
		let height = line_box_height + (line_count.saturating_sub(1) as f32 * line.line_height);

		Size::new(max_width.max(0.0), height.max(0.0))
	}

	fn font(&mut self) -> Option<&Font> {
		if matches!(self.font_state, FontState::Uninitialized) {
			self.font_state = match load_system_font() {
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
			FontState::Ready(font) => Some(&font.font),
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

fn font_line_metrics(font: &Font, font_size: f32) -> LineMetrics {
	font.horizontal_line_metrics(font_size)
		.map(|metrics| LineMetrics {
			line_height: metrics.new_line_size,
			ascent: metrics.ascent,
			descent: metrics.descent,
		})
		.unwrap_or(LineMetrics {
			line_height: font_size * FALLBACK_LINE_HEIGHT_FACTOR,
			ascent: font_size * FALLBACK_ASCENT_FACTOR,
			descent: -font_size * FALLBACK_DESCENT_FACTOR,
		})
}

fn load_system_font() -> Result<LoadedFont, String> {
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

		let Ok(font) = Font::from_bytes(bytes, FontSettings::default()) else {
			continue;
		};

		return Ok(LoadedFont { font, path });
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
		assert_eq!(a.0, 10 + a.2);
		assert_eq!(b.0, (10.0 + a.3).round() as i32 + b.2);
		assert_eq!(c.0, 10 + c.2);
		assert!(c.1 > a.1, "second line must sit below the first");
		let line = text_system.line_metrics(20.0).unwrap();
		assert!((c.1 - a.1 - line.line_height.round() as i32).abs() <= 2);

		// Measurement uses the same advances as placement.
		let measured = text_system.measure("AB", 20.0);
		assert!((measured.x() - (a.3 + b.3)).abs() < 0.001);
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
		assert_eq!(placements[0].0, space.advance_width.round() as i32 + placements[0].1);
	}
}
