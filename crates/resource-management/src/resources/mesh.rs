use crate::{
	resource,
	resources::material::{Variant, VariantModel},
	resources::skeleton::{Skeleton, SkeletonModel, SkinBinding, SkinJoint},
	solver::SolveErrors,
	types::{IndexStreamTypes, QuantizationSchemes, Stream, Streams, VertexComponent, VertexSemantics},
	Model, Reference, ReferenceModel, Resource, Solver,
};

/// The `Primitive` struct supplies one renderable geometry range and its skeletal bindings to runtime rendering.
#[derive(Debug, serde::Serialize)]
pub struct Primitive {
	pub material: Reference<Variant>,
	pub transform_node: Option<u32>,
	pub skin: Option<u32>,
	pub streams: Vec<Stream>,
	pub quantization: Option<QuantizationSchemes>,
	pub bounding_box: [[f32; 3]; 2],
	pub vertex_count: u32,
}

/// The `PrimitiveModel` struct preserves a serializable primitive for mesh processing and resource storage.
#[derive(Debug, serde::Serialize, serde::Deserialize, rkyv::Archive, rkyv::Serialize, rkyv::Deserialize)]
pub struct PrimitiveModel {
	pub material: ReferenceModel<VariantModel>,
	pub transform_node: Option<u32>,
	pub skin: Option<u32>,
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
			transform_node,
			skin,
			streams,
			quantization,
			bounding_box,
			vertex_count,
		} = self;

		Ok(Primitive {
			material: material.solve(storage_backend)?,
			transform_node,
			skin,
			streams,
			quantization,
			bounding_box,
			vertex_count,
		})
	}
}

/// The `SubMesh` struct groups runtime primitives that callers want to address as one mesh section.
#[derive(Debug, serde::Serialize)]
pub struct SubMesh {
	pub primitives: Vec<Primitive>,
}

/// The `SubMeshModel` struct preserves a serializable group of primitives for mesh section workflows.
#[derive(Debug, serde::Deserialize, rkyv::Archive, rkyv::Serialize, rkyv::Deserialize)]
pub struct SubMeshModel {
	pub primitives: Vec<PrimitiveModel>,
}

/// The `Mesh` struct supplies packed geometry, material primitives, and optional skeletal bindings to runtime rendering.
///
/// Indices:
/// 	- `Vertices`: Each entry is a "pointer" to a vertex in the vertex buffer.
/// 	- `Meshlets`: Each entry is a "pointer" to an index in the `Vertices` index stream.
/// 	- `Triangles`: Each entry is a "pointer" to a vertex in the vertex buffer.
#[derive(Debug, serde::Serialize)]
pub struct Mesh {
	pub skeleton: Option<Reference<Skeleton>>,
	pub skins: Vec<SkinBinding>,
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
	type Model = MeshModel;
}

