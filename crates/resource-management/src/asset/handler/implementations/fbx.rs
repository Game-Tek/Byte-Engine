mod error;
mod handler;
mod material;
mod mesh;
mod skeleton;

pub(crate) use error::*;
pub use handler::FBXAssetHandler;
pub(crate) use handler::*;
pub(crate) use material::*;
pub(crate) use mesh::*;
pub(crate) use skeleton::*;

#[cfg(test)]
mod tests {

	use std::alloc::{Allocator, Global};

	use super::{
		FBXAssetHandler, FbxAuthoredLevelNotes, FbxCulledPolygonCounts, FbxImportError, FbxMeshProcessingError, MaterialKey, RemappedCorners,
		canonical_animation_node_map, decode_fbx_texture_image, fbx_material, fbx_texture_source_path,
		finite_material_component, finite_material_product, import_fbx_animation, import_fbx_mesh_session, import_fbx_skeleton,
		import_fbx_skin_binding, load_fbx_scene, matrix_to_affine, remap_triangle_corners, resolve_fbx_texture_path,
		select_fbx_skin, skin_weights, used_material_keys,
	};
	#[cfg(debug_assertions)]
	use crate::{
		ProcessedAsset,
		asset::{ResourceTraceLevel, handler::AssetHandler, handler::BakeContext, handler::LoadErrors},
	};
	use crate::{
		ReferenceModel,
		asset::{
			ResourceId,
			handler::implementations::{bema::tests::MinimalTestShaderGenerator, gltf::generated_rgba8_png},
			manager::AssetManager,
			storage_backend::tests::TestStorageBackend as AssetTestStorageBackend,
		},
		r#async,
		bemx::{self, import::ImportedMaterial},
		processors::mesh::ProcessedMesh,
		resource::storage_backend::MemoryStorageBackend,
		resources::{
			animation::{AnimationModel, RotationCurve, TranslationCurve},
			image::Image,
			material::{MaterialModel, ValueModel, VariantModel},
			mesh::MeshModel,
			skeleton::{LocalTransform, SkeletonModel, SkeletonNode, SkinJoint},
		},
		types::{AlphaMode, Formats, Gamma, IndexStreamTypes, Streams, VertexSemantics},
	};

	const TRIANGLE_MOVE_FBX: &[u8] = include_bytes!("../../test_data/triangle_move_ascii.fbx");

	const ANIMATION_ONLY_FBX: &[u8] = include_bytes!("../../test_data/animation_only_ascii.fbx");

	const DEGENERATE_QUAD_FBX: &[u8] = include_bytes!("../../test_data/degenerate_quad_ascii.fbx");

	const MATERIAL_FACTORS_FBX: &[u8] = include_bytes!("../../test_data/material_factors_ascii.fbx");

	const TEXTURED_MATERIAL_FBX: &[u8] = include_bytes!("../../test_data/textured_material_ascii.fbx");

	const SKINNED_TRIANGLE_FBX: &[u8] = include_bytes!("../../test_data/skinned_triangle_ascii.fbx");

	const SHARED_EDGE_UV_SEAM_FBX: &[u8] = include_bytes!("../../test_data/shared_edge_uv_seam_ascii.fbx");

	const LEAF_LOD0_FBX: &[u8] = include_bytes!("../../test_data/leaf_lod0_ascii.fbx");

	const LEAF_LOD1_FBX: &[u8] = include_bytes!("../../test_data/leaf_lod1_ascii.fbx");

	const LEAF_LOD2_FBX: &[u8] = include_bytes!("../../test_data/leaf_lod2_ascii.fbx");

