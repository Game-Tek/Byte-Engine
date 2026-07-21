use std::{
	collections::HashMap,
	fs,
	path::{Path, PathBuf},
};

use fontdue::{Font, FontSettings};
use utils::RGBA;

use super::flow::Size;
use super::style::EdgeFeather;

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

pub(crate) struct TextSystem {
	font_state: FontState,
	measure_cache: HashMap<u32, HashMap<String, Size>>,
	reported_unavailable: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct TextClipRect {
	x: u32,
	y: u32,
	width: u32,
	height: u32,
}

impl TextClipRect {
	pub(crate) fn new(x: u32, y: u32, width: u32, height: u32) -> Self {
		Self { x, y, width, height }
	}

	fn contains(&self, x: i32, y: i32) -> bool {
		let Ok(x) = u32::try_from(x) else {
			return false;
		};
		let Ok(y) = u32::try_from(y) else {
			return false;
		};
		let right = self.x.saturating_add(self.width);
		let bottom = self.y.saturating_add(self.height);

		x >= self.x && x < right && y >= self.y && y < bottom
	}
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct TextFeatherMask {
	x: u32,
	y: u32,
	width: u32,
	height: u32,
	feather: EdgeFeather,
	corner_radius: f32,
	corner_exponent: f32,
}

impl TextFeatherMask {
	pub(crate) fn new(
		x: u32,
		y: u32,
		width: u32,
		height: u32,
		feather: EdgeFeather,
		corner_radius: f32,
		corner_exponent: f32,
	) -> Self {
		Self {
			x,
			y,
			width,
			height,
			feather,
			corner_radius,
			corner_exponent: sanitize_corner_exponent(corner_exponent),
		}
	}

	fn coverage(&self, x: i32, y: i32) -> f32 {
		let x = x as f32;
		let y = y as f32;
		let left = x - self.x as f32;
		let top = y - self.y as f32;
		let right = self.x.saturating_add(self.width) as f32 - x;
		let bottom = self.y.saturating_add(self.height) as f32 - y;

		edge_coverage(top, self.feather.top)
			* edge_coverage(right, self.feather.right)
			* edge_coverage(bottom, self.feather.bottom)
			* edge_coverage(left, self.feather.left)
			* rounded_rect_coverage(
				left,
				top,
				self.width as f32,
				self.height as f32,
				self.corner_radius,
				self.corner_exponent,
			)
	}
}

fn rounded_rect_coverage(x: f32, y: f32, width: f32, height: f32, corner_radius: f32, corner_exponent: f32) -> f32 {
	let half_width = width * 0.5;
	let half_height = height * 0.5;
	let radius = corner_radius.max(0.0).min(half_width.min(half_height));
	if radius <= 0.0 {
		return 1.0;
	}

	let centered_x = x - half_width;
	let centered_y = y - half_height;
	let rounded_extent_x = half_width - radius;
	let rounded_extent_y = half_height - radius;
	let corner_delta_x = centered_x.abs() - rounded_extent_x;
	let corner_delta_y = centered_y.abs() - rounded_extent_y;
	let abs_corner_x = corner_delta_x.max(0.0);
	let abs_corner_y = corner_delta_y.max(0.0);
	let corner_sum = abs_corner_x.powf(corner_exponent) + abs_corner_y.powf(corner_exponent);
	let corner_distance = corner_sum.powf(1.0 / corner_exponent);
	let field_distance = corner_distance + corner_delta_x.max(corner_delta_y).min(0.0) - radius;

	(1.0 - smoothstep(-1.0, 1.0, field_distance)).clamp(0.0, 1.0)
}

fn smoothstep(edge0: f32, edge1: f32, value: f32) -> f32 {
	let t = ((value - edge0) / (edge1 - edge0)).clamp(0.0, 1.0);
	t * t * (3.0 - 2.0 * t)
}

fn sanitize_corner_exponent(exponent: f32) -> f32 {
	if !exponent.is_finite() || exponent < 1.0 {
		2.0
	} else {
		exponent.clamp(1.0, 8.0)
	}
}

fn edge_coverage(distance: f32, feather_width: f32) -> f32 {
	if feather_width <= 0.0 {
		1.0
	} else {
		let t = (distance / feather_width).clamp(0.0, 1.0);
		t * t * (3.0 - 2.0 * t)
	}
}

impl TextSystem {
	pub fn new() -> Self {
		Self {
			font_state: FontState::Uninitialized,
			measure_cache: HashMap::new(),
			reported_unavailable: false,
		}
	}

