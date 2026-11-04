//! The texels of a lightmap that hold light, and the point on the surface each one traces its paths from.
//!
//! Each lit primitive's triangles are drawn into its rectangle of the image in lightmap UV space, on the CPU. A texel
//! whose center a triangle covers takes the triangle's point at its center. A texel that a triangle only crosses takes
//! the triangle's point nearest its center, so the texels along a chart's edges, which bilinear reads of the renderer
//! reach, hold light too. The rules are the Texels and Paths rows of the asset formats reference's Lightmaps section.

use super::atlas::Rectangle;

/// The image texel of a texel no triangle crosses. Matches `NO_LIGHTMAP_TEXEL` in `lightmaps/layouts/lightmap_bake.besl`.
pub(crate) const NO_LIGHTMAP_TEXEL: u32 = u32::MAX;
/// How far above its surface a texel's paths leave from, in texel widths.
const PATH_OFFSET_TEXELS: f32 = 0.25;

/// The `LightmapTexel` struct is one texel the bake traces paths from, as the trace kernel reads it.
///
/// Its layout matches `LightmapTexel` in `lightmaps/layouts/lightmap_bake.besl`.
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, bytemuck::Pod, bytemuck::Zeroable)]
pub(crate) struct LightmapTexel {
	/// The point on the surface the paths leave from, in world space.
	pub(crate) position: [f32; 3],
	/// The texel's index in the image, row after row.
	pub(crate) pixel: u32,
	/// The triangle's face normal, turned to the side its vertex normals face.
	pub(crate) normal: [f32; 3],
	/// How far above the surface the paths leave from, in meters: a quarter of the texel's width there.
	pub(crate) offset: f32,
}

/// The `TexelSurface` struct is one lit primitive as the raster reads it: its triangles in world space with their
/// lightmap UVs.
pub(crate) struct TexelSurface<'a> {
	/// Each vertex's position, in world space.
	pub(crate) positions: &'a [[f32; 3]],
	/// Each vertex's normal, in world space. Only its side of the triangle matters.
	pub(crate) normals: &'a [[f32; 3]],
	/// Each vertex's lightmap UV, from 0 through 1.
	pub(crate) uvs: &'a [[f32; 2]],
	pub(crate) triangles: &'a [[u32; 3]],
}

impl TexelSurface<'_> {
	/// Returns the area the triangles cover, in square meters, and the share of the lightmap UV square they cover, which
	/// size the primitive's rectangle with [`super::atlas::rectangle_side`].
	pub(crate) fn areas(&self) -> (f64, f64) {
		let mut world = 0.0f64;
		let mut uv = 0.0f64;
		for &[a, b, c] in self.triangles {
			let [a, b, c] = [a, b, c].map(|index| index as usize);
			world += f64::from(length(cross(sub(self.positions[b], self.positions[a]), sub(self.positions[c], self.positions[a])))) / 2.0;
			uv += f64::from(cross_2d(sub_2d(self.uvs[b], self.uvs[a]), sub_2d(self.uvs[c], self.uvs[a])).abs()) / 2.0;
		}
		(world, uv)
	}
}

/// The `TexelRaster` struct draws lit primitives into their rectangles and keeps its working memory across them.
#[derive(Default)]
pub(crate) struct TexelRaster {
	/// For each texel of the rectangle being drawn, the nearest triangle point found so far.
	candidates: Vec<Candidate>,
}

/// The nearest point of a triangle to one texel's center found so far.
#[derive(Clone, Copy)]
struct Candidate {
	/// The distance from the texel's center to the point, in texels; zero when a triangle covers the center.
	distance: f32,
	/// The triangle, or `u32::MAX` while no triangle crosses the texel.
	triangle: u32,
	/// The point's weights of the triangle's three vertices.
	weights: [f32; 3],
}

const NO_CANDIDATE: Candidate = Candidate {
	distance: f32::INFINITY,
	triangle: u32::MAX,
	weights: [0.0; 3],
};

