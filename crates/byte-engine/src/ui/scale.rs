//! UI scale: the factor that turns a window's pixels into the layout units a UI lays out in, so a UI keeps its physical
//! size on every display. See [Scale](/docs/reference/gui#scale).
//!
//! The application keeps each window's pixel size and display scale, and the `ui.scale` setting, and gives a UI its
//! layout size through [`crate::application::graphics::GraphicsApplication::layout_size`].

use crate::configuration::{Accepts, Read, Setting};
use crate::ui::Size;

/// The layout size of a window `pixels` large at a UI `scale`: the pixels divided by the scale, rounded to the nearest
/// whole unit and at least 1 each way. A scale that isn't a positive number counts as 1.
pub fn layout_size(pixels: Size, scale: f32) -> Size {
	let scale = if scale.is_finite() && scale > 0.0 { scale } else { 1.0 };
	let unit = |pixels: f32| (pixels / scale).round().max(1.0);
	Size::new(unit(pixels.x()), unit(pixels.y()))
}

/// The `ui.` settings: `scale`, which multiplies the display scale every UI lays out with.
pub(crate) const SETTINGS: &[Setting<f32>] = &[Setting {
	name: "scale",
	accepts: Accepts::Number { min: 0.0 },
	value: |scale| Read::Number(f64::from(scale)),
	apply: |_, read| {
		let scale = read.number() as f32;
		if scale <= 0.0 {
			return Err(
				"The UI scale 0 would leave no room to lay a UI out. The most likely cause is a typo. Set `ui.scale` above 0, such as 1."
					.to_string(),
			);
		}
		Ok(scale)
	},
}];

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn a_window_lays_out_at_its_pixels_over_the_scale() {
		assert_eq!(layout_size(Size::new(3024, 1890), 2.0), Size::new(1512, 945));
		assert_eq!(
			layout_size(Size::new(1000, 600), 1.5),
			Size::new(667, 400),
			"a fractional scale rounds"
		);
		assert_eq!(
			layout_size(Size::new(1, 1), 2.0),
			Size::new(1, 1),
			"a layout is at least 1 each way"
		);
		assert_eq!(
			layout_size(Size::new(800, 600), 0.0),
			Size::new(800, 600),
			"a zero scale counts as 1"
		);
	}
}
