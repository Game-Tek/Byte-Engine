//! Cutting a primitive into charts: seeds, relocation, growth, holes, and merging, following the clustered charts of
//! xatlas (Jonathan Young), whose weights and thresholds these are. The Charts table of the asset reference's Lightmap
//! UVs section defines each rule.

use std::cmp::Reverse;
use std::collections::{BTreeMap, BTreeSet, BinaryHeap};

use super::maps::{BoundaryEdge, boundary_crosses, plane_basis, project};
use super::surface::{FLIP_EPSILON, NONE, Surface, add, dot, next_corner, norm, normalize, sub};

/// The normal deviation at or above which a region never joins a chart, about 73 degrees.
const MAX_NORMAL_DEVIATION: f64 = 0.707;
const NORMAL_DEVIATION_WEIGHT: f64 = 2.0;
const ROUNDNESS_WEIGHT: f64 = 0.01;
const STRAIGHTNESS_WEIGHT: f64 = 6.0;
/// The cost up to which charts grow together.
const MAX_COST: f64 = 2.0;
/// The cost up to which a seed grows alone, and a chart that fills a hole.
const SEED_COST: f64 = MAX_COST * 0.5;
/// The dot product of two charts' normals at or above which one may absorb the other.
const MERGE_MIN_NORMAL_DOT: f64 = 0.5;
/// The relative tolerance within which two boundary lengths count as equal.
const LENGTH_EPSILON: f64 = 1e-4;

/// The `Chart` struct is one chart while charts grow: its faces and the measures its costs read.
struct Chart {
	/// The chart's faces, in the order they joined until merging sorts them.
	faces: Vec<u32>,
	normal_sum: [f64; 3],
	normal: [f64; 3],
	area: f64,
	boundary_length: f64,
	center_sum: [f64; 3],
	/// The corners on the chart's boundary. Ordered sets keep the result the same from run to run.
	boundary: BTreeSet<u32>,
	/// The regions next to the chart, not in it, that it may still take.
	frontier: BTreeSet<u32>,
	rejected: BTreeSet<u32>,
	seed: u32,
	/// The normal at the last full flip check, and the smallest dot product of a face normal with it then, which
	/// bound every face's dot product with a nearby normal without visiting them.
	checked_normal: [f64; 3],
	checked_dot: f64,
	/// Bumped on every change, so stale entries of the growth queue are skipped.
	version: u32,
}

impl Chart {
	fn new(seed: u32) -> Self {
		Self {
			faces: Vec::new(),
			normal_sum: [0.0; 3],
			normal: [0.0; 3],
			area: 0.0,
			boundary_length: 0.0,
			center_sum: [0.0; 3],
			boundary: BTreeSet::new(),
			frontier: BTreeSet::new(),
			rejected: BTreeSet::new(),
			seed,
			checked_normal: [0.0; 3],
			checked_dot: 1.0,
			version: 0,
		}
	}
}

/// A cost and the region it prices, ordered by cost and then region, for the cheapest candidate.
type Priced = (f64, u32);

/// The `Segmentation` struct cuts one primitive's faces into charts.
pub(super) struct Segmentation<'s> {
	surface: &'s Surface,
	charts: Vec<Chart>,
	/// The chart of each face, or [`NONE`].
	face_charts: Vec<u32>,
	/// Faces with area, largest first, the lowest index first among equals; seeds start from them in turn.
	seed_order: Vec<u32>,
	edges: Vec<BoundaryEdge>,
	cells: Vec<Vec<u32>>,
}

impl<'s> Segmentation<'s> {
	/// Cuts `surface` into charts, and returns each chart's faces, sorted, and normal, in chart order, with each face
	/// without area in the chart it joins.
	pub(super) fn run(surface: &'s Surface) -> Vec<(Vec<u32>, [f64; 3])> {
		let mut seed_order: Vec<u32> = (0..surface.face_count() as u32)
			.filter(|&face| surface.face_regions[face as usize] != NONE)
			.collect();
		seed_order.sort_by(|&a, &b| surface.face_areas[b as usize].total_cmp(&surface.face_areas[a as usize]));
		let mut segmentation = Self {
			surface,
			charts: Vec::new(),
			face_charts: vec![NONE; surface.face_count()],
			seed_order,
			edges: Vec::new(),
			cells: Vec::new(),
		};
		segmentation.place_seeds();
		segmentation.relocate_seeds();
		segmentation.grow();
		segmentation.place_seeds();
		// Merging and the maps look faces up by binary search.
		segmentation.charts.iter_mut().for_each(|chart| chart.faces.sort_unstable());
		segmentation.merge();
		segmentation.finish()
	}