impl TexelRaster {
	/// Draws `surface` into `rectangle` of an image `extent` texels wide, appends a texel to `texels` for each texel of
	/// the image a triangle crosses, and writes its index to that texel of `pixels`, which holds one entry per image
	/// texel.
	pub(crate) fn rasterize(
		&mut self,
		surface: &TexelSurface<'_>,
		rectangle: Rectangle,
		extent: u32,
		texels: &mut Vec<LightmapTexel>,
		pixels: &mut [u32],
	) {
		let side = rectangle.side as usize;
		self.candidates.clear();
		self.candidates.resize(side * side, NO_CANDIDATE);
		let scale = rectangle.side as f32;
		for (index, &[a, b, c]) in surface.triangles.iter().enumerate() {
			// The triangle in the rectangle's texels, whose centers lie at half-integer coordinates.
			let corners = [a, b, c].map(|vertex| surface.uvs[vertex as usize].map(|value| value * scale));
			if cross_2d(sub_2d(corners[1], corners[0]), sub_2d(corners[2], corners[0])).abs() <= f32::EPSILON {
				continue;
			}
			let low = |axis: usize| corners.iter().map(|corner| corner[axis]).fold(f32::INFINITY, f32::min);
			let high = |axis: usize| corners.iter().map(|corner| corner[axis]).fold(f32::NEG_INFINITY, f32::max);
			let span = |axis: usize| {
				let first = (low(axis).floor().max(0.0) as usize).min(side);
				let last = (high(axis).ceil().max(0.0) as usize).min(side);
				first..last
			};
			for y in span(1) {
				for x in span(0) {
					let center = [x as f32 + 0.5, y as f32 + 0.5];
					if !overlaps_texel(&corners, center) {
						continue;
					}
					let (distance, weights) = nearest_point(&corners, center);
					let candidate = &mut self.candidates[y * side + x];
					if distance < candidate.distance {
						*candidate = Candidate {
							distance,
							triangle: index as u32,
							weights,
						};
					}
				}
			}
		}

		for (local, candidate) in self.candidates.iter().enumerate() {
			if candidate.triangle == u32::MAX {
				continue;
			}
			let Some(texel) = surface_texel(surface, candidate, scale) else {
				continue;
			};
			let pixel = (rectangle.y as usize + local / side) * extent as usize + rectangle.x as usize + local % side;
			pixels[pixel] = texels.len() as u32;
			texels.push(LightmapTexel {
				pixel: pixel as u32,
				..texel
			});
		}
	}
}

/// Returns the texel at a candidate's point of its triangle, without its pixel, or `None` for a triangle without area
/// in world space.
fn surface_texel(surface: &TexelSurface<'_>, candidate: &Candidate, scale: f32) -> Option<LightmapTexel> {
	let vertices = surface.triangles[candidate.triangle as usize].map(|vertex| vertex as usize);
	let [p0, p1, p2] = vertices.map(|vertex| surface.positions[vertex]);
	let face = cross(sub(p1, p0), sub(p2, p0));
	let world_area = length(face);
	if !(world_area > 0.0) {
		return None;
	}
	let weighted = |values: [[f32; 3]; 3]| {
		std::array::from_fn::<f32, 3, _>(|axis| (0..3).map(|vertex| values[vertex][axis] * candidate.weights[vertex]).sum())
	};
	let position = weighted([p0, p1, p2]);
	let vertex_normal = weighted(vertices.map(|vertex| surface.normals[vertex]));
	let mut normal = face.map(|value| value / world_area);
	if dot(normal, vertex_normal) < 0.0 {
		normal = normal.map(|value| -value);
	}
	// A texel's width on the surface: the square root of the triangle's world area per texel of area.
	let [u0, u1, u2] = vertices.map(|vertex| surface.uvs[vertex].map(|value| value * scale));
	let texel_area = cross_2d(sub_2d(u1, u0), sub_2d(u2, u0)).abs();
	let texel_width = (world_area / texel_area).sqrt();
	Some(LightmapTexel {
		position,
		pixel: NO_LIGHTMAP_TEXEL,
		normal,
		offset: PATH_OFFSET_TEXELS * texel_width,
	})
}

