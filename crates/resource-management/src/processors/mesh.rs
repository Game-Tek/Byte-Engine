mod lightmap_uvs;
mod lod;
mod packing;
mod source;
mod validation;

pub use lightmap_uvs::{LightmapUvSettings, inset_lightmap_uv, place_lightmap_charts, place_lightmap_rectangles};
pub use lod::MeshLodSettings;
pub(crate) use packing::MESHLET_STREAM_STRIDE;
pub use packing::{MeshPrimitiveProcessingError, MeshProcessorSession, ProcessedMesh};
pub use source::{MeshPrimitiveSource, VertexSkin};
pub use validation::MeshProcessingError;

/// The `MeshSettings` struct gathers what an asset chooses about how mesh processing bakes its primitives.
///
/// Pass it to [`MeshProcessorSession::new`]. The default suits most meshes and leaves lightmap UVs out; an asset
/// overrides it through the `lods` and `lightmap_uvs` keys of its BEAD file.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct MeshSettings {
	/// How many coarser detail levels each primitive gets.
	pub lods: MeshLodSettings,
	/// The lightmap UVs each unskinned primitive gets, or `None` to leave the `Vertex.LightmapUV` stream out.
	pub lightmap_uvs: Option<LightmapUvSettings>,
}

#[cfg(test)]
mod tests {
	use std::convert::Infallible;

	use super::{
		LightmapUvSettings, MeshLodSettings, MeshPrimitiveProcessingError, MeshPrimitiveSource, MeshProcessingError,
		MeshProcessorSession, MeshSettings, ProcessedMesh, VertexSkin, packing::MESHLET_STREAM_STRIDE,
	};
	use crate::{
		ReferenceModel,
		resources::{
			ModelSpace,
			material::VariantModel,
			skeleton::{LocalTransform, SkeletonModel, SkeletonNode, SkinBinding, SkinJoint, SkinPaletteEntry},
		},
		types::{AlphaMode, IndexStreamTypes, Streams, VertexComponent, VertexSemantics},
	};

	#[test]
	fn finish_into_writes_the_owned_payload_layout_without_a_combined_buffer() {
		let layout = vec![
			VertexComponent::canonical(VertexSemantics::Position),
			VertexComponent::canonical(VertexSemantics::Normal),
			VertexComponent::canonical(VertexSemantics::UV),
		];
		let expected = process(
			layout.clone(),
			&[TestPrimitive::triangle().with_normals().with_uvs()],
			None,
			Vec::new(),
		)
		.unwrap();
		let mut session = MeshProcessorSession::new(layout, None, Vec::new(), MeshSettings::default()).unwrap();
		session
			.push_primitive(&TestPrimitive::triangle().with_normals().with_uvs())
			.unwrap();
		let payload_size = session.payload_size();
		let mut payload = Vec::new();
		let (mesh, stream_descriptions) = session.finish_into(&[test_material()], &mut payload).unwrap();

		assert_eq!(payload_size, expected.buffer.len());
		assert_eq!(payload, expected.buffer.as_ref());
		assert_eq!(mesh.streams.len(), expected.mesh.streams.len());
		assert_eq!(stream_descriptions.len(), expected.stream_descriptions.len());
		for (actual, expected) in stream_descriptions.iter().zip(&expected.stream_descriptions) {
			assert_eq!(actual.name(), expected.name());
			assert_eq!(actual.size(), expected.size());
			assert_eq!(actual.offset(), expected.offset());
		}
	}

