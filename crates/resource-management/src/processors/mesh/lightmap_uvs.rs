//! Lightmap UV generation: a second set of texture coordinates per primitive, in which no two charts overlap, that
//! lightmaps store light at.
//!
//! Triangles group into charts by the dominant axis of their normal, projected along that axis and placed in rows inside
//! one square. The layout is deliberately simple: it keeps every chart's scale, so a lightmap's texel density stays even
//! across a primitive, at the cost of more and blockier charts than an unwrapper that cuts along curvature.

use std::collections::HashMap;

/// The `LightmapUvSettings` struct lets an asset tune the lightmap UVs mesh processing gives its unskinned primitives.
///
/// Pass it to [`MeshProcessorSession::new`](super::MeshProcessorSession::new) through
/// [`MeshSettings::lightmap_uvs`](super::MeshSettings::lightmap_uvs). An asset overrides it through the `lightmap_uvs`
/// key of its BEAD file.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LightmapUvSettings {
	/// The texels per side the gaps between charts are measured at. Set it near the texels a lightmap gives the
	/// primitive, so neighboring charts don't share texels.
	pub resolution: u32,
}

impl Default for LightmapUvSettings {
	fn default() -> Self {
		Self { resolution: 64 }
	}
}

/// The texels a chart keeps from its neighbors on each side, so two charts lie twice this far apart and a chart lies
/// this far from the square's edges.
const CHART_BORDER_TEXELS: f32 = 1.0;
/// The rounds of placement that grow the charts' border until it covers [`CHART_BORDER_TEXELS`] of the square.
const PLACEMENT_ROUNDS: usize = 32;
/// The axis of a triangle without area, which takes its neighbor's.
const NO_AXIS: u8 = 6;

/// The `LightmapUvScratch` struct keeps the working memory of [`generate_lightmap_uvs`] across primitives.
#[derive(Default)]
pub(super) struct LightmapUvScratch {
	/// For each triangle, its dominant axis as `2 × axis + negative`, or [`NO_AXIS`].
	axes: Vec<u8>,
	/// The union-find parent of each triangle; roots name charts.
	parents: Vec<u32>,
	/// For each undirected edge between welded vertices, the first triangle of each axis that uses it.
	edges: HashMap<(u32, u32), [u32; 6]>,
	/// The new vertex of each (source vertex, chart) pair.
	corners: HashMap<(u32, u32), u32>,
	/// The chart of each union-find root.
	chart_of_root: HashMap<u32, u32>,
	charts: Vec<Chart>,
	order: Vec<u32>,
}

/// The `LightmapUvs` struct is one primitive's lightmap UV layout: the vertices it splits into and their coordinates.
pub(super) struct LightmapUvs {
	/// The source vertex of each new vertex.
	pub(super) xref: Vec<u32>,
	/// The lightmap UV of each new vertex, from 0 through 1.
	pub(super) uvs: Vec<[f32; 2]>,
	/// Whether the charts were too many to keep their gaps at the settings' resolution.
	pub(super) cramped: bool,
}

/// One chart's projected bounds and where placement put them.
#[derive(Clone, Copy)]
struct Chart {
	minimum: [f32; 2],
	maximum: [f32; 2],
	/// Where the chart's minimum corner lands in the square, in model units.
	placed: [f32; 2],
}

impl Chart {
	fn extent(&self) -> [f32; 2] {
		[self.maximum[0] - self.minimum[0], self.maximum[1] - self.minimum[1]]
	}
}

