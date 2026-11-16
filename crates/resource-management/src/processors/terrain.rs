//! Turn a height map into a tree of terrain nodes: where each node lies, how far it strays from the height map, and its
//! mesh.
//!
//! The steps are separate so each can be tested alone: decode the samples with [`decode_height_map_in`], scale them to
//! heights with [`HeightField::from_height_map`], lay out the tree with [`TerrainLayout::new`], measure every node
//! with [`plan_terrain`], then build each node's primitives with [`NodeGeometry::build`] and pass them to
//! [`MeshProcessorSession::push_primitive`](super::mesh::MeshProcessorSession::push_primitive). The rules are
//! specified in the [terrain reference](/docs/reference/terrain).

pub(crate) mod height_map;

use std::convert::Infallible;

pub use height_map::{HeightMap, HeightMapError, decode_height_map_in};
use math::{Point, Vector};

use super::mesh::MeshPrimitiveSource;
use crate::resources::{
	ModelSpace,
	terrain::{TerrainLevel, TerrainNodeKey, child_keys, node_index},
};

/// The `HeightField` struct gives every terrain step the height of any sample, in meters, without knowing where the
/// samples came from.
pub struct HeightField<'a> {
	extent: [u32; 2],
	/// Heights in meters, row by row from the height map's top row.
	heights: &'a [f32],
}

impl<'a> HeightField<'a> {
	/// Scales a decoded height map's values in place to heights between `heights[0]` and `heights[1]` meters, and
	/// reads them as a height field.
	///
	/// Next, lay out the terrain with [`TerrainLayout::new`] over [`Self::extent`].
	pub fn from_height_map(map: &'a mut HeightMap<'_>, heights: [f32; 2]) -> Self {
		let range = heights[1] - heights[0];
		for value in &mut map.values {
			*value = heights[0] + *value * range;
		}
		Self::new(map.extent, &map.values)
	}

	/// Reads `heights` as `extent[1]` rows of `extent[0]` heights in meters, from the top row.
	///
	/// Panics when `heights` doesn't hold exactly that many samples.
	pub fn new(extent: [u32; 2], heights: &'a [f32]) -> Self {
		assert_eq!(heights.len(), extent[0] as usize * extent[1] as usize);
		Self { extent, heights }
	}

	/// Returns the samples along X, then along Z.
	pub fn extent(&self) -> [u32; 2] {
		self.extent
	}

	/// Returns the height of the sample at `column` and `row`.
	fn at(&self, column: u32, row: u32) -> f32 {
		self.heights[row as usize * self.extent[0] as usize + column as usize]
	}
}

/// The `TerrainLayout` struct fixes which samples each node of a terrain spans, so planning, mesh building, and the
/// baked [`Terrain`](crate::resources::terrain::Terrain) agree on one tree.
pub struct TerrainLayout {
	extent: [u32; 2],
	tile_size: u32,
	levels: Vec<TerrainLevel>,
}

/// The `NodeSpan` struct is the rectangle of samples one node covers and the stride its vertices take through it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct NodeSpan {
	/// The first and last sample column, inclusive.
	columns: [u32; 2],
	/// The first and last sample row, inclusive.
	rows: [u32; 2],
	/// Samples between neighboring vertices.
	step: u32,
}

impl NodeSpan {
	/// Returns how many vertex columns, then vertex rows, the node has.
	fn vertex_counts(&self) -> [u32; 2] {
		[self.columns, self.rows].map(|[first, last]| (last - first).div_ceil(self.step) + 1)
	}

	/// Returns the sample column of vertex column `index`: every `step`-th sample from the first, then the last
	/// sample, which a narrow edge node can hold off that lattice.
	fn vertex_column(&self, index: u32) -> u32 {
		(self.columns[0] + index * self.step).min(self.columns[1])
	}

	/// Returns the sample row of vertex row `index`, as [`Self::vertex_column`] does for columns.
	fn vertex_row(&self, index: u32) -> u32 {
		(self.rows[0] + index * self.step).min(self.rows[1])
	}
}

