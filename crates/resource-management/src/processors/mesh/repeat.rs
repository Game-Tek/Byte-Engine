//! Repeating one baked primitive at many placements, as the source of one primitive that holds every copy.
//!
//! A level bake stores each cluster of placements as a mesh of its own: it reads the placed mesh's full detail back
//! with [`BakedPrimitive::read`], and pushes a [`RepeatedPrimitive`] for each primitive into a
//! [`MeshProcessorSession`](super::MeshProcessorSession), which bakes the copies as one primitive with detail levels
//! simplified across the whole cluster.

use std::convert::Infallible;

use math::{Orientation, Point, Vector};

use super::{MeshPrimitiveSource, VertexSkin};
use crate::{
	resources::{
		ModelSpace,
		mesh::{Mesh, Primitive},
	},
	types::{IndexStreamTypes, Streams, VertexSemantics},
};

/// The `Placement` struct places one copy of a mesh, as a level's `transform` payload does: scaled axis by axis, then
/// turned, then moved.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Placement {
	pub position: [f32; 3],
	pub orientation: Orientation,
	pub scale: [f32; 3],
}

impl Placement {
	/// Returns where the placement puts the model-space point `point`.
	fn point(&self, [x, y, z]: [f32; 3]) -> [f32; 3] {
		let [sx, sy, sz] = self.scale;
		let turned = self
			.orientation
			.rotate_vector(Vector::<ModelSpace>::new(x * sx, y * sy, z * sz))
			.to_array();
		std::array::from_fn(|axis| self.position[axis] + turned[axis])
	}

	/// Returns which way the model-space direction `direction` points once placed: scaled by `scale`, turned, and
	/// made unit length again. A direction that scales to nothing stays as it is.
	fn direction(&self, [x, y, z]: [f32; 3], [sx, sy, sz]: [f32; 3]) -> [f32; 3] {
		let turned = self
			.orientation
			.rotate_vector(Vector::<ModelSpace>::new(x * sx, y * sy, z * sz));
		turned.normalized().map_or([x, y, z], |unit| unit.into_vector().to_array())
	}

	/// Returns whether the placement mirrors the mesh, which an odd number of negative scale axes does.
	fn mirrors(&self) -> bool {
		self.scale.iter().filter(|axis| **axis < 0.0).count() % 2 == 1
	}

	/// Returns the largest factor the placement scales any length by.
	fn largest_scale(&self) -> f32 {
		self.scale.iter().map(|axis| axis.abs()).fold(0.0, f32::max)
	}
}

/// The `BakedPrimitive` struct holds the full detail of one primitive of a baked mesh, read back so it can be baked
/// again, such as repeated at a cluster's placements.
///
/// Read it with [`Self::read`], then repeat it with [`RepeatedPrimitive`].
pub struct BakedPrimitive {
	/// Index of the primitive's material in [`Mesh::materials`].
	material: u32,
	/// Radius of the primitive's bounding sphere, which a copy's caster radius scales.
	radius: f32,
	positions: Vec<[f32; 3]>,
	normals: Option<Vec<[f32; 3]>>,
	tangents: Option<Vec<[f32; 4]>>,
	bitangents: Option<Vec<[f32; 3]>>,
	uvs: Option<Vec<[f32; 2]>>,
	colors: Option<Vec<[f32; 4]>>,
	/// Full detail's triangles as the mesh stores them, already wound as the engine's front faces are.
	triangles: Vec<[u16; 3]>,
}