/// Lays out lightmap UVs for one primitive and rewrites `indices` to the vertices the layout splits into.
///
/// `positions` are the primitive's source vertices and `indices` its triangle list. A vertex that two charts share
/// becomes one vertex per chart; the returned [`LightmapUvs::xref`] names the source vertex of each, so every other
/// attribute is copied through it.
pub(super) fn generate_lightmap_uvs(
	positions: &[[f32; 3]],
	indices: &mut [u32],
	settings: LightmapUvSettings,
	scratch: &mut LightmapUvScratch,
) -> LightmapUvs {
	let triangle_count = indices.len() / 3;
	// Charts follow the surface's topology, not its attribute seams, so vertices that only differ in UV or normal weld.
	let positions_adapter = meshopt::VertexDataAdapter::new(bytemuck::cast_slice(positions), 12, 0)
		.expect("positions are three tightly packed floats");
	let welded = meshopt::generate_position_remap(&positions_adapter);

	classify_triangles(positions, indices, &mut scratch.axes);
	scratch.parents.clear();
	scratch.parents.extend(0..triangle_count as u32);
	join_triangles(indices, &welded, &mut scratch.axes, &mut scratch.parents, &mut scratch.edges);

	// Each chart's corners project along its axis; a source vertex gets one new vertex per chart it touches.
	scratch.chart_of_root.clear();
	scratch.charts.clear();
	scratch.corners.clear();
	let mut xref = Vec::with_capacity(positions.len());
	let mut projected = Vec::with_capacity(positions.len());
	let mut chart_of_vertex = Vec::with_capacity(positions.len());
	for triangle in 0..triangle_count {
		let root = find(&mut scratch.parents, triangle as u32);
		let next_chart = scratch.charts.len() as u32;
		let chart = *scratch.chart_of_root.entry(root).or_insert(next_chart);
		if chart == next_chart {
			scratch.charts.push(Chart {
				minimum: [f32::INFINITY; 2],
				maximum: [f32::NEG_INFINITY; 2],
				placed: [0.0; 2],
			});
		}
		// Triangles of one chart share an axis, except one without area, which joined it and projects the same way.
		let axis = scratch.axes[root as usize];
		for corner in &mut indices[triangle * 3..triangle * 3 + 3] {
			let source = *corner;
			let next_vertex = xref.len() as u32;
			let vertex = *scratch.corners.entry((source, chart)).or_insert(next_vertex);
			if vertex == next_vertex {
				let point = project(positions[source as usize], axis);
				let bounds = &mut scratch.charts[chart as usize];
				for component in 0..2 {
					bounds.minimum[component] = bounds.minimum[component].min(point[component]);
					bounds.maximum[component] = bounds.maximum[component].max(point[component]);
				}
				xref.push(source);
				projected.push(point);
				chart_of_vertex.push(chart);
			}
			*corner = vertex;
		}
	}

	let (side, cramped) = place_charts(&mut scratch.charts, &mut scratch.order, settings.resolution);
	let uvs = projected
		.iter()
		.zip(&chart_of_vertex)
		.map(|(point, &chart)| {
			let chart = &scratch.charts[chart as usize];
			[
				(point[0] - chart.minimum[0] + chart.placed[0]) / side,
				(point[1] - chart.minimum[1] + chart.placed[1]) / side,
			]
		})
		.collect();
	LightmapUvs { xref, uvs, cramped }
}

/// Places charts of the given `extents`, in model units, inside one square the way mesh processing places a
/// primitive's charts, and returns where each chart's minimum corner lands and the square's side, in model units.
///
/// Mesh generators whose charts are known, such as a box's faces, lay out their lightmap UVs with it, so they keep the
/// gaps baked meshes keep: divide a chart's corner plus its placement by the side.
pub fn place_lightmap_charts(extents: &[[f32; 2]], settings: LightmapUvSettings) -> (Vec<[f32; 2]>, f32) {
	let mut charts = extents
		.iter()
		.map(|&maximum| Chart {
			minimum: [0.0; 2],
			maximum,
			placed: [0.0; 2],
		})
		.collect::<Vec<_>>();
	let (side, _) = place_charts(&mut charts, &mut Vec::with_capacity(extents.len()), settings.resolution);
	(charts.iter().map(|chart| chart.placed).collect(), side)
}

/// Places rectangles of the given `extents` in rows, tallest first, each `border` from the square's edges and twice that
/// from its neighbors, and returns where each rectangle's minimum corner lands and the side of the square that holds
/// them, in the extents' units.
///
/// Lightmap bakes place each primitive's rectangle of a level's lightmap with it, in texels, so the rectangles keep the
/// rows mesh processing gives a primitive's charts.
pub fn place_lightmap_rectangles(extents: &[[f32; 2]], border: f32) -> (Vec<[f32; 2]>, f32) {
	let mut charts = extents
		.iter()
		.map(|&maximum| Chart {
			minimum: [0.0; 2],
			maximum,
			placed: [0.0; 2],
		})
		.collect::<Vec<_>>();
	let mut order = Vec::with_capacity(extents.len());
	tallest_first(&charts, &mut order);
	let side = place_rows(&mut charts, &order, border);
	(charts.iter().map(|chart| chart.placed).collect(), side)
}

/// Returns `uv`, a coordinate of a chart that fills the unit square, moved inside the square's border, so a generated
/// mesh with one chart keeps the gap from the edges that baked meshes keep.
pub fn inset_lightmap_uv(uv: [f32; 2], settings: LightmapUvSettings) -> [f32; 2] {
	let border = CHART_BORDER_TEXELS / settings.resolution as f32;
	uv.map(|value| border + value * (1.0 - 2.0 * border))
}

