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

	use std::{
		alloc::{Allocator, Global},
		collections::HashMap,
	};

	use super::{
		FBXAssetHandler, FbxCulledPolygonCounts, FbxImportError, FbxMeshProcessingError, MaterialKey, ResolvedFbxMaterials,
		canonical_animation_node_map, decode_fbx_texture_image, fbx_brdf_material, fbx_texture_source_path,
		finite_material_component, finite_material_product, import_fbx_animation, import_fbx_meshes, import_fbx_skeleton,
		import_fbx_skin_binding, load_fbx_scene, matrix_to_columns, remap_triangle_corners, resolve_fbx_texture_path,
		select_fbx_skin, select_unfragmented_fbx_resource, skin_weights,
	};
	#[cfg(debug_assertions)]
	use crate::{
		ProcessedAsset,
		asset::{ResourceTraceLevel, handler::BakeContext, handler::LoadErrors},
	};
	use crate::{
		ReferenceModel,
		asset::{
			ContainerDefaultResource, ResourceId, handler::AssetHandler,
			handler::implementations::bema::tests::MinimalTestShaderGenerator, manager::AssetManager,
			storage_backend::tests::TestStorageBackend as AssetTestStorageBackend,
		},
		r#async,
		pbr::{BrdfAlphaMode, BrdfMaterialDescription, BrdfNode, BrdfValue},
		processors::processor::implementations::mesh::{MeshProcessor, ProcessedMesh, TriangleFrontFaceWinding},
		resource::storage_backend::tests::TestStorageBackend as ResourceTestStorageBackend,
		resources::{
			animation::{AnimationModel, QuaternionCurve, Vector3Curve},
			image::Image,
			material::{MaterialModel, ValueModel, VariantModel},
			mesh::MeshModel,
			skeleton::{LocalTransform, SkeletonModel, SkeletonNode, SkinJoint},
		},
		types::{AlphaMode, IndexStreamTypes, VertexSemantics},
	};

	const TRIANGLE_MOVE_FBX: &[u8] = include_bytes!("../../test_data/triangle_move_ascii.fbx");

	const ANIMATION_ONLY_FBX: &[u8] = include_bytes!("../../test_data/animation_only_ascii.fbx");

	const DEGENERATE_QUAD_FBX: &[u8] = include_bytes!("../../test_data/degenerate_quad_ascii.fbx");

	const MATERIAL_FACTORS_FBX: &[u8] = include_bytes!("../../test_data/material_factors_ascii.fbx");

	const SKINNED_TRIANGLE_FBX: &[u8] = include_bytes!("../../test_data/skinned_triangle_ascii.fbx");

	/// Encodes one RGBA texel for focused external-image decoding coverage.
	fn one_pixel_rgba_png() -> Vec<u8> {
		let mut png = Vec::new();

		{
			let mut encoder = png::Encoder::new(&mut png, 1, 1);

			encoder.set_color(png::ColorType::Rgba);

			encoder.set_depth(png::BitDepth::Eight);

			let mut writer = encoder.write_header().expect("one-pixel PNG header should encode");

			writer
				.write_image_data(&[255, 64, 32, 255])
				.expect("one-pixel PNG data should encode");
		}

		png
	}

	/// Imports a fixture while discarding diagnostic counts that are not relevant to the focused assertion.
	fn import_test_fbx_meshes<'a>(
		scene: &ufbx::Scene,
		materials: &ResolvedFbxMaterials,
		skeleton: Option<ReferenceModel<SkeletonModel>>,
		source_to_skeleton: &[u32],
		allocator: &'a dyn Allocator,
	) -> Result<ProcessedMesh, FbxMeshProcessingError> {
		let mut culled_polygons = FbxCulledPolygonCounts::default();

		import_fbx_meshes(
			scene,
			materials,
			skeleton,
			source_to_skeleton,
			MeshProcessor::new(),
			allocator,
			&mut culled_polygons,
		)
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
						},
					),
					&[],
				)
				.await
		}
	}

	#[test]
	fn recognizes_fbx_and_exposes_consistent_default_winding() {
		let handler = FBXAssetHandler::new();

		assert!(handler.can_handle("fbx"));
		assert!(handler.can_handle("FBX"));
		assert!(!handler.can_handle("glb"));
		assert_eq!(handler.triangle_front_face_winding(), TriangleFrontFaceWinding::Clockwise);
	}

	#[test]
	fn unfragmented_fbx_with_geometry_remains_mesh_first() {
		let scene = load_fbx_scene(TRIANGLE_MOVE_FBX, "triangle_move.fbx").unwrap();

		assert_eq!(
			select_unfragmented_fbx_resource(&scene, None),
			Ok(ContainerDefaultResource::Mesh)
		);
	}

	#[test]
	fn imports_triangulated_mesh_attributes_and_meter_scaled_bounds() {
		let scene = load_fbx_scene(TRIANGLE_MOVE_FBX, "triangle_move.fbx").expect("fixture FBX should parse");

		let materials = ResolvedFbxMaterials {
			materials: HashMap::from([(MaterialKey::Default, test_material("default"))]),
		};

		let processed = import_test_fbx_meshes(&scene, &materials, None, &[], &Global).expect("fixture mesh should import");

		assert!(processed.mesh.skeleton.is_none());
		assert!(processed.mesh.skins.is_empty());
		assert_eq!(processed.mesh.primitives.len(), 1);
		assert_eq!(processed.mesh.primitives[0].vertex_count, 3);
		assert!(
			processed
				.mesh
				.vertex_components
				.iter()
				.any(|component| component.semantic == VertexSemantics::Position)
		);
		assert!(
			processed
				.mesh
				.vertex_components
				.iter()
				.any(|component| component.semantic == VertexSemantics::Normal)
		);
		assert!(
			processed
				.mesh
				.vertex_components
				.iter()
				.any(|component| component.semantic == VertexSemantics::UV)
		);

		let bounds = processed.mesh.primitives[0].bounding_box;

		assert_eq!(bounds[0], [0.0, 0.0, 0.0]);
		assert!((bounds[1][0] - 0.01).abs() < 1.0e-6);
		assert!((bounds[1][1] - 0.01).abs() < 1.0e-6);
		assert_eq!(bounds[1][2], 0.0);
	}

	#[test]
	fn converts_fbx_uvs_to_top_left_texture_coordinates() {
		let scene = load_fbx_scene(TRIANGLE_MOVE_FBX, "triangle_move.fbx").expect("fixture FBX should parse");

		let materials = ResolvedFbxMaterials {
			materials: HashMap::from([(MaterialKey::Default, test_material("default"))]),
		};

		let processed = import_test_fbx_meshes(&scene, &materials, None, &[], &Global).expect("fixture mesh should import");
		let uvs = primitive_f32_values::<2>(&processed, 0, VertexSemantics::UV);
		assert_eq!(uvs, [[0.0, 1.0], [1.0, 0.75], [0.25, 0.0]]);
	}

	#[test]
	fn discards_degenerate_polygons_without_rejecting_valid_mesh_geometry() {
		let scene = load_fbx_scene(DEGENERATE_QUAD_FBX, "degenerate_quad.fbx").expect("fixture FBX should parse");

		let materials = ResolvedFbxMaterials {
			materials: HashMap::from([(MaterialKey::Default, test_material("default"))]),
		};

		let processed = import_test_fbx_meshes(&scene, &materials, None, &[], &Global)
			.expect("degenerate polygons should be discarded without rejecting valid geometry");
		assert_eq!(processed.mesh.primitives[0].vertex_count, 3);
		assert_eq!(
			processed.mesh.primitives[0]
				.streams
				.iter()
				.find(|stream| {
					stream.stream_type == crate::types::Streams::Indices(crate::types::IndexStreamTypes::Triangles)
				})
				.expect("triangle stream should exist")
				.size,
			6
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
		let source = SkeletonModel {
			nodes: vec![
				SkeletonNode {
					name: Some("Root".into()),
					parent: None,
					rest_local: LocalTransform::identity(),
				},
				SkeletonNode {
					name: Some("Hips".into()),
					parent: Some(0),
					rest_local: LocalTransform::identity(),
				},
			],
		};
		let target = SkeletonModel {
			nodes: vec![
				SkeletonNode {
					name: Some("Root".into()),
					parent: None,
					rest_local: LocalTransform::identity(),
				},
				SkeletonNode {
					name: Some("ik_foot_root".into()),
					parent: Some(0),
					rest_local: LocalTransform::identity(),
				},
				SkeletonNode {
					name: Some("Hips".into()),
					parent: Some(0),
					rest_local: LocalTransform::identity(),
				},
			],
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

		let translation_track = named
			.tracks
			.iter()
			.find(|track| track.translation.is_some())
			.expect("animated node should have a translation track");

		let Some(Vector3Curve::Linear { times, values }) = &translation_track.translation else {
			panic!("FBX translation track has the wrong curve type. The most likely cause is a track conversion regression.");
		};

		assert_eq!(times.first().copied(), Some(0.0));
		assert_eq!(times.last().copied(), Some(1.0));
		assert!((values.last().unwrap()[0] - 0.02).abs() < 1.0e-6);
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

		let materials = ResolvedFbxMaterials {
			materials: HashMap::from([(MaterialKey::Default, test_material("default"))]),
		};

		assert!(matches!(
			import_test_fbx_meshes(
				&scene,
				&materials,
				Some(skeleton),
				&imported_skeleton.source_to_skeleton,
				&Global,
			),
			Err(FbxMeshProcessingError::Import(
				FbxImportError::NonInvertibleAnimatedMeshTransform
			))
		));
	}

	#[test]
	fn imports_skinned_hierarchy_binding_weights_and_remapped_rotation_track() {
		let scene = load_fbx_scene(SKINNED_TRIANGLE_FBX, "skinned_triangle.fbx").expect("skinned fixture FBX should parse");

		let imported_skeleton = import_fbx_skeleton(&scene).expect("skinned hierarchy should import");

		let root = scene
			.nodes
			.iter()
			.find(|node| node.element.name.as_ref() == "RootJoint")
			.expect("fixture should contain RootJoint");

		let child = scene
			.nodes
			.iter()
			.find(|node| node.element.name.as_ref() == "ChildJoint")
			.expect("fixture should contain ChildJoint");

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
		assert_eq!(binding.len(), 2);

		// The palette must match ufbx's evaluated clusters after expressing them in
		// the flattened vertex basis used by the imported mesh.
		let mut globals =
			vec![crate::resources::skeleton::identity_affine_matrix4x3_columns(); imported_skeleton.model.nodes.len()];

		for node in &scene.nodes {
			let mapped = imported_skeleton.source_to_skeleton[node.element.typed_id as usize] as usize;

			globals[mapped] = matrix_to_columns(&node.node_to_world).expect("fixture global matrix should be finite");
		}

		let mut palette = vec![crate::resources::skeleton::identity_affine_matrix4x3_columns(); binding.len()];

		binding
			.write_matrix_palette(&globals, &mut palette)
			.expect("fixture palette should be complete");

		let flattened_inverse = ufbx::matrix_invert(&mesh_node.geometry_to_world);

		for (matrix, cluster) in palette.into_iter().zip(&skin.clusters) {
			let expected = ufbx::matrix_mul(&cluster.geometry_to_world, &flattened_inverse);

			assert_matrix_close(
				matrix,
				matrix_to_columns(&expected).expect("expected fixture palette matrix should be finite"),
			);
		}

		let (joints, weights) = skin_weights(skin, 1, fallback_joint).expect("mixed fixture weights should import");

		assert_eq!(&joints[..2], &[1, 0]);
		assert!((weights[0] - 0.75).abs() < 1.0e-6);
		assert!((weights[1] - 0.25).abs() < 1.0e-6);

		let skeleton = test_skeleton(&imported_skeleton.model);

		let materials = ResolvedFbxMaterials {
			materials: HashMap::from([(MaterialKey::Default, test_material("default"))]),
		};

		let processed = import_test_fbx_meshes(
			&scene,
			&materials,
			Some(skeleton.clone()),
			&imported_skeleton.source_to_skeleton,
			&Global,
		)
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

		let Some(QuaternionCurve::Linear { times, values }) = &track.rotation else {
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

		let mut globals =
			vec![crate::resources::skeleton::identity_affine_matrix4x3_columns(); imported_skeleton.model.nodes.len()];

		for node in &scene.nodes {
			let mapped = imported_skeleton.source_to_skeleton[node.element.typed_id as usize] as usize;

			globals[mapped] = matrix_to_columns(&node.node_to_world).expect("fixture global matrix should be finite");
		}

		let mut palette = vec![crate::resources::skeleton::identity_affine_matrix4x3_columns(); binding.len()];

		binding
			.write_matrix_palette(&globals, &mut palette)
			.expect("fallback palette should be complete");

		assert_matrix_close(
			palette[fallback_joint as usize],
			crate::resources::skeleton::identity_affine_matrix4x3_columns(),
		);

		// Moving the mesh node after bind must move the fallback palette entry instead of freezing the vertex.
		globals[mesh_node_index as usize][3][0] += 1.0;

		binding
			.write_matrix_palette(&globals, &mut palette)
			.expect("animated fallback palette should remain complete");

		assert!((palette[fallback_joint as usize][3][0] - 1.0).abs() < 1.0e-6);

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

		let phong_brdf = fbx_brdf_material(Some(phong));

		let (base_color, metallic, roughness, emission) = brdf_values(&phong_brdf);

		assert_vec4_close(base_color, [0.2, 0.1, 0.05, 1.0]);

		assert!((metallic - 0.0).abs() < 1.0e-6);
		assert!((roughness - 0.6).abs() < 1.0e-6);

		assert_vec3_close(emission, [0.2, 0.6, 1.0]);

		assert_eq!(phong_brdf.alpha_mode, BrdfAlphaMode::Opaque);
		assert!(phong_brdf.nodes.iter().any(|node| {
			matches!(node, BrdfNode::Texture(texture) if texture.image_index == diffuse_texture.element.typed_id)
		}));

		let pbr_brdf = fbx_brdf_material(Some(metal_rough));

		let (base_color, metallic, roughness, emission) = brdf_values(&pbr_brdf);

		assert_vec4_close(base_color, [0.25, 0.5, 0.75, 0.4]);

		assert!((metallic - 0.65).abs() < 1.0e-6);
		assert!((roughness - 0.35).abs() < 1.0e-6);

		assert_vec3_close(emission, [0.05, 0.1, 0.15]);

		let materials = fixture_materials(&scene);

		let processed =
			import_test_fbx_meshes(&scene, &materials, None, &[], &Global).expect("material-part mesh should import");

		let material_ids = processed
			.mesh
			.primitives
			.iter()
			.map(|primitive| primitive.material.id().as_ref().to_string())
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

		let encoded = one_pixel_rgba_png();

		let (pixels, width, height) =
			decode_fbx_texture_image(&encoded).expect("external PNG texture should decode into RGBA pixels");

		assert_eq!((width, height, pixels.len()), (1, 1, 4));
	}

	#[r#async::test]
	async fn bakes_fbx_diffuse_textures_into_opaque_material_variants() {
		let asset_storage = AssetTestStorageBackend::new();

		let encoded = one_pixel_rgba_png();

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

		let resource_storage = ResourceTestStorageBackend::new();

		let mut asset_manager = AssetManager::new(asset_storage, resource_storage.clone());

		let mut handler = FBXAssetHandler::new();

		handler.set_shader_generator(MinimalTestShaderGenerator);

		asset_manager.add_asset_handler(handler);

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
		assert_eq!(
			variant.variables[0].name,
			crate::pbr::material_texture_variable_name(image_index)
		);

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
	fn malformed_fbx_returns_a_parse_error() {
		assert!(matches!(
			load_fbx_scene(b"not an FBX", "broken.fbx"),
			Err(FbxImportError::Parse(_))
		));
	}

	#[test]
	fn reusable_corner_remap_restores_scratch_and_rejects_invalid_indices() {
		let mut remap = vec![u32::MAX; 4];

		let batches =
			remap_triangle_corners(4, &[0, 1, 2, 2, 1, 3], &mut remap, &Global).expect("valid triangles should remap");

		assert_eq!(batches.len(), 1);
		assert_eq!(batches[0].source_corners, vec![0, 1, 2, 3]);
		assert_eq!(batches[0].indices, vec![0, 1, 2, 2, 1, 3]);
		assert!(remap.iter().all(|&slot| slot == u32::MAX));
		assert!(matches!(
			remap_triangle_corners(4, &[0, 1, 4], &mut remap, &Global),
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

		let resource_storage = ResourceTestStorageBackend::new();

		let mut asset_manager = AssetManager::new(asset_storage, resource_storage);

		asset_manager.add_asset_handler(FBXAssetHandler::new());

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

		let resource_storage = ResourceTestStorageBackend::new();

		let mut asset_manager = AssetManager::new(asset_storage, resource_storage.clone());

		asset_manager.add_asset_handler(TestVariantAssetHandler);

		asset_manager.add_asset_handler(FBXAssetHandler::new());

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

		let resource_storage = ResourceTestStorageBackend::new();

		let mut asset_manager = AssetManager::new(asset_storage, resource_storage.clone());

		asset_manager.add_asset_handler(FBXAssetHandler::new());

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

		let resource_storage = ResourceTestStorageBackend::new();

		let mut asset_manager = AssetManager::new(asset_storage, resource_storage.clone());

		asset_manager.add_asset_handler(FBXAssetHandler::new());

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

		let resource_storage = ResourceTestStorageBackend::new();

		let mut asset_manager = AssetManager::new(asset_storage, resource_storage);

		asset_manager.add_asset_handler(FBXAssetHandler::new());

		let animation: ReferenceModel<AnimationModel> = asset_manager
			.bake_if_not_exists("triangle_move.fbx")
			.await
			.expect("the BEAD default should override mesh-first FBX dispatch");

		assert_eq!(animation.class(), "Animation");
		let animation = crate::from_slice::<AnimationModel>(&animation.resource).expect("animation metadata should decode");
		assert_eq!(animation.skeleton.id().as_ref(), "triangle_move.fbx#skeleton");
	}

	#[r#async::test]
	async fn asset_manager_bakes_explicit_fbx_skeleton_fragment_without_material_work() {
		let asset_storage = AssetTestStorageBackend::new();

		asset_storage.add_file("skinned_triangle.fbx", SKINNED_TRIANGLE_FBX);

		let resource_storage = ResourceTestStorageBackend::new();

		let mut asset_manager = AssetManager::new(asset_storage, resource_storage);

		asset_manager.add_asset_handler(FBXAssetHandler::new());

		let skeleton: ReferenceModel<SkeletonModel> = asset_manager
			.bake_if_not_exists("skinned_triangle.fbx#skeleton")
			.await
			.expect("FBX skeleton fragment should bake without a shader generator");

		assert_eq!(skeleton.class(), "Skeleton");
		assert_eq!(skeleton.id().as_ref(), "skinned_triangle.fbx#skeleton");
	}

	#[r#async::test]
	async fn asset_manager_bakes_base_fbx_with_retained_skeleton_and_primitive_node() {
		let asset_storage = AssetTestStorageBackend::new();

		asset_storage.add_file("triangle_move.fbx", TRIANGLE_MOVE_FBX);

		let resource_storage = ResourceTestStorageBackend::new();

		let mut asset_manager = AssetManager::new(asset_storage, resource_storage);

		let mut handler = FBXAssetHandler::new();

		handler.set_shader_generator(MinimalTestShaderGenerator);

		asset_manager.add_asset_handler(handler);

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
			})
			.expect("test material should serialize"),
			None,
		)
	}

	/// Creates material references for every authored material in a parsed fixture scene.
	fn fixture_materials(scene: &ufbx::Scene) -> ResolvedFbxMaterials {
		ResolvedFbxMaterials {
			materials: scene
				.materials
				.iter()
				.map(|material| {
					(
						MaterialKey::Material(material.element.typed_id),
						test_material(material.element.name.as_ref()),
					)
				})
				.collect(),
		}
	}

	/// Computes the first triangle's signed XY area after applying MeshProcessor's clockwise index convention.
	fn first_clockwise_triangle_area(scene: &ufbx::Scene) -> f32 {
		let processed =
			import_test_fbx_meshes(scene, &fixture_materials(scene), None, &[], &Global).expect("fixture mesh should import");
		let positions = primitive_f32_values::<3>(&processed, 0, VertexSemantics::Position);
		let indices = primitive_triangle_indices(&processed, 0);
		let first = positions[indices[0] as usize];
		let second = positions[indices[1] as usize];
		let third = positions[indices[2] as usize];

		(second[0] - first[0]) * (third[1] - first[1]) - (second[1] - first[1]) * (third[0] - first[0])
	}

	/// Decodes one primitive's packed floating-point stream from the processed aggregate buffer.
	fn primitive_f32_values<const N: usize>(
		processed: &ProcessedMesh,
		primitive_index: usize,
		semantic: VertexSemantics,
	) -> Vec<[f32; N]> {
		let stream_type = crate::types::Streams::Vertices(semantic);
		let aggregate = processed
			.mesh
			.streams
			.iter()
			.find(|stream| stream.stream_type == stream_type)
			.expect("aggregate vertex stream should exist");
		let primitive = processed.mesh.primitives[primitive_index]
			.streams
			.iter()
			.find(|stream| stream.stream_type == stream_type)
			.expect("primitive vertex stream should exist");
		let begin = aggregate.offset + primitive.offset;
		processed.buffer[begin..begin + primitive.size]
			.chunks_exact(N * 4)
			.map(|bytes| {
				std::array::from_fn(|component| {
					let offset = component * 4;
					f32::from_le_bytes(
						bytes[offset..offset + 4]
							.try_into()
							.expect("component should contain four bytes"),
					)
				})
			})
			.collect()
	}

	/// Decodes one primitive's optimized u16 triangle stream from the processed aggregate buffer.
	fn primitive_triangle_indices(processed: &ProcessedMesh, primitive_index: usize) -> Vec<u16> {
		let stream_type = crate::types::Streams::Indices(crate::types::IndexStreamTypes::Triangles);
		let aggregate = processed
			.mesh
			.streams
			.iter()
			.find(|stream| stream.stream_type == stream_type)
			.expect("aggregate triangle stream should exist");
		let primitive = processed.mesh.primitives[primitive_index]
			.streams
			.iter()
			.find(|stream| stream.stream_type == stream_type)
			.expect("primitive triangle stream should exist");
		let begin = aggregate.offset + primitive.offset;
		processed.buffer[begin..begin + primitive.size]
			.chunks_exact(2)
			.map(|bytes| u16::from_le_bytes(bytes.try_into().expect("index should contain two bytes")))
			.collect()
	}

	/// Extracts the constant metallic-roughness values produced by the focused material fixtures.
	fn brdf_values(material: &BrdfMaterialDescription) -> ([f32; 4], f32, f32, [f32; 3]) {
		let BrdfNode::MetallicRoughness(surface) = material
			.node(material.surface)
			.expect("material surface should reference a node")
		else {
			panic!("FBX material should use a metallic-roughness surface");
		};

		let base_color = base_color_factor(material, surface.base_color);

		let BrdfValue::Scalar(metallic) = constant_value(material, surface.metallic) else {
			panic!("FBX metalness should be a scalar constant");
		};

		let BrdfValue::Scalar(roughness) = constant_value(material, surface.roughness) else {
			panic!("FBX roughness should be a scalar constant");
		};

		let emission_node = surface.emission.expect("FBX material should contain an emission node");

		let BrdfNode::Emission { color } = material.node(emission_node).expect("emission should reference a node") else {
			panic!("FBX emission should use an emission node");
		};

		let BrdfValue::Vector3(emission) = constant_value(material, *color) else {
			panic!("FBX emission should be a vector3 constant");
		};

		(base_color, metallic, roughness, emission)
	}

	/// Extracts the constant color factor from a base-color graph with an optional texture sample.
	fn base_color_factor(material: &BrdfMaterialDescription, node: crate::pbr::BrdfNodeId) -> [f32; 4] {
		match material.node(node).expect("base color should reference a node") {
			BrdfNode::Constant(BrdfValue::Vector4(value)) => *value,
			BrdfNode::Multiply { left, right } => [left, right]
				.into_iter()
				.find_map(
					|node| match material.node(*node).expect("base-color factor should reference a node") {
						BrdfNode::Constant(BrdfValue::Vector4(value)) => Some(*value),
						_ => None,
					},
				)
				.expect("textured FBX base color should retain its vector4 factor"),
			_ => panic!("FBX base color should be a vector4 constant or texture-factor product"),
		}
	}

	/// Reads one constant node from a fixture-generated BRDF graph.
	fn constant_value(material: &BrdfMaterialDescription, node: crate::pbr::BrdfNodeId) -> BrdfValue {
		match material.node(node).expect("constant should reference a node") {
			BrdfNode::Constant(value) => *value,
			_ => panic!("fixture BRDF value should be constant"),
		}
	}

	fn assert_vec3_close(actual: [f32; 3], expected: [f32; 3]) {
		for index in 0..3 {
			assert!((actual[index] - expected[index]).abs() < 1.0e-6);
		}
	}

	fn assert_vec4_close(actual: [f32; 4], expected: [f32; 4]) {
		for index in 0..4 {
			assert!(
				(actual[index] - expected[index]).abs() < 1.0e-6,
				"component {index} differs: actual {actual:?}, expected {expected:?}"
			);
		}
	}

	fn assert_matrix_close(actual: [[f32; 3]; 4], expected: [[f32; 3]; 4]) {
		for column in 0..4 {
			assert_vec3_close(actual[column], expected[column]);
		}
	}
}

use std::{
	alloc::Allocator,
	collections::{HashMap, HashSet},
	fmt,
	path::Path,
	sync::Arc,
};

use serde_json::{Value, json};
use utils::Extent;

use super::{
	ContainerDefaultResource, ResourceId, container_default_resource,
	handler::{AssetHandler, BakeContext, LoadErrors},
	manager::AssetManager,
	sanitize_material_name, store_model, store_model_owned,
};
use crate::asset::handler::implementations::bema::{ProgramGenerator, compile_shader_program};
use crate::{
	ProcessedAsset, ReferenceModel, asset,
	r#async::spawn_cpu_task,
	pbr::{
		BrdfAlphaMode, BrdfMaterialBuilder, BrdfMetallicRoughness, BrdfNode, BrdfTexture, BrdfValue,
		generate_textured_brdf_program, material_texture_variable_name,
	},
	processors::{
		processor::implementations::image::{
			ImageDescription, ImageSource, Semantic, SourceChannels, SourceEncoding, gamma_from_semantic,
			process_image_with_mip_backend_in,
		},
		processor::implementations::mesh::{
			MeshPrimitiveProcessingError, MeshPrimitiveSource, MeshProcessingError, MeshProcessor, MeshProcessorSession,
			ProcessedMesh, TriangleFrontFaceWinding, VertexSkin,
		},
	},
	resource,
	resources::{
		animation::{AnimationModel, NodeTrack, QuaternionCurve, Vector3Curve},
		image::Image,
		material::{MaterialCoverage, MaterialModel, RenderModel, Shader, ValueModel, VariantModel, VariantVariableModel},
		mips::MipGenerationBackend,
		skeleton::{
			AffineMatrix4x3Columns, LocalTransform, SkeletonModel, SkeletonNode, SkinBinding, SkinJoint, SkinPaletteEntry,
		},
	},
	types::{AlphaMode, Formats, VertexComponent, VertexSemantics},
};
