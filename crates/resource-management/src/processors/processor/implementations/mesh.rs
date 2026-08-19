mod packing;
mod source;
mod validation;

#[cfg(test)]
use packing::MESHLET_STREAM_STRIDE;
pub use packing::{
	orient_triangle_indices_for_front_face, MeshPrimitiveProcessingError, MeshProcessor, MeshProcessorSession, ProcessedMesh,
	TriangleFrontFaceWinding,
};
pub use source::{MeshPrimitiveSource, VertexSkin};
pub use validation::MeshProcessingError;

#[cfg(test)]
mod tests {
	use std::convert::Infallible;

	use super::{
		MeshPrimitiveProcessingError, MeshPrimitiveSource, MeshProcessingError, MeshProcessor, ProcessedMesh,
		TriangleFrontFaceWinding, VertexSkin,
	};
	use crate::{
		resources::{
			material::VariantModel,
			skeleton::{
				identity_affine_matrix4x3_columns, LocalTransform, SkeletonModel, SkeletonNode, SkinBinding, SkinJoint,
				SkinPaletteEntry,
			},
		},
		types::{AlphaMode, Streams, VertexComponent, VertexSemantics},
		ReferenceModel,
	};

	#[test]
	fn rewinds_triangle_order_for_clockwise_front_faces() {
		assert_eq!(
			super::orient_triangle_indices_for_front_face(vec![0, 1, 2, 3, 4, 5], TriangleFrontFaceWinding::Clockwise),
			vec![0, 2, 1, 3, 5, 4]
		);
	}

	#[test]
	fn preserves_triangle_order_for_counter_clockwise_front_faces() {
		assert_eq!(
			super::orient_triangle_indices_for_front_face(vec![0, 1, 2, 3, 4, 5], TriangleFrontFaceWinding::CounterClockwise,),
			vec![0, 1, 2, 3, 4, 5]
		);
	}

	#[test]
	fn packs_streams_from_borrowed_iterators() {
		let primitive = TestPrimitive::triangle().with_normals().with_uvs();
		let processed = process(
			vec![
				component(VertexSemantics::Position),
				component(VertexSemantics::Normal),
				component(VertexSemantics::UV),
			],
			&[primitive],
			None,
			Vec::new(),
		)
		.expect("borrowed primitive should process");

		assert_eq!(processed.mesh.primitives.len(), 1);
		assert_eq!(processed.mesh.streams.len(), 7);
		let meshlets = processed
			.mesh
			.streams
			.iter()
			.find(|stream| stream.stream_type == Streams::Meshlets)
			.expect("processed mesh should include meshlets");
		assert_eq!(meshlets.stride, super::MESHLET_STREAM_STRIDE);
		assert_eq!(meshlets.size, super::MESHLET_STREAM_STRIDE);
	}

	#[test]
	fn preserves_skin_metadata_and_streams() {
		let skeleton = test_skeleton(1);
		let primitive = TestPrimitive::triangle()
			.with_skin(0, vec![valid_vertex_skin(); 3])
			.with_transform_node(0);
		let processed = process(
			skinned_layout(),
			&[primitive],
			Some(skeleton),
			vec![test_skin(SkinJoint::Node(0))],
		)
		.expect("skinned primitive should process");

		assert_eq!(processed.mesh.skins.len(), 1);
		assert_eq!(processed.mesh.primitives[0].transform_node, Some(0));
		assert_eq!(processed.mesh.primitives[0].skin, Some(0));
		assert!(processed.mesh.primitives[0]
			.streams
			.iter()
			.any(|stream| stream.stream_type == Streams::Vertices(VertexSemantics::Joints)));
	}

	#[test]
	fn rejects_invalid_skin_and_transform_references() {
		let error = process(
			skinned_layout(),
			&[TestPrimitive::triangle().with_skin(0, vec![valid_vertex_skin(); 3])],
			Some(test_skeleton(1)),
			vec![test_skin(SkinJoint::Node(1))],
		)
		.expect_err("out-of-range skin node should fail");
		assert_eq!(
			error,
			MeshProcessingError::SkinJointOutOfRange {
				skin: 0,
				joint: 0,
				node: 1,
				nodes: 1,
			}
		);

		let error = process(
			vec![component(VertexSemantics::Position)],
			&[TestPrimitive::triangle().with_transform_node(0)],
			None,
			Vec::new(),
		)
		.expect_err("node-driven primitive should require a skeleton");
		assert_eq!(
			error,
			MeshProcessingError::TransformNodeWithoutSkeleton { primitive: 0, node: 0 }
		);
	}

	#[test]
	fn rejects_skin_data_without_matching_metadata() {
		let error = process(
			skinned_layout(),
			&[TestPrimitive::triangle().with_skin_index_without_data(0)],
			Some(test_skeleton(1)),
			vec![test_skin(SkinJoint::Node(0))],
		)
		.expect_err("skin binding should require vertex skin values");
		assert_eq!(error, MeshProcessingError::IncompleteSkinAttributes { primitive: 0 });

		let error = process(
			skinned_layout(),
			&[TestPrimitive::triangle().with_unbound_skin_data(vec![valid_vertex_skin(); 3])],
			Some(test_skeleton(1)),
			vec![test_skin(SkinJoint::Node(0))],
		)
		.expect_err("vertex skin values should require a binding");
		assert_eq!(error, MeshProcessingError::UnboundSkinAttributes { primitive: 0 });
	}