/// The `MeshModel` struct preserves processed geometry and skeletal bindings for storage and later runtime solving.
#[derive(Debug, serde::Serialize, serde::Deserialize, rkyv::Archive, rkyv::Serialize, rkyv::Deserialize)]
pub struct MeshModel {
	pub skeleton: Option<ReferenceModel<SkeletonModel>>,
	pub skins: Vec<SkinBinding>,
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
	/// Resolves mesh dependencies only after confirming its skin tables are safe for CPU pose and GPU palette workflows.
	fn solve(self, storage_backend: &dyn resource::ReadStorageBackend) -> Result<Reference<Mesh>, SolveErrors> {
		let (gr, reader) = storage_backend.read(self.id()).ok_or(SolveErrors::StorageError)?;
		let MeshModel {
			skeleton,
			skins,
			vertex_components,
			streams,
			primitives,
		} = crate::from_slice(&gr.resource).map_err(|error| {
			SolveErrors::DeserializationFailed(format!(
				"Mesh resource could not be deserialized. The most likely cause is incompatible or corrupted mesh metadata: {error}."
			))
		})?;

		let skeleton = skeleton.map(|skeleton| skeleton.solve(storage_backend)).transpose()?;
		validate_skin_metadata(skeleton.as_ref(), &skins, &vertex_components, &primitives)?;

		Ok(Reference::from_model(
			self,
			Mesh {
				skeleton,
				skins,
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

/// Validates that mesh skin tables, palette nodes, and primitive streams form a processable contract.
fn validate_skin_metadata(
	skeleton: Option<&Reference<Skeleton>>,
	skins: &[SkinBinding],
	vertex_components: &[VertexComponent],
	primitives: &[PrimitiveModel],
) -> Result<(), SolveErrors> {
	if !skins.is_empty() && skeleton.is_none() {
		return invalid_mesh_skeletal_metadata("skin bindings exist without a skeleton");
	}

	let skeleton_nodes = skeleton.map(|skeleton| skeleton.resource().nodes.len()).unwrap_or(0);
	for (skin_index, skin) in skins.iter().enumerate() {
		if skin.len() > u16::MAX as usize + 1 {
			return invalid_mesh_skeletal_metadata(format!("skin {skin_index} exceeds the u16 palette limit"));
		}
		for (joint_index, entry) in skin.entries.iter().enumerate() {
			if let SkinJoint::Node(node) = entry.joint {
				if node as usize >= skeleton_nodes {
					return invalid_mesh_skeletal_metadata(format!(
						"skin {skin_index} joint {joint_index} targets node {node} outside a {skeleton_nodes}-node skeleton"
					));
				}
			}
			if !entry
				.adjusted_inverse_bind_matrix
				.iter()
				.flatten()
				.all(|value| value.is_finite())
			{
				return invalid_mesh_skeletal_metadata(format!(
					"skin {skin_index} joint {joint_index} contains a non-finite adjusted inverse bind"
				));
			}
		}
	}

	if primitives.iter().any(|primitive| primitive.skin.is_some()) {
		validate_skin_vertex_component(vertex_components, VertexSemantics::Joints, "vec4u16")?;
		validate_skin_vertex_component(vertex_components, VertexSemantics::Weights, "vec4f")?;
	}

	for (primitive_index, primitive) in primitives.iter().enumerate() {
		if let Some(node) = primitive.transform_node {
			if skeleton.is_none() {
				return invalid_mesh_skeletal_metadata(format!(
					"primitive {primitive_index} targets transform node {node} without a skeleton"
				));
			}
			if node as usize >= skeleton_nodes {
				return invalid_mesh_skeletal_metadata(format!(
					"primitive {primitive_index} targets transform node {node} outside a {skeleton_nodes}-node skeleton"
				));
			}
		}
		let joints_stream = primitive
			.streams
			.iter()
			.find(|stream| stream.stream_type == Streams::Vertices(VertexSemantics::Joints));
		let weights_stream = primitive
			.streams
			.iter()
			.find(|stream| stream.stream_type == Streams::Vertices(VertexSemantics::Weights));

		match primitive.skin {
			Some(skin) => {
				if skin as usize >= skins.len() {
					return invalid_mesh_skeletal_metadata(format!(
						"primitive {primitive_index} targets skin {skin} outside the {}-skin table",
						skins.len()
					));
				}
				let (Some(joints_stream), Some(weights_stream)) = (joints_stream, weights_stream) else {
					return invalid_mesh_skeletal_metadata(format!(
						"primitive {primitive_index} is skinned but does not contain paired joint and weight streams"
					));
				};
				if joints_stream.stride != 8
					|| weights_stream.stride != 16
					|| joints_stream.size % joints_stream.stride != 0
					|| weights_stream.size % weights_stream.stride != 0
					|| joints_stream.count() != primitive.vertex_count as usize
					|| weights_stream.count() != primitive.vertex_count as usize
				{
					return invalid_mesh_skeletal_metadata(format!(
						"primitive {primitive_index} skin streams do not contain one vec4u16 joint and vec4f weight value per vertex"
					));
				}
			}
			None if joints_stream.is_some() || weights_stream.is_some() => {
				return invalid_mesh_skeletal_metadata(format!(
					"primitive {primitive_index} contains skin streams without a skin binding"
				));
			}
			None => {}
		}
	}

	Ok(())
}

/// Validates the shader-facing vertex declaration needed to interpret packed skin streams.
fn validate_skin_vertex_component(
	vertex_components: &[VertexComponent],
	semantic: VertexSemantics,
	expected_format: &'static str,
) -> Result<(), SolveErrors> {
	let Some(component) = vertex_components
		.iter()
		.find(|component| component.semantic == semantic && component.channel == 0)
	else {
		return invalid_mesh_skeletal_metadata(format!("the vertex layout does not declare {semantic:?} on channel 0"));
	};
	if component.format != expected_format {
		return invalid_mesh_skeletal_metadata(format!(
			"the vertex layout declares {semantic:?} as '{}' instead of {expected_format}",
			component.format
		));
	}
	Ok(())
}

fn invalid_mesh_skeletal_metadata(reason: impl std::fmt::Display) -> Result<(), SolveErrors> {
	Err(SolveErrors::DeserializationFailed(format!(
		"Mesh skeletal metadata is invalid. The most likely cause is malformed imported hierarchy or skin data: {reason}."
	)))
}

#[cfg(test)]
mod tests {
	use super::{validate_skin_metadata, Mesh, PrimitiveModel};
	use crate::{
		asset::ResourceId,
		resource::{storage_backend::tests::TestStorageBackend, WriteStorageBackend},
		resources::{
			material::VariantModel,
			skeleton::{
				identity_matrix4_columns, LocalTransform, Skeleton, SkeletonModel, SkeletonNode, SkinBinding, SkinJoint,
				SkinPaletteEntry,
			},
		},
		types::{AlphaMode, IndexStreamTypes, Stream, Streams, VertexComponent, VertexSemantics},
		ProcessedAsset, Reference, ReferenceModel, Solver,
	};

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
			skeleton: None,
			skins: Vec::new(),
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
			skeleton: None,
			skins: Vec::new(),
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
			skeleton: None,
			skins: Vec::new(),
			vertex_components: Vec::new(),
			streams: Vec::new(),
			primitives: Vec::new(),
		};

		assert_eq!(mesh.triangle_count(), 0);
		assert_eq!(mesh.primitive_count(), 0);
	}

	#[test]
	fn skin_metadata_accepts_a_complete_palette_and_paired_vertex_streams() {
		let storage = TestStorageBackend::new();
		let skeleton = test_skeleton(&storage);
		let skins = vec![SkinBinding {
			entries: vec![SkinPaletteEntry {
				joint: SkinJoint::Node(0),
				adjusted_inverse_bind_matrix: identity_matrix4_columns(),
			}],
		}];
		let primitives = vec![test_primitive(Some(0), true, true)];

		assert!(validate_skin_metadata(Some(&skeleton), &skins, &skin_vertex_layout(), &primitives).is_ok());
	}

	#[test]
	fn skin_metadata_rejects_missing_skeletons_invalid_indices_and_unpaired_streams() {
		let skin = SkinBinding {
			entries: vec![SkinPaletteEntry {
				joint: SkinJoint::Identity,
				adjusted_inverse_bind_matrix: identity_matrix4_columns(),
			}],
		};
		assert!(validate_skin_metadata(None, std::slice::from_ref(&skin), &skin_vertex_layout(), &[]).is_err());

		let storage = TestStorageBackend::new();
		let skeleton = test_skeleton(&storage);
		assert!(validate_skin_metadata(
			Some(&skeleton),
			std::slice::from_ref(&skin),
			&skin_vertex_layout(),
			&[test_primitive(Some(1), true, true)]
		)
		.is_err());
		assert!(validate_skin_metadata(
			Some(&skeleton),
			&[skin],
			&skin_vertex_layout(),
			&[test_primitive(Some(0), true, false)]
		)
		.is_err());
	}

	#[test]
	fn primitive_transform_nodes_require_a_matching_skeleton_node() {
		let mut primitive = test_primitive(None, false, false);
		primitive.transform_node = Some(0);
		assert!(validate_skin_metadata(None, &[], &[], std::slice::from_ref(&primitive)).is_err());

		let storage = TestStorageBackend::new();
		let skeleton = test_skeleton(&storage);
		assert!(validate_skin_metadata(Some(&skeleton), &[], &[], std::slice::from_ref(&primitive)).is_ok());

		primitive.transform_node = Some(1);
		assert!(validate_skin_metadata(Some(&skeleton), &[], &[], std::slice::from_ref(&primitive)).is_err());
	}

	fn skin_vertex_layout() -> Vec<VertexComponent> {
		vec![
			VertexComponent {
				semantic: VertexSemantics::Joints,
				format: "vec4u16".into(),
				channel: 0,
			},
			VertexComponent {
				semantic: VertexSemantics::Weights,
				format: "vec4f".into(),
				channel: 0,
			},
		]
	}

	fn test_skeleton(storage: &TestStorageBackend) -> Reference<Skeleton> {
		let model = SkeletonModel {
			nodes: vec![SkeletonNode {
				name: Some("root".into()),
				parent: None,
				rest_local: LocalTransform::identity(),
			}],
		};
		let reference: ReferenceModel<SkeletonModel> = storage
			.store(ProcessedAsset::new(ResourceId::new("test.skeleton"), model), &[])
			.expect("Test skeleton should store")
			.into();
		reference.solve(storage).expect("Test skeleton should solve")
	}

	fn test_primitive(skin: Option<u32>, joints: bool, weights: bool) -> PrimitiveModel {
		let mut streams = Vec::new();
		if joints {
			streams.push(stream(Streams::Vertices(VertexSemantics::Joints), 0, 8, 8));
		}
		if weights {
			streams.push(stream(Streams::Vertices(VertexSemantics::Weights), 8, 16, 16));
		}
		PrimitiveModel {
			material: ReferenceModel::new_serialized(
				"materials/test.variant",
				0,
				0,
				crate::to_vec(&VariantModel {
					material: ReferenceModel::new_serialized("materials/test.material", 0, 0, Vec::new(), None),
					variables: Vec::new(),
					alpha_mode: AlphaMode::Opaque,
				})
				.expect("Variant should serialize"),
				None,
			),
			transform_node: None,
			skin,
			streams,
			quantization: None,
			bounding_box: [[0.0; 3]; 2],
			vertex_count: 1,
		}
	}
}