	/// Imports a fixture with a test variant for each material it uses, named after the FBX material or `default`, and
	/// discards diagnostic counts that are not relevant to the focused assertion.
	fn import_test_fbx_meshes<'a>(
		scene: &ufbx::Scene,
		skeleton: Option<ReferenceModel<SkeletonModel>>,
		source_to_skeleton: &[u32],
		allocator: &'a dyn Allocator,
	) -> Result<ProcessedMesh, FbxMeshProcessingError> {
		import_test_fbx_levels(scene, &[], skeleton, source_to_skeleton, allocator).map(|(processed, _)| processed)
	}

	/// Imports a fixture as [`import_test_fbx_meshes`] does, with `levels` as its authored coarser levels, and returns
	/// what the import noted about them.
	fn import_test_fbx_levels<'a>(
		scene: &ufbx::Scene,
		levels: &[(&str, &ufbx::Scene)],
		skeleton: Option<ReferenceModel<SkeletonModel>>,
		source_to_skeleton: &[u32],
		allocator: &'a dyn Allocator,
	) -> Result<(ProcessedMesh, FbxAuthoredLevelNotes), FbxMeshProcessingError> {
		let mut notes = FbxAuthoredLevelNotes::default();
		let keys = used_material_keys(scene, allocator);
		let materials = keys
			.iter()
			.map(|key| match key {
				MaterialKey::Default => test_material("default"),
				MaterialKey::Material(id) => test_material(scene.materials[*id as usize].element.name.as_ref()),
			})
			.collect::<Vec<_>>();
		let session = import_fbx_mesh_session(
			scene,
			&keys,
			skeleton,
			source_to_skeleton,
			allocator,
			&mut FbxCulledPolygonCounts::default(),
			crate::processors::mesh::MeshSettings::default(),
			levels,
			&mut notes,
		)?;
		Ok((session.finish(&materials), notes))
	}

	/// The `TestVariantAssetHandler` struct supplies a material override without invoking a platform shader compiler.
	#[cfg(debug_assertions)]
	struct TestVariantAssetHandler;

	#[cfg(debug_assertions)]
	impl AssetHandler for TestVariantAssetHandler {
		fn can_handle(&self, resource_type: &str) -> bool {
			resource_type == "variant"
		}

		async fn bake<'a>(&'a self, context: BakeContext<'a>, id: ResourceId<'a>) -> Result<(), LoadErrors> {
			context
				.store_primary(
					ProcessedAsset::new(
						id,
						VariantModel {
							material: ReferenceModel::<MaterialModel>::new_serialized(
								"materials/test.material",
								0,
								0,
								Vec::new(),
								None,
							),
							variables: Vec::new(),
							alpha_mode: AlphaMode::Opaque,
							double_sided: false,
						},
					),
					&[],
				)
				.await
		}
	}

	#[test]
	fn imports_triangulated_mesh_attributes_and_meter_scaled_bounds() {
		let scene = load_fbx_scene(TRIANGLE_MOVE_FBX, "triangle_move.fbx").expect("fixture FBX should parse");

		let processed = import_test_fbx_meshes(&scene, None, &[], &Global).expect("fixture mesh should import");

		assert!(processed.mesh.skeleton.is_none());
		assert!(processed.mesh.skins.is_empty());
		assert_eq!(processed.mesh.primitives.len(), 1);
		assert_eq!(processed.mesh.primitives[0].vertex_count, 3);
		for semantic in [VertexSemantics::Position, VertexSemantics::Normal, VertexSemantics::UV] {
			assert!(
				processed
					.mesh
					.vertex_components
					.iter()
					.any(|component| component.semantic == semantic)
			);
		}

		let bounds = processed.mesh.primitives[0].bounding_box;

		assert_eq!(bounds.min().to_array(), [0.0, 0.0, 0.0]);
		assert_close([bounds.max().x(), bounds.max().y()], [0.01, 0.01]);
		assert_eq!(bounds.max().z(), 0.0);
	}

	#[test]
	fn converts_fbx_uvs_to_top_left_texture_coordinates() {
		let scene = load_fbx_scene(TRIANGLE_MOVE_FBX, "triangle_move.fbx").expect("fixture FBX should parse");

		let processed = import_test_fbx_meshes(&scene, None, &[], &Global).expect("fixture mesh should import");
		let uvs = primitive_f32_values::<2>(&processed, 0, VertexSemantics::UV);
		assert_eq!(uvs, [[0.0, 1.0], [1.0, 0.75], [0.25, 0.0]]);
	}

	#[test]
	fn discards_degenerate_polygons_without_rejecting_valid_mesh_geometry() {
		let scene = load_fbx_scene(DEGENERATE_QUAD_FBX, "degenerate_quad.fbx").expect("fixture FBX should parse");

		let processed = import_test_fbx_meshes(&scene, None, &[], &Global)
			.expect("degenerate polygons should be discarded without rejecting valid geometry");
		assert_eq!(processed.mesh.primitives[0].vertex_count, 3);
		assert_eq!(
			processed.mesh.primitives[0]
				.stream(Streams::Indices(IndexStreamTypes::Triangles), 0)
				.expect("triangle stream should exist")
				.size,
			6
		);
	}

	#[test]
	fn corners_with_equal_vertex_data_import_as_one_vertex_and_seams_stay_split() {
		let scene = load_fbx_scene(SHARED_EDGE_UV_SEAM_FBX, "shared_edge_uv_seam.fbx").expect("fixture FBX should parse");

		let processed = import_test_fbx_meshes(&scene, None, &[], &Global).expect("fixture mesh should import");

		// Six corners over four positions: the shared edge's two corners weld, and the seam corner on vertex 2 stays
		// apart from the first face's corner there.
		assert_eq!(processed.mesh.primitives[0].vertex_count, 5);
		assert_eq!(primitive_triangle_indices(&processed, 0).len(), 6);
		let mut uvs = primitive_f32_values::<2>(&processed, 0, VertexSemantics::UV);
		uvs.sort_by(|a, b| a.partial_cmp(b).expect("UVs should be finite"));
		assert_eq!(uvs, [[0.0, 0.0], [0.0, 1.0], [0.5, 0.0], [1.0, 0.0], [1.0, 1.0]]);
	}

	#[test]
	fn authored_levels_follow_full_detail_until_a_file_with_other_materials() {
		let scene = load_fbx_scene(LEAF_LOD0_FBX, "leaf_LOD0.fbx").expect("fixture FBX should parse");
		let lod1 = load_fbx_scene(LEAF_LOD1_FBX, "leaf_LOD1.fbx").expect("fixture FBX should parse");
		let lod2 = load_fbx_scene(LEAF_LOD2_FBX, "leaf_LOD2.fbx").expect("fixture FBX should parse");

		let (processed, notes) = import_test_fbx_levels(
			&scene,
			&[("leaf_LOD1.fbx", &lod1), ("leaf_LOD2.fbx", &lod2)],
			None,
			&[],
			&Global,
		)
		.expect("fixture mesh with authored levels should import");
		let primitive = &processed.mesh.primitives[0];
		let alone = import_test_fbx_meshes(&scene, None, &[], &Global).expect("fixture mesh should import");

		// The flat quad's 4 vertices follow full detail's in the primitive, and sort first.
		assert_eq!(primitive.vertex_count, alone.mesh.primitives[0].vertex_count + 4);
		assert_eq!(primitive.lods.len(), 2, "the billboard file ends the chain, and one quad is too small to simplify");
		assert_eq!(primitive.lods[1].vertex_count, 4);
		// The raised center lies 0.2 centimeters from the flat quad.
		assert_close([primitive.lods[1].error], [0.002]);
		assert!(notes.warnings.is_empty(), "unexpected warnings: {:?}", notes.warnings);
		assert!(
			notes.info.iter().any(|note| note.contains("leaf_LOD2.fbx") && note.contains("Billboard")),
			"the chain should end at the billboard: {:?}",
			notes.info
		);
	}

	#[test]
	fn normalizes_handedness_and_mirrored_instance_winding_before_mesh_processing() {
		let fixture = std::str::from_utf8(MATERIAL_FACTORS_FBX).expect("material fixture should be UTF-8");

		let right_handed = fixture.replace(
			"P: \"FrontAxisSign\", \"int\", \"Integer\", \"\",-1",
			"P: \"FrontAxisSign\", \"int\", \"Integer\", \"\",1",
		);

		let mirrored = fixture.replace(
			"P: \"Lcl Scaling\", \"Lcl Scaling\", \"\", \"A\",1,1,1",
			"P: \"Lcl Scaling\", \"Lcl Scaling\", \"\", \"A\",-1,1,1",
		);

		assert_ne!(right_handed, fixture);
		assert_ne!(mirrored, fixture);

		let base_scene = load_fbx_scene(MATERIAL_FACTORS_FBX, "base.fbx").expect("base fixture should parse");

		let right_handed_scene =
			load_fbx_scene(right_handed.as_bytes(), "right_handed.fbx").expect("right-handed fixture should parse");

		let mirrored_scene = load_fbx_scene(mirrored.as_bytes(), "mirrored.fbx").expect("mirrored fixture should parse");

		assert!(!right_handed_scene.meshes[0].reversed_winding);

		let base_area = first_clockwise_triangle_area(&base_scene);

		let right_handed_area = first_clockwise_triangle_area(&right_handed_scene);

		let mirrored_area = first_clockwise_triangle_area(&mirrored_scene);

		assert!(base_area.abs() > f32::EPSILON);
		assert_eq!(right_handed_area.signum(), base_area.signum());
		assert_eq!(mirrored_area.signum(), base_area.signum());
	}

	#[test]
	fn maps_animation_nodes_around_target_only_helpers() {
		let node = |name: &str, parent| SkeletonNode {
			name: Some(name.into()),
			parent,
			rest_local: LocalTransform::identity(),
		};
		let source = SkeletonModel {
			nodes: vec![node("Root", None), node("Hips", Some(0))],
		};
		let target = SkeletonModel {
			nodes: vec![node("Root", None), node("ik_foot_root", Some(0)), node("Hips", Some(0))],
		};

		assert_eq!(canonical_animation_node_map(&source, &target), Ok(vec![0, 2]));
	}

	#[test]
	fn imports_named_and_indexed_animation_fragments_with_zero_based_seconds() {
		let scene = load_fbx_scene(TRIANGLE_MOVE_FBX, "triangle_move.fbx").expect("fixture FBX should parse");

		let imported_skeleton = import_fbx_skeleton(&scene).expect("fixture skeleton should import");

		let skeleton = test_skeleton(&imported_skeleton.model);

		let named = import_fbx_animation(
			&scene,
			"animations/MoveX",
			skeleton.clone(),
			&imported_skeleton.source_to_skeleton,
		)
		.expect("named take should import");

		let indexed = import_fbx_animation(
			&scene,
			"animations/0",
			skeleton.clone(),
			&imported_skeleton.source_to_skeleton,
		)
		.expect("indexed take should import");

		let default = import_fbx_animation(&scene, "animation", skeleton.clone(), &imported_skeleton.source_to_skeleton)
			.expect("default take should import");

		assert_eq!(named.name.as_deref(), Some("MoveX"));
		assert_eq!(indexed.name, named.name);
		assert_eq!(default.name, named.name);
		assert!((named.duration - 1.0).abs() < f32::EPSILON);
		// The fixture's scene time mode is 30 frames per second.
		assert_eq!(named.frame_rate, Some(30.0));

		let translation_track = named
			.tracks
			.iter()
			.find(|track| track.translation.is_some())
			.expect("animated node should have a translation track");

		let Some(TranslationCurve::Linear { times, values }) = &translation_track.translation else {
			panic!("FBX translation track has the wrong curve type. The most likely cause is a track conversion regression.");
		};

		assert_eq!(times.first().copied(), Some(0.0));
		assert_eq!(times.last().copied(), Some(1.0));
		assert!((values.last().unwrap().x() - 0.02).abs() < 1.0e-6);
		assert!(matches!(
			import_fbx_animation(&scene, "mesh", skeleton, &imported_skeleton.source_to_skeleton,),
			Err(FbxImportError::UnsupportedFragment(_))
		));
	}

	#[test]
	fn rejects_singular_bind_transforms_for_node_driven_rigid_geometry() {
		let fixture = std::str::from_utf8(TRIANGLE_MOVE_FBX).expect("animation fixture should be UTF-8");

		let zero_scale = fixture.replace(
			"P: \"Lcl Scaling\", \"Lcl Scaling\", \"\", \"A\",1,1,1",
			"P: \"Lcl Scaling\", \"Lcl Scaling\", \"\", \"A\",0,1,1",
		);

		assert_ne!(zero_scale, fixture);

		let scene =
			load_fbx_scene(zero_scale.as_bytes(), "zero_scale_animation.fbx").expect("zero-scale FBX fixture should parse");

		let imported_skeleton = import_fbx_skeleton(&scene).expect("fixture hierarchy should import");

		let skeleton = test_skeleton(&imported_skeleton.model);

		assert!(matches!(
			import_test_fbx_meshes(&scene, Some(skeleton), &imported_skeleton.source_to_skeleton, &Global,),
			Err(FbxMeshProcessingError::Source(
				FbxImportError::NonInvertibleAnimatedMeshTransform
			))
		));
	}

	#[test]
	fn imports_skinned_hierarchy_binding_weights_and_remapped_rotation_track() {
		let scene = load_fbx_scene(SKINNED_TRIANGLE_FBX, "skinned_triangle.fbx").expect("skinned fixture FBX should parse");

		let imported_skeleton = import_fbx_skeleton(&scene).expect("skinned hierarchy should import");

		let root = ufbx::find_node(&scene, "RootJoint").expect("fixture should contain RootJoint");

		let child = ufbx::find_node(&scene, "ChildJoint").expect("fixture should contain ChildJoint");

		let root_index = imported_skeleton.source_to_skeleton[root.element.typed_id as usize];

		let child_index = imported_skeleton.source_to_skeleton[child.element.typed_id as usize];

		assert!(root_index < child_index);
		assert_eq!(imported_skeleton.model.nodes[child_index as usize].parent, Some(root_index));

		let mesh_node = scene
			.nodes
			.iter()
			.find(|node| node.mesh.is_some())
			.expect("fixture should contain a mesh node");

		let mesh_node_index = imported_skeleton.source_to_skeleton[mesh_node.element.typed_id as usize];

		let skin = select_fbx_skin(mesh_node.mesh.as_ref().unwrap())
			.expect("fixture skin should be supported")
			.expect("fixture mesh should be skinned");

		let (binding, fallback_joint) = import_fbx_skin_binding(mesh_node, skin, &imported_skeleton.source_to_skeleton)
			.expect("fixture skin binding should import");

		assert_eq!(fallback_joint, None);
		assert_eq!(
			binding.entries.iter().map(|entry| entry.joint).collect::<Vec<_>>(),
			[SkinJoint::Node(root_index), SkinJoint::Node(child_index)]
		);
		assert_eq!(binding.entries.len(), 2);

		// The palette must match ufbx's evaluated clusters after expressing them in
		// the flattened vertex basis used by the imported mesh.
		let mut globals = vec![math::AffineMatrix::identity(); imported_skeleton.model.nodes.len()];

		for node in &scene.nodes {
			let mapped = imported_skeleton.source_to_skeleton[node.element.typed_id as usize] as usize;

			globals[mapped] = matrix_to_affine(&node.node_to_world).expect("fixture global matrix should be finite");
		}

		let mut palette = vec![math::AffineMatrix::identity(); binding.entries.len()];

		binding
			.write_matrix_palette(&globals, &mut palette)
			.expect("fixture palette should be complete");

		let flattened_inverse = ufbx::matrix_invert(&mesh_node.geometry_to_world);

		for (matrix, cluster) in palette.into_iter().zip(&skin.clusters) {
			let expected = ufbx::matrix_mul(&cluster.geometry_to_world, &flattened_inverse);

			assert_matrix_close(
				matrix,
				matrix_to_affine(&expected).expect("expected fixture palette matrix should be finite"),
			);
		}

		let (joints, weights) = skin_weights(skin, 1, fallback_joint).expect("mixed fixture weights should import");

		assert_eq!(&joints[..2], &[1, 0]);
		assert_close([weights[0], weights[1]], [0.75, 0.25]);

		let skeleton = test_skeleton(&imported_skeleton.model);

		let processed = import_test_fbx_meshes(&scene, Some(skeleton.clone()), &imported_skeleton.source_to_skeleton, &Global)
			.expect("skinned fixture mesh should import");

		assert_eq!(processed.mesh.skeleton.as_ref().map(|value| value.id()), Some(skeleton.id()));
		assert_eq!(processed.mesh.skins.len(), 1);
		assert_eq!(processed.mesh.primitives[0].transform_node, Some(mesh_node_index));
		assert_eq!(processed.mesh.primitives[0].skin, Some(0));

		let animation = import_fbx_animation(&scene, "animations/Bend", skeleton, &imported_skeleton.source_to_skeleton)
			.expect("skinned fixture animation should import");

		let track = animation
			.tracks
			.iter()
			.find(|track| track.node == child_index)
			.expect("child rotation should target the remapped skeleton node");

		let Some(RotationCurve::Linear { times, values }) = &track.rotation else {
			panic!("FBX child rotation should import as a linear quaternion curve");
		};

		assert_eq!(times.first().copied(), Some(0.0));
		assert_eq!(times.last().copied(), Some(1.0));
		assert_ne!(values.first(), values.last());
	}

	#[test]
	fn routes_unweighted_vertices_to_the_animated_mesh_node() {
		let fixture = std::str::from_utf8(SKINNED_TRIANGLE_FBX).expect("skinned fixture should be UTF-8");

		let without_last_weight = fixture.replace(
			"Indexes: *2 {\n            a: 1,2\n        }\n        Weights: *2 {\n            a: 0.75,1\n        }",
			"Indexes: *1 {\n            a: 1\n        }\n        Weights: *1 {\n            a: 0.75\n        }",
		);

		assert_ne!(without_last_weight, fixture);

		let scene = load_fbx_scene(without_last_weight.as_bytes(), "unweighted_triangle.fbx")
			.expect("unweighted fixture variant should parse");

		let imported_skeleton = import_fbx_skeleton(&scene).expect("fixture hierarchy should import");

		let mesh_node = scene
			.nodes
			.iter()
			.find(|node| node.mesh.is_some())
			.expect("fixture should contain a mesh node");

		let skin = select_fbx_skin(mesh_node.mesh.as_ref().unwrap())
			.expect("fixture skin should be supported")
			.expect("fixture mesh should be skinned");

		let (binding, fallback_joint) = import_fbx_skin_binding(mesh_node, skin, &imported_skeleton.source_to_skeleton)
			.expect("unweighted fixture binding should import");

		let fallback_joint = fallback_joint.expect("unweighted vertices require a mesh-node palette entry");

		let mesh_node_index = imported_skeleton.source_to_skeleton[mesh_node.element.typed_id as usize];

		assert_eq!(
			binding.entries[fallback_joint as usize].joint,
			SkinJoint::Node(mesh_node_index)
		);

		let mut globals = vec![math::AffineMatrix::identity(); imported_skeleton.model.nodes.len()];

		for node in &scene.nodes {
			let mapped = imported_skeleton.source_to_skeleton[node.element.typed_id as usize] as usize;

			globals[mapped] = matrix_to_affine(&node.node_to_world).expect("fixture global matrix should be finite");
		}

		let mut palette = vec![math::AffineMatrix::identity(); binding.entries.len()];

		binding
			.write_matrix_palette(&globals, &mut palette)
			.expect("fallback palette should be complete");

		assert_matrix_close(palette[fallback_joint as usize], math::AffineMatrix::identity());

		// Moving the mesh node after bind must move the fallback palette entry instead of freezing the vertex.
		let mut moved = globals[mesh_node_index as usize].columns();
		moved[3][0] += 1.0;
		globals[mesh_node_index as usize] = math::AffineMatrix::from_columns(moved);

		binding
			.write_matrix_palette(&globals, &mut palette)
			.expect("animated fallback palette should remain complete");

		assert!((palette[fallback_joint as usize].columns()[3][0] - 1.0).abs() < 1.0e-6);

		let (joints, weights) = skin_weights(skin, 2, Some(fallback_joint)).expect("unweighted vertex should import");

		assert_eq!(joints[0], fallback_joint);
		assert_eq!(weights, [1.0, 0.0, 0.0, 0.0]);
	}

	#[test]
	fn accepts_pure_dual_quaternion_and_rejects_blended_or_multiple_skin_deformers() {
		let fixture = std::str::from_utf8(SKINNED_TRIANGLE_FBX).expect("skinned fixture should be UTF-8");

		let dual_quaternion = fixture.replace("SkinningType: \"Linear\"", "SkinningType: \"DualQuaternion\"");

		assert_ne!(dual_quaternion, fixture);

		let scene = load_fbx_scene(dual_quaternion.as_bytes(), "dual_quaternion.fbx")
			.expect("dual-quaternion fixture variant should parse");

		let mesh = scene.meshes.first().expect("fixture should contain a mesh");

		assert!(
			select_fbx_skin(mesh)
				.expect("pure dual-quaternion skin should be supported")
				.is_some()
		);

		let blended = fixture.replace("SkinningType: \"Linear\"", "SkinningType: \"Blend\"");

		assert_ne!(blended, fixture);

		let scene = load_fbx_scene(blended.as_bytes(), "blended_dual_quaternion.fbx")
			.expect("blended dual-quaternion fixture variant should parse");

		let mesh = scene.meshes.first().expect("fixture should contain a mesh");

		assert!(matches!(
			select_fbx_skin(mesh),
			Err(FbxImportError::UnsupportedBlendedDualQuaternionSkinning)
		));

		let extra_skin = r#"    Deformer: 1300, "Deformer::ExtraSkin", "Skin" {
        Version: 101
        Link_DeformAcuracy: 50
        SkinningType: "Linear"
    }