	#[test]
	fn rejects_invalid_skin_layout_and_vertex_values() {
		let primitive = TestPrimitive::triangle().with_skin(0, vec![valid_vertex_skin(); 3]);
		let error = process(
			vec![component(VertexSemantics::Position), component(VertexSemantics::Joints)],
			&[primitive],
			Some(test_skeleton(1)),
			vec![test_skin(SkinJoint::Node(0))],
		)
		.expect_err("incomplete skin layout should fail");
		assert_eq!(
			error,
			MeshProcessingError::MissingSkinVertexComponent(VertexSemantics::Weights)
		);

		for semantic in [VertexSemantics::Joints, VertexSemantics::Weights] {
			let mut layout = skinned_layout();
			layout
				.iter_mut()
				.find(|component| component.semantic == semantic)
				.expect("skin component should exist")
				.format = "wrong".to_string();
			let error = process(
				layout,
				&[TestPrimitive::triangle().with_skin(0, vec![valid_vertex_skin(); 3])],
				Some(test_skeleton(1)),
				vec![test_skin(SkinJoint::Node(0))],
			)
			.expect_err("mistyped skin layout should fail");
			assert!(matches!(
				error,
				MeshProcessingError::InvalidSkinVertexComponentFormat {
					semantic: actual,
					..
				} if actual == semantic
			));
		}

		let error = process(
			skinned_layout(),
			&[TestPrimitive::triangle().with_skin(0, vec![valid_vertex_skin(); 2])],
			Some(test_skeleton(1)),
			vec![test_skin(SkinJoint::Node(0))],
		)
		.expect_err("skin value count should match positions");
		assert_eq!(
			error,
			MeshProcessingError::SkinVertexCountMismatch {
				primitive: 0,
				values: 2,
				positions: 3,
			}
		);

		for (value, expected) in [
			(
				VertexSkin {
					joints: [1, 0, 0, 0],
					weights: [1.0, 0.0, 0.0, 0.0],
				},
				MeshProcessingError::VertexJointOutOfRange {
					primitive: 0,
					vertex: 0,
					lane: 0,
					joint: 1,
					palette_len: 1,
				},
			),
			(
				VertexSkin {
					joints: [0; 4],
					weights: [f32::NAN, 0.0, 0.0, 0.0],
				},
				MeshProcessingError::NonFiniteSkinWeight {
					primitive: 0,
					vertex: 0,
					lane: 0,
				},
			),
			(
				VertexSkin {
					joints: [0; 4],
					weights: [-1.0, 2.0, 0.0, 0.0],
				},
				MeshProcessingError::NegativeSkinWeight {
					primitive: 0,
					vertex: 0,
					lane: 0,
				},
			),
			(
				VertexSkin {
					joints: [0; 4],
					weights: [0.0; 4],
				},
				MeshProcessingError::NonPositiveSkinWeightTotal { primitive: 0, vertex: 0 },
			),
			(
				VertexSkin {
					joints: [0; 4],
					weights: [0.4, 0.4, 0.0, 0.0],
				},
				MeshProcessingError::NonNormalizedSkinWeights { primitive: 0, vertex: 0 },
			),
		] {
			let mut values = vec![valid_vertex_skin(); 3];
			values[0] = value;
			let error = process(
				skinned_layout(),
				&[TestPrimitive::triangle().with_skin(0, values)],
				Some(test_skeleton(1)),
				vec![test_skin(SkinJoint::Node(0))],
			)
			.expect_err("invalid vertex skin should fail");
			assert_eq!(error, expected);
		}
	}

	#[test]
	fn rejects_duplicate_semantics_and_omits_unused_streams() {
		let duplicate = vec![component(VertexSemantics::UV), component(VertexSemantics::UV)];
		let error = MeshProcessor::new()
			.begin(duplicate, None, Vec::new())
			.err()
			.expect("duplicate layout should fail");
		assert_eq!(error, MeshProcessingError::DuplicateVertexSemantic(VertexSemantics::UV));

		let processed = process(
			vec![component(VertexSemantics::Position), component(VertexSemantics::BiTangent)],
			&[TestPrimitive::triangle()],
			None,
			Vec::new(),
		)
		.expect("unused optional stream should be omitted");
		assert_eq!(processed.mesh.vertex_components.len(), 1);
		assert_eq!(processed.mesh.vertex_components[0].semantic, VertexSemantics::Position);
	}

	#[test]
	fn rejects_iterator_lengths_that_do_not_match_positions() {
		let mut primitive = TestPrimitive::triangle().with_normals();
		primitive.normals.as_mut().expect("normal values should exist").pop();
		let error = process(
			vec![component(VertexSemantics::Position), component(VertexSemantics::Normal)],
			&[primitive],
			None,
			Vec::new(),
		)
		.expect_err("normal count should match positions");
		assert_eq!(
			error,
			MeshProcessingError::AttributeLengthMismatch(VertexSemantics::Normal, 0)
		);
	}

