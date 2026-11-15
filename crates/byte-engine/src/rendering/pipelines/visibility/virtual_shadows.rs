//! Virtual shadow maps: the alternative to cascades for the first shadowed directional light.
//!
//! The light's shadow is a clipmap of [`VIRTUAL_SHADOW_LEVELS`] levels around the first sink's camera. Each level is a
//! virtual map of [`VIRTUAL_SHADOW_RESOLUTION`] texels a side, cut into pages, and only the pages receivers need hold
//! depth, in a pool that keeps them across frames. The design follows *Virtual Shadow Maps* (Sakmary, Ryan, Hall,
//! Lustri) in *GPU Zen 3*, and its companion code; see the
//! [design note](/docs/develop/rendering/visibility/virtual-shadow-maps) for where the engine departs from it.
//!
//! This module places the levels on the CPU, a pure function of the light, the camera, and the previous placement.
//! [`super::render_pass::virtual_shadows::VirtualShadowMaps`] owns the GPU state and records the page passes.

use math::{Matrix, Point, UnitVector, inverse};
use maths_rs::{Vec3f, Vec4f};

use crate::rendering::View;

/// Levels in a light's clipmap. Level `l` spans the first level's size times `2^l`.
pub(crate) const VIRTUAL_SHADOW_LEVELS: usize = 16;
/// Texels per side of every level's virtual map.
pub(crate) const VIRTUAL_SHADOW_RESOLUTION: u32 = 4096;
/// Texels per side of a page.
pub(crate) const VIRTUAL_SHADOW_PAGE_TEXELS: u32 = 128;
/// Pages per side of a level, and of its page table.
pub(crate) const VIRTUAL_SHADOW_PAGES: u32 = VIRTUAL_SHADOW_RESOLUTION / VIRTUAL_SHADOW_PAGE_TEXELS;
/// How far each level's depth range reaches toward and away from the light, from the camera, in meters.
pub(crate) const VIRTUAL_SHADOW_DEPTH_REACH: f32 = 1000.0;
/// The most pages the pool holds: 128 pages a side keep the pool within 16,384 texels, which every backend creates.
pub(crate) const MAX_VIRTUAL_SHADOW_POOL_PAGES: u32 = 16384;
/// The bias added to page grid coordinates on the GPU, a multiple of [`VIRTUAL_SHADOW_PAGES`], so they stay positive and
/// a page's table slot is the remainder of an unsigned division. It covers grids of a million pages each way.
pub(crate) const VIRTUAL_SHADOW_ORIGIN_BIAS: i32 = 1 << 20;

/// The `DirectionalShadowTechnique` enum chooses how directional lights cast shadows.
///
/// Pass it to [`super::VisibilityPipelineSettings::with_directional_shadow_technique`], or set
/// `render.directional-shadows.technique` at startup.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum DirectionalShadowTechnique {
	/// Four cascades per light, up to four lights, fitted to what the camera sees and redrawn on a texel budget.
	#[default]
	Cascades,
	/// One virtual shadow map, for the first directional light: 16 levels whose pages hold depth only where receivers
	/// need it, and keep it until something changes it.
	Virtual,
}

impl std::str::FromStr for DirectionalShadowTechnique {
	type Err = String;

	fn from_str(name: &str) -> Result<Self, Self::Err> {
		match name {
			"cascades" => Ok(Self::Cascades),
			"virtual" => Ok(Self::Virtual),
			_ => Err(format!(
				"Directional shadow technique was not set. The most likely cause is that `{name}` is neither `cascades` nor `virtual`."
			)),
		}
	}
}

/// The `VirtualShadowSettings` struct sizes a virtual shadow map: how large its finest level is, how coarse a level
/// receivers pick, and how many pages hold depth at once.
///
/// Pass it to [`super::VisibilityPipelineSettings::with_virtual_shadow_settings`].
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct VirtualShadowSettings {
	first_level_size: f32,
	level_bias: f32,
	pool_pages: u32,
}