	/// Starts a chart from each face in no chart, largest first, and grows each alone up to the seed cost.
	fn place_seeds(&mut self) {
		for next in 0..self.seed_order.len() {
			let face = self.seed_order[next];
			if self.face_charts[face as usize] != NONE {
				continue;
			}
			let chart = self.charts.len() as u32;
			self.charts.push(Chart::new(face));
			self.add_region(chart, self.surface.face_regions[face as usize]);
			while let Some((cost, region)) = self.cheapest(chart) {
				if cost > SEED_COST {
					break;
				}
				self.try_add(chart, region);
			}
		}
	}

	/// Moves each chart's seed to its face nearest the mean of its faces' centers, and empties every chart back to
	/// its seed's region.
	fn relocate_seeds(&mut self) {
		for chart in &mut self.charts {
			let mean = chart.center_sum.map(|value| value / chart.faces.len() as f64);
			let mut nearest = (f64::INFINITY, NONE);
			for &face in &chart.faces {
				let offset = sub(self.surface.face_center(face as usize), mean);
				let distance = dot(offset, offset);
				if distance < nearest.0 || (distance == nearest.0 && face < nearest.1) {
					nearest = (distance, face);
				}
			}
			chart.seed = nearest.1;
		}
		self.face_charts.iter_mut().for_each(|chart| *chart = NONE);
		for chart in 0..self.charts.len() {
			let seed = self.charts[chart].seed;
			self.charts[chart] = Chart::new(seed);
			self.add_region(chart as u32, self.surface.face_regions[seed as usize]);
		}
	}

	/// Grows every chart together: the cheapest candidate of all joins its chart, while it costs at most the maximum.
	fn grow(&mut self) {
		let mut queue: BinaryHeap<Reverse<(Ordered, u32, u32, u32)>> = BinaryHeap::new();
		for chart in 0..self.charts.len() as u32 {
			self.queue_cheapest(chart, &mut queue);
		}
		while let Some(Reverse((Ordered(cost), chart, version, region))) = queue.pop() {
			if self.charts[chart as usize].version != version {
				continue;
			}
			// Another chart may have taken the region since it was priced.
			if self.region_chart(region) != NONE {
				self.charts[chart as usize].version += 1;
				self.queue_cheapest(chart, &mut queue);
				continue;
			}
			if cost > MAX_COST {
				break;
			}
			self.try_add(chart, region);
			self.queue_cheapest(chart, &mut queue);
		}
	}

	/// Queues the cheapest candidate of `chart` at its current version.
	fn queue_cheapest(&mut self, chart: u32, queue: &mut BinaryHeap<Reverse<(Ordered, u32, u32, u32)>>) {
		if let Some((cost, region)) = self.cheapest(chart) {
			queue.push(Reverse((Ordered(cost), chart, self.charts[chart as usize].version, region)));
		}
	}

	/// Returns the chart that holds `region`, or [`NONE`].
	fn region_chart(&self, region: u32) -> u32 {
		self.face_charts[self.surface.faces_of(region)[0] as usize]
	}

	/// Returns the cheapest candidate of `chart`, the lowest region first among equals, after dropping candidates
	/// another chart took.
	fn cheapest(&mut self, chart: u32) -> Option<Priced> {
		let (surface, face_charts) = (self.surface, &self.face_charts);
		self.charts[chart as usize]
			.frontier
			.retain(|&region| face_charts[surface.faces_of(region)[0] as usize] == NONE);
		let mut best: Option<Priced> = None;
		// Regions come in ascending order, so a later region replaces the best only when strictly cheaper.
		for &region in &self.charts[chart as usize].frontier {
			let cost = self.cost(chart, region);
			if cost.is_finite() && best.is_none_or(|(lowest, _)| cost < lowest) {
				best = Some((cost, region));
			}
		}
		best
	}

