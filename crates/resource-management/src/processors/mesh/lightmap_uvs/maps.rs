//! Flattening charts: orthographic projections, their stretch, least squares conformal maps, and the checks that make a
//! map valid. The Parameterization table of the asset reference's Lightmap UVs section defines each rule.

use super::surface::{NONE, Surface, cross, dot, next_corner, norm, normalize, sub};

/// The stretch at or below which a chart keeps its orthographic projection.
const MAX_ORTHOGRAPHIC_STRETCH: f64 = 1.1;
/// The largest single-triangle stretch at or below which a chart keeps its orthographic projection.
const MAX_ORTHOGRAPHIC_TRIANGLE_STRETCH: f64 = 1.25;
/// The residual, relative to the right-hand side, at which the conformal map's solve stops.
const SOLVE_TOLERANCE: f64 = 1e-6;
/// The solve's iterations per unknown, at most.
const SOLVE_ITERATIONS_PER_UNKNOWN: usize = 5;
/// The projected area, relative to a face's own, at or below which a face counts as having none.
const FLAT_EPSILON: f64 = 1e-9;

/// Returns the axes of the plane perpendicular to `normal`, so projecting onto them keeps the faces that face `normal`
/// counterclockwise.
pub(super) fn plane_basis(normal: [f64; 3]) -> ([f64; 3], [f64; 3]) {
	// The axis least aligned with the normal gives a tangent that never degenerates.
	let axis = (0..3)
		.min_by(|&a, &b| normal[a].abs().total_cmp(&normal[b].abs()))
		.expect("three axes");
	let mut helper = [0.0; 3];
	helper[axis] = 1.0;
	let tangent = normalize(cross(helper, normal));
	let bitangent = cross(normal, tangent);
	(tangent, bitangent)
}

/// Projects `point` onto the plane of `basis`.
pub(super) fn project(point: [f64; 3], (tangent, bitangent): ([f64; 3], [f64; 3])) -> [f64; 2] {
	[dot(point, tangent), dot(point, bitangent)]
}

/// The `BoundaryEdge` struct is one boundary edge of a chart in the plane: its two ends and their welded vertices.
#[derive(Clone, Copy)]
pub(super) struct BoundaryEdge {
	pub(super) from: [f64; 2],
	pub(super) to: [f64; 2],
	pub(super) vertices: [u32; 2],
}

/// Returns whether two of `edges` cross or touch other than at a welded vertex they share.
///
/// Edges are bucketed in a uniform grid over their bounds, so only edges that share a cell are tested.
pub(super) fn boundary_crosses(edges: &[BoundaryEdge], cells: &mut Vec<Vec<u32>>) -> bool {
	if edges.len() < 2 {
		return false;
	}
	let mut minimum = [f64::INFINITY; 2];
	let mut maximum = [f64::NEG_INFINITY; 2];
	for edge in edges {
		for point in [edge.from, edge.to] {
			for axis in 0..2 {
				minimum[axis] = minimum[axis].min(point[axis]);
				maximum[axis] = maximum[axis].max(point[axis]);
			}
		}
	}
	let side = (edges.len() as f64).sqrt().ceil().max(1.0) as usize;
	let extent = [0, 1].map(|axis| (maximum[axis] - minimum[axis]).max(f64::MIN_POSITIVE));
	let cell_of = |value: f64, axis: usize| {
		(((value - minimum[axis]) / extent[axis] * side as f64) as usize).min(side - 1)
	};
	// The grid only grows, so cells keep their memory across calls of different sizes.
	if cells.len() < side * side {
		cells.resize_with(side * side, Vec::new);
	}
	let cells = &mut cells[..side * side];
	cells.iter_mut().for_each(Vec::clear);
	for (index, edge) in edges.iter().enumerate() {
		let [x0, x1] = [edge.from[0].min(edge.to[0]), edge.from[0].max(edge.to[0])].map(|x| cell_of(x, 0));
		let [y0, y1] = [edge.from[1].min(edge.to[1]), edge.from[1].max(edge.to[1])].map(|y| cell_of(y, 1));
		for y in y0..=y1 {
			for x in x0..=x1 {
				cells[y * side + x].push(index as u32);
			}
		}
	}
	for cell in cells.iter() {
		for (position, &a) in cell.iter().enumerate() {
			for &b in &cell[position + 1..] {
				let (a, b) = (&edges[a as usize], &edges[b as usize]);
				if a.vertices.iter().any(|vertex| b.vertices.contains(vertex)) {
					continue;
				}
				if segments_meet(a.from, a.to, b.from, b.to) {
					return true;
				}
			}
		}
	}
	false
}

