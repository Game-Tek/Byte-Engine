use futures::future::try_join_all;
use polodb_core::bson;
use serde::Deserialize;

use crate::{material::{Variant, VariantModel}, resource::resource_handler::ReadTargets, types::{IndexStreamTypes, QuantizationSchemes, Stream, Streams, VertexComponent, VertexSemantics}, LoadResults, Loader, Model, Reference, ReferenceModel, Resource, SolveErrors, Solver, StorageBackend};

#[derive(Debug, serde::Serialize)]
pub struct Primitive<'a> {
	pub material: Reference<'a, Variant<'a>>,
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

impl <'a> Primitive<'a> {
	pub fn meshlet_stream(&self) -> Option<&Stream> {
		self.streams.iter().find(|s| s.stream_type == Streams::Meshlets)
	}
}

impl <'a> Resource for Primitive<'a> {
	fn get_class(&self) -> &'static str { "Primitive" }

	type Model = PrimitiveModel;
}

impl Model for PrimitiveModel {
	fn get_class() -> &'static str {
		"Primitive"
	}
}

impl <'a, 'de> Solver<'de, Primitive<'a>> for PrimitiveModel {
	async fn solve(self, storage_backend: &dyn StorageBackend) -> Result<Primitive<'a>, SolveErrors> {
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
pub struct SubMesh<'a> {
	pub primitives: Vec<Primitive<'a>>,
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
pub struct Mesh<'a> {
	pub vertex_components: Vec<VertexComponent>,
	pub streams: Vec<Stream>,
	pub primitives: Vec<Primitive<'a>>,
}

impl <'a> Mesh<'a> {
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

impl <'a> Resource for Mesh<'a> {
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

impl <'a, 'de> Solver<'de, Reference<'a, Mesh<'a>>> for ReferenceModel<MeshModel> {
	async fn solve(self, storage_backend: &dyn StorageBackend) -> Result<Reference<'a, Mesh<'a>>, SolveErrors> {
		let (gr, reader) = storage_backend.read(&self.id).await.ok_or_else(|| SolveErrors::StorageError)?;
		let MeshModel { vertex_components, streams, primitives } = MeshModel::deserialize(bson::Deserializer::new(gr.resource.clone().into())).map_err(|e| SolveErrors::DeserializationFailed(e.to_string()))?;

		Ok(Reference::new(&self.id, self.hash, gr.size, Mesh {
			vertex_components,
			streams,
			primitives: try_join_all(primitives.into_iter().map(|p| p.solve(storage_backend))).await?,
		}, reader))
	}
}

impl <'a> Loader for Reference<'a, Mesh<'a>> {
	async fn load(mut self,) -> Result<Self, crate::LoadResults> {
		let position_stream = self.resource().position_stream();
		let normal_stream = self.resource().normal_stream();
		let tangent_stream = self.resource().tangent_stream();
		let uv_stream = self.resource().uv_stream();
		let triangle_indices_stream = self.resource().triangle_indices_stream();
		let vertex_indices_stream = self.resource().vertex_indices_stream();
		let meshlet_indices_stream = self.resource().meshlet_indices_stream();
		let meshlets_stream = self.resource().meshlets_stream();

		if let Some(read_target) = self.read_target.as_mut() {
			match read_target { // Use the cloned value in the match statement
				ReadTargets::Streams(streams) => {
					for stream in streams {
						let v_stream = match stream.name {
							"Vertex.Position" => { position_stream.clone() }
							"Vertex.Normal" => { normal_stream.clone() }
							"Vertex.Tangent" => { tangent_stream.clone() }
							"Vertex.UV" => { uv_stream.clone() }
							"TriangleIndices" => { triangle_indices_stream.clone() }
							"VertexIndices" => { vertex_indices_stream.clone() }
							"MeshletIndices" => { meshlet_indices_stream.clone() }
							"Meshlets" => { meshlets_stream.clone() }
							_ => {
								log::error!("Unknown buffer tag: {}", stream.name);
								None
							}
						};

						if let Some(v_stream) = v_stream {
							self.reader.read_into(v_stream.offset, stream.buffer()).await.ok_or(LoadResults::LoadFailed)?; // Keep an eye on this
						} else {
							log::error!("Failed to read stream: {}", stream.name);
						}
					}
				}
				_ => {
					return Err(LoadResults::NoReadTarget);
				}	
			}
		}

		Ok(self)
	}
}