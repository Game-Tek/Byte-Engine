//! Packing flattened charts into one square of texels: each chart's footprint, first-fit placement, and the density
//! loop. The Packing table of the asset reference's Lightmap UVs section defines each rule.

use super::maps::ChartMap;
use super::{LightmapUvSettings, place_lightmap_charts};

/// The density charts pack at again, relative to the last, when one of them finds no position.
const DENSITY_STEP: f64 = 0.9;
/// The densities tried before the charts lie in rows instead.
const DENSITY_ROUNDS: usize = 32;

/// The `Placement` struct is where packing put one chart, so its map's points become lightmap UVs.
#[derive(Clone, Copy)]
pub(super) struct Placement {
	/// The UV of the map's origin, after the turn.
	offset: [f64; 2],
	/// The UV per unit of the map.
	scale: f64,
	/// The map's height, when the chart is turned a quarter turn.
	turn: Option<f64>,
}

impl Placement {
	/// Returns the lightmap UV of `point`, a point of the chart's map.
	pub(super) fn uv(&self, point: [f64; 2]) -> [f32; 2] {
		// A quarter turn counterclockwise, moved back to the positive quadrant.
		let point = self.turn.map_or(point, |height| [height - point[1], point[0]]);
		[0, 1].map(|axis| (point[axis] * self.scale + self.offset[axis]) as f32)
	}
}

/// The `Packing` struct is a primitive's charts placed in one square.
pub(super) struct Packing {
	/// Each chart's placement, in chart order.
	pub(super) placements: Vec<Placement>,
	/// Whether the charts lie in rows because no density fit them.
	pub(super) cramped: bool,
}

/// The `Bitmap` struct is a grid of texels, one bit each, in rows of whole 64-bit words.
#[derive(Default)]
struct Bitmap {
	width: usize,
	height: usize,
	/// The words of one row.
	words: usize,
	bits: Vec<u64>,
}

impl Bitmap {
	/// Clears the bitmap to `width` × `height` texels, keeping its memory.
	fn reset(&mut self, width: usize, height: usize) {
		self.width = width;
		self.height = height;
		self.words = width.div_ceil(64);
		self.bits.clear();
		self.bits.resize(self.words * height, 0);
	}

	fn set(&mut self, x: usize, y: usize) {
		self.bits[y * self.words + x / 64] |= 1 << (x % 64);
	}

	fn row(&self, y: usize) -> &[u64] {
		&self.bits[y * self.words..(y + 1) * self.words]
	}
}

/// The `Packer` struct keeps the bitmaps of a primitive's packing across its charts and densities.
#[derive(Default)]
struct Packer {
	/// The texels of the square that the footprints of placed charts take.
	occupied: Bitmap,
	covered: Bitmap,
	/// One footprint per orientation: as is, and turned.
	footprints: [Bitmap; 2],
}

/// Places the charts of `maps`, whose faces are `charts` and whose corners' points `corner_locals` names, in one square
/// of `resolution` texels per side.
pub(super) fn pack(
	maps: &[ChartMap],
	charts: &[(Vec<u32>, [f64; 3])],
	corner_locals: &[u32],
	resolution: u32,
) -> Packing {
	let resolution = resolution as usize;
	let extents: Vec<[f64; 2]> = maps.iter().map(extent).collect();
	// The largest rectangle first; a stable sort keeps the lowest index first among equals.
	let mut order: Vec<usize> = (0..maps.len()).collect();
	order.sort_by(|&a, &b| (extents[b][0] * extents[b][1]).total_cmp(&(extents[a][0] * extents[a][1])));

	let area: f64 = maps.iter().map(|map| map.area).sum();
	let mut density = if area > 0.0 { resolution as f64 / area.sqrt() } else { 1.0 };
	let mut packer = Packer::default();
	let mut placements = vec![
		Placement {
			offset: [0.0; 2],
			scale: 0.0,
			turn: None,
		};
		maps.len()
	];
	for _ in 0..DENSITY_ROUNDS {
		packer.occupied.reset(resolution, resolution);
		let fits = order.iter().all(|&chart| {
			match packer.place(&maps[chart], extents[chart], &charts[chart].0, corner_locals, density, resolution) {
				Some(placement) => {
					placements[chart] = placement;
					true
				}
				None => false,
			}
		});
		if fits {
			return Packing {
				placements,
				cramped: false,
			};
		}
		density *= DENSITY_STEP;
	}
	Packing {
		placements: place_in_rows(&extents, resolution as u32),
		cramped: true,
	}
}