/// Returns twice the signed area of the triangle `a`, `b`, `c`.
fn orientation(a: [f64; 2], b: [f64; 2], c: [f64; 2]) -> f64 {
	(b[0] - a[0]) * (c[1] - a[1]) - (b[1] - a[1]) * (c[0] - a[0])
}

/// Returns whether the segments `a`–`b` and `c`–`d` share any point.
fn segments_meet(a: [f64; 2], b: [f64; 2], c: [f64; 2], d: [f64; 2]) -> bool {
	let [o1, o2, o3, o4] = [orientation(a, b, c), orientation(a, b, d), orientation(c, d, a), orientation(c, d, b)];
	if o1 * o2 < 0.0 && o3 * o4 < 0.0 {
		return true;
	}
	// A collinear end touches the other segment when it lies within its bounds.
	let within = |p: [f64; 2], q: [f64; 2], r: [f64; 2]| {
		r[0] >= p[0].min(q[0]) && r[0] <= p[0].max(q[0]) && r[1] >= p[1].min(q[1]) && r[1] <= p[1].max(q[1])
	};
	(o1 == 0.0 && within(a, b, c))
		|| (o2 == 0.0 && within(a, b, d))
		|| (o3 == 0.0 && within(c, d, a))
		|| (o4 == 0.0 && within(c, d, b))
}

/// The `ChartMap` struct is one chart flattened: the plane point of each of its welded vertices.
pub(super) struct ChartMap {
	/// The plane point of each local vertex, in the same units as the surface.
	pub(super) points: Vec<[f64; 2]>,
	/// The chart's surface area.
	pub(super) area: f64,
}

/// The `MapScratch` struct keeps working memory across the charts of a primitive.
#[derive(Default)]
pub(super) struct MapScratch {
	/// The local vertex of each welded vertex in the chart being mapped, or [`NONE`].
	local_of_vertex: Vec<u32>,
	/// The welded vertex of each local vertex.
	vertices: Vec<u32>,
	pub(super) cells: Vec<Vec<u32>>,
	boundary: Vec<BoundaryEdge>,
}

/// Flattens the chart of `faces`, whose normal is `normal`, and writes each of its corners' local vertex to
/// `corner_locals`.
///
/// The chart keeps its orthographic projection when it stretches little, and takes its conformal map otherwise, unless
/// that map is invalid. The result is scaled to the chart's surface area and turned so the smallest rectangle around
/// it lies along the axes.
pub(super) fn map_chart(
	surface: &Surface,
	faces: &[u32],
	normal: [f64; 3],
	corner_locals: &mut [u32],
	scratch: &mut MapScratch,
) -> ChartMap {
	scratch.local_of_vertex.resize(surface.positions.len(), NONE);
	scratch.vertices.clear();
	for &face in faces {
		for corner in face as usize * 3..face as usize * 3 + 3 {
			let vertex = surface.corner_vertices[corner];
			let local = &mut scratch.local_of_vertex[vertex as usize];
			if *local == NONE {
				*local = scratch.vertices.len() as u32;
				scratch.vertices.push(vertex);
			}
			corner_locals[corner] = *local;
		}
	}
	let basis = plane_basis(if norm(normal) > 0.0 { normal } else { [0.0, 0.0, 1.0] });
	let orthographic: Vec<[f64; 2]> = scratch
		.vertices
		.iter()
		.map(|&vertex| project(surface.positions[vertex as usize], basis))
		.collect();
	let area: f64 = faces.iter().map(|&face| surface.face_areas[face as usize]).sum();

	let (stretch, max_stretch) = stretch(surface, faces, corner_locals, &orthographic);
	let mut points = if stretch <= MAX_ORTHOGRAPHIC_STRETCH && max_stretch <= MAX_ORTHOGRAPHIC_TRIANGLE_STRETCH {
		orthographic
	} else {
		match conformal_map(surface, faces, corner_locals, &orthographic) {
			Some(conformal) if valid_map(surface, faces, corner_locals, &conformal, scratch) => conformal,
			_ => orthographic,
		}
	};

	// Scaled to the chart's surface area, then turned onto its smallest rectangle.
	let planar_area = faces
		.iter()
		.map(|&face| {
			let [a, b, c] = corner_points(&points, corner_locals, face as usize);
			orientation(a, b, c) / 2.0
		})
		.sum::<f64>()
		.abs();
	if planar_area > 0.0 && area > 0.0 {
		let factor = (area / planar_area).sqrt();
		points.iter_mut().for_each(|point| *point = point.map(|value| value * factor));
	}
	align_to_smallest_rectangle(&mut points);

	for &vertex in &scratch.vertices {
		scratch.local_of_vertex[vertex as usize] = NONE;
	}
	ChartMap { points, area }
}