impl TerrainLayout {
	/// Lays out the node tree of a height map `extent` samples wide and tall, with `tile_size` quads along each side of
	/// a node.
	///
	/// `extent` needs at least two samples on each side and `tile_size` must be at least one. Next, measure the nodes
	/// with [`plan_terrain`].
	pub fn new(extent: [u32; 2], tile_size: u32) -> Self {
		debug_assert!(extent.iter().all(|&side| side >= 2) && tile_size >= 1);
		let quads = extent.map(|side| u64::from(side - 1));
		let mut levels = Vec::new();
		let mut first_node = 0u32;
		loop {
			let size = u64::from(tile_size) << levels.len();
			let [columns, rows] = quads.map(|quads| quads.div_ceil(size) as u32);
			levels.push(TerrainLevel {
				columns,
				rows,
				first_node,
			});
			first_node += columns * rows;
			if columns == 1 && rows == 1 {
				break;
			}
		}
		Self {
			extent,
			tile_size,
			levels,
		}
	}

	/// Returns the node grid of each level, from level 0, the finest.
	pub fn levels(&self) -> &[TerrainLevel] {
		&self.levels
	}

	/// Returns the key of every node, in the order the baked terrain lists them: by level, then row, then column.
	pub fn keys(&self) -> impl Iterator<Item = TerrainNodeKey> + '_ {
		self.levels.iter().zip(0..).flat_map(|(grid, level)| {
			(0..grid.rows).flat_map(move |row| (0..grid.columns).map(move |column| TerrainNodeKey { level, column, row }))
		})
	}

	/// Returns how many nodes the layout has.
	pub fn node_count(&self) -> usize {
		self.levels
			.last()
			.map_or(0, |root| (root.first_node + root.columns * root.rows) as usize)
	}

	/// Returns where `key` is in [`Self::keys`], or `None` when the layout has no such node.
	pub fn index(&self, key: TerrainNodeKey) -> Option<usize> {
		node_index(&self.levels, key)
	}

	/// Returns the lowest and highest corner of the box that holds `key`'s mesh and the meshes of every node below it.
	///
	/// `key` must name a node of this layout, which `plan` measured.
	pub fn bounds(&self, space: TerrainSpace, key: TerrainNodeKey, plan: &TerrainPlan) -> ([f32; 3], [f32; 3]) {
		let span = self.span(key);
		let node = plan.nodes[self.index(key).expect("the key belongs to the layout")];
		let skirt = plan.skirt_depths[key.level as usize];
		(
			[space.x(span.columns[0]), node.lowest - skirt, space.z(span.rows[1])],
			[space.x(span.columns[1]), node.highest, space.z(span.rows[0])],
		)
	}

	/// Returns the samples `key` spans. `key` must name a node of this layout.
	fn span(&self, key: TerrainNodeKey) -> NodeSpan {
		let size = self.tile_size << key.level;
		let edge = |index: u32, samples: u32| [index * size, ((index + 1) * size).min(samples - 1)];
		NodeSpan {
			columns: edge(key.column, self.extent[0]),
			rows: edge(key.row, self.extent[1]),
			step: 1 << key.level,
		}
	}
}

/// The `TerrainPlan` struct holds what a terrain bake measures across the whole tree before it builds any mesh, since
/// a node's error and skirt depth depend on other nodes.
pub struct TerrainPlan {
	/// Each node's measurements, in [`TerrainLayout::keys`] order.
	pub nodes: Vec<NodeMeasure>,
	/// How far below its edge each level's skirts reach, in meters, from level 0.
	pub skirt_depths: Vec<f32>,
}

/// The `NodeMeasure` struct is what a renderer needs to know about one node before loading its mesh.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct NodeMeasure {
	/// The largest vertical distance between the node's surface and the samples below it, or any child's, in meters.
	pub error: f32,
	/// The lowest sample the node spans, in meters.
	pub lowest: f32,
	/// The highest sample the node spans, in meters.
	pub highest: f32,
}