/// Writes each triangle's dominant axis to `axes`: the axis of its normal's largest component, with its sign.
fn classify_triangles(positions: &[[f32; 3]], indices: &[u32], axes: &mut Vec<u8>) {
	axes.clear();
	axes.extend(indices.as_chunks::<3>().0.iter().map(|triangle| {
		let [a, b, c] = triangle.map(|index| positions[index as usize]);
		let edge1 = [b[0] - a[0], b[1] - a[1], b[2] - a[2]];
		let edge2 = [c[0] - a[0], c[1] - a[1], c[2] - a[2]];
		let normal = [
			edge1[1] * edge2[2] - edge1[2] * edge2[1],
			edge1[2] * edge2[0] - edge1[0] * edge2[2],
			edge1[0] * edge2[1] - edge1[1] * edge2[0],
		];
		let axis = (0..3)
			.max_by(|&x, &y| normal[x].abs().total_cmp(&normal[y].abs()))
			.expect("three axes");
		if normal[axis] == 0.0 || !normal[axis].is_finite() {
			NO_AXIS
		} else {
			(axis as u8) * 2 + u8::from(normal[axis] < 0.0)
		}
	}));
}

/// Joins triangles that share a welded edge and an axis into one chart, then joins each triangle without area to the
/// chart of its first neighbor, whose axis it takes.
fn join_triangles(
	indices: &[u32],
	welded: &[u32],
	axes: &mut [u8],
	parents: &mut [u32],
	edges: &mut HashMap<(u32, u32), [u32; 6]>,
) {
	edges.clear();
	let edge_keys = |triangle: usize| {
		let [a, b, c] = [0, 1, 2].map(|corner| welded[indices[triangle * 3 + corner] as usize]);
		[(a, b), (b, c), (c, a)].map(|(first, second)| (first.min(second), first.max(second)))
	};
	let triangle_count = axes.len();
	for triangle in 0..triangle_count {
		let axis = axes[triangle];
		if axis == NO_AXIS {
			continue;
		}
		for key in edge_keys(triangle) {
			let first = &mut edges.entry(key).or_insert([u32::MAX; 6])[axis as usize];
			if *first == u32::MAX {
				*first = triangle as u32;
			} else {
				let other = *first;
				union(parents, triangle as u32, other);
			}
		}
	}
	for triangle in 0..triangle_count {
		if axes[triangle] != NO_AXIS {
			continue;
		}
		let neighbor = edge_keys(triangle)
			.into_iter()
			.filter_map(|key| edges.get(&key))
			.flat_map(|firsts| firsts.iter().copied())
			.find(|&first| first != u32::MAX);
		match neighbor {
			Some(neighbor) => {
				axes[triangle] = axes[neighbor as usize];
				union(parents, triangle as u32, neighbor);
			}
			// Alone, it projects along any axis: it covers no area there either way.
			None => axes[triangle] = 0,
		}
	}
}

/// Returns `position` projected along `axis` onto the plane of the other two axes.
fn project(position: [f32; 3], axis: u8) -> [f32; 2] {
	match axis / 2 {
		0 => [position[2], position[1]],
		1 => [position[0], position[2]],
		_ => [position[0], position[1]],
	}
}

/// Places `charts` in rows inside one square, tallest first, and returns the square's side in model units and whether
/// the border fell short of [`CHART_BORDER_TEXELS`] at `resolution`.
///
/// The border a chart needs is a share of the square, which grows with the border, so placement repeats with the
/// border the last round's square asks for until the border covers it.
fn place_charts(charts: &mut [Chart], order: &mut Vec<u32>, resolution: u32) -> (f32, bool) {
	tallest_first(charts, order);

	let texels = resolution as f32;
	let mut border = 0.0f32;
	let mut side = 0.0f32;
	for _ in 0..PLACEMENT_ROUNDS {
		side = place_rows(charts, order, border);
		let needed = side * CHART_BORDER_TEXELS / texels;
		if side > 0.0 && border >= needed * 0.9999 {
			return (side, false);
		}
		// A slightly larger border than asked converges in fewer rounds.
		border = needed * 1.01;
	}
	if side <= 0.0 {
		// Every chart is a point, so any square holds them.
		return (1.0, false);
	}
	(side, true)
}

/// Writes the indices of `charts` to `order`, tallest first; charts of one height keep their order.
fn tallest_first(charts: &[Chart], order: &mut Vec<u32>) {
	order.clear();
	order.extend(0..charts.len() as u32);
	order.sort_by(|&a, &b| charts[b as usize].extent()[1].total_cmp(&charts[a as usize].extent()[1]));
}