/// Returns the plane points of face `face`'s three corners.
fn corner_points(points: &[[f64; 2]], corner_locals: &[u32], face: usize) -> [[f64; 2]; 3] {
	[0, 1, 2].map(|corner| points[corner_locals[face * 3 + corner] as usize])
}

/// Returns Sander et al.'s L2 stretch of `points` as a map of the chart's faces, and its largest per-triangle stretch,
/// both scaled by the square root of the map's area over the surface's.
fn stretch(surface: &Surface, faces: &[u32], corner_locals: &[u32], points: &[[f64; 2]]) -> (f64, f64) {
	let (mut sum, mut largest, mut geometric, mut parametric) = (0.0, 0.0f64, 0.0, 0.0);
	for &face in faces {
		let face = face as usize;
		let [[t1, s1], [t2, s2], [t3, s3]] = corner_points(points, corner_locals, face);
		let parametric_area = (((s2 - s1) * (t3 - t1) - (s3 - s1) * (t2 - t1)) / 2.0).abs();
		let geometric_area = surface.face_areas[face];
		if parametric_area <= geometric_area * FLAT_EPSILON || geometric_area <= 0.0 {
			continue;
		}
		let [p1, p2, p3] = [0, 1, 2].map(|corner| surface.corner_position(face * 3 + corner));
		let combine = |w1: f64, w2: f64, w3: f64| {
			[0, 1, 2].map(|axis| (p1[axis] * w1 + p2[axis] * w2 + p3[axis] * w3) / (2.0 * parametric_area))
		};
		let ss = combine(t2 - t3, t3 - t1, t1 - t2);
		let st = combine(s3 - s2, s1 - s3, s2 - s1);
		let (a, b, c) = (dot(ss, ss), dot(ss, st), dot(st, st));
		let root = ((a - c) * (a - c) + 4.0 * b * b).sqrt();
		let sigma2 = (0.5 * (a + c + root)).max(0.0).sqrt();
		sum += (a + c) * 0.5 * geometric_area;
		largest = largest.max(sigma2);
		geometric += geometric_area;
		parametric += parametric_area;
	}
	if geometric <= 0.0 {
		return (f64::INFINITY, f64::INFINITY);
	}
	let factor = (parametric / geometric).sqrt();
	((sum / geometric).sqrt() * factor, largest * factor)
}

/// Returns whether `points` map the chart's faces with area without flipping any, with every face's area of one sign,
/// and without two boundary edges crossing.
fn valid_map(
	surface: &Surface,
	faces: &[u32],
	corner_locals: &[u32],
	points: &[[f64; 2]],
	scratch: &mut MapScratch,
) -> bool {
	let (mut positive, mut negative) = (false, false);
	for &face in faces {
		let face = face as usize;
		if surface.face_areas[face] <= 0.0 {
			continue;
		}
		let [a, b, c] = corner_points(points, corner_locals, face);
		let area = orientation(a, b, c) / 2.0;
		if area.abs() <= surface.face_areas[face] * FLAT_EPSILON || !area.is_finite() {
			return false;
		}
		positive |= area > 0.0;
		negative |= area < 0.0;
	}
	if positive && negative {
		return false;
	}
	scratch.boundary.clear();
	for &face in faces {
		for corner in face as usize * 3..face as usize * 3 + 3 {
			let other = surface.opposite[corner];
			if other != NONE && surface.face_regions[other as usize / 3] != NONE && faces.binary_search(&(other / 3)).is_ok() {
				continue;
			}
			scratch.boundary.push(BoundaryEdge {
				from: points[corner_locals[corner] as usize],
				to: points[corner_locals[next_corner(corner)] as usize],
				vertices: surface.edge_vertices(corner),
			});
		}
	}
	let MapScratch { boundary, cells, .. } = scratch;
	!boundary_crosses(boundary, cells)
}

