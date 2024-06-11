use futures::future::try_join_all;
use polodb_core::bson;
use serde::Deserialize;

use crate::{material::{Variant, VariantModel}, types::{IndexStreamTypes, QuantizationSchemes, Stream, Streams, VertexComponent, VertexSemantics}, Model, Reference, ReferenceModel, Resource, SolveErrors, Solver, StorageBackend};

#[derive(Debug, serde::Serialize)]
pub struct Primitive {
	pub material: Reference<Variant>,
	pub streams: Vec<Stream>,
	pub quantization: Option<QuantizationSchemes>,
	pub bounding_box: [[f32; 3]; 2],
	pub vertex_count: u32,
}

#[derive(Debug, serde::Serialize, serde::Deserialize)]
pub struct PrimitiveModel {
	pub material: ReferenceModel<VariantModel>,
	pub streams: Vec<Stream>,
	pub quantization: Option<QuantizationSchemes>,
	pub bounding_box: [[f32; 3]; 2],
	pub vertex_count: u32,
}

impl Primitive {
	pub fn meshlet_stream(&self) -> Option<&Stream> {
		self.streams.iter().find(|s| s.stream_type == Streams::Meshlets)
	}
}

impl Resource for Primitive {
	fn get_class(&self) -> &'static str { "Primitive" }

	type Model = PrimitiveModel;
}

impl Model for PrimitiveModel {
	fn get_class() -> &'static str {
		"Primitive"
	}
}

impl <'de> Solver<'de, Primitive> for PrimitiveModel {
	async fn solve(self, storage_backend: &dyn StorageBackend) -> Result<Primitive, SolveErrors> {
		let PrimitiveModel { material, streams, quantization, bounding_box, vertex_count } = self;

		Ok(Primitive {
			material: material.solve(storage_backend).await?,
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

#[derive(Debug, serde::Deserialize)]
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
	pub fn position_stream(&self) -> Option<Stream> {
		self.streams.iter().find(|s| s.stream_type == Streams::Vertices(VertexSemantics::Position)).cloned()
	}

	pub fn normal_stream(&self) -> Option<Stream> {
		self.streams.iter().find(|s| s.stream_type == Streams::Vertices(VertexSemantics::Normal)).cloned()
	}

	pub fn tangent_stream(&self) -> Option<Stream> {
		self.streams.iter().find(|s| s.stream_type == Streams::Vertices(VertexSemantics::Tangent)).cloned()
	}

	pub fn bi_tangent_stream(&self) -> Option<Stream> {
		self.streams.iter().find(|s| s.stream_type == Streams::Vertices(VertexSemantics::BiTangent)).cloned()
	}

	pub fn uv_stream(&self) -> Option<Stream> {
		self.streams.iter().find(|s| s.stream_type == Streams::Vertices(VertexSemantics::UV)).cloned()
	}

	pub fn color_stream(&self) -> Option<&Stream> {
		self.streams.iter().find(|s| s.stream_type == Streams::Vertices(VertexSemantics::Color))
	}

	pub fn triangle_indices_stream(&self) -> Option<Stream> {
		self.streams.iter().find(|s| s.stream_type == Streams::Indices(IndexStreamTypes::Triangles)).cloned()
	}

	pub fn vertex_indices_stream(&self) -> Option<Stream> {
		self.streams.iter().find(|s| s.stream_type == Streams::Indices(IndexStreamTypes::Vertices)).cloned()
	}

	pub fn meshlet_indices_stream(&self) -> Option<Stream> {
		self.streams.iter().find(|s| s.stream_type == Streams::Indices(IndexStreamTypes::Meshlets)).cloned()
	}

	pub fn meshlets_stream(&self) -> Option<Stream> {
		self.streams.iter().find(|s| s.stream_type == Streams::Meshlets).cloned()
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
	fn get_class(&self) -> &'static str { "Mesh" }

	type Model = MeshModel;
}

#[derive(Debug, serde::Serialize, serde::Deserialize)]
pub struct MeshModel {
	pub vertex_components: Vec<VertexComponent>,
	pub streams: Vec<Stream>,
	pub primitives: Vec<PrimitiveModel>,
}

impl super::Model for MeshModel {
	fn get_class() -> &'static str {
		"Mesh"
	}
}

impl <'de> Solver<'de, Reference<Mesh>> for ReferenceModel<MeshModel> {
	async fn solve(self, storage_backend: &dyn StorageBackend) -> Result<Reference<Mesh>, SolveErrors> {
		let (gr, reader) = storage_backend.read(&self.id).await.ok_or_else(|| SolveErrors::StorageError)?;
		let MeshModel { vertex_components, streams, primitives } = MeshModel::deserialize(bson::Deserializer::new(gr.resource.into())).map_err(|e| SolveErrors::DeserializationFailed(e.to_string()))?;

		Ok(Reference::from_model(self, Mesh {
			vertex_components,
			streams,
			primitives: try_join_all(primitives.into_iter().map(|p| p.solve(storage_backend))).await?,
		}, reader))
	}
}