impl Default for VirtualShadowSettings {
	fn default() -> Self {
		Self {
			first_level_size: 4.0,
			level_bias: 1.0,
			pool_pages: 4096,
		}
	}
}

impl VirtualShadowSettings {
	/// Returns settings whose finest level spans `first_level_size` meters, whose receivers measure their distance
	/// scaled by `level_bias`, and whose pool holds `pool_pages` pages.
	///
	/// # Errors
	///
	/// Returns an error when the size or bias isn't a positive finite number, or the pool holds no pages or more than
	/// 16,384, which keeps the pool within 16,384 texels a side.
	pub fn new(first_level_size: f32, level_bias: f32, pool_pages: u32) -> Result<Self, String> {
		if !(first_level_size.is_finite() && first_level_size > 0.0) {
			return Err(format!(
				"Virtual shadow level size was not set. The most likely cause is that {first_level_size} is not a positive number of meters. Set `render.directional-shadows.first-level-size` to a size such as 4."
			));
		}
		if !(level_bias.is_finite() && level_bias > 0.0) {
			return Err(format!(
				"Virtual shadow level bias was not set. The most likely cause is that {level_bias} is not a positive number. Set `render.directional-shadows.level-bias` to a bias such as 1."
			));
		}
		if !(1..=MAX_VIRTUAL_SHADOW_POOL_PAGES).contains(&pool_pages) {
			return Err(format!(
				"Virtual shadow pool was not set. The most likely cause is that {pool_pages} pages lie outside 1 through {MAX_VIRTUAL_SHADOW_POOL_PAGES}. Set `render.directional-shadows.pool-pages` to a count in that range, such as 4096."
			));
		}
		Ok(Self {
			first_level_size,
			level_bias,
			pool_pages,
		})
	}

	/// Returns the side of the finest level, in meters.
	pub fn first_level_size(&self) -> f32 {
		self.first_level_size
	}

	/// Returns the scale on the distance the receiver level rule measures.
	pub fn level_bias(&self) -> f32 {
		self.level_bias
	}

	/// Returns how many pages hold depth at once.
	pub fn pool_pages(&self) -> u32 {
		self.pool_pages
	}

	/// Returns the pool's pages per row: the pool is a square of pages, rounded up, so its texels stay within a
	/// texture side every backend creates.
	pub(crate) fn pool_pages_per_row(&self) -> u32 {
		(self.pool_pages as f32).sqrt().ceil() as u32
	}
}

/// The `VirtualShadowLevels` struct is one frame's placement of a light's clipmap: each level's view and where its
/// pages lie on the world-fixed page grid.
///
/// Make it with [`place_virtual_shadow_levels`] each frame, from the previous frame's placement, and write it into the
/// views and the page passes' frame data.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct VirtualShadowLevels {
	/// Each level's orthographic view, looking along the light.
	pub(crate) views: [View; VIRTUAL_SHADOW_LEVELS],
	/// Each level's first page column and row on the level's world-fixed page grid. Columns grow along the light
	/// view's x axis, rows against its y axis, as texel rows grow down the map.
	pub(crate) origin_pages: [[i32; 2]; VIRTUAL_SHADOW_LEVELS],
	/// Each level's origin pages last frame, or this frame's when there was none.
	pub(crate) previous_origin_pages: [[i32; 2]; VIRTUAL_SHADOW_LEVELS],
	/// The light-space depth of every level's near plane, which follows the camera along the light.
	pub(crate) front: f32,
	/// Whether every page must be freed: there was no previous placement, or the light turned.
	pub(crate) free_all: bool,
	/// The direction the light travels, which the next frame compares against.
	pub(crate) light_direction: UnitVector,
}

/// Returns the side, in meters, of level `level` of a clipmap whose finest level spans `first_level_size`.
pub(crate) fn virtual_shadow_level_size(first_level_size: f32, level: usize) -> f32 {
	first_level_size * (1u32 << level) as f32
}