"#;

		let multiple = fixture
			.replace(
				"    Deformer: 1301, \"Deformer::TriangleSkin\", \"Skin\" {",
				&format!("{extra_skin}    Deformer: 1301, \"Deformer::TriangleSkin\", \"Skin\" {{"),
			)
			.replace("    C: \"OO\",1301,1001", "    C: \"OO\",1300,1001\n    C: \"OO\",1301,1001");

		assert_ne!(multiple, fixture);

		let scene =
			load_fbx_scene(multiple.as_bytes(), "multiple_skins.fbx").expect("multiple-skin fixture variant should parse");

		let mesh = scene.meshes.first().expect("fixture should contain a mesh");

		assert!(matches!(select_fbx_skin(mesh), Err(FbxImportError::MultipleSkinDeformers)));
	}

	#[test]
	fn preserves_explicit_opacity_and_diffuse_textures_for_legacy_phong_materials() {
		let scene = load_fbx_scene(MATERIAL_FACTORS_FBX, "material_factors.fbx").expect("material fixture should parse");

		let phong = ufbx::find_material(&scene, "FactoredPhong").expect("Phong material should exist");

		let metal_rough = ufbx::find_material(&scene, "FactoredMetalRough").expect("PBR material should exist");

		let diffuse_texture = phong
			.pbr
			.base_color
			.texture
			.as_ref()
			.expect("Phong material should retain its diffuse texture");

		assert!(
			!phong.pbr.opacity.has_value,
			"the fixture must require the raw FBX Opacity fallback rather than ufbx's normalized PBR opacity map"
		);
		assert_eq!(
			phong
				.element
				.props
				.find_prop("Opacity")
				.expect("Phong material should retain its raw Opacity property")
				.value_vec4
				.x,
			1.0
		);

		let phong_material = fbx_material(Some(phong)).expect("Phong material should convert");

		let (base_color, metallic, roughness, emission) = material_values(&phong_material);

		assert_close(base_color, [0.2, 0.1, 0.05, 1.0]);

		assert_close([metallic, roughness], [0.0, 0.6]);

		assert_close(emission, [0.2, 0.6, 1.0]);

		assert_eq!(phong_material.alpha_mode, AlphaMode::Opaque);
		assert_eq!(phong_material.images, vec![diffuse_texture.element.typed_id]);

		let pbr_material = fbx_material(Some(metal_rough)).expect("PBR material should convert");

		let (base_color, metallic, roughness, emission) = material_values(&pbr_material);

		assert_close(base_color, [0.25, 0.5, 0.75, 0.4]);

		assert_close([metallic, roughness], [0.65, 0.35]);

		assert_close(emission, [0.05, 0.1, 0.15]);

		let processed = import_test_fbx_meshes(&scene, None, &[], &Global).expect("material-part mesh should import");

		let material_ids = processed
			.mesh
			.primitives
			.iter()
			.map(|primitive| {
				processed.mesh.materials[primitive.material as usize]
					.id()
					.as_ref()
					.to_string()
			})
			.collect::<Vec<_>>();

		assert_eq!(processed.mesh.primitives.len(), 2);
		assert!(material_ids.iter().any(|id| id.ends_with("FactoredPhong.variant")));
		assert!(material_ids.iter().any(|id| id.ends_with("FactoredMetalRough.variant")));
	}

	#[test]
	fn resolves_windows_fbx_texture_paths_and_decodes_external_images() {
		let scene =
			load_fbx_scene(MATERIAL_FACTORS_FBX, "materials/material_factors.fbx").expect("material fixture should parse");

		let phong = ufbx::find_material(&scene, "FactoredPhong").expect("Phong material should exist");

		let texture = phong
			.pbr
			.base_color
			.texture
			.as_ref()
			.expect("Phong material should retain its diffuse texture");

		let path = fbx_texture_source_path(texture).expect("diffuse texture should retain its file-local path");

		assert_eq!(path, "textures\\factored_diffuse.png");
		assert_eq!(
			resolve_fbx_texture_path(ResourceId::new("materials/material_factors.fbx"), path)
				.expect("Windows-authored texture path should resolve"),
			"materials/textures/factored_diffuse.png"
		);

		let encoded = generated_rgba8_png(1, 1, &[255, 64, 32, 255]);

		let (pixels, width, height) =
			decode_fbx_texture_image(&encoded).expect("external PNG texture should decode into RGBA pixels");

		assert_eq!((width, height, pixels.len()), (1, 1, 4));
	}

	#[r#async::test]
	async fn a_material_fragment_bakes_that_material_without_the_mesh() {
		let bake = |id: String| async move {
			let asset_storage = AssetTestStorageBackend::new();
			asset_storage.add_file("material_factors.fbx", MATERIAL_FACTORS_FBX);
			asset_storage.add_file(
				"textures/factored_diffuse.png",
				&generated_rgba8_png(1, 1, &[255, 64, 32, 255]),
			);

			let resource_storage = MemoryStorageBackend::new();
			let mut asset_manager = AssetManager::new(asset_storage, resource_storage.clone());
			asset_manager.set_program_generator(MinimalTestShaderGenerator);
			asset_manager.add_asset_handler(FBXAssetHandler::default());

			let result = asset_manager.bake(&id).await;
			(result, resource_storage.get_resources())
		};

		// The mesh bake names the variant ID a material fragment uses.
		let (_, resources) = bake("material_factors.fbx".to_string()).await;
		let variant_id = resources
			.iter()
			.find(|resource| resource.class == "Variant" && resource.id.ends_with("FactoredPhong.variant"))
			.expect("the mesh bake should store the Phong variant")
			.id
			.clone();

		let (result, resources) = bake(variant_id.clone()).await;
		result.expect("a material fragment should bake");

		let mut classes = resources.iter().map(|resource| resource.class.as_str()).collect::<Vec<_>>();
		classes.sort();
		assert_eq!(classes, ["Image", "Material", "Shader", "Variant"]);
		assert!(resources.iter().any(|resource| resource.id == variant_id));

		let (result, _) = bake(variant_id.trim_end_matches(".variant").to_string()).await;
		assert!(result.is_err(), "a material fragment without its suffix names no resource");
	}

	#[r#async::test]
	async fn bakes_fbx_diffuse_textures_into_opaque_material_variants() {
		let asset_storage = AssetTestStorageBackend::new();

		let encoded = generated_rgba8_png(1, 1, &[255, 64, 32, 255]);

		let scene = load_fbx_scene(MATERIAL_FACTORS_FBX, "material_factors.fbx").expect("material fixture should parse");

		let image_index = ufbx::find_material(&scene, "FactoredPhong")
			.expect("Phong material should exist")
			.pbr
			.base_color
			.texture
			.as_ref()
			.expect("Phong material should retain its diffuse texture")
			.element
			.typed_id;

		asset_storage.add_file("material_factors.fbx", MATERIAL_FACTORS_FBX);

		asset_storage.add_file("textures/factored_diffuse.png", &encoded);

		let resource_storage = MemoryStorageBackend::new();

		let mut asset_manager = AssetManager::new(asset_storage, resource_storage.clone());

		asset_manager.set_program_generator(MinimalTestShaderGenerator);

		asset_manager.add_asset_handler(FBXAssetHandler::default());

		asset_manager
			.bake("material_factors.fbx")
			.await
			.expect("FBX material with a diffuse texture should bake");

		let variant_resource = resource_storage
			.get_resources()
			.into_iter()
			.find(|resource| resource.class == "Variant" && resource.id.ends_with("FactoredPhong.variant"))
			.expect("Phong material variant should be stored");

		let variant: VariantModel = crate::from_slice(&variant_resource.resource).expect("Phong variant should deserialize");

		assert_eq!(variant.alpha_mode, AlphaMode::Opaque);
		assert_eq!(variant.variables.len(), 1);
		assert_eq!(variant.variables[0].r#type, "Texture2D");
		assert_eq!(variant.variables[0].name, crate::bemx::material_texture_parameter_name(0));

		let ValueModel::Image(image) = &variant.variables[0].value else {
			panic!("Phong diffuse texture should become an image variable");
		};

		assert_eq!(image.id().as_ref(), format!("material_factors.fbx#images/{image_index}"));

		let image_resource = resource_storage
			.get_resource(ResourceId::new(image.id().as_ref()))
			.expect("diffuse image should be stored with the generated material");

		let image: Image = crate::from_slice(&image_resource.resource).expect("diffuse image should deserialize");

		assert_eq!(image_resource.class, "Image");
		assert_eq!(image.extent, [1, 1, 0]);
	}

	#[test]
	fn pbr_texture_maps_feed_their_surface_inputs() {
		let scene = load_fbx_scene(TEXTURED_MATERIAL_FBX, "textured_material.fbx").expect("textured fixture should parse");
		let source = ufbx::find_material(&scene, "TexturedMetalRough").expect("textured material should exist");
		let texture = |name: &str| {
			scene
				.textures
				.iter()
				.find(|texture| texture.element.name.as_ref() == name)
				.unwrap_or_else(|| panic!("texture '{name}' should exist"))
				.element
				.typed_id
		};

		let material = fbx_material(Some(source)).expect("textured material should convert");
		let graph = &material.graph;
		let surface = graph.surface();
		// Returns the container image an image node samples.
		let image = |node: bemx::NodeId| match graph.node(node) {
			bemx::Node::Image { texture, .. } => material.image(texture),
			other => panic!("Expected an image node, got {other:#?}"),
		};
		// Returns the constant and the image read of a `factor * texture.x` product.
		let factor_times_red = |node: bemx::NodeId| {
			let bemx::Node::Binary {
				operator: bemx::BinaryOperator::Multiply,
				left,
				right,
			} = graph.node(node)
			else {
				panic!("Expected a factor product, got {:#?}", graph.node(node));
			};
			let bemx::Node::Extract { value, index: 0 } = graph.node(*right) else {
				panic!("Expected a red channel read, got {:#?}", graph.node(*right));
			};

			(graph.node(*left).clone(), image(*value))
		};

		assert_eq!(material.alpha_mode, AlphaMode::Mask(0.5));

		let bemx::Node::NormalMap { value, .. } = graph.node(surface.normal.expect("normal map should feed the surface"))
		else {
			panic!("A normal map should become a normal map node.");
		};
		assert_eq!(image(*value), Some(texture("Normal")));

		assert_eq!(
			factor_times_red(surface.roughness.unwrap()),
			(bemx::Node::Constant(bemx::Value::Float(0.8)), Some(texture("Rough")))
		);

		let (_, cutout) = factor_times_red(surface.opacity.unwrap());
		assert_eq!(cutout, Some(texture("Cutout")));

		assert_eq!(constant_value(graph, surface.metalness), bemx::Value::Float(0.25));
		assert!(surface.occlusion.is_none());
	}

	#[r#async::test]
	async fn fbx_textures_bake_with_the_semantic_of_the_input_that_samples_them() {
		let asset_storage = AssetTestStorageBackend::new();
		let pixels = [128u8; 4 * 4 * 4];
		let png = generated_rgba8_png(4, 4, &pixels);

		asset_storage.add_file("textured_material.fbx", TEXTURED_MATERIAL_FBX);
		for path in [
			"textures/base.png",
			"textures/normal.png",
			"textures/rough.png",
			"textures/cutout.png",
		] {
			asset_storage.add_file(path, &png);
		}

		let resource_storage = MemoryStorageBackend::new();
		let mut asset_manager = AssetManager::new(asset_storage, resource_storage.clone());
		asset_manager.set_program_generator(MinimalTestShaderGenerator);
		asset_manager.add_asset_handler(FBXAssetHandler::default());

		asset_manager
			.bake("textured_material.fbx")
			.await
			.expect("FBX with PBR texture maps should bake");

		let resources = resource_storage.get_resources();
		let variant = resources
			.iter()
			.find(|resource| resource.class == "Variant")
			.expect("the generated variant should be stored");
		let variant: VariantModel = crate::from_slice(&variant.resource).expect("variant should deserialize");

		assert_eq!(variant.alpha_mode, AlphaMode::Mask(0.5));
		assert_eq!(variant.variables.len(), 4, "every texture map should bind an image");

		let scene = load_fbx_scene(TEXTURED_MATERIAL_FBX, "textured_material.fbx").expect("textured fixture should parse");
		let image = |name: &str| -> Image {
			let texture = scene
				.textures
				.iter()
				.find(|texture| texture.element.name.as_ref() == name)
				.unwrap_or_else(|| panic!("texture '{name}' should exist"));
			let id = format!("textured_material.fbx#images/{}", texture.element.typed_id);
			let resource = resource_storage
				.get_resource(ResourceId::new(&id))
				.unwrap_or_else(|| panic!("image '{id}' should bake with the material"));

			crate::from_slice(&resource.resource).expect("image should deserialize")
		};

		let base = image("Base");
		let normal = image("Normal");
		let rough = image("Rough");
		let cutout = image("Cutout");

		assert_eq!((base.format, base.gamma), (Formats::BC7SRGB, Gamma::SRGB));
		assert_eq!((normal.format, normal.gamma), (Formats::BC5, Gamma::Linear));
		assert_eq!((rough.format, rough.gamma), (Formats::BC7, Gamma::Linear));
		assert_eq!((cutout.format, cutout.gamma), (Formats::BC7, Gamma::Linear));
	}

	#[test]
	fn reusable_corner_remap_restores_scratch_and_rejects_invalid_indices() {
		let mut remap = vec![u32::MAX; 4];

		let mut remapped = RemappedCorners::with_capacity_in(6, &Global);

		remap_triangle_corners(4, &[0, 1, 2, 2, 1, 3], &mut remap, &mut remapped).expect("valid triangles should remap");

		assert_eq!(
			remapped.primitives().collect::<Vec<_>>(),
			[(&[0, 1, 2, 3][..], &[0, 1, 2, 2, 1, 3][..])]
		);
		assert!(remap.iter().all(|&slot| slot == u32::MAX));
		assert!(matches!(
			remap_triangle_corners(4, &[0, 1, 4], &mut remap, &mut remapped),
			Err(FbxImportError::InvalidCornerIndex)
		));
	}

	#[test]
	fn material_numeric_conversion_replaces_non_finite_and_overflowing_values() {
		assert_eq!(finite_material_component(f64::MAX, 0.25), 0.25);
		assert_eq!(finite_material_component(f64::NAN, 0.5), 0.5);
		assert_eq!(finite_material_product(f32::MAX, f32::MAX, 0.0), 0.0);
	}

	#[r#async::test]
	async fn asset_manager_bakes_animation_fragment_without_a_shader_generator() {
		let asset_storage = AssetTestStorageBackend::new();

		asset_storage.add_file("triangle_move.fbx", TRIANGLE_MOVE_FBX);

		let resource_storage = MemoryStorageBackend::new();

		let mut asset_manager = AssetManager::new(asset_storage, resource_storage);

		asset_manager.add_asset_handler(FBXAssetHandler::default());

		let animation: ReferenceModel<AnimationModel> = asset_manager
			.bake_if_not_exists("triangle_move.fbx#animations/MoveX")
			.await
			.expect("FBX animation fragment should bake");

		assert_eq!(animation.class(), "Animation");
		assert_eq!(animation.id().as_ref(), "triangle_move.fbx#animations/MoveX");
	}

	#[cfg(debug_assertions)]
	#[r#async::test]
	async fn asset_manager_associates_culled_geometry_info_with_the_baked_fbx_resource() {
		let asset_storage = AssetTestStorageBackend::new();

		asset_storage.add_file("degenerate_quad.fbx", DEGENERATE_QUAD_FBX);

		asset_storage.add_file(
			"degenerate_quad.fbx.bead",
			br#"{ "asset": { "default": { "asset": "materials/test.variant" } } }"#,
		);

		let resource_storage = MemoryStorageBackend::new();

		let mut asset_manager = AssetManager::new(asset_storage, resource_storage.clone());

		asset_manager.add_asset_handler(TestVariantAssetHandler);

		asset_manager.add_asset_handler(FBXAssetHandler::default());

		let result = asset_manager.bake("degenerate_quad.fbx").await;

		assert!(
			result.is_ok(),
			"valid geometry should remain after the degenerate quad is culled: {result:?}; trace: {:?}",
			asset_manager.resource_trace().items("degenerate_quad.fbx")
		);

		let items = asset_manager.resource_trace().items("degenerate_quad.fbx");

		assert_eq!(items.len(), 1);
		assert_eq!(items[0].level(), ResourceTraceLevel::Info);
		assert_eq!(
			items[0].message(),
			"Culled degenerate FBX geometry: 0 triangle(s), 1 quad(s), and 0 other polygon(s). The most likely cause is repeated or collinear vertex positions, which produce zero-area triangles and undefined normal data."
		);
		assert!(
			resource_storage
				.get_resource(ResourceId::new("degenerate_quad.fbx"))
				.is_some()
		);
	}

	#[cfg(debug_assertions)]
	#[r#async::test]
	async fn malformed_fbx_keeps_its_handler_error_without_creating_a_resource() {
		let asset_storage = AssetTestStorageBackend::new();

		asset_storage.add_file("broken.fbx", b"not an FBX");

		let resource_storage = MemoryStorageBackend::new();

		let mut asset_manager = AssetManager::new(asset_storage, resource_storage.clone());

		asset_manager.add_asset_handler(FBXAssetHandler::default());

		assert!(asset_manager.bake("broken.fbx").await.is_err());
		assert!(resource_storage.get_resource(ResourceId::new("broken.fbx")).is_none());

		let items = asset_manager.resource_trace().items("broken.fbx");

		assert_eq!(items.len(), 1);
		assert_eq!(items[0].level(), ResourceTraceLevel::Error);
		assert!(items[0].message().starts_with("Failed to import FBX asset 'broken.fbx':"));
	}

	#[r#async::test]
	async fn asset_manager_bakes_unfragmented_animation_only_fbx_as_animation() {
		let asset_storage = AssetTestStorageBackend::new();

		asset_storage.add_file("animation_only.fbx", ANIMATION_ONLY_FBX);

		let resource_storage = MemoryStorageBackend::new();

		let mut asset_manager = AssetManager::new(asset_storage, resource_storage.clone());

		asset_manager.add_asset_handler(FBXAssetHandler::default());

		asset_manager
			.bake("animation_only.fbx")
			.await
			.expect("an unfragmented animation-only FBX should bake as Animation");

		let animation = resource_storage
			.get_resource(crate::asset::ResourceId::new("animation_only.fbx"))
			.expect("the bare FBX Animation resource should be stored");

		assert_eq!(animation.class, "Animation");

		let animation = crate::from_slice::<AnimationModel>(&animation.resource).unwrap();

		assert_eq!(animation.skeleton.id().as_ref(), "animation_only.fbx#skeleton");
	}

	#[r#async::test]
	async fn bead_can_make_a_single_clip_fbx_with_geometry_default_to_animation() {
		let asset_storage = AssetTestStorageBackend::new();

		asset_storage.add_file("triangle_move.fbx", TRIANGLE_MOVE_FBX);

		asset_storage.add_file(
			"triangle_move.fbx.bead",
			br#"{
				// Animation sidecars use JSON5 and may select a canonical skeleton dependency.
				default_resource: 'animation',
				skeleton: 'triangle_move.fbx#skeleton',
			}"#,
		);

		let resource_storage = MemoryStorageBackend::new();

		let mut asset_manager = AssetManager::new(asset_storage, resource_storage);

		asset_manager.add_asset_handler(FBXAssetHandler::default());

		let animation: ReferenceModel<AnimationModel> = asset_manager
			.bake_if_not_exists("triangle_move.fbx")
			.await
			.expect("the BEAD default should override mesh-first FBX dispatch");

		assert_eq!(animation.class(), "Animation");
		let animation = crate::from_slice::<AnimationModel>(&animation.resource).expect("animation metadata should decode");
		assert_eq!(animation.skeleton.id().as_ref(), "triangle_move.fbx#skeleton");
	}

	#[r#async::test]
	async fn asset_manager_bakes_base_fbx_with_retained_skeleton_and_primitive_node() {
		let asset_storage = AssetTestStorageBackend::new();

		asset_storage.add_file("triangle_move.fbx", TRIANGLE_MOVE_FBX);

		let resource_storage = MemoryStorageBackend::new();

		let mut asset_manager = AssetManager::new(asset_storage, resource_storage);

		asset_manager.set_program_generator(MinimalTestShaderGenerator);

		asset_manager.add_asset_handler(FBXAssetHandler::default());

		let mesh: ReferenceModel<MeshModel> = asset_manager
			.bake_if_not_exists("triangle_move.fbx")
			.await
			.expect("animated FBX base mesh should bake");

		let mesh = crate::from_slice::<MeshModel>(&mesh.resource).expect("animated FBX mesh should deserialize");

		assert_eq!(
			mesh.skeleton
				.as_ref()
				.expect("animated FBX mesh should retain its skeleton")
				.id()
				.as_ref(),
			"triangle_move.fbx#skeleton"
		);
		assert!(mesh.skins.is_empty());
		assert_eq!(mesh.primitives.len(), 1);
		assert!(mesh.primitives[0].transform_node.is_some());
		assert_eq!(mesh.primitives[0].skin, None);
	}

	/// Creates a serialized reference for fixture-local skeleton and animation imports.
	fn test_skeleton(model: &SkeletonModel) -> ReferenceModel<SkeletonModel> {
		ReferenceModel::new_serialized(
			"fixtures/model.fbx#skeleton",
			0,
			0,
			crate::to_vec(model).expect("fixture skeleton should serialize"),
			None,
		)
	}

	fn test_material(name: &str) -> ReferenceModel<VariantModel> {
		ReferenceModel::new_serialized(
			&format!("materials/{name}.variant"),
			0,
			0,
			crate::to_vec(&VariantModel {
				material: ReferenceModel::<MaterialModel>::new_serialized("materials/test.material", 0, 0, Vec::new(), None),
				variables: Vec::new(),
				alpha_mode: AlphaMode::Opaque,
				double_sided: false,
			})
			.expect("test material should serialize"),
			None,
		)
	}

	/// Computes the first triangle's signed XY area after applying MeshProcessorSession's clockwise index convention.
	fn first_clockwise_triangle_area(scene: &ufbx::Scene) -> f32 {
		let processed = import_test_fbx_meshes(scene, None, &[], &Global).expect("fixture mesh should import");
		let positions = primitive_f32_values::<3>(&processed, 0, VertexSemantics::Position);
		let indices = primitive_triangle_indices(&processed, 0);
		let first = positions[indices[0] as usize];
		let second = positions[indices[1] as usize];
		let third = positions[indices[2] as usize];

		(second[0] - first[0]) * (third[1] - first[1]) - (second[1] - first[1]) * (third[0] - first[0])
	}

	/// Returns one primitive's bytes of `stream_type` from the processed aggregate buffer, joining its detail levels
	/// coarsest first, which is the order of the primitive's vertices.
	fn primitive_stream(processed: &ProcessedMesh, primitive_index: usize, stream_type: Streams) -> Vec<u8> {
		let primitive = &processed.mesh.primitives[primitive_index];
		let bytes = (0..primitive.lods.len() as u8)
			.rev()
			.filter_map(|level| {
				let range = primitive.stream(stream_type, level)?;
				let aggregate = processed
					.mesh
					.streams
					.iter()
					.find(|stream| stream.stream_type == stream_type && stream.level == level)
					.expect("aggregate stream should exist");
				let begin = aggregate.offset + range.offset;
				Some(&processed.buffer[begin..begin + range.size])
			})
			.collect::<Vec<_>>();
		assert!(!bytes.is_empty(), "primitive stream should exist");
		bytes.concat()
	}

	/// Decodes one primitive's packed floating-point stream from the processed aggregate buffer.
	fn primitive_f32_values<const N: usize>(
		processed: &ProcessedMesh,
		primitive_index: usize,
		semantic: VertexSemantics,
	) -> Vec<[f32; N]> {
		primitive_stream(processed, primitive_index, Streams::Vertices(semantic))
			.chunks_exact(N * 4)
			.map(|bytes| std::array::from_fn(|component| f32::from_le_bytes(bytes.as_chunks::<4>().0[component])))
			.collect()
	}

	/// Decodes one primitive's optimized u16 triangle stream from the processed aggregate buffer.
	fn primitive_triangle_indices(processed: &ProcessedMesh, primitive_index: usize) -> Vec<u16> {
		primitive_stream(processed, primitive_index, Streams::Indices(IndexStreamTypes::Triangles))
			.as_chunks::<2>()
			.0
			.iter()
			.map(|bytes| u16::from_le_bytes(*bytes))
			.collect()
	}

	/// Extracts the constant metallic-roughness values produced by the focused material fixtures.
	fn material_values(material: &ImportedMaterial) -> ([f32; 4], f32, f32, [f32; 3]) {
		let graph = &material.graph;
		let surface = graph.surface();

		let Some(bemx::Node::Swizzle { value: base_color, .. }) = surface.base_color.map(|node| graph.node(node)) else {
			panic!("FBX base color should be the colour channels of the factor product");
		};

		let bemx::Value::Float(metallic) = constant_value(graph, surface.metalness) else {
			panic!("FBX metalness should be a scalar constant");
		};

		let bemx::Value::Float(roughness) = constant_value(graph, surface.roughness) else {
			panic!("FBX roughness should be a scalar constant");
		};

		let bemx::Value::Vector3(emission) = constant_value(graph, surface.emission) else {
			panic!("FBX emission should be a vector3 constant");
		};

		(base_color_factor(graph, *base_color), metallic, roughness, emission)
	}

	/// Extracts the constant color factor from a base-color graph with an optional texture sample.
	fn base_color_factor(graph: &bemx::Graph, node: bemx::NodeId) -> [f32; 4] {
		match graph.node(node) {
			bemx::Node::Constant(bemx::Value::Vector4(value)) => *value,
			bemx::Node::Binary { left, right, .. } => [left, right]
				.into_iter()
				.find_map(|node| match graph.node(*node) {
					bemx::Node::Constant(bemx::Value::Vector4(value)) => Some(*value),
					_ => None,
				})
				.expect("textured FBX base color should retain its vector4 factor"),
			_ => panic!("FBX base color should be a vector4 constant or texture-factor product"),
		}
	}

	/// Reads one constant surface input from a fixture-generated graph.
	fn constant_value(graph: &bemx::Graph, node: Option<bemx::NodeId>) -> bemx::Value {
		match node.map(|node| graph.node(node)) {
			Some(bemx::Node::Constant(value)) => *value,
			_ => panic!("fixture surface input should be constant"),
		}
	}

	fn assert_close<const N: usize>(actual: [f32; N], expected: [f32; N]) {
		for index in 0..N {
			assert!(
				(actual[index] - expected[index]).abs() < 1.0e-6,
				"component {index} differs: actual {actual:?}, expected {expected:?}"
			);
		}
	}

	fn assert_matrix_close(actual: math::AffineMatrix, expected: math::AffineMatrix) {
		for column in 0..4 {
			assert_close(actual.columns()[column], expected.columns()[column]);
		}
	}
}

