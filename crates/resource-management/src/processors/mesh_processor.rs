mod packing;
mod source;
mod validation;

#[cfg(test)]
use packing::MESHLET_STREAM_STRIDE;
pub use packing::{MeshProcessor, ProcessedMesh, TriangleFrontFaceWinding, orient_triangle_indices_for_front_face};
pub use source::{
	MeshAttributeData, MeshIndexData, MeshPrimitiveSource, MeshSource, OwnedMeshAttribute, OwnedMeshAttributeData,
	OwnedMeshPrimitive, OwnedMeshSource,
};
pub use validation::MeshProcessingError;

#[cfg(test)]
mod tests {
	use super::{
		MeshProcessingError, MeshProcessor, OwnedMeshAttribute, OwnedMeshAttributeData, OwnedMeshPrimitive, OwnedMeshSource,
		TriangleFrontFaceWinding,
	};
	use crate::types::VertexSemantics;
	use crate::{
		ReferenceModel,
		resources::{
			material::VariantModel,
			skeleton::{
				LocalTransform, SkeletonModel, SkeletonNode, SkinBinding, SkinJoint, SkinPaletteEntry,
				identity_affine_matrix4x3_columns,
			},
		},
		types::{AlphaMode, VertexComponent},
	};

	#[test]
	fn rewinds_triangle_order_for_clockwise_front_faces() {
		let indices = vec![0, 1, 2, 3, 4, 5];

		let oriented = super::orient_triangle_indices_for_front_face(indices, TriangleFrontFaceWinding::Clockwise);

		assert_eq!(oriented, vec![0, 2, 1, 3, 5, 4]);
	}

	#[test]
	fn preserves_triangle_order_for_counter_clockwise_front_faces() {
		let indices = vec![0, 1, 2, 3, 4, 5];

		let oriented = super::orient_triangle_indices_for_front_face(indices, TriangleFrontFaceWinding::CounterClockwise);

		assert_eq!(oriented, vec![0, 1, 2, 3, 4, 5]);
	}

	#[test]
	fn packs_mesh_streams_from_a_query_based_source() {
		let source = OwnedMeshSource::new(
			vec![
				VertexComponent {
					semantic: VertexSemantics::Position,
					format: "vec3f".to_string(),
					channel: 0,
				},
				VertexComponent {
					semantic: VertexSemantics::Normal,
					format: "vec3f".to_string(),
					channel: 0,
				},
				VertexComponent {
					semantic: VertexSemantics::UV,
					format: "vec2f".to_string(),
					channel: 0,
				},
			],
			vec![
				OwnedMeshPrimitive::new(test_material(), [[0.0, 0.0, 0.0], [1.0, 1.0, 0.0]], vec![0, 1, 2])
					.with_attribute(OwnedMeshAttribute::new(
						VertexSemantics::Position,
						0,
						OwnedMeshAttributeData::F32x3(vec![[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]]),
					))
					.with_attribute(OwnedMeshAttribute::new(
						VertexSemantics::Normal,
						0,
						OwnedMeshAttributeData::F32x3(vec![[0.0, 0.0, 1.0]; 3]),
					))
					.with_attribute(OwnedMeshAttribute::new(
						VertexSemantics::UV,
						0,
						OwnedMeshAttributeData::F32x2(vec![[0.0, 0.0], [1.0, 0.0], [0.0, 1.0]]),
					)),
			],
		);

		let processed = MeshProcessor::new().process(&source).expect("Mesh processing should succeed");

		assert_eq!(processed.mesh.primitives.len(), 1);
		assert_eq!(processed.mesh.streams.len(), 7);
		assert_eq!(
			processed.mesh.streams[0].stream_type,
			crate::types::Streams::Vertices(VertexSemantics::Position)
		);
		assert_eq!(
			processed.mesh.streams[4].stream_type,
			crate::types::Streams::Indices(crate::types::IndexStreamTypes::Triangles)
		);
		let meshlet_stream = processed
			.mesh
			.streams
			.iter()
			.find(|stream| stream.stream_type == crate::types::Streams::Meshlets)
			.expect("Processed mesh should include a packed meshlet stream");
		assert_eq!(meshlet_stream.stride, super::MESHLET_STREAM_STRIDE);
		assert_eq!(meshlet_stream.size, super::MESHLET_STREAM_STRIDE);
		assert!(!processed.buffer.is_empty());
	}

