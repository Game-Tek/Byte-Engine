//! The layout of a lightmap's pages: one square rectangle per lit primitive, placed in rows across as many square pages
//! as they need.
//!
//! The rectangles follow the Lightmaps section of the asset formats reference: each one's side comes from its
//! primitive's surface area and the texel density, and rows of rectangles, largest first, fill each page before the
//! next one starts, so every rectangle keeps its density however large the level is.

use std::cmp::Reverse;

/// The smallest side a rectangle takes, in texels, so a small primitive still holds a few texels of light.
pub(crate) const MIN_RECTANGLE_SIDE: u32 = 16;
/// The texels a rectangle keeps from its page's edges; two rectangles keep twice this between them.
const RECTANGLE_BORDER: u32 = 1;
/// The most rounds that shrink the rectangles until they fit the most pages a lightmap may have.
const SHRINK_ROUNDS: u32 = 64;

/// The `Atlas` struct is a lightmap's layout: the side of each of its pages, and the page and place of each primitive's
/// rectangle.
///
/// [`layout`] makes it. The bake rasterizes each primitive into its rectangle, page after page, and stores each
/// rectangle as the primitive's page, scale, and offset with [`Self::scale_offset`].
#[derive(Debug, PartialEq)]
pub(crate) struct Atlas {
	/// Each page's side, in texels: a power of two. Every page but the last is as large as the layout allows.
	pub(crate) pages: Vec<u32>,
	/// Each rectangle's page, minimum corner, and side, in texels, in the order the sides were given.
	pub(crate) rectangles: Vec<Rectangle>,
	/// The factor every side shrank by to fit the most pages a lightmap may have, or `None` when nothing shrank.
	pub(crate) shrunk: Option<f32>,
	/// How many rectangles were larger than a page holds, and took the largest side a page holds instead.
	pub(crate) clamped: usize,
}

/// The `Rectangle` struct is one primitive's square of a lightmap page, in texels.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub(crate) struct Rectangle {
	pub(crate) page: u32,
	pub(crate) x: u32,
	pub(crate) y: u32,
	pub(crate) side: u32,
}