/// Places a light's clipmap around `camera_position` for this frame.
///
/// Every level is centered on the camera across the light, snapped to whole pages of its own world-fixed grid, so
/// moving the camera scrolls pages instead of moving texels. Its depth range reaches
/// [`VIRTUAL_SHADOW_DEPTH_REACH`] toward and away from the light from the camera's own depth along the light, without
/// snapping: each page keeps the depth it was drawn with, which receivers correct for. `previous` is last frame's
/// placement; without one, or when the light turned since, every page is freed.
pub(crate) fn place_virtual_shadow_levels(
	light_direction: UnitVector,
	camera_position: Point,
	first_level_size: f32,
	previous: Option<&VirtualShadowLevels>,
) -> VirtualShadowLevels {
	// The light's orientation at the world origin, as the cascades take it. Light-space coordinates relative to the
	// origin are fixed in the world, so page grids in them stay still as the camera moves.
	let light_rotation: Matrix =
		View::new_orthographic(-1.0, 1.0, -1.0, 1.0, 0.0, 1.0, Point::origin(), light_direction).view();
	let camera = light_rotation * Vec4f::from((camera_position.into_maths(), 1.0));
	let front = camera.z - VIRTUAL_SHADOW_DEPTH_REACH;
	let depth = 2.0 * VIRTUAL_SHADOW_DEPTH_REACH;
	let to_world = inverse(light_rotation);

	let mut origin_pages = [[0; 2]; VIRTUAL_SHADOW_LEVELS];
	let views = std::array::from_fn(|level| {
		let size = virtual_shadow_level_size(first_level_size, level);
		let page = size / VIRTUAL_SHADOW_PAGES as f32;
		// The page corner nearest the camera centers the level, so the camera lies within half a page of its center.
		let center_column = (camera.x / page).round() as i32;
		let center_row = (-camera.y / page).round() as i32;
		let half_pages = VIRTUAL_SHADOW_PAGES as i32 / 2;
		origin_pages[level] = [center_column - half_pages, center_row - half_pages];
		let center = to_world * Vec4f::new(center_column as f32 * page, -(center_row as f32) * page, front, 1.0);
		let half = size / 2.0;
		View::new_orthographic(
			-half,
			half,
			-half,
			half,
			0.0,
			depth,
			Point::from_maths(Vec3f::new(center.x, center.y, center.z)),
			light_direction,
		)
	});
	let turned = previous.is_none_or(|previous| previous.light_direction != light_direction);
	VirtualShadowLevels {
		views,
		origin_pages,
		previous_origin_pages: match previous {
			Some(previous) if !turned => previous.origin_pages,
			_ => origin_pages,
		},
		front,
		free_all: turned,
		light_direction,
	}
}

/// Returns the smallest range that covers both `a` and `b`, where an empty range covers nothing. The manager merges the
/// instance records each update rewrites with it, until a consumer such as the virtual shadow map's invalidation reads
/// them.
pub(crate) fn range_union(a: std::ops::Range<usize>, b: std::ops::Range<usize>) -> std::ops::Range<usize> {
	match (a.is_empty(), b.is_empty()) {
		(true, true) => 0..0,
		(false, true) => a,
		(true, false) => b,
		(false, false) => a.start.min(b.start)..a.end.max(b.end),
	}
}

/// The `ShaderVirtualShadowFrame` struct is the frame data every page pass and the samplers read: where each level's
/// pages lie, what the pool holds, and the receiver level rule's constants. It matches `VirtualShadowFrame` in
/// `layouts/virtual_shadows.besl`.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, bytemuck::Pod, bytemuck::Zeroable)]
pub(crate) struct ShaderVirtualShadowFrame {
	/// Each level's first page column and row this frame, then last frame's, each plus [`VIRTUAL_SHADOW_ORIGIN_BIAS`].
	pub(crate) origin_pages: [[u32; 4]; VIRTUAL_SHADOW_LEVELS],
	/// The camera's world position in xyz, and how far from it receivers are shadowed in w.
	pub(crate) camera: [f32; 4],
	/// The first level's size, the level bias, the light-space depth of the near plane, and the depth range, in meters.
	pub(crate) constants: [f32; 4],
	/// Nonzero to free every page this frame.
	pub(crate) free_all: u32,
	/// The pool's pages per row, and how many pages it holds.
	pub(crate) pool_pages_per_row: u32,
	pub(crate) pool_pages: u32,
	/// The instance records that may have changed since the pages last saw them, from the first up to the end, and how
	/// many records the scene holds.
	pub(crate) changed_first: u32,
	pub(crate) changed_end: u32,
	pub(crate) instance_count: u32,
	pub(crate) _padding: [u32; 2],
}