	fn process(
		layout: Vec<VertexComponent>,
		primitives: &[TestPrimitive],
		skeleton: Option<ReferenceModel<SkeletonModel>>,
		skins: Vec<SkinBinding>,
	) -> Result<ProcessedMesh, MeshProcessingError> {
		let mut processor = MeshProcessor::new().begin(layout, skeleton, skins)?;
		for primitive in primitives {
			processor.push_primitive(primitive).map_err(|error| match error {
				MeshPrimitiveProcessingError::Source(never) => match never {},
				MeshPrimitiveProcessingError::Processing(error) => error,
			})?;
		}
		Ok(processor.finish())
	}

	fn component(semantic: VertexSemantics) -> VertexComponent {
		VertexComponent {
			semantic,
			format: match semantic {
				VertexSemantics::Position | VertexSemantics::Normal | VertexSemantics::BiTangent => "vec3f",
				VertexSemantics::UV => "vec2f",
				VertexSemantics::Joints => "vec4u16",
				VertexSemantics::Tangent | VertexSemantics::Color | VertexSemantics::Weights => "vec4f",
			}
			.to_string(),
			channel: 0,
		}
	}

	fn skinned_layout() -> Vec<VertexComponent> {
		vec![
			component(VertexSemantics::Position),
			component(VertexSemantics::Joints),
			component(VertexSemantics::Weights),
		]
	}

	fn valid_vertex_skin() -> VertexSkin {
		VertexSkin {
			joints: [0; 4],
			weights: [1.0, 0.0, 0.0, 0.0],
		}
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
			.expect("variant should serialize"),
			None,
		)
	}

	struct TestPrimitive {
		material: ReferenceModel<VariantModel>,
		indices: Vec<u32>,
		positions: Vec<[f32; 3]>,
		normals: Option<Vec<[f32; 3]>>,
		uvs: Option<Vec<[f32; 2]>>,
		vertex_skin: Option<Vec<VertexSkin>>,
		transform_node: Option<u32>,
		skin: Option<u32>,
	}

	impl TestPrimitive {
		fn triangle() -> Self {
			Self {
				material: test_material(),
				indices: vec![0, 1, 2],
				positions: vec![[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]],
				normals: None,
				uvs: None,
				vertex_skin: None,
				transform_node: None,
				skin: None,
			}
		}

		fn with_normals(mut self) -> Self {
			self.normals = Some(vec![[0.0, 0.0, 1.0]; 3]);
			self
		}

		fn with_uvs(mut self) -> Self {
			self.uvs = Some(vec![[0.0, 0.0], [1.0, 0.0], [0.0, 1.0]]);
			self
		}

		fn with_transform_node(mut self, transform_node: u32) -> Self {
			self.transform_node = Some(transform_node);
			self
		}

		fn with_skin(mut self, skin: u32, values: Vec<VertexSkin>) -> Self {
			self.skin = Some(skin);
			self.vertex_skin = Some(values);
			self
		}

		fn with_skin_index_without_data(mut self, skin: u32) -> Self {
			self.skin = Some(skin);
			self
		}

		fn with_unbound_skin_data(mut self, values: Vec<VertexSkin>) -> Self {
			self.vertex_skin = Some(values);
			self
		}
	}

	impl MeshPrimitiveSource for TestPrimitive {
		type Error = Infallible;

		fn material(&self) -> &ReferenceModel<VariantModel> {
			&self.material
		}

		fn transform_node(&self) -> Option<u32> {
			self.transform_node
		}

		fn skin(&self) -> Option<u32> {
			self.skin
		}

		fn indices(&self) -> Result<impl ExactSizeIterator<Item = Result<u32, Self::Error>> + '_, Self::Error> {
			Ok(self.indices.iter().copied().map(Ok))
		}

		fn positions(&self) -> Result<impl ExactSizeIterator<Item = Result<[f32; 3], Self::Error>> + '_, Self::Error> {
			Ok(self.positions.iter().copied().map(Ok))
		}

		fn normals(&self) -> Result<Option<impl ExactSizeIterator<Item = Result<[f32; 3], Self::Error>> + '_>, Self::Error> {
			Ok(self.normals.as_deref().map(|values| values.iter().copied().map(Ok)))
		}

		fn uvs(&self) -> Result<Option<impl ExactSizeIterator<Item = Result<[f32; 2], Self::Error>> + '_>, Self::Error> {
			Ok(self.uvs.as_deref().map(|values| values.iter().copied().map(Ok)))
		}

		fn vertex_skin(
			&self,
		) -> Result<Option<impl ExactSizeIterator<Item = Result<VertexSkin, Self::Error>> + '_>, Self::Error> {
			Ok(self.vertex_skin.as_deref().map(|values| values.iter().copied().map(Ok)))
		}
	}
}