	#[test]
	fn stores_each_distinct_material_once_in_first_use_order() {
		// Slots 0 and 2 resolve to the same variant, and no primitive uses slot 3.
		let materials = [
			variant("materials/a.variant"),
			variant("materials/b.variant"),
			variant("materials/a.variant"),
			variant("materials/c.variant"),
		];
		let mut session = MeshProcessorSession::new(
			vec![VertexComponent::canonical(VertexSemantics::Position)],
			None,
			Vec::new(),
			MeshSettings::default(),
		)
		.unwrap();
		for slot in [2, 1, 0] {
			session
				.push_primitive(&TestPrimitive::triangle().with_material_slot(slot))
				.unwrap();
		}
		let processed = session.finish(&materials);

		let material_ids = processed
			.mesh
			.materials
			.iter()
			.map(|material| material.id().as_ref().to_owned())
			.collect::<Vec<_>>();
		let primitive_materials = processed
			.mesh
			.primitives
			.iter()
			.map(|primitive| primitive.material)
			.collect::<Vec<_>>();
		assert_eq!(material_ids, ["materials/a.variant", "materials/b.variant"]);
		assert_eq!(primitive_materials, [0, 1, 0]);
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
			vec![VertexComponent::canonical(VertexSemantics::Position)],
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
		let error = process_skinned(TestPrimitive::triangle().with_skin_index_without_data(0))
			.expect_err("skin binding should require vertex skin values");
		assert_eq!(error, MeshProcessingError::IncompleteSkinAttributes { primitive: 0 });

		let error = process_skinned(TestPrimitive::triangle().with_unbound_skin_data(vec![valid_vertex_skin(); 3]))
			.expect_err("vertex skin values should require a binding");
		assert_eq!(error, MeshProcessingError::UnboundSkinAttributes { primitive: 0 });
	}

