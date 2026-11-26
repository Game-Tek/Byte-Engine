//! Lightmap UV generation: a second set of texture coordinates per primitive, in which no two charts overlap, that
//! lightmaps store light at.
//!
//! Generation follows the clustered charts of [xatlas](https://github.com/jpcy/xatlas) (Jonathan Young): it cuts each
//! primitive into charts that grow while their faces stay close to the chart's normal and their boundary stays short
//! and straight ([`charts`]), flattens each chart orthographically or with a least squares conformal map (Lévy et al.,
//! 2002) ([`maps`]), and packs the charts' texel footprints into one square ([`atlas`]). Curved and scanned surfaces keep
//! large charts. Mesh generators whose charts are known, such as a box's faces, place them with
//! [`place_lightmap_charts`].

mod atlas;
mod charts;
mod maps;
mod surface;

use surface::{NONE, Surface};

/// The `LightmapUvSettings` struct lets an asset tune the lightmap UVs mesh processing gives its unskinned primitives.
///
/// Pass it to [`MeshProcessorSession::new`](super::MeshProcessorSession::new) through
/// [`MeshSettings::lightmap_uvs`](super::MeshSettings::lightmap_uvs). An asset overrides it through the `lightmap_uvs`
/// key of its BEAD file.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LightmapUvSettings {
	/// Where the lightmap UVs come from.
	pub source: LightmapUvSource,
	/// The texels per side the gaps between generated charts are measured at. Set it near the texels a lightmap gives
	/// the primitive, so neighboring charts don't share texels.
	pub resolution: u32,
}

impl Default for LightmapUvSettings {
	fn default() -> Self {
		Self {
			source: LightmapUvSource::Generated,
			resolution: 64,
		}
	}
}

/// The `LightmapUvSource` enum lets an asset choose whether mesh processing unwraps its lightmap UVs or reuses the UVs
/// its textures map with.
///
/// Reuse the texture UVs of a model whose texture layout is already unique, such as a photogrammetry scan, whose
/// noisy normals the generated charts split into tiny pieces.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum LightmapUvSource {
	/// Charts cut, flattened, and packed into one square by mesh processing.
	#[default]
	Generated,
	/// The primitive's texture UVs, as they are. They must lie in the unit square and not overlap.
	Texture,
}

/// The texels a chart keeps from its neighbors on each side, so two charts lie twice this far apart and a chart lies
/// this far from the square's edges.
const CHART_BORDER_TEXELS: f32 = 1.0;
/// The rounds of placement that grow the charts' border until it covers [`CHART_BORDER_TEXELS`] of the square.
const PLACEMENT_ROUNDS: usize = 32;

/// The `LightmapUvs` struct is one primitive's lightmap UV layout: the vertices it splits into and their coordinates.
pub(super) struct LightmapUvs {
	/// The source vertex of each new vertex, or empty when the layout keeps the source vertices.
	pub(super) xref: Vec<u32>,
	/// The lightmap UV of each new vertex, from 0 through 1.
	pub(super) uvs: Vec<[f32; 2]>,
	/// Whether the charts were too many to keep their gaps at the settings' resolution.
	pub(super) cramped: bool,
}

/// Lays out lightmap UVs for one primitive and rewrites `indices` to the vertices the layout splits into.
///
/// `positions` are the primitive's source vertices and `indices` its triangle list. A vertex that two charts share
/// becomes one vertex per chart; the returned [`LightmapUvs::xref`] names the source vertex of each, so every other
/// attribute is copied through it.
pub(super) fn generate_lightmap_uvs(positions: &[[f32; 3]], indices: &mut [u32], settings: LightmapUvSettings) -> LightmapUvs {
	let surface = Surface::new(positions, indices);
	let charts = charts::Segmentation::run(&surface);
	let mut corner_locals = vec![NONE; indices.len()];
	let mut scratch = maps::MapScratch::default();
	let maps: Vec<maps::ChartMap> = charts
		.iter()
		.map(|(faces, normal)| maps::map_chart(&surface, faces, *normal, &mut corner_locals, &mut scratch))
		.collect();
	let packing = atlas::pack(&maps, &charts, &corner_locals, settings.resolution);

	// One new vertex per source vertex and chart, in chart order and then the order of its corners.
	let mut new_of_source = vec![NONE; positions.len()];
	let mut xref = Vec::with_capacity(indices.len());
	let mut uvs = Vec::with_capacity(indices.len());
	for (chart, (faces, _)) in charts.iter().enumerate() {
		let first = xref.len();
		for &face in faces {
			for corner in face as usize * 3..face as usize * 3 + 3 {
				let source = indices[corner];
				let new = &mut new_of_source[source as usize];
				if *new == NONE {
					*new = xref.len() as u32;
					xref.push(source);
					let point = maps[chart].points[corner_locals[corner] as usize];
					uvs.push(packing.placements[chart].uv(point).map(|value| value.clamp(0.0, 1.0)));
				}
				indices[corner] = *new;
			}
		}
		// The next chart splits the source vertices this one took.
		for &source in &xref[first..] {
			new_of_source[source as usize] = NONE;
		}
	}
	LightmapUvs {
		xref,
		uvs,
		cramped: packing.cramped,
	}
}

