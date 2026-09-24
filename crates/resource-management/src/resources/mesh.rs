use crate::{
	Reference, ReferenceModel, Solver, resource,
	resources::material::VariantModel,
	resources::skeleton::{Skeleton, SkeletonModel, SkinBinding, SkinJoint},
	solver::SolveErrors,
	types::{IndexStreamTypes, QuantizationSchemes, Stream, Streams, VertexComponent, VertexSemantics},
};

/// The `Primitive` struct supplies one renderable geometry range and its skeletal bindings to runtime rendering.
///
/// The same type is stored and loaded: a primitive names its material by index into [`Mesh::materials`], so
/// primitives that share a material share one reference and nothing is solved per primitive.
#[derive(Debug, serde::Serialize, serde::Deserialize, rkyv::Archive, rkyv::Serialize, rkyv::Deserialize)]
pub struct Primitive {
	/// Index of this primitive's material variant in [`Mesh::materials`].
	pub material: u32,
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

super::impl_resource_model!(Primitive, Primitive, "Primitive");

/// The `Mesh` struct supplies packed geometry, material primitives, and optional skeletal bindings to runtime rendering.
///
/// The index streams use these meanings:
///
/// - `Vertices` entries index the vertex buffer.
/// - `Meshlets` entries index the `Vertices` stream.
/// - `Triangles` entries index the vertex buffer.
///
/// Material variants stay unsolved: request each one by its ID when it is needed, which lets renderers load and
/// deduplicate materials on their own schedule instead of once per primitive.
#[derive(Debug, serde::Serialize)]
pub struct Mesh {
	pub skeleton: Option<Reference<Skeleton>>,
	pub skins: Vec<SkinBinding>,
	pub vertex_components: Vec<VertexComponent>,
	pub streams: Vec<Stream>,
	/// The distinct material variants the primitives draw with.
	pub materials: Vec<ReferenceModel<VariantModel>>,
	pub primitives: Vec<Primitive>,
}

// Named stream accessors remain part of the public API while sharing one exact cloned lookup implementation.
macro_rules! cloned_stream_accessor {
	($name:ident, $stream_type:expr) => {
		pub fn $name(&self) -> Option<Stream> {
			self.stream($stream_type).cloned()
		}
	};
}

impl Mesh {
	pub fn primitives(&self) -> impl Iterator<Item = &Primitive> {
		self.primitives.iter()
	}

	/// Returns the material variant a primitive draws with.
	pub fn material(&self, primitive: &Primitive) -> &ReferenceModel<VariantModel> {
		&self.materials[primitive.material as usize]
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

	cloned_stream_accessor!(position_stream, Streams::Vertices(VertexSemantics::Position));
	cloned_stream_accessor!(normal_stream, Streams::Vertices(VertexSemantics::Normal));
	cloned_stream_accessor!(tangent_stream, Streams::Vertices(VertexSemantics::Tangent));
	cloned_stream_accessor!(bi_tangent_stream, Streams::Vertices(VertexSemantics::BiTangent));
	cloned_stream_accessor!(uv_stream, Streams::Vertices(VertexSemantics::UV));

	pub fn color_stream(&self) -> Option<&Stream> {
		self.vertex_stream(VertexSemantics::Color)
	}

	cloned_stream_accessor!(triangle_indices_stream, Streams::Indices(IndexStreamTypes::Triangles));
	cloned_stream_accessor!(vertex_indices_stream, Streams::Indices(IndexStreamTypes::Vertices));
	cloned_stream_accessor!(meshlet_indices_stream, Streams::Indices(IndexStreamTypes::Meshlets));
	cloned_stream_accessor!(meshlets_stream, Streams::Meshlets);

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

/// The `MeshModel` struct preserves processed geometry and skeletal bindings for storage and later runtime solving.
#[derive(Debug, serde::Serialize, serde::Deserialize, rkyv::Archive, rkyv::Serialize, rkyv::Deserialize)]
pub struct MeshModel {
	pub skeleton: Option<ReferenceModel<SkeletonModel>>,
	pub skins: Vec<SkinBinding>,
	pub vertex_components: Vec<VertexComponent>,
	pub streams: Vec<Stream>,
	/// The distinct material variants, each stored once however many primitives use it.
	pub materials: Vec<ReferenceModel<VariantModel>>,
	pub primitives: Vec<Primitive>,
}

super::impl_resource_model!(Mesh, MeshModel, "Mesh");

impl crate::StoredModel for MeshModel {
	type Resource = Mesh;

	/// Resolves mesh dependencies only after confirming its skin tables are safe for CPU pose and GPU palette workflows.
	fn solve_stored<'de>(
		gr: crate::SerializableResource,
		reader: crate::resource::resource_handler::MultiResourceReader,
		storage_backend: &'de dyn resource::DynReadStorageBackend,
	) -> crate::r#async::BoxedFuture<'de, Result<Reference<Mesh>, SolveErrors>> {
		crate::r#async::future(async move {
			let MeshModel {
				skeleton,
				skins,
				vertex_components,
				streams,
				materials,
				primitives,
			} = crate::from_slice(&gr.resource).map_err(|error| {
				SolveErrors::DeserializationFailed(format!(
					"Mesh resource could not be deserialized. The most likely cause is incompatible or corrupted mesh metadata: {error}."
				))
			})?;

			let skeleton = match skeleton {
				Some(skeleton) => Some(skeleton.solve(storage_backend).await?),
				None => None,
			};
			validate_skin_metadata(skeleton.as_ref(), &skins, &vertex_components, &primitives)?;
			validate_material_indices(materials.len(), &primitives)?;

			Ok(Reference::from_stored(
				gr,
				Mesh {
					skeleton,
					skins,
					vertex_components,
					streams,
					materials,
					primitives,
				},
				reader,
			))
		})
	}
}