/// Returns whether the triangle `corners` overlaps the texel centered at `center`, by the separating axis test: the
/// texel's axes and the normals of the triangle's edges.
fn overlaps_texel(corners: &[[f32; 2]; 3], center: [f32; 2]) -> bool {
	let separated = |axis: [f32; 2]| {
		let projections = corners.map(|corner| dot_2d(corner, axis));
		let (low, high) = (
			projections.iter().copied().fold(f32::INFINITY, f32::min),
			projections.iter().copied().fold(f32::NEG_INFINITY, f32::max),
		);
		let middle = dot_2d(center, axis);
		let reach = 0.5 * (axis[0].abs() + axis[1].abs());
		high <= middle - reach || low >= middle + reach
	};
	let edges = [0, 1, 2].map(|edge| {
		let [x, y] = sub_2d(corners[(edge + 1) % 3], corners[edge]);
		[-y, x]
	});
	!([[1.0, 0.0], [0.0, 1.0]].into_iter().chain(edges).any(separated))
}

/// Returns the distance from `point` to the triangle `corners` and the weights of its vertices at the triangle's point
/// nearest it: zero and the point's own weights when the triangle covers it.
fn nearest_point(corners: &[[f32; 2]; 3], point: [f32; 2]) -> (f32, [f32; 3]) {
	let weights = barycentric(corners, point);
	if weights.iter().all(|&weight| weight >= 0.0) {
		return (0.0, weights);
	}
	let mut nearest = (f32::INFINITY, weights);
	for edge in 0..3 {
		let (start, end) = (corners[edge], corners[(edge + 1) % 3]);
		let along = sub_2d(end, start);
		let length_squared = dot_2d(along, along);
		let t = if length_squared > 0.0 {
			(dot_2d(sub_2d(point, start), along) / length_squared).clamp(0.0, 1.0)
		} else {
			0.0
		};
		let closest = [start[0] + along[0] * t, start[1] + along[1] * t];
		let offset = sub_2d(point, closest);
		let distance = dot_2d(offset, offset).sqrt();
		if distance < nearest.0 {
			let mut edge_weights = [0.0; 3];
			edge_weights[edge] = 1.0 - t;
			edge_weights[(edge + 1) % 3] = t;
			nearest = (distance, edge_weights);
		}
	}
	nearest
}

/// Returns the weights of the triangle's three vertices at `point`, which sum to one.
fn barycentric(corners: &[[f32; 2]; 3], point: [f32; 2]) -> [f32; 3] {
	let area = cross_2d(sub_2d(corners[1], corners[0]), sub_2d(corners[2], corners[0]));
	let w1 = cross_2d(sub_2d(point, corners[0]), sub_2d(corners[2], corners[0])) / area;
	let w2 = cross_2d(sub_2d(corners[1], corners[0]), sub_2d(point, corners[0])) / area;
	[1.0 - w1 - w2, w1, w2]
}

