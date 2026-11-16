//! Decode a source image into a small RGBA8 picture for a tool to show, such as an editor's texture preview.

/// The `Preview` struct is a source image decoded small enough for a tool to show, with the size of the image it
/// shows.
pub struct Preview {
	/// The picture's width, in pixels.
	pub width: u32,
	/// The picture's height, in pixels.
	pub height: u32,
	/// The picture's RGBA8 pixels, row by row from the top.
	pub pixels: Vec<u8>,
	/// The source image's width and height, in pixels, before it was scaled down.
	pub source: [u32; 2],
}

/// Decodes the PNG, JPEG, or other source image in `encoded` into a [`Preview`] whose longer side is at most
/// `longest` pixels.
///
/// The picture is for a person to look at, not to sample: it keeps the image's aspect, scales down only, and clamps
/// high-dynamic-range values to the displayable range. Call it off any hot path, since a large image takes tens of
/// milliseconds to decode.
pub fn preview_rgba8(encoded: &[u8], longest: u32) -> Result<Preview, String> {
	let image = image::load_from_memory(encoded).map_err(|error| {
		format!(
			"The image could not be decoded for a preview. The most likely cause is a format the engine doesn't read, or a damaged file ({error}). Save it as a PNG or JPEG."
		)
	})?;
	let source = [image.width(), image.height()];
	// `thumbnail` scales down to fit and keeps the aspect; a smaller image keeps its size.
	let image = if image.width() > longest || image.height() > longest {
		image.thumbnail(longest, longest)
	} else {
		image
	};
	let pixels = image.into_rgba8();
	Ok(Preview {
		width: pixels.width(),
		height: pixels.height(),
		source,
		pixels: pixels.into_raw(),
	})
}

#[cfg(test)]
mod tests {
	use super::preview_rgba8;

	/// Encodes a `width` by `height` red PNG.
	fn red_png(width: u32, height: u32) -> Vec<u8> {
		let pixels = image::RgbaImage::from_pixel(width, height, image::Rgba([255, 0, 0, 255]));
		let mut encoded = std::io::Cursor::new(Vec::new());
		pixels.write_to(&mut encoded, image::ImageFormat::Png).unwrap();
		encoded.into_inner()
	}

	#[test]
	fn a_large_image_scales_down_to_fit_keeping_its_aspect() {
		let preview = preview_rgba8(&red_png(64, 32), 16).unwrap();
		assert_eq!((preview.width, preview.height), (16, 8));
		assert_eq!(preview.source, [64, 32]);
		assert_eq!(preview.pixels.len(), 16 * 8 * 4);
		assert_eq!(&preview.pixels[..4], [255, 0, 0, 255]);
	}

	#[test]
	fn a_small_image_keeps_its_size() {
		let preview = preview_rgba8(&red_png(4, 2), 16).unwrap();
		assert_eq!((preview.width, preview.height), (4, 2));
	}

	#[test]
	fn bytes_that_are_not_an_image_fail() {
		assert!(preview_rgba8(b"{ nodes: [] }", 16).is_err());
	}
}