/// Measures every node of `layout` over `field`: its error, its lowest and highest sample, and each level's skirt
/// depth, where `spacing` is the meters between neighboring samples.
///
/// Every level but the finest reads each sample about once, so the cost grows with the samples times the levels.
pub fn plan_terrain(field: &HeightField<'_>, layout: &TerrainLayout, spacing: f32) -> TerrainPlan {
	let mut nodes: Vec<NodeMeasure> = Vec::with_capacity(layout.node_count());
	for key in layout.keys() {
		let span = layout.span(key);
		let measure = if key.level == 0 {
			// Level 0 vertices are every sample, so its surface is the height map itself.
			let (lowest, highest) = sample_range(field, span);
			NodeMeasure {
				error: 0.0,
				lowest,
				highest,
			}
		} else {
			// Children come earlier in key order, and together they span exactly the node's samples.
			let own = NodeMeasure {
				error: surface_error(field, span),
				lowest: f32::INFINITY,
				highest: f32::NEG_INFINITY,
			};
			child_keys(layout.levels(), key)
				.map(|child| nodes[layout.index(child).expect("children belong to the layout")])
				.fold(own, |node, child| NodeMeasure {
					error: node.error.max(child.error),
					lowest: node.lowest.min(child.lowest),
					highest: node.highest.max(child.highest),
				})
		};
		nodes.push(measure);
	}

	let level_errors = layout
		.levels()
		.iter()
		.map(|grid| {
			let first = grid.first_node as usize;
			nodes[first..first + (grid.columns * grid.rows) as usize]
				.iter()
				.fold(0.0f32, |error, node| error.max(node.error))
		})
		.collect::<Vec<_>>();
	// A level's skirts must reach the deepest gap to a neighbor one level coarser, which is at most both nodes'
	// errors, and those never exceed the coarser level's largest. The root has no coarser neighbor.
	let skirt_depths = (0..level_errors.len())
		.map(|level| (2.0 * level_errors[(level + 1).min(level_errors.len() - 1)]).max(spacing))
		.collect();
	TerrainPlan { nodes, skirt_depths }
}

/// Returns the lowest and highest sample in `span`.
fn sample_range(field: &HeightField<'_>, span: NodeSpan) -> (f32, f32) {
	(span.rows[0]..=span.rows[1])
		.flat_map(|row| (span.columns[0]..=span.columns[1]).map(move |column| field.at(column, row)))
		.fold((f32::INFINITY, f32::NEG_INFINITY), |(lowest, highest), height| {
			(lowest.min(height), highest.max(height))
		})
}

/// Returns the largest vertical distance between a sample in `span` and the surface of the node's own vertices above
/// or below it.
fn surface_error(field: &HeightField<'_>, span: NodeSpan) -> f32 {
	let [width, height] = span.vertex_counts();
	let mut error = 0.0f32;
	for quad_row in 0..height - 1 {
		let (r0, r1) = (span.vertex_row(quad_row), span.vertex_row(quad_row + 1));
		for quad_column in 0..width - 1 {
			let (c0, c1) = (span.vertex_column(quad_column), span.vertex_column(quad_column + 1));
			// The quad's corners, named as the triangles `a b c` and `a c d` name them.
			let (a, b, c, d) = (field.at(c0, r0), field.at(c1, r0), field.at(c1, r1), field.at(c0, r1));
			for row in r0..=r1 {
				let v = (row - r0) as f32 / (r1 - r0) as f32;
				for column in c0..=c1 {
					let u = (column - c0) as f32 / (c1 - c0) as f32;
					let surface = if u >= v {
						a + u * (b - a) + v * (c - b)
					} else {
						a + v * (d - a) + u * (c - d)
					};
					error = error.max((field.at(column, row) - surface).abs());
				}
			}
		}
	}
	error
}

/// The `TerrainSpace` struct places samples in terrain space, so every node and its bounds share one origin: the
/// terrain's center.
#[derive(Clone, Copy)]
pub struct TerrainSpace {
	extent: [u32; 2],
	spacing: f32,
}

impl TerrainSpace {
	/// Places the samples of a height map `extent` samples wide and tall, `spacing` meters apart.
	pub fn new(extent: [u32; 2], spacing: f32) -> Self {
		Self { extent, spacing }
	}

	/// Returns the X coordinate of sample column `column`.
	fn x(&self, column: u32) -> f32 {
		(column as f32 - (self.extent[0] - 1) as f32 / 2.0) * self.spacing
	}

	/// Returns the Z coordinate of sample row `row`. Rows run from the image's top, which faces +Z.
	fn z(&self, row: u32) -> f32 {
		((self.extent[1] - 1) as f32 / 2.0 - row as f32) * self.spacing
	}
}