	#[test]
	fn rejects_invalid_skin_layout_and_vertex_values() {
		let primitive = TestPrimitive::triangle().with_skin(0, vec![valid_vertex_skin(); 3]);
		let error = process(
			[VertexSemantics::Position, VertexSemantics::Joints]
				.map(VertexComponent::canonical)
				.to_vec(),
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
				MeshProcessingError::InvalidVertexComponentFormat {
					semantic: actual,
					..
				} if actual == semantic
			));
		}

		let error = process_skinned(TestPrimitive::triangle().with_skin(0, vec![valid_vertex_skin(); 2]))
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
			let error =
				process_skinned(TestPrimitive::triangle().with_skin(0, values)).expect_err("invalid vertex skin should fail");
			assert_eq!(error, expected);
		}
	}

	#[test]
	fn rejects_duplicate_semantics_and_omits_unused_streams() {
		let duplicate = [VertexSemantics::UV, VertexSemantics::UV]
			.map(VertexComponent::canonical)
			.to_vec();
		let error = MeshProcessorSession::new(duplicate, None, Vec::new(), MeshSettings::default())
			.err()
			.expect("duplicate layout should fail");
		assert_eq!(error, MeshProcessingError::DuplicateVertexSemantic(VertexSemantics::UV));

		let processed = process(
			[VertexSemantics::Position, VertexSemantics::BiTangent]
				.map(VertexComponent::canonical)
				.to_vec(),
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
			[VertexSemantics::Position, VertexSemantics::Normal]
				.map(VertexComponent::canonical)
				.to_vec(),
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

	#[test]
	fn detailed_primitives_gain_coarser_levels_over_a_shared_vertex_prefix() {
		for with_normals in [true, false] {
			let mut layout = vec![VertexComponent::canonical(VertexSemantics::Position)];
			let mut primitive = TestPrimitive::wavy_grid(32);
			if with_normals {
				layout.push(VertexComponent::canonical(VertexSemantics::Normal));
				primitive = primitive.with_normals();
			}
			let (vertex_count, index_count) = (primitive.positions.len(), primitive.indices.len());
			let processed = process(layout, &[primitive], None, Vec::new()).unwrap();
			let lods = &processed.mesh.primitives[0].lods;
			assert!(
				lods.len() >= 2,
				"a 2,048-triangle surface should get at least one coarser level"
			);

			// Each meshlet record starts with its vertex count and its triangle count.
			let triangles = (0..lods.len() as u8)
				.map(|level| {
					let records = stream_bytes(&processed, Streams::Meshlets, level);
					let records = records.as_chunks::<MESHLET_STREAM_STRIDE>().0;
					assert_eq!(records.len(), lods[level as usize].meshlet_count as usize);
					records.iter().map(|record| record[1] as usize).sum::<usize>()
				})
				.collect::<Vec<_>>();
			assert_eq!((triangles[0], lods[0].error), (index_count / 3, 0.0));
			assert!(triangles.is_sorted_by(|finer, coarser| finer > coarser));
			assert!(lods[1..].is_sorted_by(|finer, coarser| finer.error < coarser.error) && lods[1].error > 0.0);

			// Each level's meshlets index a prefix of the vertices that shrinks toward coarser levels, and each level's
			// position stream holds what it adds to that prefix.
			assert_eq!(lods[0].vertex_count as usize, vertex_count);
			assert!(lods.is_sorted_by(|finer, coarser| finer.vertex_count >= coarser.vertex_count));
			assert!(lods.last().unwrap().vertex_count < lods[0].vertex_count);
			for (level, lod) in lods.iter().enumerate() {
				let level = level as u8;
				let vertex_indices = stream_bytes(&processed, Streams::Indices(IndexStreamTypes::Vertices), level);
				assert!(
					vertex_indices
						.as_chunks::<2>()
						.0
						.iter()
						.all(|index| u32::from(u16::from_le_bytes(*index)) < lod.vertex_count)
				);
				assert_eq!(
					stream_bytes(&processed, Streams::Indices(IndexStreamTypes::Meshlets), level).len(),
					triangles[level as usize] * 3
				);
				let added = lod.vertex_count - lods.get(level as usize + 1).map_or(0, |coarser| coarser.vertex_count);
				assert_eq!(
					stream_bytes(&processed, Streams::Vertices(VertexSemantics::Position), level).len(),
					added as usize * 12
				);
			}
			// The plain triangle list holds full detail, at level 0, for renderers that draw without meshlets.
			assert_eq!(
				stream_bytes(&processed, Streams::Indices(IndexStreamTypes::Triangles), 0).len(),
				index_count * size_of::<u16>()
			);
		}
	}

	#[test]
	fn a_level_with_more_meshlets_than_a_level_holds_fails_to_bake() {
		// Triangles over scattered vertices share few of them, so meshlets fill on vertices at about 62 triangles each.
		// 400,000 such triangles need over 6,000 meshlets while staying under the vertex limit.
		let vertex_count = 1u32 << 16;
		let mut state = 0x9E37_79B9u32;
		let mut next = || {
			state ^= state << 13;
			state ^= state >> 17;
			state ^= state << 5;
			state % vertex_count
		};
		let positions = (0..vertex_count)
			.map(|vertex| [(vertex % 256) as f32, (vertex / 256) as f32, ((vertex * 7) % 13) as f32])
			.collect();
		let indices = (0..400_000).flat_map(|_| [next(), next(), next()]).collect();
		let scattered = TestPrimitive {
			indices,
			positions,
			..TestPrimitive::triangle()
		};
		let settings = MeshSettings {
			lods: MeshLodSettings {
				max_levels: 1,
				..MeshLodSettings::default()
			},
			..MeshSettings::default()
		};

		let error = process_with(
			vec![VertexComponent::canonical(VertexSemantics::Position)],
			&[scattered],
			None,
			Vec::new(),
			settings,
		)
		.expect_err("a level past the meshlet limit should fail to bake");

		assert!(
			matches!(error, MeshProcessingError::TooManyLevelMeshlets { primitive: 0, level: 0, meshlets } if meshlets > 4096),
			"unexpected error: {error:?}"
		);
	}

	#[test]
	fn a_primitive_past_the_vertex_limit_fails_without_blaming_lightmap_uvs() {
		let oversized = TestPrimitive {
			positions: (0..=1u32 << 16).map(|vertex| [vertex as f32, 0.0, 0.0]).collect(),
			..TestPrimitive::triangle()
		};

		let error = process(
			vec![VertexComponent::canonical(VertexSemantics::Position)],
			&[oversized],
			None,
			Vec::new(),
		)
		.expect_err("a primitive past the vertex limit should fail to bake");

		assert_eq!(
			error,
			MeshProcessingError::TooManyVertices {
				primitive: 0,
				vertices: 65_537
			}
		);
	}

	#[test]
	fn authored_levels_follow_full_detail_and_seed_the_generated_ones() {
		// Full detail is a 32 by 32 rippled grid; the authored level samples the same surface with 4 by 4 quads and
		// appends its own vertices.
		let full = TestPrimitive::wavy_grid(32);
		let coarse = TestPrimitive::wavy_grid_spanning(4, 32.0 * 0.25);
		let full_vertices = full.positions.len() as u32;
		let full_indices = full.indices.len();
		let mut authored = full;
		authored.positions.extend(&coarse.positions);
		authored.indices.extend(coarse.indices.iter().map(|index| index + full_vertices));
		authored.level_starts = vec![full_indices];

		let processed = process(
			vec![VertexComponent::canonical(VertexSemantics::Position)],
			&[authored],
			None,
			Vec::new(),
		)
		.expect("a primitive with an authored level should bake");
		let lods = &processed.mesh.primitives[0].lods;

		// The authored level draws only its own 25 vertices, which sort ahead of full detail's.
		assert_eq!(lods[1].vertex_count, 25);
		assert_eq!(stream_bytes(&processed, Streams::Indices(IndexStreamTypes::Meshlets), 1).len(), 32 * 3);
		assert!(lods[1].error > 0.0 && lods[1].error.is_finite());
		assert!(lods.windows(2).all(|pair| pair[0].error <= pair[1].error));
		assert_eq!(lods[0].vertex_count as usize, 33 * 33 + 25);
	}

	#[test]
	fn small_skinned_and_limited_primitives_keep_fewer_levels() {
		let level_count = |processed: ProcessedMesh| processed.mesh.primitives[0].lods.len();
		let position_layout = || vec![VertexComponent::canonical(VertexSemantics::Position)];
		let limited_to = |max_levels| {
			let settings = MeshSettings {
				lods: MeshLodSettings {
					max_levels,
					..MeshLodSettings::default()
				},
				..MeshSettings::default()
			};
			let grid = [TestPrimitive::wavy_grid(32)];
			level_count(process_with(position_layout(), &grid, None, Vec::new(), settings).unwrap())
		};

		assert_eq!(
			level_count(process(position_layout(), &[TestPrimitive::triangle()], None, Vec::new()).unwrap()),
			1
		);
		assert_eq!(limited_to(1), 1);
		assert_eq!(limited_to(2), 2);

		let grid = TestPrimitive::wavy_grid(32);
		let skin = vec![valid_vertex_skin(); grid.positions.len()];
		assert_eq!(level_count(process_skinned(grid.with_skin(0, skin)).unwrap()), 1);
	}

	#[test]
	fn lightmap_uvs_follow_every_level_and_keep_each_vertex_attributes() {
		let layout = vec![
			VertexComponent::canonical(VertexSemantics::Position),
			VertexComponent::canonical(VertexSemantics::UV),
		];
		let mut grid = TestPrimitive::wavy_grid(32);
		// Each authored UV is its vertex's x and y, so a split vertex shows whether its attributes followed it.
		grid.uvs = Some(grid.positions.iter().map(|position| [position[0], position[1]]).collect());
		let triangles = grid.indices.len() / 3;
		let settings = MeshSettings {
			lightmap_uvs: Some(LightmapUvSettings::default()),
			..MeshSettings::default()
		};
		let processed = process_with(layout, &[grid], None, Vec::new(), settings).unwrap();
		let primitive = &processed.mesh.primitives[0];

		assert!(primitive.lods.len() > 1, "lightmap UVs keep coarser levels");
		assert_eq!(
			stream_bytes(&processed, Streams::Indices(IndexStreamTypes::Triangles), 0).len(),
			triangles * 3 * size_of::<u16>()
		);
		let floats = |bytes: &[u8]| {
			bytes
				.as_chunks::<4>()
				.0
				.iter()
				.map(|bytes| f32::from_le_bytes(*bytes))
				.collect::<Vec<_>>()
		};
		for level in 0..primitive.lods.len() as u8 {
			let positions = floats(stream_bytes(&processed, Streams::Vertices(VertexSemantics::Position), level));
			let uvs = floats(stream_bytes(&processed, Streams::Vertices(VertexSemantics::UV), level));
			let lightmap_uvs = floats(stream_bytes(
				&processed,
				Streams::Vertices(VertexSemantics::LightmapUV),
				level,
			));
			assert_eq!(
				lightmap_uvs.len() / 2,
				positions.len() / 3,
				"level {level} has a lightmap UV per vertex"
			);
			assert!(lightmap_uvs.iter().all(|value| (0.0..=1.0).contains(value)));
			for (position, uv) in positions.as_chunks::<3>().0.iter().zip(uvs.as_chunks::<2>().0) {
				assert_eq!([position[0], position[1]], *uv, "a split vertex keeps its authored UV");
			}
		}
	}

	#[test]
	fn skinned_primitives_and_bakes_without_lightmaps_have_no_lightmap_uvs() {
		let has_lightmap_uvs = |processed: &ProcessedMesh| {
			processed
				.mesh
				.streams
				.iter()
				.any(|stream| stream.stream_type == Streams::Vertices(VertexSemantics::LightmapUV))
		};
		let layout = vec![VertexComponent::canonical(VertexSemantics::Position)];
		let without = process(layout.clone(), &[TestPrimitive::wavy_grid(4)], None, Vec::new()).unwrap();
		assert!(!has_lightmap_uvs(&without));

		let grid = TestPrimitive::wavy_grid(4);
		let skin = vec![valid_vertex_skin(); grid.positions.len()];
		let settings = MeshSettings {
			lightmap_uvs: Some(LightmapUvSettings::default()),
			..MeshSettings::default()
		};
		let skinned = process_with(
			skinned_layout(),
			&[grid.with_skin(0, skin)],
			Some(test_skeleton(1)),
			vec![test_skin(SkinJoint::Node(0))],
			settings,
		)
		.unwrap();
		assert!(!has_lightmap_uvs(&skinned));
	}

	/// Returns the bytes of one stream at one level of a mesh that was processed from a single primitive.
	fn stream_bytes(processed: &ProcessedMesh, stream_type: Streams, level: u8) -> &[u8] {
		let stream = processed
			.mesh
			.streams
			.iter()
			.find(|stream| stream.stream_type == stream_type && stream.level == level)
			.expect("processed mesh should hold the stream");
		&processed.buffer[stream.offset..stream.offset + stream.size]
	}

	fn process(
		layout: Vec<VertexComponent>,
		primitives: &[TestPrimitive],
		skeleton: Option<ReferenceModel<SkeletonModel>>,
		skins: Vec<SkinBinding>,
	) -> Result<ProcessedMesh, MeshProcessingError> {
		process_with(layout, primitives, skeleton, skins, MeshSettings::default())
	}

	fn process_with(
		layout: Vec<VertexComponent>,
		primitives: &[TestPrimitive],
		skeleton: Option<ReferenceModel<SkeletonModel>>,
		skins: Vec<SkinBinding>,
		settings: MeshSettings,
	) -> Result<ProcessedMesh, MeshProcessingError> {
		let mut processor = MeshProcessorSession::new(layout, skeleton, skins, settings)?;
		for primitive in primitives {
			processor.push_primitive(primitive).map_err(|error| match error {
				MeshPrimitiveProcessingError::Source(never) => match never {},
				MeshPrimitiveProcessingError::Processing(error) => error,
			})?;
		}
		Ok(processor.finish(&[test_material()]))
	}

	/// Processes one primitive with the skinned layout, a one-node skeleton, and one skin bound to that node.
	fn process_skinned(primitive: TestPrimitive) -> Result<ProcessedMesh, MeshProcessingError> {
		process(
			skinned_layout(),
			&[primitive],
			Some(test_skeleton(1)),
			vec![test_skin(SkinJoint::Node(0))],
		)
	}

	fn skinned_layout() -> Vec<VertexComponent> {
		vec![
			VertexComponent::canonical(VertexSemantics::Position),
			VertexComponent::canonical(VertexSemantics::Joints),
			VertexComponent::canonical(VertexSemantics::Weights),
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
				adjusted_inverse_bind_matrix: math::AffineMatrix::identity(),
			}],
		}
	}

	fn test_material() -> ReferenceModel<VariantModel> {
		variant("materials/test.variant")
	}

	fn variant(id: &str) -> ReferenceModel<VariantModel> {
		ReferenceModel::new_serialized(
			id,
			0,
			0,
			crate::to_vec(&VariantModel {
				material: ReferenceModel::new_serialized("materials/test.material", 0, 0, Vec::new(), None),
				variables: Vec::new(),
				alpha_mode: AlphaMode::Opaque,
				double_sided: false,
			})
			.expect("variant should serialize"),
			None,
		)
	}

	struct TestPrimitive {
		indices: Vec<u32>,
		positions: Vec<[f32; 3]>,
		normals: Option<Vec<[f32; 3]>>,
		uvs: Option<Vec<[f32; 2]>>,
		vertex_skin: Option<Vec<VertexSkin>>,
		material_slot: usize,
		transform_node: Option<u32>,
		skin: Option<u32>,
		/// Where each authored coarser level starts in `indices`.
		level_starts: Vec<usize>,
	}

	impl TestPrimitive {
		fn triangle() -> Self {
			Self {
				indices: vec![0, 1, 2],
				positions: vec![[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]],
				normals: None,
				uvs: None,
				vertex_skin: None,
				material_slot: 0,
				transform_node: None,
				skin: None,
				level_starts: Vec::new(),
			}
		}

		/// Returns a rippled square surface of `side` by `side` quads, z = 0.5 sin(x) cos(y).
		///
		/// The ripples make simplification cost measurable error; a flat grid would simplify for free.
		fn wavy_grid(side: u32) -> Self {
			Self::wavy_grid_spanning(side, side as f32 * 0.25)
		}

		/// Returns the surface of [`Self::wavy_grid`] over a square `extent` wide, sampled by `side` by `side` quads.
		fn wavy_grid_spanning(side: u32, extent: f32) -> Self {
			let step = extent / side as f32;
			let mut positions = Vec::new();
			for row in 0..=side {
				for column in 0..=side {
					let (x, y) = (column as f32 * step, row as f32 * step);
					positions.push([x, y, 0.5 * x.sin() * y.cos()]);
				}
			}
			let mut indices = Vec::new();
			for row in 0..side {
				for column in 0..side {
					let corner = row * (side + 1) + column;
					let (right, above) = (corner + 1, corner + side + 1);
					indices.extend([corner, right, above, right, above + 1, above]);
				}
			}
			Self {
				indices,
				positions,
				..Self::triangle()
			}
		}

		fn with_material_slot(mut self, material_slot: usize) -> Self {
			self.material_slot = material_slot;
			self
		}

		fn with_normals(mut self) -> Self {
			self.normals = Some(vec![[0.0, 0.0, 1.0]; self.positions.len()]);
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

		fn material_slot(&self) -> usize {
			self.material_slot
		}

		fn transform_node(&self) -> Option<u32> {
			self.transform_node
		}

		fn skin(&self) -> Option<u32> {
			self.skin
		}

		fn coarser_level_starts(&self) -> &[usize] {
			&self.level_starts
		}

		fn indices(&self) -> Result<impl ExactSizeIterator<Item = Result<u32, Self::Error>> + '_, Self::Error> {
			Ok(self.indices.iter().copied().map(Ok))
		}

		fn positions(
			&self,
		) -> Result<impl ExactSizeIterator<Item = Result<math::Point<ModelSpace>, Self::Error>> + '_, Self::Error> {
			Ok(self
				.positions
				.iter()
				.copied()
				.map(|position| Ok(math::Point::from_array(position))))
		}

		fn normals(
			&self,
		) -> Result<Option<impl ExactSizeIterator<Item = Result<math::Vector<ModelSpace>, Self::Error>> + '_>, Self::Error> {
			Ok(self
				.normals
				.as_deref()
				.map(|values| values.iter().copied().map(|normal| Ok(math::Vector::from_array(normal)))))
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