impl Atlas {
	/// Returns what maps a lightmap UV of the rectangle at `index` into its page: `uv * [x, y] + [z, w]`.
	pub(crate) fn scale_offset(&self, index: usize) -> [f32; 4] {
		let Rectangle { page, x, y, side } = self.rectangles[index];
		let extent = self.pages[page as usize] as f32;
		[
			side as f32 / extent,
			side as f32 / extent,
			x as f32 / extent,
			y as f32 / extent,
		]
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

/// Places square rectangles of the given `sides` in pages of `max_extent` texels, as few as hold them, and makes the
/// last page the smallest power-of-two square that holds what's left for it.
///
/// A rectangle larger than a page holds takes the largest side a page holds. When the rectangles need more than
/// `max_pages` pages, every side shrinks by the same factor until they don't. Returns `None` when even rectangles one
/// texel wide need more pages, which only a level with more primitives than `max_pages` pages have texels reaches.
pub(crate) fn layout(sides: &[u32], max_extent: u32, max_pages: usize) -> Option<Atlas> {
	let largest = max_extent - 2 * RECTANGLE_BORDER;
	let clamped = sides.iter().filter(|&&side| side > largest).count();
	// Largest first, so each row's first rectangle is its tallest. Shrinking keeps the order.
	let mut order: Vec<u32> = (0..sides.len() as u32).collect();
	order.sort_by_key(|&index| Reverse(sides[index as usize]));
	let mut scaled = vec![0; sides.len()];
	let mut rectangles = vec![Rectangle::default(); sides.len()];
	let mut factor = 1.0f32;
	for _ in 0..SHRINK_ROUNDS {
		for (scaled, &side) in scaled.iter_mut().zip(sides) {
			*scaled = ((side.min(largest) as f32 * factor).floor() as u32).max(1);
		}
		let page_count = place_rows(&scaled, &order, max_extent, 0, &mut rectangles)
			.expect("every side is clamped to what a page holds");
		if page_count <= max_pages {
			let pages = shrink_last_page(&scaled, &order, max_extent, page_count, &mut rectangles);
			return Some(Atlas {
				pages,
				rectangles,
				shrunk: (factor < 1.0).then_some(factor),
				clamped,
			});
		}
		if scaled.iter().all(|&side| side == 1) {
			return None;
		}
		// Area grows with the square of the side; rows waste a little of each page, so a little more converges in a
		// few rounds.
		factor *= (max_pages as f32 / page_count as f32).sqrt().min(0.98);
	}
	None
}

/// Places the rectangles of `sides` in `order` in rows across pages of `extent` texels, numbered from `first_page`,
/// writes each one's place to `rectangles`, and returns how many pages they took, or `None` when a rectangle is wider
/// than a page holds.
///
/// Each rectangle keeps [`RECTANGLE_BORDER`] texels on every side. A row ends when the next rectangle doesn't fit its
/// width, and a page when the next row doesn't fit its height. `order` lists the largest first, so a row's first
/// rectangle is its tallest and every later one fits under it.
fn place_rows(sides: &[u32], order: &[u32], extent: u32, first_page: u32, rectangles: &mut [Rectangle]) -> Option<usize> {
	let (mut page, mut x, mut y, mut row_height) = (first_page, 0, 0, 0);
	for &index in order {
		let side = sides[index as usize];
		let padded = side + 2 * RECTANGLE_BORDER;
		if padded > extent {
			return None;
		}
		if x > 0 && x + padded > extent {
			y += row_height;
			x = 0;
			row_height = 0;
		}
		if y > 0 && y + padded > extent {
			page += 1;
			y = 0;
		}
		rectangles[index as usize] = Rectangle {
			page,
			x: x + RECTANGLE_BORDER,
			y: y + RECTANGLE_BORDER,
			side,
		};
		x += padded;
		row_height = row_height.max(padded);
	}
	Some(if order.is_empty() { 0 } else { (page - first_page) as usize + 1 })
}

/// Returns the side of each of `page_count` pages of `max_extent` texels, with the last one halved while the
/// rectangles `rectangles` places on it still fit one page, and places those rectangles in it again.
fn shrink_last_page(
	sides: &[u32],
	order: &[u32],
	max_extent: u32,
	page_count: usize,
	rectangles: &mut [Rectangle],
) -> Vec<u32> {
	let mut pages = vec![max_extent; page_count];
	let Some(last) = page_count.checked_sub(1) else {
		return pages;
	};
	let last_order: Vec<u32> = order
		.iter()
		.copied()
		.filter(|&index| rectangles[index as usize].page == last as u32)
		.collect();
	let mut extent = max_extent;
	while extent > 1 && place_rows(sides, &last_order, extent / 2, last as u32, rectangles) == Some(1) {
		extent /= 2;
	}
	// The trial that failed may have moved some rectangles, so they go back where the last extent that held them puts
	// them.
	place_rows(sides, &last_order, extent, last as u32, rectangles);
	pages[last] = extent;
	pages
}

#[cfg(test)]
mod tests {
	use super::*;

	/// Asserts every rectangle lies inside its page, a border from its edges, and twice that from every other one on
	/// the page.
	fn assert_apart(atlas: &Atlas) {
		for (index, a) in atlas.rectangles.iter().enumerate() {
			let extent = atlas.pages[a.page as usize];
			assert!(a.x >= 1 && a.y >= 1, "rectangle {index} keeps a texel from the edges");
			assert!(
				a.x + a.side < extent && a.y + a.side < extent,
				"rectangle {index} lies inside its page"
			);
			for b in atlas.rectangles[index + 1..].iter().filter(|b| b.page == a.page) {
				let apart =
					a.x + a.side + 2 <= b.x || b.x + b.side + 2 <= a.x || a.y + a.side + 2 <= b.y || b.y + b.side + 2 <= a.y;
				assert!(apart, "rectangles {a:?} and {b:?} keep two texels apart");
			}
		}
	}

	#[test]
	fn rectangles_hold_their_area_at_the_density_and_never_fall_below_the_smallest_side() {
		// A 4 × 4 m floor whose triangles cover a quarter of the UV square stands for 8 m per side of the square.
		assert_eq!(rectangle_side(16.0, 0.25, 8.0), 64);
		assert_eq!(rectangle_side(0.01, 1.0, 8.0), MIN_RECTANGLE_SIDE);
		assert_eq!(rectangle_side(0.0, 0.0, 8.0), MIN_RECTANGLE_SIDE);
	}

	#[test]
	fn a_few_rectangles_take_one_page_of_the_smallest_power_of_two_side() {
		let atlas = layout(&[64, 16, 16, 30], 2048, 15).expect("small rectangles fit");

		assert_eq!(atlas.shrunk, None);
		assert_eq!(atlas.pages.len(), 1);
		assert!(atlas.pages[0].is_power_of_two());
		assert!(atlas.pages[0] >= 64 + 2 && atlas.pages[0] <= 256);
		assert_apart(&atlas);
		assert_eq!(atlas.rectangles[1].side, 16);
		let [scale_u, scale_v, offset_u, offset_v] = atlas.scale_offset(0);
		assert_eq!([scale_u, scale_v], [64.0 / atlas.pages[0] as f32; 2]);
		assert_eq!(
			[offset_u, offset_v],
			[
				atlas.rectangles[0].x as f32 / atlas.pages[0] as f32,
				atlas.rectangles[0].y as f32 / atlas.pages[0] as f32
			]
		);
	}

	#[test]
	fn rectangles_past_one_page_spill_to_more_pages_at_their_own_density() {
		// Twenty 100-texel squares fill more than one 256-texel page, which holds two rows of two.
		let sides = [100; 20];
		let atlas = layout(&sides, 256, 15).expect("the rectangles fit in pages");

		assert_eq!(atlas.shrunk, None);
		assert_eq!(atlas.pages.len(), 5);
		assert!(atlas.rectangles.iter().all(|rectangle| rectangle.side == 100));
		assert_apart(&atlas);
	}

	#[test]
	fn only_the_last_page_is_smaller() {
		// Two full pages of four, then a page with one 100-texel square and a 20-texel one in a row.
		let mut sides = vec![100; 9];
		sides.push(20);
		let atlas = layout(&sides, 256, 15).expect("the rectangles fit in pages");

		let (last, full) = atlas.pages.split_last().expect("some pages");
		assert!(full.iter().all(|&page| page == 256));
		assert_eq!(*last, 128, "the smallest page that holds 102 + 22 texels in a row");
		assert_apart(&atlas);
	}

	#[test]
	fn a_rectangle_larger_than_a_page_takes_the_largest_side_a_page_holds() {
		let atlas = layout(&[1000, 16], 256, 15).expect("the rectangles fit in pages");

		assert_eq!(atlas.clamped, 1);
		assert_eq!(atlas.rectangles[0].side, 254);
		assert_eq!(atlas.shrunk, None);
		assert_apart(&atlas);
	}

	#[test]
	fn rectangles_past_the_most_pages_shrink_by_one_factor() {
		let sides = [100; 20];
		let atlas = layout(&sides, 256, 2).expect("the rectangles shrink until they fit");

		assert!(atlas.pages.len() <= 2);
		let factor = atlas.shrunk.expect("the rectangles shrank");
		for (rectangle, side) in atlas.rectangles.iter().zip(sides) {
			assert_eq!(rectangle.side, ((side as f32 * factor).floor() as u32).max(1));
		}
		assert_apart(&atlas);
	}

	#[test]
	fn a_level_without_lit_primitives_has_no_pages() {
		let atlas = layout(&[], 2048, 15).expect("nothing fits");

		assert!(atlas.pages.is_empty());
	}
}