/// Returns the extent of the rectangle around `map`, whose points start at the origin.
fn extent(map: &ChartMap) -> [f64; 2] {
	map.points.iter().fold([0.0f64; 2], |extent, point| [extent[0].max(point[0]), extent[1].max(point[1])])
}

impl Packer {
	/// Places one chart at `density`, as is or turned, and marks its footprint taken, or returns `None` when it finds no
	/// position.
	fn place(
		&mut self,
		map: &ChartMap,
		extent: [f64; 2],
		faces: &[u32],
		corner_locals: &[u32],
		density: f64,
		resolution: usize,
	) -> Option<Placement> {
		// Map points land one texel in from the footprint's corner, so the ring around the covered texels fits.
		let local = |turned: bool| {
			let turn = turned.then_some(extent[1]);
			Placement {
				offset: [1.0; 2],
				scale: density,
				turn,
			}
		};
		let mut best: Option<(usize, usize, usize, bool)> = None;
		for (orientation, turned) in [false, true].into_iter().enumerate() {
			let transform = local(turned);
			if !self.footprint(map, faces, corner_locals, transform, orientation, resolution) {
				return None;
			}
			let footprint = &self.footprints[orientation];
			if let Some((x, y)) = first_fit(&self.occupied, footprint) {
				let reach = (x + footprint.width).max(y + footprint.height);
				if best.is_none_or(|(least, ..)| reach < least) {
					best = Some((reach, x, y, turned));
				}
			}
		}
		let (_, x, y, turned) = best?;
		let orientation = turned as usize;
		stamp(&mut self.occupied, &self.footprints[orientation], x, y);
		let resolution = resolution as f64;
		Some(Placement {
			offset: [(x as f64 + 1.0) / resolution, (y as f64 + 1.0) / resolution],
			scale: density / resolution,
			turn: turned.then_some(extent[1]),
		})
	}

	/// Rasterizes the chart's covered texels under `transform` and writes its footprint to `footprints[orientation]`, or
	/// returns `false` when the footprint is wider or taller than the square.
	fn footprint(
		&mut self,
		map: &ChartMap,
		faces: &[u32],
		corner_locals: &[u32],
		transform: Placement,
		orientation: usize,
		resolution: usize,
	) -> bool {
		let mut maximum = [0.0f64; 2];
		for &point in &map.points {
			let point = transform.uv(point);
			maximum = [0, 1].map(|axis| maximum[axis].max(f64::from(point[axis])));
		}
		// The covered texels span 1 through the floor of the maximum, and the footprint one more on each side.
		let [width, height] = maximum.map(|value| value.floor() as usize + 2);
		if width > resolution || height > resolution {
			return false;
		}
		self.covered.reset(width, height);
		for &face in faces {
			let corners = [0, 1, 2].map(|corner| {
				let point = map.points[corner_locals[face as usize * 3 + corner] as usize];
				transform.uv(point).map(f64::from)
			});
			cover_triangle(&mut self.covered, corners);
		}
		dilate(&self.covered, &mut self.footprints[orientation]);
		true
	}
}

/// Marks every texel of `bitmap` that the closed triangle `corners` overlaps, edges and corners included.
///
/// It tests each texel of the triangle's bounds by separating axes: the two axes and the three edges' normals.
fn cover_triangle(bitmap: &mut Bitmap, corners: [[f64; 2]; 3]) {
	let low = [0, 1].map(|axis| corners.iter().map(|corner| corner[axis]).fold(f64::INFINITY, f64::min));
	let high = [0, 1].map(|axis| corners.iter().map(|corner| corner[axis]).fold(f64::NEG_INFINITY, f64::max));
	let first = low.map(|value| value.floor().max(0.0) as usize);
	let last = [
		(high[0].floor() as usize).min(bitmap.width - 1),
		(high[1].floor() as usize).min(bitmap.height - 1),
	];
	// Each edge's normal and the interval the triangle projects to on it.
	let axes = [0, 1, 2].map(|edge| {
		let (a, b, c) = (corners[edge], corners[(edge + 1) % 3], corners[(edge + 2) % 3]);
		let normal = [b[1] - a[1], a[0] - b[0]];
		let [from, to] = [a, c].map(|point| normal[0] * point[0] + normal[1] * point[1]);
		(normal, from.min(to), from.max(to))
	});
	for y in first[1]..=last[1] {
		for x in first[0]..=last[0] {
			let center = [x as f64 + 0.5, y as f64 + 0.5];
			let separated = axes.iter().any(|&(normal, low, high)| {
				let middle = normal[0] * center[0] + normal[1] * center[1];
				let radius = (normal[0].abs() + normal[1].abs()) * 0.5;
				middle + radius < low || middle - radius > high
			});
			if !separated {
				bitmap.set(x, y);
			}
		}
	}
}