fn sub(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
	[a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}

fn cross(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
	[a[1] * b[2] - a[2] * b[1], a[2] * b[0] - a[0] * b[2], a[0] * b[1] - a[1] * b[0]]
}

fn dot(a: [f32; 3], b: [f32; 3]) -> f32 {
	a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

fn length(a: [f32; 3]) -> f32 {
	dot(a, a).sqrt()
}

fn sub_2d(a: [f32; 2], b: [f32; 2]) -> [f32; 2] {
	[a[0] - b[0], a[1] - b[1]]
}

fn dot_2d(a: [f32; 2], b: [f32; 2]) -> f32 {
	a[0] * b[0] + a[1] * b[1]
}

fn cross_2d(a: [f32; 2], b: [f32; 2]) -> f32 {
	a[0] * b[1] - a[1] * b[0]
}

#[cfg(test)]
mod tests {
	use super::*;

	/// A 2 × 2 m floor at y = 1, facing up, whose lightmap UVs fill the square, as two triangles.
	fn floor() -> ([[f32; 3]; 4], [[f32; 3]; 4], [[f32; 2]; 4], [[u32; 3]; 2]) {
		(
			[[0.0, 1.0, 0.0], [2.0, 1.0, 0.0], [2.0, 1.0, 2.0], [0.0, 1.0, 2.0]],
			[[0.0, 1.0, 0.0]; 4],
			[[0.0, 0.0], [1.0, 0.0], [1.0, 1.0], [0.0, 1.0]],
			[[0, 2, 1], [0, 3, 2]],
		)
	}

	#[test]
	fn every_texel_a_surface_covers_traces_from_its_center_above_the_surface() {
		let (positions, normals, uvs, triangles) = floor();
		let surface = TexelSurface {
			positions: &positions,
			normals: &normals,
			uvs: &uvs,
			triangles: &triangles,
		};
		let (world, uv) = surface.areas();
		assert!((world - 4.0).abs() < 1.0e-6 && (uv - 1.0).abs() < 1.0e-6);

		let extent = 8;
		let rectangle = Rectangle { x: 2, y: 3, side: 4 };
		let mut pixels = vec![NO_LIGHTMAP_TEXEL; (extent * extent) as usize];
		let mut texels = Vec::new();
		TexelRaster::default().rasterize(&surface, rectangle, extent, &mut texels, &mut pixels);

		assert_eq!(texels.len(), 16, "the rectangle's 4 × 4 texels hold light");
		for y in 0..extent {
			for x in 0..extent {
				let inside = (2..6).contains(&x) && (3..7).contains(&y);
				assert_eq!(pixels[(y * extent + x) as usize] != NO_LIGHTMAP_TEXEL, inside, "texel ({x}, {y})");
			}
		}
		// The rectangle's texel (1, 2) has its center at UV (0.375, 0.625), half a meter per texel on the floor.
		let texel = texels[pixels[(5 * extent + 3) as usize] as usize];
		assert_eq!(texel.pixel, 5 * extent + 3);
		assert!((texel.position[0] - 0.75).abs() < 1.0e-5 && (texel.position[2] - 1.25).abs() < 1.0e-5);
		assert_eq!(texel.normal, [0.0, 1.0, 0.0], "the face normal turns to the side the vertex normals face");
		assert!((texel.offset - 0.125).abs() < 1.0e-5, "paths leave a quarter of a texel's width above");
	}

	#[test]
	fn a_texel_a_triangle_only_crosses_takes_the_triangles_point_nearest_its_center() {
		// A thin triangle that crosses texel (0, 0) of a 4-texel rectangle below its center.
		let positions = [[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0, 0.0, 0.1]];
		let normals = [[0.0, -1.0, 0.0]; 3];
		let uvs = [[0.0, 0.0], [1.0, 0.0], [0.0, 0.1]];
		let triangles = [[0, 1, 2]];
		let surface = TexelSurface {
			positions: &positions,
			normals: &normals,
			uvs: &uvs,
			triangles: &triangles,
		};
		let mut pixels = vec![NO_LIGHTMAP_TEXEL; 16];
		let mut texels = Vec::new();
		TexelRaster::default().rasterize(&surface, Rectangle { x: 0, y: 0, side: 4 }, 4, &mut texels, &mut pixels);

		let texel = texels[pixels[0] as usize];
		// The center (0.5, 0.5) lies above the hypotenuse; its nearest point on the triangle is on that edge.
		assert!(texel.position[2] > 0.0 && texel.position[2] < 0.1);
		assert_eq!(texel.normal, [0.0, -1.0, 0.0]);
		assert_eq!(pixels[4], NO_LIGHTMAP_TEXEL, "a texel the triangle doesn't reach holds nothing");
	}
}
