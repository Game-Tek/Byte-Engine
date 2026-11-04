//! The layout of a lightmap's image: one square rectangle per lit primitive, placed in rows.
//!
//! The rectangles follow the Lightmaps section of the asset formats reference: each one's side comes from its
//! primitive's surface area and the texel density, and the shared row packer of mesh processing places them, so they
//! keep the gaps lightmap UV charts keep.

use resource_management::processors::mesh::place_lightmap_rectangles;

/// The smallest side a rectangle takes, in texels, so a small primitive still holds a few texels of light.
pub(crate) const MIN_RECTANGLE_SIDE: u32 = 16;
/// The texels a rectangle keeps from the image's edges; two rectangles keep twice this between them.
const RECTANGLE_BORDER: f32 = 1.0;
/// The most rounds that shrink the rectangles until they fit the image.
const SHRINK_ROUNDS: u32 = 64;

/// The `Atlas` struct is a lightmap image's layout: its side and where each primitive's rectangle lies in it.
///
/// [`layout`] makes it. The bake rasterizes each primitive into its rectangle, and stores each rectangle as the
/// primitive's scale and offset with [`Self::scale_offset`].
#[derive(Debug, PartialEq)]
pub(crate) struct Atlas {
	/// The image's side, in texels: a power of two.
	pub(crate) extent: u32,
	/// Each rectangle's minimum corner and side, in texels, in the order the sides were given.
	pub(crate) rectangles: Vec<Rectangle>,
	/// The factor every side shrank by to fit the image's largest side, or `None` when nothing shrank.
	pub(crate) shrunk: Option<f32>,
}

/// The `Rectangle` struct is one primitive's square of a lightmap image, in texels.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Rectangle {
	pub(crate) x: u32,
	pub(crate) y: u32,
	pub(crate) side: u32,
}

impl Atlas {
	/// Returns what maps a lightmap UV of the rectangle at `index` into the image: `uv * [x, y] + [z, w]`.
	pub(crate) fn scale_offset(&self, index: usize) -> [f32; 4] {
		let Rectangle { x, y, side } = self.rectangles[index];
		let extent = self.extent as f32;
		[side as f32 / extent, side as f32 / extent, x as f32 / extent, y as f32 / extent]
	}
}

/// Returns the side, in texels, of the rectangle of a primitive whose triangles cover `world_area` square meters and
/// `uv_area` of the lightmap UV square, at `texel_density` texels per meter.
///
/// The UV square stands for `sqrt(world_area / uv_area)` meters on each side, so its rectangle holds that many meters
/// at the density. A primitive without area takes the smallest side.
pub(crate) fn rectangle_side(world_area: f64, uv_area: f64, texel_density: f32) -> u32 {
	if !(world_area > 0.0 && uv_area > 0.0) {
		return MIN_RECTANGLE_SIDE;
	}
	let side = ((world_area / uv_area).sqrt() * f64::from(texel_density)).ceil();
	if side.is_finite() {
		(side.min(f64::from(u32::MAX)) as u32).max(MIN_RECTANGLE_SIDE)
	} else {
		MIN_RECTANGLE_SIDE
	}
}

/// Places square rectangles of the given `sides` in the smallest power-of-two image that holds them, shrinking every
/// side by the same factor when they don't fit in `max_extent` texels.
///
/// Returns `None` when even rectangles one texel wide don't fit, which only a level with more primitives than the image
/// has texels reaches.
pub(crate) fn layout(sides: &[u32], max_extent: u32) -> Option<Atlas> {
	let mut factor = 1.0f32;
	for _ in 0..SHRINK_ROUNDS {
		let scaled = |side: u32| ((side as f32 * factor).floor() as u32).max(1);
		let extents: Vec<[f32; 2]> = sides
			.iter()
			.map(|&side| {
				let side = scaled(side) as f32;
				[side, side]
			})
			.collect();
		let (placed, packed) = place_lightmap_rectangles(&extents, RECTANGLE_BORDER);
		let extent = (packed.ceil() as u32).max(1).next_power_of_two();
		if extent <= max_extent {
			return Some(Atlas {
				extent,
				rectangles: placed
					.iter()
					.zip(sides)
					.map(|(&[x, y], &side)| Rectangle {
						x: x as u32,
						y: y as u32,
						side: scaled(side),
					})
					.collect(),
				shrunk: (factor < 1.0).then_some(factor),
			});
		}
		if sides.iter().all(|&side| scaled(side) == 1) {
			return None;
		}
		// Rows waste some of the square, so shrinking by the sides' ratio alone may fall short; a little more converges
		// in a few rounds.
		factor *= (max_extent as f32 / packed).min(0.98);
	}
	None
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn rectangles_hold_their_area_at_the_density_and_never_fall_below_the_smallest_side() {
		// A 4 × 4 m floor whose triangles cover a quarter of the UV square stands for 8 m per side of the square.
		assert_eq!(rectangle_side(16.0, 0.25, 8.0), 64);
		assert_eq!(rectangle_side(0.01, 1.0, 8.0), MIN_RECTANGLE_SIDE);
		assert_eq!(rectangle_side(0.0, 0.0, 8.0), MIN_RECTANGLE_SIDE);
	}

	#[test]
	fn the_image_is_the_smallest_power_of_two_square_that_holds_the_rectangles() {
		let atlas = layout(&[64, 16, 16, 30], 2048).expect("small rectangles fit");

		assert_eq!(atlas.shrunk, None);
		assert!(atlas.extent.is_power_of_two());
		assert!(atlas.extent >= 64 + 2 && atlas.extent <= 256);
		for (index, rectangle) in atlas.rectangles.iter().enumerate() {
			assert!(rectangle.x >= 1 && rectangle.y >= 1, "rectangle {index} keeps a texel from the edges");
			assert!(rectangle.x + rectangle.side < atlas.extent && rectangle.y + rectangle.side < atlas.extent);
		}
		assert_eq!(atlas.rectangles[1].side, 16);
		let [scale_u, scale_v, offset_u, offset_v] = atlas.scale_offset(0);
		assert_eq!([scale_u, scale_v], [64.0 / atlas.extent as f32; 2]);
		assert_eq!([offset_u, offset_v], [atlas.rectangles[0].x as f32 / atlas.extent as f32, atlas.rectangles[0].y as f32 / atlas.extent as f32]);
	}

	#[test]
	fn rectangles_that_overflow_the_largest_side_shrink_by_one_factor() {
		let sides = [400, 400, 400, 400, 200];
		let atlas = layout(&sides, 256).expect("the rectangles shrink until they fit");

		assert_eq!(atlas.extent, 256);
		let factor = atlas.shrunk.expect("the rectangles shrank");
		assert!(factor < 0.5);
		for (rectangle, side) in atlas.rectangles.iter().zip(sides) {
			assert_eq!(rectangle.side, ((side as f32 * factor).floor() as u32).max(1));
			assert!(rectangle.x + rectangle.side < 256 && rectangle.y + rectangle.side < 256);
		}
	}
}