/// Writes to `footprint` the texels of `covered` and every texel that touches one of them, diagonals included.
fn dilate(covered: &Bitmap, footprint: &mut Bitmap) {
	footprint.reset(covered.width, covered.height);
	let words = covered.words;
	for y in 0..covered.height {
		let row = covered.row(y);
		// The row with each texel's left and right neighbors.
		for word in 0..words {
			let carry_in = if word > 0 { row[word - 1] >> 63 } else { 0 };
			let carry_out = if word + 1 < words { row[word + 1] << 63 } else { 0 };
			let widened = row[word] | (row[word] << 1) | carry_in | (row[word] >> 1) | carry_out;
			for target in y.saturating_sub(1)..=(y + 1).min(covered.height - 1) {
				footprint.bits[target * words + word] |= widened;
			}
		}
	}
	// A texel past the width in the last word would make the footprint wider than it is.
	let tail = covered.width % 64;
	if tail != 0 {
		let mask = (1u64 << tail) - 1;
		for y in 0..covered.height {
			footprint.bits[y * words + words - 1] &= mask;
		}
	}
}

/// Returns the first position, by row then column, at which `footprint` lies inside `occupied` and shares no texel with
/// it.
fn first_fit(occupied: &Bitmap, footprint: &Bitmap) -> Option<(usize, usize)> {
	if footprint.width > occupied.width || footprint.height > occupied.height {
		return None;
	}
	for y in 0..=occupied.height - footprint.height {
		for x in 0..=occupied.width - footprint.width {
			if fits(occupied, footprint, x, y) {
				return Some((x, y));
			}
		}
	}
	None
}

/// Returns whether `footprint` at `x`, `y` shares no texel with `occupied`.
fn fits(occupied: &Bitmap, footprint: &Bitmap, x: usize, y: usize) -> bool {
	let shift = x % 64;
	for row in 0..footprint.height {
		let target = occupied.row(y + row);
		for (word, &bits) in footprint.row(row).iter().enumerate() {
			if bits == 0 {
				continue;
			}
			let base = x / 64 + word;
			if target[base] & (bits << shift) != 0 {
				return false;
			}
			if shift > 0 && base + 1 < occupied.words && target[base + 1] & (bits >> (64 - shift)) != 0 {
				return false;
			}
		}
	}
	true
}

/// Marks the texels of `footprint` at `x`, `y` taken in `occupied`.
fn stamp(occupied: &mut Bitmap, footprint: &Bitmap, x: usize, y: usize) {
	let shift = x % 64;
	let words = occupied.words;
	for row in 0..footprint.height {
		let target = (y + row) * words;
		for word in 0..footprint.words {
			let bits = footprint.bits[row * footprint.words + word];
			let base = x / 64 + word;
			if bits == 0 {
				continue;
			}
			occupied.bits[target + base] |= bits << shift;
			if shift > 0 && base + 1 < words {
				occupied.bits[target + base + 1] |= bits >> (64 - shift);
			}
		}
	}
}

/// Places charts of `extents` in rows, tallest first, scaled together into the unit square.
fn place_in_rows(extents: &[[f64; 2]], resolution: u32) -> Vec<Placement> {
	let extents: Vec<[f32; 2]> = extents.iter().map(|extent| extent.map(|value| value as f32)).collect();
	let settings = LightmapUvSettings {
		resolution,
		..LightmapUvSettings::default()
	};
	let (placed, side) = place_lightmap_charts(&extents, settings);
	let side = f64::from(side);
	placed
		.iter()
		.map(|corner| Placement {
			offset: corner.map(|value| f64::from(value) / side),
			scale: 1.0 / side,
			turn: None,
		})
		.collect()
}