/// The `NodeGeometry` struct builds one node's surface and skirt primitives in storage it keeps, so a bake that builds
/// many nodes allocates once.
#[derive(Default)]
pub struct NodeGeometry {
	surface: GridPrimitive,
	skirt: GridPrimitive,
}

impl NodeGeometry {
	/// Creates empty geometry. Next, call [`Self::build`] for each node.
	pub fn new() -> Self {
		Self::default()
	}

	/// Builds `key`'s surface and its skirt, which hangs `skirt_depth` meters below the node's edge.
	///
	/// Next, push each of [`Self::primitives`] into a mesh processing session.
	pub fn build(
		&mut self,
		field: &HeightField<'_>,
		layout: &TerrainLayout,
		space: TerrainSpace,
		key: TerrainNodeKey,
		skirt_depth: f32,
	) {
		let span = layout.span(key);
		let [width, height] = span.vertex_counts();
		self.surface.clear();
		self.skirt.clear();

		for row in 0..height {
			for column in 0..width {
				let (column, row) = (span.vertex_column(column), span.vertex_row(row));
				self.surface.push_sample(field, space, span.step, column, row, 0.0);
			}
		}
		for row in 0..height - 1 {
			for column in 0..width - 1 {
				let a = row * width + column;
				let (b, c, d) = (a + 1, a + width + 1, a + width);
				// Sources wind triangles the other way around from their front faces; mesh processing rewinds them.
				self.surface.indices.extend([a, c, b, a, d, c]);
			}
		}

		// The edge, clockwise seen from above: the top row left to right, the right column down, the bottom row right
		// to left, and the left column up. Each corner appears once, so the strip closes on itself.
		let (last_column, last_row) = (width - 1, height - 1);
		let edge = (0..last_column)
			.map(|column| (column, 0))
			.chain((0..last_row).map(|row| (last_column, row)))
			.chain((1..=last_column).rev().map(|column| (column, last_row)))
			.chain((1..=last_row).rev().map(|row| (0, row)));
		for (column, row) in edge {
			let (column, row) = (span.vertex_column(column), span.vertex_row(row));
			self.skirt.push_sample(field, space, span.step, column, row, 0.0);
			self.skirt.push_sample(field, space, span.step, column, row, skirt_depth);
		}
		let corners = self.skirt.positions.len() as u32 / 2;
		for corner in 0..corners {
			// Even vertices lie on the edge and odd ones below it; this pair and the next make a quad that faces outward
			// once mesh processing rewinds it.
			let (top, bottom) = (corner * 2, corner * 2 + 1);
			let next = (corner + 1) % corners * 2;
			self.skirt.indices.extend([top, next, next + 1, top, next + 1, bottom]);
		}
	}

	/// Returns the node's two primitives: its surface, one vertex per vertex column and row, then its skirt, a strip
	/// that hangs from its edge.
	pub fn primitives(&self) -> [&impl MeshPrimitiveSource<Error = Infallible>; 2] {
		[&self.surface, &self.skirt]
	}
}

/// The `GridPrimitive` struct holds one primitive's vertex streams as mesh processing reads them.
#[derive(Default)]
struct GridPrimitive {
	positions: Vec<[f32; 3]>,
	normals: Vec<[f32; 3]>,
	tangents: Vec<[f32; 4]>,
	uvs: Vec<[f32; 2]>,
	indices: Vec<u32>,
}

impl GridPrimitive {
	fn clear(&mut self) {
		self.positions.clear();
		self.normals.clear();
		self.tangents.clear();
		self.uvs.clear();
		self.indices.clear();
	}

	/// Adds a vertex at the sample at `column` and `row`, `drop` meters below it, shaded by the slope measured `step`
	/// samples to each side.
	fn push_sample(&mut self, field: &HeightField<'_>, space: TerrainSpace, step: u32, column: u32, row: u32, drop: f32) {
		let [width, height] = field.extent();
		let (left, right) = (column.saturating_sub(step), (column + step).min(width - 1));
		let (up, down) = (row.saturating_sub(step), (row + step).min(height - 1));
		// Every map has two samples on each side, so both pairs are distinct.
		let dx = (field.at(right, row) - field.at(left, row)) / ((right - left) as f32 * space.spacing);
		// Rows run toward -Z, so the upper row is the farther one along +Z.
		let dz = (field.at(column, up) - field.at(column, down)) / ((down - up) as f32 * space.spacing);

		self.positions
			.push([space.x(column), field.at(column, row) - drop, space.z(row)]);
		self.normals.push(normalize([-dx, 1.0, -dz]));
		let [x, y, z] = normalize([1.0, dx, 0.0]);
		// The bitangent follows +V, which runs down the image toward -Z, and the normal times the tangent points there.
		self.tangents.push([x, y, z, 1.0]);
		self.uvs
			.push([column as f32 / (width - 1) as f32, row as f32 / (height - 1) as f32]);
	}
}

