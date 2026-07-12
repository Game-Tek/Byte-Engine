use crate::{
	resource,
	resources::material::{Variant, VariantModel},
	solver::SolveErrors,
	types::{IndexStreamTypes, QuantizationSchemes, Stream, Streams, VertexComponent, VertexSemantics},
	Model, Reference, ReferenceModel, Resource, Solver,
};

#[derive(Debug, serde::Serialize)]
pub struct Primitive {
	pub material: Reference<Variant>,
	pub streams: Vec<Stream>,
	pub quantization: Option<QuantizationSchemes>,
	pub bounding_box: [[f32; 3]; 2],
	pub vertex_count: u32,
}

#[derive(Debug, serde::Serialize, serde::Deserialize, rkyv::Archive, rkyv::Serialize, rkyv::Deserialize)]
pub struct PrimitiveModel {
	pub material: ReferenceModel<VariantModel>,
	pub streams: Vec<Stream>,
	pub quantization: Option<QuantizationSchemes>,
	pub bounding_box: [[f32; 3]; 2],
	pub vertex_count: u32,
}

impl Primitive {
	pub fn stream(&self, stream_type: Streams) -> Option<&Stream> {
		self.streams.iter().find(|stream| stream.stream_type == stream_type)
	}

	pub fn meshlet_stream(&self) -> Option<&Stream> {
		self.stream(Streams::Meshlets)
	}
}

impl Resource for Primitive {
	fn get_class(&self) -> &'static str {
		"Primitive"
	}

	type Model = PrimitiveModel;
}

impl Model for PrimitiveModel {
	fn get_class() -> &'static str {
		"Primitive"
	}
}

impl<'de> Solver<'de, Primitive> for PrimitiveModel {
	fn solve(self, storage_backend: &dyn resource::ReadStorageBackend) -> Result<Primitive, SolveErrors> {
		let PrimitiveModel {
			material,
			streams,
			quantization,
			bounding_box,
			vertex_count,
		} = self;

		Ok(Primitive {
			material: material.solve(storage_backend)?,
			streams,
			quantization,
			bounding_box,
			vertex_count,
		})
	}
}

#[derive(Debug, serde::Serialize)]
pub struct SubMesh {
	pub primitives: Vec<Primitive>,
}

#[derive(Debug, serde::Deserialize, rkyv::Archive, rkyv::Serialize, rkyv::Deserialize)]
pub struct SubMeshModel {
	pub primitives: Vec<PrimitiveModel>,
}

#[derive(Debug, serde::Serialize)]
/// Mesh represent a piece of geometry.
/// It is composed of multiple sub meshes that can be rendered with different materials.
/// Indices:
/// 	- `Vertices`: Each entry is a "pointer" to a vertex in the vertex buffer.
/// 	- `Meshlets`: Each entry is a "pointer" to an index in the `Vertices` index stream.
/// 	- `Triangles`: Each entry is a "pointer" to a vertex in the vertex buffer.
pub struct Mesh {
	pub vertex_components: Vec<VertexComponent>,
	pub streams: Vec<Stream>,
	pub primitives: Vec<Primitive>,
}

impl Mesh {
	pub fn primitives(&self) -> impl Iterator<Item = &Primitive> {
		self.primitives.iter()
	}

	pub fn stream(&self, stream_type: Streams) -> Option<&Stream> {
		self.streams.iter().find(|stream| stream.stream_type == stream_type)
	}

	pub fn vertex_stream(&self, semantic: VertexSemantics) -> Option<&Stream> {
		self.stream(Streams::Vertices(semantic))
	}

	pub fn index_stream(&self, stream_type: IndexStreamTypes) -> Option<&Stream> {
		self.stream(Streams::Indices(stream_type))
	}

	pub fn position_stream(&self) -> Option<Stream> {
		self.vertex_stream(VertexSemantics::Position).cloned()
	}

	pub fn normal_stream(&self) -> Option<Stream> {
		self.vertex_stream(VertexSemantics::Normal).cloned()
	}

	pub fn tangent_stream(&self) -> Option<Stream> {
		self.vertex_stream(VertexSemantics::Tangent).cloned()
	}

	pub fn bi_tangent_stream(&self) -> Option<Stream> {
		self.vertex_stream(VertexSemantics::BiTangent).cloned()
	}

	pub fn uv_stream(&self) -> Option<Stream> {
		self.vertex_stream(VertexSemantics::UV).cloned()
	}

	pub fn color_stream(&self) -> Option<&Stream> {
		self.vertex_stream(VertexSemantics::Color)
	}

	pub fn triangle_indices_stream(&self) -> Option<Stream> {
		self.index_stream(IndexStreamTypes::Triangles).cloned()
	}

	pub fn vertex_indices_stream(&self) -> Option<Stream> {
		self.index_stream(IndexStreamTypes::Vertices).cloned()
	}

	pub fn meshlet_indices_stream(&self) -> Option<Stream> {
		self.index_stream(IndexStreamTypes::Meshlets).cloned()
	}

	pub fn meshlets_stream(&self) -> Option<Stream> {
		self.stream(Streams::Meshlets).cloned()
	}