/// Places `charts` in `order` in rows, each chart padded by `border` on every side, and returns the side of the square
/// that holds the rows.
fn place_rows(charts: &mut [Chart], order: &[u32], border: f32) -> f32 {
	let padded = |chart: &Chart| {
		let [width, height] = chart.extent();
		[width + 2.0 * border, height + 2.0 * border]
	};
	let area: f32 = charts.iter().map(|chart| padded(chart)[0] * padded(chart)[1]).sum();
	let widest = charts.iter().map(|chart| padded(chart)[0]).fold(0.0f32, f32::max);
	// Rows as wide as a square of the charts' area come out close to square.
	let row_width = area.sqrt().max(widest);

	let (mut x, mut y, mut row_height, mut width) = (0.0f32, 0.0f32, 0.0f32, 0.0f32);
	for &chart in order {
		let chart = &mut charts[chart as usize];
		let [chart_width, chart_height] = padded(chart);
		if x > 0.0 && x + chart_width > row_width {
			y += row_height;
			x = 0.0;
			row_height = 0.0;
		}
		chart.placed = [x + border, y + border];
		x += chart_width;
		width = width.max(x);
		row_height = row_height.max(chart_height);
	}
	width.max(y + row_height)
}

fn find(parents: &mut [u32], mut node: u32) -> u32 {
	while parents[node as usize] != node {
		// Path halving keeps the trees shallow without recursion.
		let grandparent = parents[parents[node as usize] as usize];
		parents[node as usize] = grandparent;
		node = grandparent;
	}
	node
}

fn union(parents: &mut [u32], a: u32, b: u32) {
	let (a, b) = (find(parents, a), find(parents, b));
	if a != b {
		parents[a.max(b) as usize] = a.min(b);
	}
}

#[cfg(test)]
mod tests {
	use super::{LightmapUvScratch, LightmapUvSettings, generate_lightmap_uvs, place_lightmap_rectangles};

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

	/// Returns an L-shaped strip: two quads on the floor and one rising from its end, sharing welded edges.
	fn l_shape() -> (Vec<[f32; 3]>, Vec<u32>) {
		let positions = vec![
			[0.0, 0.0, 0.0],
			[1.0, 0.0, 0.0],
			[2.0, 0.0, 0.0],
			[0.0, 0.0, 1.0],
			[1.0, 0.0, 1.0],
			[2.0, 0.0, 1.0],
			[2.0, 1.0, 0.0],
			[2.0, 1.0, 1.0],
		];
		let indices = vec![0, 3, 4, 0, 4, 1, 1, 4, 5, 1, 5, 2, 2, 5, 7, 2, 7, 6];
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
		let settings = LightmapUvSettings { resolution: 64 };
		let layout = generate_lightmap_uvs(&positions, &mut indices, settings, &mut LightmapUvScratch::default());

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

	#[test]
	fn cube_faces_become_separate_charts_inside_the_square() {
		let (positions, indices) = cube();
		assert_layout(&positions, indices);
	}

	#[test]
	fn welded_quads_facing_one_axis_share_a_chart_and_a_wall_gets_its_own() {
		let (positions, mut indices) = l_shape();
		let layout = generate_lightmap_uvs(
			&positions,
			&mut indices,
			LightmapUvSettings::default(),
			&mut LightmapUvScratch::default(),
		);
		// The floor's six vertices stay single, and the wall splits the two it shares with the floor.
		assert_eq!(layout.xref.len(), 6 + 4);
		assert_layout(&positions, l_shape().1);
	}

	#[test]
	fn charts_keep_two_texels_apart_at_the_resolution() {
		let (positions, mut indices) = cube();
		let settings = LightmapUvSettings { resolution: 64 };
		let layout = generate_lightmap_uvs(&positions, &mut indices, settings, &mut LightmapUvScratch::default());
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

	#[test]
	fn rectangles_keep_their_border_from_the_edges_and_twice_it_from_each_other() {
		let extents = [
			[16.0, 16.0],
			[40.0, 40.0],
			[16.0, 16.0],
			[24.0, 24.0],
			[100.0, 100.0],
			[16.0, 16.0],
		];
		let (placed, side) = place_lightmap_rectangles(&extents, 1.0);

		let rectangles: Vec<[f32; 4]> = placed
			.iter()
			.zip(extents)
			.map(|(&[x, y], [width, height])| [x, y, x + width, y + height])
			.collect();
		for (index, &[x0, y0, x1, y1]) in rectangles.iter().enumerate() {
			assert!(
				x0 >= 1.0 && y0 >= 1.0 && x1 <= side - 1.0 && y1 <= side - 1.0,
				"rectangle {index} keeps 1 from the edges"
			);
			assert_eq!(
				[x0.fract(), y0.fract()],
				[0.0, 0.0],
				"whole extents and borders land on whole texels"
			);
			for &[u0, v0, u1, v1] in &rectangles[index + 1..] {
				let apart = u0 >= x1 + 2.0 || x0 >= u1 + 2.0 || v0 >= y1 + 2.0 || y0 >= v1 + 2.0;
				assert!(apart, "rectangles keep 2 apart");
			}
		}
		assert_eq!(placed[4], [1.0, 1.0], "the tallest rectangle leads the first row");
	}
}