use std::{alloc::Allocator, collections::HashSet, fmt};

use math::{AffineMatrix, Orientation, Point, Scale, UnitVector, Vector};
use serde_json::Value;
use utils::Extent;

use super::{
	ANIMATION_FRAGMENT_PREFIX, ContainerDefaultResource, DEFAULT_ANIMATION_FRAGMENT, MATERIAL_FRAGMENT_PREFIX, ResourceId,
	SKELETON_FRAGMENT, commit_mesh, generated_skeleton_id,
	handler::{AssetHandler, BakeContext, LoadErrors},
	mesh_settings, missing_generated_material, names_generated_material, sanitize_material_name, select_unfragmented_resource,
	store_imported_image, store_model,
};
use crate::asset::handler::implementations::bema::{
	GeneratedMaterial, MaterialSource, TemplateScope, binds_variant_asset, material_binding, resolve_container_materials,
};
use crate::{
	ProcessedAsset, ReferenceModel, asset,
	bemx::{
		self,
		import::{ImportedGraphBuilder, ImportedMaterial},
	},
	processors::{
		image::{ImageSource, Semantic, SourceChannels, SourceEncoding},
		mesh::{MeshPrimitiveProcessingError, MeshPrimitiveSource, MeshProcessorSession, MeshSettings, VertexSkin},
	},
	resources::{
		ModelSpace,
		animation::{AnimationModel, Curve, NodeTrack},
		material::VariantModel,
		skeleton::{LocalTransform, SkeletonModel, SkeletonNode, SkinBinding, SkinJoint, SkinPaletteEntry},
	},
	types::{AlphaMode, VertexComponent, VertexSemantics},
};