	pub fn vertex_count(&self) -> usize {
		self.primitives.iter().map(|p| p.vertex_count as usize).sum()
	}

	pub fn triangle_count(&self) -> usize {
		self.meshlet_indices_stream().map(|s| s.count()).unwrap_or(0) / 3
	}

	pub fn primitive_count(&self) -> usize {
		self.vertex_indices_stream().map(|s| s.count()).unwrap_or(0)
	}
}

impl Resource for Mesh {
	fn get_class(&self) -> &'static str {
		"Mesh"
	}

	type Model = MeshModel;
}

#[derive(Debug, serde::Serialize, serde::Deserialize, rkyv::Archive, rkyv::Serialize, rkyv::Deserialize)]
pub struct MeshModel {
	pub vertex_components: Vec<VertexComponent>,
	pub streams: Vec<Stream>,
	pub primitives: Vec<PrimitiveModel>,
}

impl Model for MeshModel {
	fn get_class() -> &'static str {
		"Mesh"
	}
}

impl<'de> Solver<'de, Reference<Mesh>> for ReferenceModel<MeshModel> {
	fn solve(self, storage_backend: &dyn resource::ReadStorageBackend) -> Result<Reference<Mesh>, SolveErrors> {
		let (gr, reader) = storage_backend.read(self.id()).ok_or(SolveErrors::StorageError)?;
		let MeshModel {
			vertex_components,
			streams,
			primitives,
		} = crate::from_slice(&gr.resource).map_err(|e| SolveErrors::DeserializationFailed(e.to_string()))?;

		Ok(Reference::from_model(
			self,
			Mesh {
				vertex_components,
				streams,
				primitives: primitives
					.into_iter()
					.map(|p| p.solve(storage_backend))
					.collect::<Result<Vec<_>, _>>()?,
			},
			reader,
		))
	}
}

#[cfg(test)]
mod tests {
	use super::Mesh;
	use crate::types::{IndexStreamTypes, Stream, Streams, VertexSemantics};

	fn stream(stream_type: Streams, offset: usize, size: usize, stride: usize) -> Stream {
		Stream {
			stream_type,
			offset,
			size,
			stride,
		}
	}

	#[test]
	fn semantic_accessors_select_only_the_requested_stream() {
		let mesh = Mesh {
			vertex_components: Vec::new(),
			streams: vec![
				stream(Streams::Vertices(VertexSemantics::Position), 0, 36, 12),
				stream(Streams::Vertices(VertexSemantics::Normal), 36, 36, 12),
				stream(Streams::Vertices(VertexSemantics::Tangent), 72, 48, 16),
				stream(Streams::Vertices(VertexSemantics::BiTangent), 120, 36, 12),
				stream(Streams::Vertices(VertexSemantics::UV), 156, 24, 8),
				stream(Streams::Vertices(VertexSemantics::Color), 180, 48, 16),
			],
			primitives: Vec::new(),
		};

		assert_eq!(mesh.position_stream().map(|value| value.offset), Some(0));
		assert_eq!(mesh.normal_stream().map(|value| value.offset), Some(36));
		assert_eq!(mesh.tangent_stream().map(|value| value.offset), Some(72));
		assert_eq!(mesh.bi_tangent_stream().map(|value| value.offset), Some(120));
		assert_eq!(mesh.uv_stream().map(|value| value.offset), Some(156));
		assert_eq!(mesh.color_stream().map(|value| value.offset), Some(180));
		assert!(mesh.vertex_stream(VertexSemantics::Weights).is_none());
	}

	#[test]
	fn topology_counts_are_derived_from_their_designated_streams() {
		let mesh = Mesh {
			vertex_components: Vec::new(),
			streams: vec![
				stream(Streams::Indices(IndexStreamTypes::Vertices), 0, 24, 4),
				stream(Streams::Indices(IndexStreamTypes::Meshlets), 24, 36, 1),
				stream(Streams::Indices(IndexStreamTypes::Triangles), 60, 18, 1),
				stream(Streams::Meshlets, 78, 64, 32),
			],
			primitives: Vec::new(),
		};

		assert_eq!(mesh.primitive_count(), 6);
		assert_eq!(mesh.triangle_count(), 12);
		assert_eq!(mesh.vertex_indices_stream().map(|value| value.offset), Some(0));
		assert_eq!(mesh.meshlet_indices_stream().map(|value| value.offset), Some(24));
		assert_eq!(mesh.triangle_indices_stream().map(|value| value.offset), Some(60));
		assert_eq!(mesh.meshlets_stream().map(|value| value.offset), Some(78));
		assert_eq!(mesh.vertex_count(), 0);
		assert_eq!(mesh.primitives().count(), 0);
	}

	#[test]
	fn absent_topology_streams_produce_zero_counts() {
		let mesh = Mesh {
			vertex_components: Vec::new(),
			streams: Vec::new(),
			primitives: Vec::new(),
		};

		assert_eq!(mesh.triangle_count(), 0);
		assert_eq!(mesh.primitive_count(), 0);
	}
}