/// Solves the least squares conformal map (Lévy et al., 2002) of the chart's faces, with the two local vertices
/// farthest apart in `orthographic` pinned there, or returns `None` when the chart has no two distinct vertices.
///
/// Each face with area gives two rows of the conformality condition in its own plane; the normal equations are solved
/// by conjugate gradients with a Jacobi preconditioner, from the orthographic points.
fn conformal_map(
	surface: &Surface,
	faces: &[u32],
	corner_locals: &[u32],
	orthographic: &[[f64; 2]],
) -> Option<Vec<[f64; 2]>> {
	let (pin0, pin1) = diameter(orthographic)?;
	let unknowns = orthographic.len() * 2;

	// Rows of five coefficients each: (unknown, coefficient).
	let mut rows: Vec<[(u32, f64); 5]> = Vec::with_capacity(faces.len() * 2);
	for &face in faces {
		let face = face as usize;
		if surface.face_areas[face] <= 0.0 {
			continue;
		}
		let [p0, p1, p2] = [0, 1, 2].map(|corner| surface.corner_position(face * 3 + corner));
		let [v0, v1, v2] = [0, 1, 2].map(|corner| corner_locals[face * 3 + corner]);
		// The triangle in its own plane: z0 at the origin, z1 along x.
		let x = normalize(sub(p1, p0));
		let z = normalize(cross(sub(p1, p0), sub(p2, p0)));
		let y = cross(z, x);
		let a = norm(sub(p1, p0));
		let (c, d) = (dot(sub(p2, p0), x), dot(sub(p2, p0), y));
		let (u0, w0, u1, w1, u2, w2) = (2 * v0, 2 * v0 + 1, 2 * v1, 2 * v1 + 1, 2 * v2, 2 * v2 + 1);
		rows.push([(u0, -a + c), (w0, -d), (u1, -c), (w1, d), (u2, a)]);
		rows.push([(u0, d), (w0, -a + c), (u1, -d), (w1, -c), (w2, a)]);
	}

	// The solve works on the points in place, starting from the orthographic ones.
	let mut points = orthographic.to_vec();
	let values = points.as_flattened_mut();
	let pinned = |unknown: u32| unknown / 2 == pin0 || unknown / 2 == pin1;
	// The pinned unknowns move to the right-hand side; their coefficients then drop out of the rows, so the solve
	// sees the free unknowns only.
	let mut rhs = vec![0.0; unknowns];
	let mut diagonal = vec![0.0; unknowns];
	for row in &mut rows {
		let fixed: f64 = row
			.iter()
			.filter(|(unknown, _)| pinned(*unknown))
			.map(|&(unknown, coefficient)| coefficient * values[unknown as usize])
			.sum();
		for (unknown, coefficient) in row.iter_mut() {
			if pinned(*unknown) {
				*coefficient = 0.0;
			} else {
				rhs[*unknown as usize] -= *coefficient * fixed;
				diagonal[*unknown as usize] += *coefficient * *coefficient;
			}
		}
	}
	// AᵀA·x over the free unknowns.
	let apply = |x: &[f64], out: &mut [f64]| {
		out.iter_mut().for_each(|value| *value = 0.0);
		for row in &rows {
			let sum: f64 = row.iter().map(|&(unknown, coefficient)| coefficient * x[unknown as usize]).sum();
			for &(unknown, coefficient) in row {
				out[unknown as usize] += coefficient * sum;
			}
		}
	};
	// Unknowns no row reaches keep their orthographic point.
	let inverse: Vec<f64> = diagonal.iter().map(|&value| if value > 0.0 { 1.0 / value } else { 0.0 }).collect();

	let mut product = vec![0.0; unknowns];
	apply(values, &mut product);
	let mut residual: Vec<f64> = rhs.iter().zip(&product).map(|(b, ax)| b - ax).collect();
	let mut preconditioned: Vec<f64> = residual.iter().zip(&inverse).map(|(r, m)| r * m).collect();
	let mut direction = preconditioned.clone();
	let mut rho: f64 = residual.iter().zip(&preconditioned).map(|(r, z)| r * z).sum();
	let target = SOLVE_TOLERANCE * rhs.iter().map(|value| value * value).sum::<f64>().sqrt().max(f64::MIN_POSITIVE);
	for _ in 0..unknowns * SOLVE_ITERATIONS_PER_UNKNOWN {
		if residual.iter().map(|value| value * value).sum::<f64>().sqrt() <= target {
			break;
		}
		apply(&direction, &mut product);
		let curvature: f64 = direction.iter().zip(&product).map(|(p, ap)| p * ap).sum();
		if curvature <= 0.0 || !curvature.is_finite() {
			break;
		}
		let step = rho / curvature;
		for unknown in 0..unknowns {
			values[unknown] += step * direction[unknown];
			residual[unknown] -= step * product[unknown];
			preconditioned[unknown] = residual[unknown] * inverse[unknown];
		}
		let next_rho: f64 = residual.iter().zip(&preconditioned).map(|(r, z)| r * z).sum();
		let beta = next_rho / rho;
		rho = next_rho;
		for unknown in 0..unknowns {
			direction[unknown] = preconditioned[unknown] + beta * direction[unknown];
		}
	}
	if values.iter().any(|value| !value.is_finite()) {
		return None;
	}
	Some(points)
}