	/// Returns the cost of adding `region` to `chart`.
	fn cost(&self, chart: u32, region: u32) -> f64 {
		let surface = self.surface;
		let state = &self.charts[chart as usize];
		let deviation = (1.0 - dot(surface.regions[region as usize].normal, state.normal)).min(1.0);
		if deviation >= MAX_NORMAL_DEVIATION {
			return f64::INFINITY;
		}
		let (mut inside, mut outside) = (0.0, 0.0);
		for &corner in surface.edges_of(region) {
			let length = surface.edge_lengths[corner as usize];
			if borders(surface, &self.face_charts, corner, chart) {
				inside += length;
			} else {
				outside += length;
			}
		}
		let boundary = (state.boundary_length + outside - inside).max(0.0);
		let area = state.area + surface.regions[region as usize].area;
		let new_roundness = boundary * boundary / area;
		let roundness = if new_roundness > 0.0 {
			1.0 - (state.boundary_length * state.boundary_length / state.area) / new_roundness
		} else {
			0.0
		};
		let straightness = if inside + outside > 0.0 {
			((outside - inside) / (outside + inside)).min(0.0)
		} else {
			0.0
		};
		NORMAL_DEVIATION_WEIGHT * deviation + ROUNDNESS_WEIGHT * roundness + STRAIGHTNESS_WEIGHT * straightness
	}

	/// Adds `region` to `chart` when the chart's projection stays valid, and rejects it for good otherwise.
	fn try_add(&mut self, chart: u32, region: u32) {
		if self.projection_valid_with(chart, region) {
			self.add_region(chart, region);
		} else {
			let state = &mut self.charts[chart as usize];
			state.rejected.insert(region);
			state.frontier.remove(&region);
			state.version += 1;
		}
	}

	/// Returns whether the orthographic projection of `chart` with `region`'s faces, along their combined normal, keeps
	/// every face facing it and no two boundary edges crossing.
	fn projection_valid_with(&mut self, chart: u32, region: u32) -> bool {
		let surface = self.surface;
		let state = &self.charts[chart as usize];
		let normal = normalize(add(state.normal_sum, surface.regions[region as usize].normal_sum));
		if !surface.faces_of(region).iter().all(|&face| surface.faces_toward(face, normal)) {
			return false;
		}
		// No face's dot product with the normal drops more than the normal moved since the last full check.
		let drift = norm(sub(normal, state.checked_normal));
		let full_check = state.checked_dot - drift <= FLIP_EPSILON;
		if full_check && !state.faces.iter().all(|&face| surface.faces_toward(face, normal)) {
			return false;
		}

		// The boundary of the chart with the region: the chart's, less the edges the region closes, plus the region's
		// own outer edges that don't border the chart. A chart's boundary corner is closed when its opposite face lies
		// in the region.
		let face_charts = &self.face_charts;
		let closes = |corner: u32| {
			let other = surface.opposite[corner as usize];
			other != NONE && surface.face_regions[other as usize / 3] == region
		};
		let corners = state.boundary.iter().copied().filter(|&corner| !closes(corner)).chain(
			surface
				.edges_of(region)
				.iter()
				.copied()
				.filter(|&corner| !borders(surface, face_charts, corner, chart)),
		);
		boundary_edges(surface, corners, plane_basis(normal), &mut self.edges);
		if boundary_crosses(&self.edges, &mut self.cells) {
			return false;
		}
		if full_check {
			// The region's faces join the bound when they are added.
			let state = &mut self.charts[chart as usize];
			state.checked_normal = normal;
			state.checked_dot = state
				.faces
				.iter()
				.filter(|&&face| surface.face_areas[face as usize] > 0.0)
				.map(|&face| dot(surface.face_normals[face as usize], normal))
				.fold(1.0, f64::min);
		}
		true
	}