impl BakedPrimitive {
	/// Reads `primitive` of `mesh` from the mesh's whole loaded `payload`.
	///
	/// Every vertex stream the mesh stores is read, except generated lightmap UVs, which a bake generates again for
	/// the geometry it makes. Returns the most likely cause and its fix for a primitive that can't be read: one that is
	/// skinned or animated by a node, or one whose payload doesn't hold what the mesh describes.
	pub fn read(mesh: &Mesh, primitive: &Primitive, payload: &[u8]) -> Result<Self, &'static str> {
		if primitive.skin.is_some() || primitive.transform_node.is_some() {
			return Err(
				"The most likely cause is that the mesh is skinned or animated, and repeated geometry never moves. Use a static mesh.",
			);
		}
		const UNREADABLE: &str = "The most likely cause is a mesh baked by another engine version. Bake the mesh again.";
		// A primitive's range of a stream is relative to the mesh's stream of that type and level.
		let bytes = |stream_type: Streams, level: u8| -> Option<&[u8]> {
			let block = mesh.stream(stream_type, level)?;
			let range = primitive.stream(stream_type, level)?;
			let start = block.offset.checked_add(range.offset)?;
			payload.get(start..start.checked_add(range.size)?)
		};
		// Each level's vertex streams hold the vertices it adds to the coarser levels, so they are read coarsest first.
		let vertices = |semantic: VertexSemantics| -> Result<Option<Vec<f32>>, &'static str> {
			if !mesh.vertex_components.iter().any(|component| component.semantic == semantic) {
				return Ok(None);
			}
			let mut values = Vec::new();
			for level in (0..primitive.lods.len() as u8).rev() {
				let Some(level_bytes) = bytes(Streams::Vertices(semantic), level) else {
					// A level that adds no vertices has no streams.
					continue;
				};
				values.extend(level_bytes.as_chunks::<4>().0.iter().map(|value| f32::from_le_bytes(*value)));
			}
			Ok(Some(values))
		};
		let count = primitive.vertex_count as usize;
		// Every stream is stored as f32 components, so a stream of the right length has `N` components a vertex.
		fn group<const N: usize>(values: Option<Vec<f32>>, count: usize) -> Result<Option<Vec<[f32; N]>>, &'static str> {
			values
				.map(|values| match values.as_chunks::<N>() {
					(chunks, []) if chunks.len() == count => Ok(chunks.to_vec()),
					_ => Err("The most likely cause is a mesh baked by another engine version. Bake the mesh again."),
				})
				.transpose()
		}
		let positions = group::<3>(vertices(VertexSemantics::Position)?, count)?.ok_or(UNREADABLE)?;
		let triangles = bytes(Streams::Indices(IndexStreamTypes::Triangles), 0).ok_or(UNREADABLE)?;
		let triangles: Vec<[u16; 3]> = triangles
			.as_chunks::<6>()
			.0
			.iter()
			.map(|triangle| std::array::from_fn(|corner| u16::from_le_bytes([triangle[corner * 2], triangle[corner * 2 + 1]])))
			.collect();
		if triangles.iter().flatten().any(|&index| index as usize >= count) {
			return Err(UNREADABLE);
		}
		Ok(Self {
			material: primitive.material,
			radius: primitive.bounding_sphere[3],
			positions,
			normals: group::<3>(vertices(VertexSemantics::Normal)?, count)?,
			tangents: group::<4>(vertices(VertexSemantics::Tangent)?, count)?,
			bitangents: group::<3>(vertices(VertexSemantics::BiTangent)?, count)?,
			uvs: group::<2>(vertices(VertexSemantics::UV)?, count)?,
			colors: group::<4>(vertices(VertexSemantics::Color)?, count)?,
			triangles,
		})
	}

	/// Returns how many vertices the primitive has.
	pub fn vertex_count(&self) -> usize {
		self.positions.len()
	}
}

/// The `RepeatedPrimitive` struct is a [`MeshPrimitiveSource`] that holds one [`BakedPrimitive`] once at each of a
/// cluster's placements.
///
/// Push it into a [`MeshProcessorSession`](super::MeshProcessorSession) with the materials of the mesh the baked
/// primitive came from. The copies follow one another in the order of the placements, each with every attribute of
/// the baked primitive placed: positions moved, normals, tangents, and bitangents turned, and the rest copied. Its
/// iterators compute each value as they go, so repeating a primitive allocates nothing.
pub struct RepeatedPrimitive<'a> {
	source: &'a BakedPrimitive,
	placements: &'a [Placement],
}

impl<'a> RepeatedPrimitive<'a> {
	/// Repeats `source` once at each of `placements`.
	pub fn new(source: &'a BakedPrimitive, placements: &'a [Placement]) -> Self {
		Self { source, placements }
	}