/// Returns the two points of `points` farthest apart, the lowest indices first among equals, or `None` when no two
/// points differ.
fn diameter(points: &[[f64; 2]]) -> Option<(u32, u32)> {
	let hull = convex_hull(points);
	let mut best: Option<(f64, u32, u32)> = None;
	for (position, &a) in hull.iter().enumerate() {
		for &b in &hull[position + 1..] {
			let [dx, dy] = [points[b as usize][0] - points[a as usize][0], points[b as usize][1] - points[a as usize][1]];
			let distance = dx * dx + dy * dy;
			let pair = (a.min(b), a.max(b));
			let better = match best {
				None => true,
				Some((longest, first, second)) => distance > longest || (distance == longest && pair < (first, second)),
			};
			if better {
				best = Some((distance, pair.0, pair.1));
			}
		}
	}
	best.filter(|&(distance, _, _)| distance > 0.0).map(|(_, a, b)| (a, b))
}

/// Returns the indices of the convex hull of `points`, counterclockwise, by Andrew's monotone chain.
fn convex_hull(points: &[[f64; 2]]) -> Vec<u32> {
	let mut order: Vec<u32> = (0..points.len() as u32).collect();
	order.sort_by(|&a, &b| {
		let (a, b) = (points[a as usize], points[b as usize]);
		a[0].total_cmp(&b[0]).then(a[1].total_cmp(&b[1]))
	});
	order.dedup_by(|a, b| points[*a as usize] == points[*b as usize]);
	if order.len() < 3 {
		return order;
	}
	let mut hull: Vec<u32> = Vec::with_capacity(order.len() * 2);
	// The lower hull left to right, then the upper hull right to left.
	for pass in 0..2 {
		let start = hull.len();
		for step in 0..order.len() {
			let index = order[if pass == 0 { step } else { order.len() - 1 - step }];
			while hull.len() >= start + 2
				&& orientation(
					points[hull[hull.len() - 2] as usize],
					points[hull[hull.len() - 1] as usize],
					points[index as usize],
				) <= 0.0
			{
				hull.pop();
			}
			hull.push(index);
		}
		hull.pop();
	}
	hull
}

/// Turns `points` so the smallest-area rectangle around them, which lies along one edge of their convex hull, lies
/// along the axes, then moves them so the rectangle starts at the origin.
fn align_to_smallest_rectangle(points: &mut [[f64; 2]]) {
	let hull = convex_hull(points);
	let mut best: Option<(f64, [f64; 2])> = None;
	for position in 0..hull.len() {
		let (a, b) = (points[hull[position] as usize], points[hull[(position + 1) % hull.len()] as usize]);
		let edge = [b[0] - a[0], b[1] - a[1]];
		let length = (edge[0] * edge[0] + edge[1] * edge[1]).sqrt();
		if length <= 0.0 {
			continue;
		}
		let axis = [edge[0] / length, edge[1] / length];
		let (mut low, mut high) = ([f64::INFINITY; 2], [f64::NEG_INFINITY; 2]);
		for &index in &hull {
			let point = points[index as usize];
			let turned = turn(point, axis);
			for component in 0..2 {
				low[component] = low[component].min(turned[component]);
				high[component] = high[component].max(turned[component]);
			}
		}
		let area = (high[0] - low[0]) * (high[1] - low[1]);
		if best.is_none_or(|(smallest, _)| area < smallest) {
			best = Some((area, axis));
		}
	}
	let axis = best.map_or([1.0, 0.0], |(_, axis)| axis);
	let mut low = [f64::INFINITY; 2];
	for point in points.iter_mut() {
		*point = turn(*point, axis);
		for component in 0..2 {
			low[component] = low[component].min(point[component]);
		}
	}
	for point in points.iter_mut() {
		*point = [point[0] - low[0], point[1] - low[1]];
	}
}

/// Returns `point` turned so the unit vector `axis` lies along the first axis.
fn turn(point: [f64; 2], axis: [f64; 2]) -> [f64; 2] {
	[point[0] * axis[0] + point[1] * axis[1], -point[0] * axis[1] + point[1] * axis[0]]
}