	/// Adds `region`'s faces to `chart` and updates its measures, boundary, and candidates.
	fn add_region(&mut self, chart: u32, region: u32) {
		let surface = self.surface;
		let first = self.charts[chart as usize].faces.is_empty();
		for &corner in surface.edges_of(region) {
			let length = surface.edge_lengths[corner as usize];
			let closes = borders(surface, &self.face_charts, corner, chart);
			let state = &mut self.charts[chart as usize];
			if closes {
				state.boundary.remove(&surface.opposite[corner as usize]);
				state.boundary_length -= length;
			} else {
				state.boundary.insert(corner);
				state.boundary_length += length;
			}
		}
		for &face in surface.faces_of(region) {
			self.face_charts[face as usize] = chart;
		}
		let state = &mut self.charts[chart as usize];
		state.boundary_length = state.boundary_length.max(0.0);
		let data = &surface.regions[region as usize];
		state.area += data.area;
		state.normal_sum = add(state.normal_sum, data.normal_sum);
		state.normal = normalize(state.normal_sum);
		for &face in surface.faces_of(region) {
			state.center_sum = add(state.center_sum, surface.face_center(face as usize));
			state.faces.push(face);
		}
		if first {
			state.checked_normal = state.normal;
		}
		// Faces added without a full check still bound the dot products the next check skips.
		state.checked_dot = surface
			.faces_of(region)
			.iter()
			.map(|&face| dot(surface.face_normals[face as usize], state.checked_normal))
			.fold(state.checked_dot, f64::min);
		state.frontier.remove(&region);
		state.version += 1;
		for &corner in surface.edges_of(region) {
			let other = surface.opposite[corner as usize];
			if other == NONE || self.face_charts[other as usize / 3] != NONE {
				continue;
			}
			let neighbor = surface.face_regions[other as usize / 3];
			let state = &mut self.charts[chart as usize];
			if !state.rejected.contains(&neighbor) {
				state.frontier.insert(neighbor);
			}
		}
	}