fn normalize([x, y, z]: [f32; 3]) -> [f32; 3] {
	let length = (x * x + y * y + z * z).sqrt();
	[x / length, y / length, z / length]
}

impl MeshPrimitiveSource for GridPrimitive {
	type Error = Infallible;

	fn material_slot(&self) -> usize {
		0
	}

	fn indices(&self) -> Result<impl ExactSizeIterator<Item = Result<u32, Infallible>> + '_, Infallible> {
		Ok(self.indices.iter().copied().map(Ok))
	}

	fn positions(&self) -> Result<impl ExactSizeIterator<Item = Result<Point<ModelSpace>, Infallible>> + '_, Infallible> {
		Ok(self.positions.iter().map(|&position| Ok(Point::from_array(position))))
	}

	fn normals(
		&self,
	) -> Result<Option<impl ExactSizeIterator<Item = Result<Vector<ModelSpace>, Infallible>> + '_>, Infallible> {
		Ok(Some(self.normals.iter().map(|&normal| Ok(Vector::from_array(normal)))))
	}

	fn tangents(&self) -> Result<Option<impl ExactSizeIterator<Item = Result<[f32; 4], Infallible>> + '_>, Infallible> {
		Ok(Some(self.tangents.iter().copied().map(Ok)))
	}

	fn uvs(&self) -> Result<Option<impl ExactSizeIterator<Item = Result<[f32; 2], Infallible>> + '_>, Infallible> {
		Ok(Some(self.uvs.iter().copied().map(Ok)))
	}
}

#[cfg(test)]
mod tests {
	use super::{GridPrimitive, HeightField, NodeGeometry, TerrainLayout, TerrainSpace, plan_terrain};
	use crate::resources::terrain::{TerrainLevel, TerrainNodeKey, child_keys};

	fn key(level: u32, column: u32, row: u32) -> TerrainNodeKey {
		TerrainNodeKey { level, column, row }
	}

	/// Returns `width` by `height` heights from `height_at(column, row)`, row by row.
	fn heights(width: u32, height: u32, height_at: impl Fn(u32, u32) -> f32) -> Vec<f32> {
		(0..height)
			.flat_map(|row| (0..width).map(move |column| (column, row)))
			.map(|(column, row)| height_at(column, row))
			.collect()
	}

	/// Returns a rolling surface with detail at every scale, so every level but the finest strays from it.
	fn hills(column: u32, row: u32) -> f32 {
		let (x, z) = (column as f32, row as f32);
		(x * 0.7).sin() * 3.0 + (z * 0.31).cos() * 5.0 + ((x + z) * 0.13).sin() * 8.0
	}

	/// Builds `key` over `field`, with samples 2 meters apart.
	fn built(field: &HeightField<'_>, layout: &TerrainLayout, key: TerrainNodeKey, skirt: f32) -> NodeGeometry {
		let mut geometry = NodeGeometry::new();
		geometry.build(field, layout, TerrainSpace::new(field.extent(), 2.0), key, skirt);
		geometry
	}