	#[test]
	fn preserves_processable_skin_metadata_and_joint_streams() {
		let source = OwnedMeshSource::new(skinned_layout(), vec![skinned_primitive(true).with_transform_node(0)])
			.with_skeleton(test_skeleton(1))
			.with_skins(vec![test_skin(SkinJoint::Node(0))]);

		let processed = MeshProcessor::new()
			.process_owned(source)
			.expect("Skinned mesh processing should succeed");

		assert!(processed.mesh.skeleton.is_some());
		assert_eq!(processed.mesh.skins.len(), 1);
		assert_eq!(processed.mesh.primitives[0].transform_node, Some(0));
		assert_eq!(processed.mesh.primitives[0].skin, Some(0));
		assert!(
			processed.mesh.primitives[0]
				.streams
				.iter()
				.any(|stream| stream.stream_type == crate::types::Streams::Vertices(VertexSemantics::Joints))
		);
		assert!(
			processed.mesh.primitives[0]
				.streams
				.iter()
				.any(|stream| stream.stream_type == crate::types::Streams::Vertices(VertexSemantics::Weights))
		);
	}

	#[test]
	fn rejects_skin_nodes_outside_the_source_skeleton() {
		let source = OwnedMeshSource::new(skinned_layout(), vec![skinned_primitive(true)])
			.with_skeleton(test_skeleton(1))
			.with_skins(vec![test_skin(SkinJoint::Node(1))]);

		let error = MeshProcessor::new()
			.process(&source)
			.expect_err("Out-of-range palette nodes should be rejected before packing");

		assert_eq!(
			error,
			MeshProcessingError::SkinJointOutOfRange {
				skin: 0,
				joint: 0,
				node: 1,
				nodes: 1,
			}
		);
	}

	#[test]
	fn rejects_skinned_primitives_without_paired_joint_and_weight_attributes() {
		let source = OwnedMeshSource::new(skinned_layout(), vec![skinned_primitive(false)])
			.with_skeleton(test_skeleton(1))
			.with_skins(vec![test_skin(SkinJoint::Node(0))]);

		let error = MeshProcessor::new()
			.process(&source)
			.expect_err("Skinned primitives should require both joint and weight attributes");

		assert_eq!(error, MeshProcessingError::IncompleteSkinAttributes { primitive: 0 });
	}

	#[test]
	fn rejects_skin_bindings_without_a_skeleton() {
		let source = OwnedMeshSource::new(Vec::new(), Vec::new()).with_skins(vec![test_skin(SkinJoint::Identity)]);

		let error = MeshProcessor::new()
			.process(&source)
			.expect_err("Skin bindings should require a skeleton reference");

		assert_eq!(error, MeshProcessingError::SkinWithoutSkeleton);
	}

	#[test]
	fn rejects_skinned_primitives_when_required_layout_components_are_missing_or_mistyped() {
		for semantic in [VertexSemantics::Joints, VertexSemantics::Weights] {
			let mut source = valid_skinned_source();
			source.vertex_layout_mut().retain(|component| component.semantic != semantic);
			let error = MeshProcessor::new()
				.process(&source)
				.expect_err("A missing skin layout component should be rejected");
			assert_eq!(error, MeshProcessingError::MissingSkinVertexComponent(semantic));
		}

		for (semantic, expected) in [(VertexSemantics::Joints, "vec4u16"), (VertexSemantics::Weights, "vec4f")] {
			let mut source = valid_skinned_source();
			source
				.vertex_layout_mut()
				.iter_mut()
				.find(|component| component.semantic == semantic)
				.expect("Skin component should exist")
				.format = "wrong".into();
			let error = MeshProcessor::new()
				.process(&source)
				.expect_err("A mistyped skin layout component should be rejected");
			assert_eq!(
				error,
				MeshProcessingError::InvalidSkinVertexComponentFormat {
					semantic,
					expected,
					actual: "wrong".into(),
				}
			);
		}
	}