	/// Merges charts, from the last to the first, while a chart absorbs a neighbor by one of the merge rules.
	///
	/// An absorbed chart is left without faces until the charts are compacted at the end.
	fn merge(&mut self) {
		'restart: loop {
			for a in (0..self.charts.len()).rev() {
				if self.charts[a].faces.is_empty() {
					continue;
				}
				let (shared, external) = self.shared_boundaries(a as u32);
				let boundary_a = self.charts[a].boundary_length;
				for (&b, &(length, edges)) in shared.iter().rev() {
					let b = b as usize;
					let (chart_a, chart_b) = (&self.charts[a], &self.charts[b]);
					if dot(chart_a.normal, chart_b.normal) < MERGE_MIN_NORMAL_DOT {
						continue;
					}
					let boundary_b = chart_b.boundary_length;
					let absorbs = (chart_b.faces.len() == 1 && chart_a.faces.len() > 1 && chart_b.area <= chart_a.area * 0.1)
						|| (chart_b.faces.len() == 2 && edges >= 2)
						|| (length - boundary_b).abs() <= LENGTH_EPSILON * boundary_b.max(1.0)
						|| length > 0.2 * (boundary_a - external).max(0.0)
						|| length > 0.75 * boundary_b;
					if absorbs && self.merged_projection_valid(a, b) {
						self.absorb(a, b, length);
						continue 'restart;
					}
				}
			}
			break;
		}
		self.charts.retain(|chart| !chart.faces.is_empty());
		// Faces point at the compacted chart list.
		for (chart, state) in self.charts.iter().enumerate() {
			for &face in &state.faces {
				self.face_charts[face as usize] = chart as u32;
			}
		}
	}

	/// Returns, for `chart`, the length of boundary and the number of edges it shares with each other chart, by chart,
	/// and the length of its boundary on the mesh's boundary.
	fn shared_boundaries(&self, chart: u32) -> (BTreeMap<u32, (f64, u32)>, f64) {
		let (mut shared, mut external) = (BTreeMap::new(), 0.0);
		for &corner in &self.charts[chart as usize].boundary {
			let other = self.surface.opposite[corner as usize];
			let length = self.surface.edge_lengths[corner as usize];
			if other == NONE {
				external += length;
				continue;
			}
			let entry = shared.entry(self.face_charts[other as usize / 3]).or_insert((0.0, 0));
			entry.0 += length;
			entry.1 += 1;
		}
		(shared, external)
	}

	/// Returns whether the orthographic projection of charts `a` and `b` together is valid.
	fn merged_projection_valid(&mut self, a: usize, b: usize) -> bool {
		let surface = self.surface;
		let normal = normalize(add(self.charts[a].normal_sum, self.charts[b].normal_sum));
		let (chart_a, chart_b) = (&self.charts[a], &self.charts[b]);
		if !chart_a.faces.iter().chain(&chart_b.faces).all(|&face| surface.faces_toward(face, normal)) {
			return false;
		}
		// The edges the two charts share are inside the merged chart.
		let face_charts = &self.face_charts;
		let corners = chart_a.boundary.iter().chain(&chart_b.boundary).copied().filter(|&corner| {
			!borders(surface, face_charts, corner, a as u32) && !borders(surface, face_charts, corner, b as u32)
		});
		boundary_edges(surface, corners, plane_basis(normal), &mut self.edges);
		!boundary_crosses(&self.edges, &mut self.cells)
	}

	/// Moves chart `b`'s faces into chart `a`, which share `shared` of boundary length.
	fn absorb(&mut self, a: usize, b: usize, shared: f64) {
		let absorbed = std::mem::replace(&mut self.charts[b], Chart::new(NONE));
		for &face in &absorbed.faces {
			self.face_charts[face as usize] = a as u32;
		}
		let surface = self.surface;
		let chart = &mut self.charts[a];
		// Edges the two charts shared are inside now.
		chart.boundary.retain(|&corner| {
			let other = surface.opposite[corner as usize];
			other == NONE || absorbed.faces.binary_search(&(other / 3)).is_err()
		});
		for &corner in &absorbed.boundary {
			let other = surface.opposite[corner as usize];
			if other == NONE || chart.faces.binary_search(&(other / 3)).is_err() {
				chart.boundary.insert(corner);
			}
		}
		chart.faces.extend(&absorbed.faces);
		chart.faces.sort_unstable();
		chart.area += absorbed.area;
		chart.normal_sum = add(chart.normal_sum, absorbed.normal_sum);
		chart.normal = normalize(chart.normal_sum);
		chart.center_sum = add(chart.center_sum, absorbed.center_sum);
		chart.boundary_length = (chart.boundary_length + absorbed.boundary_length - 2.0 * shared).max(0.0);
	}

	/// Puts each face without area in the chart of its first neighbor with area, edge by edge, or a chart of its
	/// own, and returns the charts.
	fn finish(mut self) -> Vec<(Vec<u32>, [f64; 3])> {
		let surface = self.surface;
		for face in 0..surface.face_count() {
			if surface.face_areas[face] > 0.0 {
				continue;
			}
			let neighbor = (face * 3..face * 3 + 3)
				.map(|corner| surface.edge_neighbors[corner])
				.find(|&neighbor| neighbor != NONE);
			let chart = match neighbor {
				Some(neighbor) => self.face_charts[neighbor as usize],
				None => {
					self.charts.push(Chart::new(face as u32));
					(self.charts.len() - 1) as u32
				}
			};
			self.face_charts[face] = chart;
			let faces = &mut self.charts[chart as usize].faces;
			let position = faces.binary_search(&(face as u32)).unwrap_err();
			faces.insert(position, face as u32);
		}
		self.charts.into_iter().map(|chart| (chart.faces, chart.normal)).collect()
	}
}

/// Returns whether the edge of `corner` borders a face of `chart`, given each face's chart.
fn borders(surface: &Surface, face_charts: &[u32], corner: u32, chart: u32) -> bool {
	let other = surface.opposite[corner as usize];
	other != NONE && face_charts[other as usize / 3] == chart
}

/// Writes to `edges` the boundary edges of `corners`, projected onto the plane of `basis`.
fn boundary_edges(
	surface: &Surface,
	corners: impl Iterator<Item = u32>,
	basis: ([f64; 3], [f64; 3]),
	edges: &mut Vec<BoundaryEdge>,
) {
	edges.clear();
	edges.extend(corners.map(|corner| {
		let corner = corner as usize;
		BoundaryEdge {
			from: project(surface.corner_position(corner), basis),
			to: project(surface.corner_position(next_corner(corner)), basis),
			vertices: surface.edge_vertices(corner),
		}
	}));
}

/// The `Ordered` struct orders costs totally, so they can key the growth queue.
#[derive(Clone, Copy, PartialEq)]
struct Ordered(f64);

impl Eq for Ordered {}

impl PartialOrd for Ordered {
	fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
		Some(self.cmp(other))
	}
}

impl Ord for Ordered {
	fn cmp(&self, other: &Self) -> std::cmp::Ordering {
		self.0.total_cmp(&other.0)
	}
}