/// Places charts of the given `extents`, in model units, in rows inside one square, and returns where each chart's
/// minimum corner lands and the square's side, in model units.
///
/// Mesh generators whose charts are known, such as a box's faces, lay out their lightmap UVs with it, and mesh
/// processing falls back to it when a primitive's charts don't pack: divide a chart's corner plus its placement by the
/// side. Charts lie two texels apart at the settings' resolution where they fit.
pub fn place_lightmap_charts(extents: &[[f32; 2]], settings: LightmapUvSettings) -> (Vec<[f32; 2]>, f32) {
	let mut placed = vec![[0.0; 2]; extents.len()];
	let side = place_charts(extents, &mut placed, settings.resolution);
	(placed, side)
}

/// Returns `uv`, a coordinate of a chart that fills the unit square, moved inside the square's border, so a generated
/// mesh with one chart keeps the gap from the edges that baked meshes keep.
pub fn inset_lightmap_uv(uv: [f32; 2], settings: LightmapUvSettings) -> [f32; 2] {
	let border = CHART_BORDER_TEXELS / settings.resolution as f32;
	uv.map(|value| border + value * (1.0 - 2.0 * border))
}

/// Places charts of `extents` in rows inside one square, tallest first, writes where each lands to `placed`, and
/// returns the square's side in model units.
///
/// The border a chart needs is a share of the square, which grows with the border, so placement repeats with the
/// border the last round's square asks for until the border covers [`CHART_BORDER_TEXELS`] at `resolution`, or the
/// rounds run out.
fn place_charts(extents: &[[f32; 2]], placed: &mut [[f32; 2]], resolution: u32) -> f32 {
	// Tallest first; charts of one height keep their order.
	let mut order: Vec<u32> = (0..extents.len() as u32).collect();
	order.sort_by(|&a, &b| extents[b as usize][1].total_cmp(&extents[a as usize][1]));

	let texels = resolution as f32;
	let mut border = 0.0f32;
	let mut side = 0.0f32;
	for _ in 0..PLACEMENT_ROUNDS {
		side = place_rows(extents, &order, placed, border);
		let needed = side * CHART_BORDER_TEXELS / texels;
		if side > 0.0 && border >= needed * 0.9999 {
			return side;
		}
		// A slightly larger border than asked converges in fewer rounds.
		border = needed * 1.01;
	}
	// When every chart is a point, any square holds them.
	if side > 0.0 { side } else { 1.0 }
}

/// Places charts of `extents` in `order` in rows, each padded by `border` on every side, writes where each lands to
/// `placed`, and returns the side of the square that holds the rows.
fn place_rows(extents: &[[f32; 2]], order: &[u32], placed: &mut [[f32; 2]], border: f32) -> f32 {
	let padded = |[width, height]: [f32; 2]| [width + 2.0 * border, height + 2.0 * border];
	let area: f32 = extents.iter().map(|&extent| padded(extent)[0] * padded(extent)[1]).sum();
	let widest = extents.iter().map(|&extent| padded(extent)[0]).fold(0.0f32, f32::max);
	// Rows as wide as a square of the charts' area come out close to square.
	let row_width = area.sqrt().max(widest);

	let (mut x, mut y, mut row_height, mut width) = (0.0f32, 0.0f32, 0.0f32, 0.0f32);
	for &chart in order {
		let [chart_width, chart_height] = padded(extents[chart as usize]);
		if x > 0.0 && x + chart_width > row_width {
			y += row_height;
			x = 0.0;
			row_height = 0.0;
		}
		placed[chart as usize] = [x + border, y + border];
		x += chart_width;
		width = width.max(x);
		row_height = row_height.max(chart_height);
	}
	width.max(y + row_height)
}