	pub fn measure(&mut self, text: &str, font_size: f32) -> Size {
		if text.is_empty() {
			return Size::new(0.0, 0.0);
		}

		let font_size = font_size.max(1.0);
		let font_size_key = font_size.to_bits();
		if let Some(size) = self.measure_cache.get(&font_size_key).and_then(|sizes| sizes.get(text)) {
			return *size;
		}

		let size = match self.font() {
			Some(font) => measure_with_font(font, text, font_size),
			None => measure_with_fallback(text, font_size),
		};
		self.measure_cache
			.entry(font_size_key)
			.or_default()
			.insert(text.to_owned(), size);
		size
	}

	/// Rasterizes a text run into the provided RGBA texture using source-over alpha blending.
	pub fn rasterize(
		&mut self,
		target: &mut [u8],
		target_width: u32,
		target_height: u32,
		position: (u32, u32),
		text: &str,
		font_size: f32,
		color: RGBA,
		clip: Option<TextClipRect>,
		feather_mask: Option<TextFeatherMask>,
	) -> bool {
		if text.is_empty() || target_width == 0 || target_height == 0 {
			return false;
		}

		let font_size = font_size.max(1.0);
		let Some(font) = self.font() else {
			return false;
		};

		let (line_height, ascent, _) = line_metrics(font, font_size);
		let mut baseline_y = position.1 as f32 + ascent.max(font_size * FALLBACK_ASCENT_FACTOR);
		let mut pen_x = position.0 as f32;
		let mut drew_anything = false;

		for character in text.chars() {
			if character == '\n' {
				pen_x = position.0 as f32;
				baseline_y += line_height;
				continue;
			}

			let (metrics, bitmap) = font.rasterize(character, font_size);
			let glyph_x = pen_x.round() as i32 + metrics.xmin;
			let glyph_y = baseline_y.round() as i32 - metrics.height as i32 - metrics.ymin;

			if metrics.width > 0 && metrics.height > 0 && !bitmap.is_empty() {
				drew_anything |= blend_glyph(
					target,
					target_width,
					target_height,
					glyph_x,
					glyph_y,
					metrics.width,
					metrics.height,
					&bitmap,
					color,
					clip,
					feather_mask,
				);
			}

			pen_x += metrics.advance_width;
		}

		drew_anything
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

fn measure_with_font(font: &Font, text: &str, font_size: f32) -> Size {
	let (line_height, ascent, descent) = line_metrics(font, font_size);
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

		current_width += font.metrics(character, font_size).advance_width;
	}

	max_width = max_width.max(current_width);

	let line_box_height = (ascent - descent).max(font_size);
	let height = line_box_height + (line_count.saturating_sub(1) as f32 * line_height);

	Size::new(max_width.max(0.0), height.max(0.0))
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

fn line_metrics(font: &Font, font_size: f32) -> (f32, f32, f32) {
	font.horizontal_line_metrics(font_size)
		.map(|metrics| (metrics.new_line_size, metrics.ascent, metrics.descent))
		.unwrap_or((
			font_size * FALLBACK_LINE_HEIGHT_FACTOR,
			font_size * FALLBACK_ASCENT_FACTOR,
			-font_size * FALLBACK_DESCENT_FACTOR,
		))
}

/// Blends a glyph bitmap into the target texture while clipping to the texture bounds.
fn blend_glyph(
	target: &mut [u8],
	target_width: u32,
	target_height: u32,
	glyph_x: i32,
	glyph_y: i32,
	glyph_width: usize,
	glyph_height: usize,
	bitmap: &[u8],
	color: RGBA,
	clip: Option<TextClipRect>,
	feather_mask: Option<TextFeatherMask>,
) -> bool {
	let source_r = color.r.clamp(0.0, 1.0);
	let source_g = color.g.clamp(0.0, 1.0);
	let source_b = color.b.clamp(0.0, 1.0);
	let source_a = color.a.clamp(0.0, 1.0);
	let mut drew_anything = false;

	for row in 0..glyph_height {
		let target_y = glyph_y + row as i32;
		if target_y < 0 || target_y >= target_height as i32 {
			continue;
		}

		for column in 0..glyph_width {
			let target_x = glyph_x + column as i32;
			if target_x < 0 || target_x >= target_width as i32 {
				continue;
			}
			if clip.is_some_and(|clip| !clip.contains(target_x, target_y)) {
				continue;
			}

			let coverage = bitmap[row * glyph_width + column] as f32 / 255.0
				* feather_mask.map(|mask| mask.coverage(target_x, target_y)).unwrap_or(1.0);
			if coverage <= 0.0 {
				continue;
			}

			let src_alpha = source_a * coverage;
			let pixel_index = ((target_y as u32 * target_width + target_x as u32) * 4) as usize;

			let dst_r = target[pixel_index] as f32 / 255.0;
			let dst_g = target[pixel_index + 1] as f32 / 255.0;
			let dst_b = target[pixel_index + 2] as f32 / 255.0;
			let dst_alpha = target[pixel_index + 3] as f32 / 255.0;

			let out_alpha = src_alpha + dst_alpha * (1.0 - src_alpha);
			let src_r_premultiplied = source_r * src_alpha;
			let src_g_premultiplied = source_g * src_alpha;
			let src_b_premultiplied = source_b * src_alpha;
			let dst_r_premultiplied = dst_r * dst_alpha;
			let dst_g_premultiplied = dst_g * dst_alpha;
			let dst_b_premultiplied = dst_b * dst_alpha;

			let out_r_premultiplied = src_r_premultiplied + dst_r_premultiplied * (1.0 - src_alpha);
			let out_g_premultiplied = src_g_premultiplied + dst_g_premultiplied * (1.0 - src_alpha);
			let out_b_premultiplied = src_b_premultiplied + dst_b_premultiplied * (1.0 - src_alpha);

			let (out_r, out_g, out_b) = if out_alpha > 0.0 {
				(
					out_r_premultiplied / out_alpha,
					out_g_premultiplied / out_alpha,
					out_b_premultiplied / out_alpha,
				)
			} else {
				(0.0, 0.0, 0.0)
			};

			target[pixel_index] = (out_r.clamp(0.0, 1.0) * 255.0).round() as u8;
			target[pixel_index + 1] = (out_g.clamp(0.0, 1.0) * 255.0).round() as u8;
			target[pixel_index + 2] = (out_b.clamp(0.0, 1.0) * 255.0).round() as u8;
			target[pixel_index + 3] = (out_alpha.clamp(0.0, 1.0) * 255.0).round() as u8;
			drew_anything = true;
		}
	}

	drew_anything
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

#[cfg(test)]
mod tests {
	use utils::RGBA;

	use super::{blend_glyph, TextClipRect, TextSystem};

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
	fn clipped_glyph_reports_no_draw_when_all_pixels_are_outside_clip() {
		let mut target = [0u8; 4];
		let drew = blend_glyph(
			&mut target,
			1,
			1,
			0,
			0,
			1,
			1,
			&[255],
			RGBA::white(),
			Some(TextClipRect::new(1, 1, 1, 1)),
			None,
		);

		assert!(!drew);
		assert_eq!(target, [0, 0, 0, 0]);
	}

	#[test]
	fn clipped_glyph_draws_pixels_inside_clip() {
		let mut target = [0u8; 4];
		let drew = blend_glyph(
			&mut target,
			1,
			1,
			0,
			0,
			1,
			1,
			&[255],
			RGBA::white(),
			Some(TextClipRect::new(0, 0, 1, 1)),
			None,
		);

		assert!(drew);
		assert_eq!(target, [255, 255, 255, 255]);
	}

	#[test]
	fn feathered_glyph_reduces_alpha_near_mask_edge() {
		let mut target = [0u8; 8];
		let drew = blend_glyph(
			&mut target,
			2,
			1,
			0,
			0,
			2,
			1,
			&[255, 255],
			RGBA::white(),
			None,
			Some(super::TextFeatherMask::new(
				0,
				0,
				2,
				1,
				super::EdgeFeather::edges(0.0, 0.0, 0.0, 2.0),
				0.0,
				2.0,
			)),
		);

		assert!(drew);
		assert_eq!(target[3], 0);
		assert_eq!(target[7], 128);
	}
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