	/// Returns one value for each vertex of every copy, from the baked primitive's `values` and how one copy places one
	/// of them.
	fn repeated<T: Copy, U>(
		&self,
		values: &'a [T],
		place: impl Fn(&Placement, T) -> U + 'a,
	) -> impl ExactSizeIterator<Item = Result<U, Infallible>> + 'a {
		let (placements, count) = (self.placements, values.len());
		(0..placements.len() * count).map(move |index| Ok(place(&placements[index / count], values[index % count])))
	}
}

impl MeshPrimitiveSource for RepeatedPrimitive<'_> {
	type Error = Infallible;

	fn material_slot(&self) -> usize {
		self.source.material as usize
	}

	fn caster_radius(&self) -> Option<f32> {
		// Every copy's sphere is the baked sphere scaled, so the largest copy is the one scaled the most.
		let largest = self.placements.iter().map(Placement::largest_scale).fold(0.0, f32::max);
		Some(self.source.radius * largest)
	}

	fn indices(&self) -> Result<impl ExactSizeIterator<Item = Result<u32, Self::Error>> + '_, Self::Error> {
		let (triangles, count) = (&self.source.triangles, self.source.vertex_count());
		Ok((0..self.placements.len() * triangles.len() * 3).map(move |index| {
			let (copy, corner) = (index / (triangles.len() * 3), index % (triangles.len() * 3));
			let (triangle, mut corner) = (corner / 3, corner % 3);
			// Mesh processing swaps the last two corners of every triangle it receives, which the stored triangles
			// already had done. Swapping them back here returns them to how they were authored, and the processor's
			// swap restores the engine's winding. A mirrored copy is wound the other way by its placement, so it keeps
			// the stored order, which the processor's swap then turns back to face out.
			if !self.placements[copy].mirrors() && corner != 0 {
				corner = 3 - corner;
			}
			Ok((copy * count) as u32 + u32::from(triangles[triangle][corner]))
		}))
	}

	fn positions(&self) -> Result<impl ExactSizeIterator<Item = Result<Point<ModelSpace>, Self::Error>> + '_, Self::Error> {
		Ok(self.repeated(&self.source.positions, |placement, position| {
			let [x, y, z] = placement.point(position);
			Point::new(x, y, z)
		}))
	}

	fn normals(
		&self,
	) -> Result<Option<impl ExactSizeIterator<Item = Result<Vector<ModelSpace>, Self::Error>> + '_>, Self::Error> {
		// A normal scales by the inverse of the placement's scale, so it stays perpendicular to its surface.
		Ok(self.source.normals.as_deref().map(|normals| {
			self.repeated(normals, |placement, normal| {
				let [x, y, z] = placement.direction(normal, placement.scale.map(f32::recip));
				Vector::new(x, y, z)
			})
		}))
	}

	fn tangents(&self) -> Result<Option<impl ExactSizeIterator<Item = Result<[f32; 4], Self::Error>> + '_>, Self::Error> {
		// A tangent lies along its surface, so it scales as the surface does. Mirroring flips the bitangent's side.
		Ok(self.source.tangents.as_deref().map(|tangents| {
			self.repeated(tangents, |placement, [x, y, z, w]| {
				let [x, y, z] = placement.direction([x, y, z], placement.scale);
				[x, y, z, if placement.mirrors() { -w } else { w }]
			})
		}))
	}

	fn bitangents(
		&self,
	) -> Result<Option<impl ExactSizeIterator<Item = Result<Vector<ModelSpace>, Self::Error>> + '_>, Self::Error> {
		Ok(self.source.bitangents.as_deref().map(|bitangents| {
			self.repeated(bitangents, |placement, bitangent| {
				let [x, y, z] = placement.direction(bitangent, placement.scale);
				Vector::new(x, y, z)
			})
		}))
	}

	fn uvs(&self) -> Result<Option<impl ExactSizeIterator<Item = Result<[f32; 2], Self::Error>> + '_>, Self::Error> {
		Ok(self.source.uvs.as_deref().map(|uvs| self.repeated(uvs, |_, uv| uv)))
	}

	fn colors(&self) -> Result<Option<impl ExactSizeIterator<Item = Result<[f32; 4], Self::Error>> + '_>, Self::Error> {
		Ok(self
			.source
			.colors
			.as_deref()
			.map(|colors| self.repeated(colors, |_, color| color)))
	}

	fn vertex_skin(&self) -> Result<Option<impl ExactSizeIterator<Item = Result<VertexSkin, Self::Error>> + '_>, Self::Error> {
		// Repeated geometry never moves, so it has no skin.
		Ok(None::<std::iter::Empty<Result<VertexSkin, Self::Error>>>)
	}
}