pub(super) fn find(parents: &mut [u32], mut node: u32) -> u32 {
	while parents[node as usize] != node {
		// Path halving keeps the trees shallow without recursion.
		let grandparent = parents[parents[node as usize] as usize];
		parents[node as usize] = grandparent;
		node = grandparent;
	}
	node
}

pub(super) fn union(parents: &mut [u32], a: u32, b: u32) {
	let (a, b) = (find(parents, a), find(parents, b));
	if a != b {
		parents[a.max(b) as usize] = a.min(b);
	}
}

#[cfg(test)]
mod tests {
	use super::{LightmapUvSettings, generate_lightmap_uvs};

	/// Returns a unit cube's 24 vertices, split per face as importers deliver them, and its 12 triangles.
	fn cube() -> (Vec<[f32; 3]>, Vec<u32>) {
		let mut positions = Vec::new();
		let mut indices = Vec::new();
		for axis in 0..3 {
			for side in [0.0, 1.0] {
				let base = positions.len() as u32;
				for corner in [[0.0, 0.0], [1.0, 0.0], [1.0, 1.0], [0.0, 1.0]] {
					let mut position = [0.0; 3];
					position[axis] = side;
					position[(axis + 1) % 3] = corner[0];
					position[(axis + 2) % 3] = corner[1];
					positions.push(position);
				}
				indices.extend([base, base + 1, base + 2, base, base + 2, base + 3]);
			}
		}
		(positions, indices)
	}

	/// Returns a bumpy square sheet like a scanned rock's face: a grid of `side` × `side` quads whose heights and
	/// in-plane positions carry deterministic noise, so neighboring triangles face slightly different ways.
	fn scanned_sheet(side: u32) -> (Vec<[f32; 3]>, Vec<u32>) {
		let noise = |x: u32, y: u32, salt: u32| {
			let hash = (x.wrapping_mul(73_856_093) ^ y.wrapping_mul(19_349_663) ^ salt.wrapping_mul(83_492_791)) % 1000;
			hash as f32 / 1000.0 - 0.5
		};
		let mut positions = Vec::new();
		for y in 0..=side {
			for x in 0..=side {
				positions.push([
					x as f32 + noise(x, y, 1) * 0.3,
					noise(x, y, 2) * 0.6,
					y as f32 + noise(x, y, 3) * 0.3,
				]);
			}
		}
		let mut indices = Vec::new();
		let row = side + 1;
		for y in 0..side {
			for x in 0..side {
				let corner = y * row + x;
				indices.extend([corner, corner + row, corner + row + 1, corner, corner + row + 1, corner + 1]);
			}
		}
		(positions, indices)
	}

	/// Rasterizes every triangle's lightmap UVs at `resolution` and returns, per texel, how many triangles cover its
	/// center, plus the texels each triangle covers.
	fn coverage(indices: &[u32], uvs: &[[f32; 2]], resolution: usize) -> Vec<Vec<usize>> {
		let mut owners = vec![Vec::new(); resolution * resolution];
		for (triangle, corners) in indices.as_chunks::<3>().0.iter().enumerate() {
			let [a, b, c] = corners.map(|corner| uvs[corner as usize].map(|value| value * resolution as f32));
			let edge = |p: [f32; 2], q: [f32; 2], point: [f32; 2]| {
				(q[0] - p[0]) * (point[1] - p[1]) - (q[1] - p[1]) * (point[0] - p[0])
			};
			for y in 0..resolution {
				for x in 0..resolution {
					let point = [x as f32 + 0.5, y as f32 + 0.5];
					let weights = [edge(a, b, point), edge(b, c, point), edge(c, a, point)];
					let inside = weights.iter().all(|&weight| weight >= 0.0) || weights.iter().all(|&weight| weight <= 0.0);
					if inside && edge(a, b, c) != 0.0 {
						owners[y * resolution + x].push(triangle);
					}
				}
			}
		}
		owners
	}

