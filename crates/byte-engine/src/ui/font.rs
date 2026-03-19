use std::{
	fs,
	path::{Path, PathBuf},
};

use fontdue::{Font, FontSettings};
use utils::RGBA;

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

pub(crate) struct TextSystem {
	font_state: FontState,
	reported_unavailable: bool,
}

impl TextSystem {
	pub fn new() -> Self {
		Self {
			font_state: FontState::Uninitialized,
			reported_unavailable: false,
		}
	}

	pub fn measure(&mut self, text: &str, font_size: f32) -> Size {
		if text.is_empty() {
			return Size::new(0, 0);
		}

		let font_size = font_size.max(1.0);

		match self.font() {
			Some(font) => measure_with_font(font, text, font_size),
			None => measure_with_fallback(text, font_size),
		}
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
				blend_glyph(
					target,
					target_width,
					target_height,
					glyph_x,
					glyph_y,
					metrics.width,
					metrics.height,
					&bitmap,
					color,
				);
				drew_anything = true;
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

	Size::new(max_width.ceil().max(0.0) as u32, height.ceil().max(0.0) as u32)
}

fn measure_with_fallback(text: &str, font_size: f32) -> Size {
	let lines = text.lines().collect::<Vec<_>>();
	let line_count = lines.len().max(1) as f32;
	let max_width = lines
		.iter()
		.map(|line| line.chars().count() as f32 * font_size * FALLBACK_WIDTH_FACTOR)
		.fold(0.0, f32::max);
	let height = line_count * font_size * FALLBACK_LINE_HEIGHT_FACTOR;

	Size::new(max_width.ceil().max(0.0) as u32, height.ceil().max(0.0) as u32)
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
) {
	let source_r = color.r.clamp(0.0, 1.0);
	let source_g = color.g.clamp(0.0, 1.0);
	let source_b = color.b.clamp(0.0, 1.0);
	let source_a = color.a.clamp(0.0, 1.0);

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

			let coverage = bitmap[row * glyph_width + column] as f32 / 255.0;
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
		}
	}
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