#[cfg(test)]
mod tests {
	use std::convert::Infallible;

	use math::{Orientation, Point, Vector};

	use super::{BakedPrimitive, Placement, RepeatedPrimitive};
	use crate::{
		ReferenceModel,
		processors::mesh::{MeshPrimitiveSource, MeshProcessorSession, MeshSettings},
		resources::{ModelSpace, material::VariantModel, mesh::Mesh},
		types::{AlphaMode, VertexComponent, VertexSemantics},
	};

	/// The `Quad` struct is a source of two triangles that face +Z, as authored, with normals and tangents.
	struct Quad;

	const QUAD: [[f32; 3]; 4] = [[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [1.0, 1.0, 0.0], [0.0, 1.0, 0.0]];

	impl MeshPrimitiveSource for Quad {
		type Error = Infallible;

		fn material_slot(&self) -> usize {
			0
		}

		fn indices(&self) -> Result<impl ExactSizeIterator<Item = Result<u32, Self::Error>> + '_, Self::Error> {
			Ok([0, 1, 2, 0, 2, 3].into_iter().map(Ok))
		}

		fn positions(&self) -> Result<impl ExactSizeIterator<Item = Result<Point<ModelSpace>, Self::Error>> + '_, Self::Error> {
			Ok(QUAD.into_iter().map(|[x, y, z]| Ok(Point::new(x, y, z))))
		}