	/// Checks that the layout keeps every corner's source vertex, stays inside the square, keeps each triangle's area,
	/// and lets no more than the two triangles along a shared edge cover one texel center.
	fn assert_layout(positions: &[[f32; 3]], mut indices: Vec<u32>) {
		let source = indices.clone();
		let settings = LightmapUvSettings::default();
		let layout = generate_lightmap_uvs(&positions, &mut indices, settings);

		assert!(!layout.cramped);
		assert_eq!(layout.xref.len(), layout.uvs.len());
		for (new, old) in indices.iter().zip(&source) {
			assert_eq!(layout.xref[*new as usize], *old, "each corner keeps its source vertex");
		}
		for uv in &layout.uvs {
			assert!(
				uv.iter().all(|value| (0.0..=1.0).contains(value)),
				"UV {uv:?} leaves the square"
			);
		}
		for corners in indices.as_chunks::<3>().0 {
			let [a, b, c] = corners.map(|corner| layout.uvs[corner as usize]);
			let area = (b[0] - a[0]) * (c[1] - a[1]) - (c[0] - a[0]) * (b[1] - a[1]);
			assert!(area.abs() > 0.0, "a triangle with area keeps it");
		}
		// Overlapping charts share texel centers; triangles of one chart only share their edges.
		let owners = coverage(&indices, &layout.uvs, 256);
		for texel in &owners {
			assert!(texel.len() <= 2, "texel covered by triangles {texel:?}");
		}
	}

	/// Returns a grid of `columns` × `rows` quads over `point(u, v)`, with `u` and `v` from 0 through 1.
	fn grid(columns: u32, rows: u32, point: impl Fn(f32, f32) -> [f32; 3]) -> (Vec<[f32; 3]>, Vec<u32>) {
		let mut positions = Vec::new();
		for y in 0..=rows {
			for x in 0..=columns {
				positions.push(point(x as f32 / columns as f32, y as f32 / rows as f32));
			}
		}
		let mut indices = Vec::new();
		let row = columns + 1;
		for y in 0..rows {
			for x in 0..columns {
				let corner = y * row + x;
				indices.extend([corner, corner + 1, corner + row + 1, corner, corner + row + 1, corner + row]);
			}
		}
		(positions, indices)
	}

	/// Returns each triangle's area in the lightmap over its area on the surface.
	fn area_ratios(positions: &[[f32; 3]], source: &[u32], indices: &[u32], uvs: &[[f32; 2]]) -> Vec<f32> {
		source
			.as_chunks::<3>()
			.0
			.iter()
			.zip(indices.as_chunks::<3>().0)
			.map(|(old, new)| {
				let [a, b, c] = old.map(|vertex| positions[vertex as usize]);
				let (ab, ac) = ([0, 1, 2].map(|i| b[i] - a[i]), [0, 1, 2].map(|i| c[i] - a[i]));
				let cross = [ab[1] * ac[2] - ab[2] * ac[1], ab[2] * ac[0] - ab[0] * ac[2], ab[0] * ac[1] - ab[1] * ac[0]];
				let surface = (cross[0] * cross[0] + cross[1] * cross[1] + cross[2] * cross[2]).sqrt();
				let [a, b, c] = new.map(|vertex| uvs[vertex as usize]);
				let lightmap = ((b[0] - a[0]) * (c[1] - a[1]) - (c[0] - a[0]) * (b[1] - a[1])).abs();
				lightmap / surface
			})
			.collect()
	}

	#[test]
	fn cube_faces_become_separate_charts_inside_the_square() {
		let (positions, mut indices) = cube();
		assert_layout(&positions, indices.clone());
		// Each face is a chart of its own, so no vertex, already split per face, splits again.
		let layout = generate_lightmap_uvs(&positions, &mut indices, LightmapUvSettings::default());
		assert_eq!(layout.xref.len(), 24);
	}

	#[test]
	fn a_flat_grid_becomes_one_chart() {
		let (positions, mut indices) = grid(8, 8, |u, v| [u * 4.0, 0.0, v * 4.0]);
		assert_layout(&positions, indices.clone());
		let layout = generate_lightmap_uvs(&positions, &mut indices, LightmapUvSettings::default());
		assert_eq!(layout.xref.len(), positions.len(), "no vertex splits");
	}