/// Rejects a primitive whose material index falls outside the mesh's material list.
fn validate_material_indices(material_count: usize, primitives: &[Primitive]) -> Result<(), SolveErrors> {
	match primitives
		.iter()
		.position(|primitive| primitive.material as usize >= material_count)
	{
		Some(index) => Err(SolveErrors::DeserializationFailed(format!(
			"Mesh primitive {index} references material {} of {material_count}. The most likely cause is corrupted mesh metadata.",
			primitives[index].material
		))),
		None => Ok(()),
	}
}

/// Validates that mesh skin tables, palette nodes, and primitive streams form a processable contract.
fn validate_skin_metadata(
	skeleton: Option<&Reference<Skeleton>>,
	skins: &[SkinBinding],
	vertex_components: &[VertexComponent],
	primitives: &[Primitive],
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
			if let SkinJoint::Node(node) = entry.joint
				&& node as usize >= skeleton_nodes
			{
				return invalid_mesh_skeletal_metadata(format!(
					"skin {skin_index} joint {joint_index} targets node {node} outside a {skeleton_nodes}-node skeleton"
				));
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
	use super::{Mesh, Primitive, validate_material_indices, validate_skin_metadata};
	use crate::{
		ProcessedAsset, Reference, ReferenceModel, Solver,
		asset::ResourceId,
		resource::{WriteStorageBackend, storage_backend::tests::TestStorageBackend},
		resources::skeleton::{
			LocalTransform, Skeleton, SkeletonModel, SkeletonNode, SkinBinding, SkinJoint, SkinPaletteEntry,
			identity_affine_matrix4x3_columns,
		},
		types::{IndexStreamTypes, Stream, Streams, VertexComponent, VertexSemantics},
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
			materials: Vec::new(),
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
			materials: Vec::new(),
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
			materials: Vec::new(),
			primitives: Vec::new(),
		};

		assert_eq!(mesh.triangle_count(), 0);
		assert_eq!(mesh.primitive_count(), 0);
	}

	#[crate::r#async::test]
	async fn skin_metadata_accepts_a_complete_palette_and_paired_vertex_streams() {
		let storage = TestStorageBackend::new();
		let skeleton = test_skeleton(&storage).await;
		let skins = vec![SkinBinding {
			entries: vec![SkinPaletteEntry {
				joint: SkinJoint::Node(0),
				adjusted_inverse_bind_matrix: identity_affine_matrix4x3_columns(),
			}],
		}];
		let primitives = vec![test_primitive(Some(0), true, true)];

		assert!(validate_skin_metadata(Some(&skeleton), &skins, &skin_vertex_layout(), &primitives).is_ok());
	}

	#[crate::r#async::test]
	async fn skin_metadata_rejects_missing_skeletons_invalid_indices_and_unpaired_streams() {
		let skin = SkinBinding {
			entries: vec![SkinPaletteEntry {
				joint: SkinJoint::Identity,
				adjusted_inverse_bind_matrix: identity_affine_matrix4x3_columns(),
			}],
		};

		assert!(validate_skin_metadata(None, std::slice::from_ref(&skin), &skin_vertex_layout(), &[]).is_err());

		let storage = TestStorageBackend::new();
		let skeleton = test_skeleton(&storage).await;

		assert!(
			validate_skin_metadata(
				Some(&skeleton),
				std::slice::from_ref(&skin),
				&skin_vertex_layout(),
				&[test_primitive(Some(1), true, true)]
			)
			.is_err()
		);
		assert!(
			validate_skin_metadata(
				Some(&skeleton),
				&[skin],
				&skin_vertex_layout(),
				&[test_primitive(Some(0), true, false)]
			)
			.is_err()
		);
	}

	#[crate::r#async::test]
	async fn primitive_transform_nodes_require_a_matching_skeleton_node() {
		let mut primitive = test_primitive(None, false, false);
		primitive.transform_node = Some(0);

		assert!(validate_skin_metadata(None, &[], &[], std::slice::from_ref(&primitive)).is_err());

		let storage = TestStorageBackend::new();
		let skeleton = test_skeleton(&storage).await;

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

	async fn test_skeleton(storage: &TestStorageBackend) -> Reference<Skeleton> {
		let model = SkeletonModel {
			nodes: vec![SkeletonNode {
				name: Some("root".into()),
				parent: None,
				rest_local: LocalTransform::identity(),
			}],
		};
		let reference: ReferenceModel<SkeletonModel> = storage
			.store(ProcessedAsset::new(ResourceId::new("test.skeleton"), model), &[])
			.await
			.expect("Test skeleton should store")
			.into();
		reference.solve(storage).await.expect("Test skeleton should solve")
	}

	fn test_primitive(skin: Option<u32>, joints: bool, weights: bool) -> Primitive {
		let mut streams = Vec::new();
		if joints {
			streams.push(stream(Streams::Vertices(VertexSemantics::Joints), 0, 8, 8));
		}
		if weights {
			streams.push(stream(Streams::Vertices(VertexSemantics::Weights), 8, 16, 16));
		}
		Primitive {
			material: 0,
			transform_node: None,
			skin,
			streams,
			quantization: None,
			bounding_box: [[0.0; 3]; 2],
			vertex_count: 1,
		}
	}

	#[test]
	fn material_indices_must_address_the_mesh_material_list() {
		let mut primitive = test_primitive(None, false, false);

		assert!(validate_material_indices(1, std::slice::from_ref(&primitive)).is_ok());

		primitive.material = 1;

		assert!(validate_material_indices(1, std::slice::from_ref(&primitive)).is_err());
	}
}