impl ShaderVirtualShadowFrame {
	/// Returns the frame data of `levels` for a pool sized by `settings`, a camera at `camera_position`, receivers out
	/// to `distance` meters, the instance records in `changed` that may have changed since the pages last saw them, and
	/// `instance_count` records in the scene.
	pub(crate) fn new(
		levels: &VirtualShadowLevels,
		settings: VirtualShadowSettings,
		camera_position: Point,
		distance: f32,
		changed: std::ops::Range<u32>,
		instance_count: u32,
	) -> Self {
		let camera = camera_position.into_maths();
		Self {
			origin_pages: std::array::from_fn(|level| {
				let [x, y] = levels.origin_pages[level];
				let [previous_x, previous_y] = levels.previous_origin_pages[level];
				[x, y, previous_x, previous_y].map(|page| (page + VIRTUAL_SHADOW_ORIGIN_BIAS) as u32)
			}),
			camera: [camera.x, camera.y, camera.z, distance],
			constants: [
				settings.first_level_size,
				settings.level_bias,
				levels.front,
				2.0 * VIRTUAL_SHADOW_DEPTH_REACH,
			],
			free_all: u32::from(levels.free_all),
			pool_pages_per_row: settings.pool_pages_per_row(),
			pool_pages: settings.pool_pages,
			changed_first: changed.start,
			changed_end: changed.end,
			instance_count,
			_padding: [0; 2],
		}
	}
}

#[cfg(test)]
mod tests {
	use math::Vector;

	use super::*;

	fn sun() -> UnitVector {
		Vector::new(0.3, -1.0, 0.2).normalized().expect("nonzero light direction")
	}

	/// Projects a world point into `view`'s normalized device coordinates.
	fn ndc(view: &View, point: [f32; 3]) -> [f32; 3] {
		let clip = view.view_projection() * Vec4f::new(point[0], point[1], point[2], 1.0);
		[clip.x / clip.w, clip.y / clip.w, clip.z / clip.w]
	}

	#[test]
	fn every_level_centers_the_camera_within_half_a_page_and_doubles_the_one_before() {
		let camera = Point::new(13.7, 2.0, -41.3);
		let levels = place_virtual_shadow_levels(sun(), camera, 4.0, None);
		let page_ndc = 2.0 / VIRTUAL_SHADOW_PAGES as f32;
		for (level, view) in levels.views.iter().enumerate() {
			let [x, y, z] = ndc(view, [13.7, 2.0, -41.3]);
			assert!(
				x.abs() <= page_ndc * 0.5 + 0.0001 && y.abs() <= page_ndc * 0.5 + 0.0001,
				"level {level}: {x} {y}"
			);
			// The camera lies halfway through the depth range, which is reversed: the light's side is one.
			assert!((z - 0.5).abs() < 0.001, "level {level}: depth {z}");
			// The first row of an orthographic view-projection maps one meter across to its share of two units.
			let matrix = view.view_projection();
			let width = 2.0 / (matrix[0] * matrix[0] + matrix[1] * matrix[1] + matrix[2] * matrix[2]).sqrt();
			assert!(
				(width - virtual_shadow_level_size(4.0, level)).abs() < width * 0.0001,
				"level {level}: {width}"
			);
		}
	}