		fn normals(
			&self,
		) -> Result<Option<impl ExactSizeIterator<Item = Result<Vector<ModelSpace>, Self::Error>> + '_>, Self::Error> {
			Ok(Some((0..4).map(|_| Ok(Vector::new(0.0, 0.0, 1.0)))))
		}

		fn tangents(&self) -> Result<Option<impl ExactSizeIterator<Item = Result<[f32; 4], Self::Error>> + '_>, Self::Error> {
			Ok(Some((0..4).map(|_| Ok([1.0, 0.0, 0.0, 1.0]))))
		}
	}

	/// Bakes the primitives `push` pushes into a mesh with positions, normals, and tangents, and returns it with its
	/// payload.
	fn bake(push: impl FnOnce(&mut MeshProcessorSession)) -> (Mesh, Box<[u8]>) {
		let layout = [VertexSemantics::Position, VertexSemantics::Normal, VertexSemantics::Tangent]
			.map(VertexComponent::canonical)
			.to_vec();
		let mut session = MeshProcessorSession::new(layout, None, Vec::new(), MeshSettings::default()).unwrap();
		push(&mut session);
		let variant = VariantModel {
			material: ReferenceModel::new_serialized("quad.material", 0, 0, Vec::new(), None),
			variables: Vec::new(),
			alpha_mode: AlphaMode::Opaque,
			double_sided: false,
		};
		let material = ReferenceModel::new_serialized("quad.variant", 0, 0, crate::to_vec(&variant).unwrap(), None);
		let processed = session.finish(&[material]);
		let model = processed.mesh;
		let mesh = Mesh {
			skeleton: None,
			skins: model.skins,
			vertex_components: model.vertex_components,
			streams: model.streams,
			materials: model.materials,
			primitives: model.primitives,
		};
		(mesh, processed.buffer)
	}

	/// Returns, for each triangle of `primitive`, the dot product of its face normal, by its stored winding, with its
	/// first corner's normal.
	fn facing(primitive: &BakedPrimitive) -> Vec<f32> {
		let normals = primitive.normals.as_ref().unwrap();
		primitive
			.triangles
			.iter()
			.map(|triangle| {
				let [a, b, c] = triangle.map(|index| primitive.positions[index as usize]);
				let (u, v) = (
					std::array::from_fn::<f32, 3, _>(|i| b[i] - a[i]),
					std::array::from_fn::<f32, 3, _>(|i| c[i] - a[i]),
				);
				let face = [
					u[1] * v[2] - u[2] * v[1],
					u[2] * v[0] - u[0] * v[2],
					u[0] * v[1] - u[1] * v[0],
				];
				let normal = normals[triangle[0] as usize];
				face.iter().zip(normal).map(|(f, n)| f * n).sum()
			})
			.collect()
	}

	#[test]
	fn copies_are_placed_and_mirrored_copies_keep_facing_out() {
		let (quad, payload) = bake(|session| session.push_primitive(&Quad).unwrap());
		let source = BakedPrimitive::read(&quad, &quad.primitives[0], &payload).unwrap();
		let turned = Orientation::try_from_axis_angle(
			Vector::<ModelSpace>::new(0.0, 1.0, 0.0).normalized().unwrap(),
			math::Radians::new(std::f32::consts::PI),
		)
		.unwrap();
		let placements = [
			Placement {
				position: [10.0, 0.0, 0.0],
				orientation: Orientation::identity(),
				scale: [1.0; 3],
			},
			// Mirrored along x and stretched along y.
			Placement {
				position: [0.0, 0.0, 5.0],
				orientation: turned,
				scale: [-1.0, 2.0, 1.0],
			},
		];
		let repeated = RepeatedPrimitive::new(&source, &placements);
		assert_eq!(repeated.caster_radius(), Some(2.0 * source.radius));
		let (cluster, payload) = bake(|session| session.push_primitive(&repeated).unwrap());
		let cluster = BakedPrimitive::read(&cluster, &cluster.primitives[0], &payload).unwrap();

		assert_eq!(cluster.vertex_count(), 2 * source.vertex_count());
		assert_eq!(cluster.triangles.len(), 2 * source.triangles.len());
		// Every triangle faces the way its normals point, as the quad's do.
		let expected = facing(&source)[0].signum();
		assert!(
			facing(&cluster).iter().all(|facing| facing.signum() == expected),
			"{:?}",
			facing(&cluster)
		);
		// The first copy is moved, and the second mirrored, stretched, turned half a turn about y, and moved.
		let corners = |filter: fn(&[f32; 3]) -> bool| {
			let mut corners: Vec<[f32; 3]> = cluster.positions.iter().copied().filter(filter).collect();
			corners.sort_by(|a, b| a.partial_cmp(b).unwrap());
			corners
		};
		let near =
			|a: &[[f32; 3]], b: &[[f32; 3]]| a.iter().flatten().zip(b.iter().flatten()).all(|(a, b)| (a - b).abs() < 1e-5);
		assert!(near(
			&corners(|p| p[0] >= 9.0),
			&[[10.0, 0.0, 0.0], [10.0, 1.0, 0.0], [11.0, 0.0, 0.0], [11.0, 1.0, 0.0]]
		));
		assert!(near(
			&corners(|p| p[0] < 9.0),
			&[[0.0, 0.0, 5.0], [0.0, 2.0, 5.0], [1.0, 0.0, 5.0], [1.0, 2.0, 5.0]]
		));
		// The mirrored copy faces -Z after its half turn, with its bitangent on the other side.
		let normals = cluster.normals.as_ref().unwrap();
		let tangents = cluster.tangents.as_ref().unwrap();
		for (index, position) in cluster.positions.iter().enumerate() {
			let (normal, tangent) = (normals[index], tangents[index]);
			if position[0] >= 9.0 {
				assert!(near(&[normal], &[[0.0, 0.0, 1.0]]) && tangent == [1.0, 0.0, 0.0, 1.0]);
			} else {
				assert!(near(&[normal], &[[0.0, 0.0, -1.0]]), "{normal:?}");
				assert!(
					near(&[[tangent[0], tangent[1], tangent[2]]], &[[1.0, 0.0, 0.0]]) && tangent[3] == -1.0,
					"{tangent:?}"
				);
			}
		}
	}
}