	/// Returns each triangle's unnormalized front-face normal once mesh processing rewinds it, and its centroid. A
	/// source triangle `a b c` becomes `a c b`, whose front face's normal is `(c − a) × (b − a)`.
	fn faces(primitive: &GridPrimitive) -> impl Iterator<Item = ([f32; 3], [f32; 3])> + '_ {
		primitive.indices.as_chunks::<3>().0.iter().map(|triangle| {
			let [a, c, b] = triangle.map(|vertex| primitive.positions[vertex as usize]);
			let (u, v) = (
				[b[0] - a[0], b[1] - a[1], b[2] - a[2]],
				[c[0] - a[0], c[1] - a[1], c[2] - a[2]],
			);
			let normal = [
				u[1] * v[2] - u[2] * v[1],
				u[2] * v[0] - u[0] * v[2],
				u[0] * v[1] - u[1] * v[0],
			];
			let centroid = [0, 1, 2].map(|axis| (a[axis] + b[axis] + c[axis]) / 3.0);
			(normal, centroid)
		})
	}

	#[test]
	fn each_level_halves_the_grid_until_one_node_covers_the_terrain() {
		let layout = TerrainLayout::new([10, 7], 4);

		assert_eq!(
			layout.levels(),
			[
				TerrainLevel {
					columns: 3,
					rows: 2,
					first_node: 0
				},
				TerrainLevel {
					columns: 2,
					rows: 1,
					first_node: 6
				},
				TerrainLevel {
					columns: 1,
					rows: 1,
					first_node: 8
				},
			]
		);
		assert_eq!(layout.node_count(), 9);
		assert_eq!(layout.keys().nth(4), Some(key(0, 1, 1)));
		assert_eq!(TerrainLayout::new([5, 5], 4).levels().len(), 1);
	}

	#[test]
	fn planes_bake_without_error_at_every_level() {
		let samples = heights(33, 17, |column, row| column as f32 * 0.5 - row as f32 * 2.0 + 7.0);
		let field = HeightField::new([33, 17], &samples);
		let layout = TerrainLayout::new([33, 17], 4);

		let plan = plan_terrain(&field, &layout, 1.0);

		assert!(plan.nodes.iter().all(|node| node.error.abs() < 1e-4));
		assert_eq!(plan.skirt_depths, vec![1.0; layout.levels().len()]);
	}

	#[test]
	fn a_spike_between_vertices_is_the_error_of_every_node_above_it() {
		// Column 3 lies between level 1's vertex columns 2 and 4, so only level 0 holds the spike.
		let samples = heights(9, 9, |column, row| if (column, row) == (3, 1) { 6.0 } else { 0.0 });
		let field = HeightField::new([9, 9], &samples);
		let layout = TerrainLayout::new([9, 9], 4);

		let plan = plan_terrain(&field, &layout, 0.5);
		let error = |key| plan.nodes[layout.index(key).unwrap()].error;

		assert_eq!(error(key(0, 0, 0)), 0.0);
		assert_eq!(error(key(1, 0, 0)), 6.0);
		// Level 0's skirts reach twice the coarser level's largest error, and the root's twice its own.
		assert_eq!(plan.skirt_depths, [12.0, 12.0]);
	}

	#[test]
	fn a_node_never_strays_less_than_its_children_and_its_bounds_hold_them() {
		let samples = heights(65, 41, hills);
		let field = HeightField::new([65, 41], &samples);
		let layout = TerrainLayout::new([65, 41], 4);
		let space = TerrainSpace::new([65, 41], 2.0);
		let plan = plan_terrain(&field, &layout, 2.0);

		for key in layout.keys().filter(|key| key.level > 0) {
			let node = plan.nodes[layout.index(key).unwrap()];
			let (min, max) = layout.bounds(space, key, &plan);
			for child in child_keys(layout.levels(), key) {
				assert!(node.error >= plan.nodes[layout.index(child).unwrap()].error);
				let (child_min, child_max) = layout.bounds(space, child, &plan);
				assert!((0..3).all(|axis| min[axis] <= child_min[axis] && child_max[axis] <= max[axis]));
			}
		}
		assert!(plan.nodes[layout.index(key(4, 0, 0)).unwrap()].error > 0.0);
	}

	#[test]
	fn surfaces_face_up_and_skirts_face_away_from_their_node() {
		let samples = heights(17, 17, hills);
		let field = HeightField::new([17, 17], &samples);
		let layout = TerrainLayout::new([17, 17], 4);

		for key in layout.keys() {
			let geometry = built(&field, &layout, key, 3.0);
			assert!(faces(&geometry.surface).all(|(normal, _)| normal[1] > 0.0));

			let positions = &geometry.surface.positions;
			let center = positions
				.iter()
				.fold([0.0; 3], |sum, p| [sum[0] + p[0], sum[1] + p[1], sum[2] + p[2]])
				.map(|value| value / positions.len() as f32);
			assert!(faces(&geometry.skirt).all(|(normal, centroid)| {
				normal[0] * (centroid[0] - center[0]) + normal[2] * (centroid[2] - center[2]) > 0.0
			}));
		}
	}

	#[test]
	fn vertices_lie_on_their_samples_in_terrain_space() {
		let samples = heights(9, 5, |column, row| (column * 10 + row) as f32);
		let field = HeightField::new([9, 5], &samples);
		let layout = TerrainLayout::new([9, 5], 4);

		let geometry = built(&field, &layout, key(0, 1, 0), 1.0);
		let surface = &geometry.surface;

		// Sample (4, 0) is the second node's first vertex. Samples are 2 meters apart around the center sample (4, 2).
		assert_eq!(surface.positions[0], [0.0, 40.0, 4.0]);
		assert_eq!(surface.uvs[0], [0.5, 0.0]);
		assert_eq!(*surface.positions.last().unwrap(), [8.0, 84.0, -4.0]);
		assert_eq!(*surface.uvs.last().unwrap(), [1.0, 1.0]);
		// A skirt's lower vertex hangs straight below its edge vertex.
		assert_eq!(geometry.skirt.positions[1], [0.0, 39.0, 4.0]);
	}

	#[test]
	fn narrow_edge_nodes_end_at_the_last_sample() {
		// 11 samples hold 10 quads: each level 1 node spans 8, so the second holds the last 2 between two vertices.
		let samples = heights(11, 3, hills);
		let field = HeightField::new([11, 3], &samples);
		let layout = TerrainLayout::new([11, 3], 4);

		let geometry = built(&field, &layout, key(1, 1, 0), 1.0);
		let columns = geometry.surface.uvs.iter().map(|uv| (uv[0] * 10.0).round() as u32);

		assert_eq!(columns.take(3).collect::<Vec<_>>(), [8, 10, 8]);
	}

	#[test]
	fn neighbors_of_one_level_share_their_edge_vertices() {
		let samples = heights(17, 17, hills);
		let field = HeightField::new([17, 17], &samples);
		let layout = TerrainLayout::new([17, 17], 4);
		let (left, right) = (
			built(&field, &layout, key(1, 0, 0), 1.0),
			built(&field, &layout, key(1, 1, 0), 1.0),
		);

		// Each node has 5 vertex columns; the left node's last column is the right node's first.
		let column = |geometry: &NodeGeometry, column: usize| {
			let surface = &geometry.surface;
			(0..5)
				.map(|row| row * 5 + column)
				.map(|vertex| (surface.positions[vertex], surface.normals[vertex]))
				.collect::<Vec<_>>()
		};
		assert_eq!(column(&left, 4), column(&right, 0));
	}

	#[test]
	fn skirts_cover_the_seam_between_neighbors_one_level_apart() {
		let samples = heights(33, 33, hills);
		let field = HeightField::new([33, 33], &samples);
		let layout = TerrainLayout::new([33, 33], 4);
		let plan = plan_terrain(&field, &layout, 2.0);
		let space = TerrainSpace::new([33, 33], 2.0);

		// Level 0's node (2, 0) borders level 1's node (0, 0) along sample column 8, rows 0 through 4.
		let (fine, coarse) = (
			built(&field, &layout, key(0, 2, 0), 0.0),
			built(&field, &layout, key(1, 0, 0), 0.0),
		);
		// Returns the height of `geometry`'s surface along column 8 at `z`, between the edge vertices around it.
		let edge_height = |geometry: &NodeGeometry, z: f32| {
			let mut edge = geometry
				.surface
				.positions
				.iter()
				.filter(|position| position[0] == space.x(8))
				.collect::<Vec<_>>();
			edge.sort_by(|a, b| b[2].total_cmp(&a[2]));
			let segment = edge.windows(2).find(|pair| pair[0][2] >= z && z >= pair[1][2]).unwrap();
			let t = (segment[0][2] - z) / (segment[0][2] - segment[1][2]);
			segment[0][1] + t * (segment[1][1] - segment[0][1])
		};
		let gap = (0..=4)
			.map(|row| space.z(row))
			.map(|z| (edge_height(&fine, z) - edge_height(&coarse, z)).abs())
			.fold(0.0f32, f32::max);

		assert!(gap > 0.0);
		assert!(gap <= plan.skirt_depths[0] && gap <= plan.skirt_depths[1]);
	}
}