	#[test]
	fn rejects_skin_attributes_with_the_wrong_typed_payload() {
		let mut source = valid_skinned_source();
		let attribute = source.primitives_mut()[0]
			.attributes
			.iter_mut()
			.find(|attribute| attribute.semantic == VertexSemantics::Joints)
			.expect("Joint attribute should exist");
		attribute.data = OwnedMeshAttributeData::F32x2(vec![[0.0; 2]; 3]);

		let error = MeshProcessor::new()
			.process(&source)
			.expect_err("A mistyped joint payload should be rejected");
		assert_eq!(error, MeshProcessingError::InvalidAttributeFormat(VertexSemantics::Joints));
	}

	#[test]
	fn rejects_vertex_joint_indices_outside_the_selected_palette() {
		let mut source = valid_skinned_source();
		let OwnedMeshAttributeData::U16x4(joints) = skin_attribute_data_mut(&mut source, VertexSemantics::Joints) else {
			panic!("Joint test data should use U16x4")
		};
		joints[0][2] = 1;

		let error = MeshProcessor::new()
			.process(&source)
			.expect_err("An out-of-range vertex joint should be rejected");
		assert_eq!(
			error,
			MeshProcessingError::VertexJointOutOfRange {
				primitive: 0,
				vertex: 0,
				lane: 2,
				joint: 1,
				palette_len: 1,
			}
		);
	}

	#[test]
	fn rejects_non_finite_negative_zero_total_and_non_normalized_skin_weights() {
		let cases = [
			(
				[f32::NAN, 0.0, 0.0, 0.0],
				MeshProcessingError::NonFiniteSkinWeight {
					primitive: 0,
					vertex: 0,
					lane: 0,
				},
			),
			(
				[-0.25, 1.25, 0.0, 0.0],
				MeshProcessingError::NegativeSkinWeight {
					primitive: 0,
					vertex: 0,
					lane: 0,
				},
			),
			(
				[0.0; 4],
				MeshProcessingError::NonPositiveSkinWeightTotal { primitive: 0, vertex: 0 },
			),
			(
				[0.4, 0.4, 0.0, 0.0],
				MeshProcessingError::NonNormalizedSkinWeights { primitive: 0, vertex: 0 },
			),
		];

		for (weights, expected) in cases {
			let mut source = valid_skinned_source();
			let OwnedMeshAttributeData::F32x4(values) = skin_attribute_data_mut(&mut source, VertexSemantics::Weights) else {
				panic!("Weight test data should use F32x4")
			};
			values[0] = weights;
			let error = MeshProcessor::new()
				.process(&source)
				.expect_err("Invalid skin weights should be rejected");
			assert_eq!(error, expected);
		}
	}

	#[test]
	fn rejects_duplicate_vertex_semantics_in_the_layout() {
		let source = OwnedMeshSource::new(
			vec![
				VertexComponent {
					semantic: VertexSemantics::UV,
					format: "vec2f".to_string(),
					channel: 0,
				},
				VertexComponent {
					semantic: VertexSemantics::UV,
					format: "vec2f".to_string(),
					channel: 1,
				},
			],
			Vec::new(),
		);

		let error = MeshProcessor::new()
			.process(&source)
			.expect_err("Mesh processing should reject duplicate semantics");

		assert_eq!(error, MeshProcessingError::DuplicateVertexSemantic(VertexSemantics::UV));
	}