	#[test]
	fn page_corners_stay_on_a_world_fixed_grid_as_the_camera_moves() {
		let first = place_virtual_shadow_levels(sun(), Point::new(0.0, 0.0, 0.0), 4.0, None);
		let moved = place_virtual_shadow_levels(sun(), Point::new(3.1, 0.4, -2.2), 4.0, Some(&first));
		for level in 0..VIRTUAL_SHADOW_LEVELS {
			// The world origin falls on the same texel position relative to each level's first page.
			let texel = |levels: &VirtualShadowLevels| {
				let [x, y, _] = ndc(&levels.views[level], [0.0, 0.0, 0.0]);
				let texels = VIRTUAL_SHADOW_RESOLUTION as f32;
				let page = VIRTUAL_SHADOW_PAGE_TEXELS as f32;
				let [origin_x, origin_y] = levels.origin_pages[level];
				[
					(x * 0.5 + 0.5) * texels + origin_x as f32 * page,
					(0.5 - y * 0.5) * texels + origin_y as f32 * page,
				]
			};
			// Within f32 rounding of the view matrices: a twentieth of a texel, a millimeter's twentieth at level zero.
			let [before, after] = [texel(&first), texel(&moved)];
			assert!(
				(before[0] - after[0]).abs() < 0.05 && (before[1] - after[1]).abs() < 0.05,
				"level {level}: {before:?} {after:?}"
			);
			assert_eq!(moved.previous_origin_pages[level], first.origin_pages[level]);
		}
		assert!(!moved.free_all);
	}

	#[test]
	fn moving_along_the_light_moves_only_the_depth_range() {
		let light = sun();
		let first = place_virtual_shadow_levels(light, Point::new(1.0, 2.0, 3.0), 4.0, None);
		let step = light.into_maths() * 7.0;
		let moved = place_virtual_shadow_levels(light, Point::new(1.0 + step.x, 2.0 + step.y, 3.0 + step.z), 4.0, Some(&first));
		assert_eq!(moved.origin_pages, first.origin_pages);
		assert!(
			(moved.front - first.front - 7.0).abs() < 0.001,
			"{} {}",
			first.front,
			moved.front
		);
	}

	#[test]
	fn the_first_placement_and_a_turned_light_free_every_page() {
		let first = place_virtual_shadow_levels(sun(), Point::origin(), 4.0, None);
		assert!(first.free_all);
		let still = place_virtual_shadow_levels(sun(), Point::origin(), 4.0, Some(&first));
		assert!(!still.free_all);
		let turned_direction = Vector::new(0.31, -1.0, 0.2).normalized().expect("nonzero light direction");
		let turned = place_virtual_shadow_levels(turned_direction, Point::origin(), 4.0, Some(&still));
		assert!(turned.free_all);
		assert_eq!(turned.previous_origin_pages, turned.origin_pages);
	}

	#[test]
	fn range_union_covers_both_ranges_and_ignores_empty_ones() {
		assert_eq!(range_union(0..0, 10..10), 0..0);
		assert_eq!(range_union(3..5, 10..10), 3..5);
		assert_eq!(range_union(0..0, 8..10), 8..10);
		assert_eq!(range_union(2..4, 10..12), 2..12);
	}

	#[test]
	fn settings_reject_values_they_cannot_use() {
		assert!(VirtualShadowSettings::new(0.0, 1.0, 4096).is_err());
		assert!(VirtualShadowSettings::new(4.0, f32::NAN, 4096).is_err());
		assert!(VirtualShadowSettings::new(4.0, 1.0, 0).is_err());
		assert!(VirtualShadowSettings::new(4.0, 1.0, MAX_VIRTUAL_SHADOW_POOL_PAGES + 1).is_err());
		let settings = VirtualShadowSettings::new(4.0, 1.0, 4096).expect("valid settings");
		assert_eq!(settings.pool_pages_per_row(), 64);
		assert_eq!(
			VirtualShadowSettings::new(4.0, 1.0, 4097)
				.expect("valid settings")
				.pool_pages_per_row(),
			65
		);
	}
}