	#[test]
	fn a_curved_surface_stays_one_chart_without_stretching() {
		// A 90-degree arc of a cylinder: its ends stretch by 1/cos 45° flat, past the orthographic limit, yet its normals
		// stay close enough to keep one chart.
		let (positions, mut indices) = grid(24, 4, |u, v| {
			let angle = (u - 0.5) * 90f32.to_radians();
			[angle.sin() * 3.0, angle.cos() * 3.0, v * 2.0]
		});
		let source = indices.clone();
		assert_layout(&positions, indices.clone());
		let layout = generate_lightmap_uvs(&positions, &mut indices, LightmapUvSettings::default());
		assert_eq!(layout.xref.len(), positions.len(), "no vertex splits");

		// A conformal map of a developable surface keeps every triangle's share of the area.
		let ratios = area_ratios(&positions, &source, &indices, &layout.uvs);
		let mean = ratios.iter().sum::<f32>() / ratios.len() as f32;
		for ratio in ratios {
			assert!((ratio / mean - 1.0).abs() < 0.1, "a triangle's area scales by {} of the mean", ratio / mean);
		}
	}

	#[test]
	fn a_scanned_surface_keeps_large_charts_whose_triangles_keep_their_area() {
		let (positions, mut indices) = scanned_sheet(48);
		let source = indices.clone();
		let settings = LightmapUvSettings {
			resolution: 256,
			..LightmapUvSettings::default()
		};
		let layout = generate_lightmap_uvs(&positions, &mut indices, settings);

		assert!(!layout.cramped);
		for (new, old) in indices.iter().zip(&source) {
			assert_eq!(layout.xref[*new as usize], *old, "each corner keeps its source vertex");
		}
		// Splitting by each triangle's dominant axis cut this sheet into single triangles that each shrank to a point;
		// the charts must instead keep most of the square covered, and every triangle its own share of it.
		let areas = indices
			.as_chunks::<3>()
			.0
			.iter()
			.map(|corners| {
				let [a, b, c] = corners.map(|corner| layout.uvs[corner as usize]);
				((b[0] - a[0]) * (c[1] - a[1]) - (c[0] - a[0]) * (b[1] - a[1])).abs() / 2.0
			})
			.collect::<Vec<_>>();
		let covered: f32 = areas.iter().sum();
		assert!(covered > 0.4, "the charts cover {covered} of the square");
		let mean = covered / areas.len() as f32;
		assert!(
			areas.iter().all(|&area| area > mean * 0.05),
			"every triangle keeps a share of the square near its share of the surface"
		);
	}

	#[test]
	fn charts_keep_two_texels_apart_at_the_resolution() {
		let (positions, mut indices) = cube();
		let settings = LightmapUvSettings::default();
		let layout = generate_lightmap_uvs(&positions, &mut indices, settings);
		// Cube faces are unit squares; charts are separate when their UV boxes lie at least two texels apart.
		let boxes = indices
			.as_chunks::<6>()
			.0
			.iter()
			.map(|face| {
				let uvs = face.map(|corner| layout.uvs[corner as usize]);
				let minimum = [0, 1].map(|axis| uvs.iter().map(|uv| uv[axis]).fold(f32::INFINITY, f32::min));
				let maximum = [0, 1].map(|axis| uvs.iter().map(|uv| uv[axis]).fold(f32::NEG_INFINITY, f32::max));
				(minimum, maximum)
			})
			.collect::<Vec<_>>();
		let gap = 2.0 / 64.0 * 0.999;
		for (index, a) in boxes.iter().enumerate() {
			assert!(
				a.0.iter().all(|&value| value >= 1.0 / 64.0 * 0.999),
				"a chart keeps a texel from the edges"
			);
			assert!(a.1.iter().all(|&value| value <= 1.0 - 1.0 / 64.0 * 0.999));
			for b in &boxes[index + 1..] {
				let apart = (0..2).any(|axis| a.1[axis] + gap <= b.0[axis] || b.1[axis] + gap <= a.0[axis]);
				assert!(apart, "charts {a:?} and {b:?} lie closer than two texels");
			}
		}
	}
}