	#[test]
	fn skips_disabled_vertex_streams_from_the_layout() {
		let source = OwnedMeshSource::new(
			vec![
				VertexComponent {
					semantic: VertexSemantics::Position,
					format: "vec3f".to_string(),
					channel: 0,
				},
				VertexComponent {
					semantic: VertexSemantics::BiTangent,
					format: "vec3f".to_string(),
					channel: 0,
				},
			],
			vec![
				OwnedMeshPrimitive::new(test_material(), [[0.0, 0.0, 0.0], [1.0, 1.0, 0.0]], vec![0, 1, 2]).with_attribute(
					OwnedMeshAttribute::new(
						VertexSemantics::Position,
						0,
						OwnedMeshAttributeData::F32x3(vec![[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]]),
					),
				),
			],
		);

		let processed = MeshProcessor::new().process(&source).expect("Mesh processing should succeed");

		assert_eq!(processed.mesh.vertex_components.len(), 1);
		assert_eq!(processed.mesh.vertex_components[0].semantic, VertexSemantics::Position);
		assert!(
			processed
				.mesh
				.streams
				.iter()
				.all(|stream| stream.stream_type != crate::types::Streams::Vertices(VertexSemantics::BiTangent))
		);
	}

	fn test_material() -> ReferenceModel<VariantModel> {
		ReferenceModel::new_serialized(
			"materials/test.variant",
			0,
			0,
			crate::to_vec(&VariantModel {
				material: ReferenceModel::new_serialized("materials/test.material", 0, 0, Vec::new(), None),
				variables: Vec::new(),
				alpha_mode: AlphaMode::Opaque,
			})
			.expect("Variant model should serialize"),
			None,
		)
	}

	fn skinned_layout() -> Vec<VertexComponent> {
		vec![
			VertexComponent {
				semantic: VertexSemantics::Position,
				format: "vec3f".to_string(),
				channel: 0,
			},
			VertexComponent {
				semantic: VertexSemantics::Joints,
				format: "vec4u16".to_string(),
				channel: 0,
			},
			VertexComponent {
				semantic: VertexSemantics::Weights,
				format: "vec4f".to_string(),
				channel: 0,
			},
		]
	}

	fn skinned_primitive(include_weights: bool) -> OwnedMeshPrimitive {
		let mut primitive = OwnedMeshPrimitive::new(test_material(), [[0.0, 0.0, 0.0], [1.0, 1.0, 0.0]], vec![0, 1, 2])
			.with_skin(0)
			.with_attribute(OwnedMeshAttribute::new(
				VertexSemantics::Position,
				0,
				OwnedMeshAttributeData::F32x3(vec![[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]]),
			))
			.with_attribute(OwnedMeshAttribute::new(
				VertexSemantics::Joints,
				0,
				OwnedMeshAttributeData::U16x4(vec![[0, 0, 0, 0]; 3]),
			));
		if include_weights {
			primitive.add_attribute(OwnedMeshAttribute::new(
				VertexSemantics::Weights,
				0,
				OwnedMeshAttributeData::F32x4(vec![[1.0, 0.0, 0.0, 0.0]; 3]),
			));
		}
		primitive
	}

	fn test_skeleton(node_count: usize) -> ReferenceModel<SkeletonModel> {
		ReferenceModel::new(
			"skeletons/test.skeleton",
			0,
			0,
			&SkeletonModel {
				nodes: (0..node_count)
					.map(|index| SkeletonNode {
						name: None,
						parent: index.checked_sub(1).map(|parent| parent as u32),
						rest_local: LocalTransform::identity(),
					})
					.collect(),
			},
			None,
		)
	}

	fn test_skin(joint: SkinJoint) -> SkinBinding {
		SkinBinding {
			entries: vec![SkinPaletteEntry {
				joint,
				adjusted_inverse_bind_matrix: identity_affine_matrix4x3_columns(),
			}],
		}
	}

	fn valid_skinned_source() -> OwnedMeshSource {
		OwnedMeshSource::new(skinned_layout(), vec![skinned_primitive(true)])
			.with_skeleton(test_skeleton(1))
			.with_skins(vec![test_skin(SkinJoint::Node(0))])
	}

	fn skin_attribute_data_mut(source: &mut OwnedMeshSource, semantic: VertexSemantics) -> &mut OwnedMeshAttributeData {
		&mut source.primitives_mut()[0]
			.attributes
			.iter_mut()
			.find(|attribute| attribute.semantic == semantic)
			.expect("Skin test attribute should exist")
			.data
	}
}
