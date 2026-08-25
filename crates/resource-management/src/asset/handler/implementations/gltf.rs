mod handler;
mod io;
mod material;
mod mesh;
mod skeleton;

pub use handler::GLTFAssetHandler;
pub(crate) use handler::*;
pub(crate) use io::*;
pub(crate) use material::*;
pub(crate) use mesh::*;
pub(crate) use skeleton::*;

#[cfg(test)]

mod tests {

	use maths_rs::mat::MatNew4;
	use utils::json;

	use super::{
		GLTFAssetHandler, GltfSkeletalImportError, GltfTextureDependency, TriangleFrontFaceWinding,
		collect_gltf_texture_dependencies, generated_gltf_image_id, generated_image_fragment_index, generated_material_base_id,
		gltf_normal_transform, gltf_primitive_transform_node, gltf_transform_orientation, gltf_vertex_component,
		has_vertex_component, import_gltf_animation, import_gltf_node_graph, import_gltf_skin_binding, import_gltf_vertex_skin,
		load_gltf_buffers, material_override, normalize_vertex_layouts, sanitize_material_name,
		select_unfragmented_gltf_resource, transform_gltf_tangent, transform_gltf_unit_direction, unique_gltf_materials,
		validate_affine_matrix, validate_gltf_flattened_animation_transform, validate_gltf_skin_attribute_sets,
	};
	use crate::r#async;
	use crate::{
		ReferenceModel,
		asset::{
			ContainerDefaultResource, ResourceId, handler::AssetHandler,
			handler::implementations::bema::tests::MinimalTestShaderGenerator, manager::AssetManager,
			storage_backend::tests::TestStorageBackend as AssetTestStorageBackend,
		},
		pbr::{BrdfAlphaMode, BrdfChannel, BrdfMaterialBuilder, BrdfMetallicRoughness, BrdfNode, BrdfTexture, BrdfValue},
		processors::{
			processor::implementations::image::Semantic,
			processor::implementations::mesh::orient_triangle_indices_for_front_face,
		},
		resource::storage_backend::tests::TestStorageBackend as ResourceTestStorageBackend,
		resources::{
			animation::{AnimationModel, QuaternionCurve, Vector3Curve},
			image::Image,
			mesh::MeshModel,
			skeleton::{SkeletonModel, SkinJoint},
		},
		types::{VertexComponent, VertexSemantics},
	};

	#[test]
	fn parses_json5_gltf_documents() {
		let gltf = super::parse_gltf_json(
			br#"{
				// glTF source JSON follows the resource-management JSON5 policy.
				asset: { version: '2.0', },
				meshes: [],
			}"#,
		)
		.expect("JSON5 glTF should parse");

		assert_eq!(gltf.meshes().len(), 0);
	}

	#[test]
	fn compact_skin_matrices_allow_rounding_noise_but_reject_projection() {
		let almost_affine = maths_rs::Mat4f::new(
			1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.000001, 0.0, 0.0, 1.0,
		);

		assert_eq!(validate_affine_matrix(&almost_affine, "fixture"), Ok(()));

		let projective = maths_rs::Mat4f::new(
			1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.01, 0.0, 0.0, 1.0,
		);

		assert_eq!(
			validate_affine_matrix(&projective, "fixture"),
			Err(GltfSkeletalImportError::NonAffine("fixture"))
		);
	}

	/// Appends one aligned binary payload and returns the byte range used by its glTF buffer view.
	fn append_fixture_bytes(binary: &mut Vec<u8>, bytes: &[u8]) -> (usize, usize) {
		while !binary.len().is_multiple_of(4) {
			binary.push(0);
		}

		let offset = binary.len();

		binary.extend_from_slice(bytes);

		(offset, bytes.len())
	}

	/// Appends little-endian floating-point data used by generated accessors.
	fn append_fixture_f32(binary: &mut Vec<u8>, values: &[f32]) -> (usize, usize) {
		let bytes = values.iter().flat_map(|value| value.to_le_bytes()).collect::<Vec<_>>();

		append_fixture_bytes(binary, &bytes)
	}

	/// Builds a minimal indexed triangle document and its binary vertex/index payload.
	fn generated_triangle_gltf() -> (serde_json::Value, Vec<u8>) {
		let mut binary = Vec::new();

		let positions = append_fixture_f32(&mut binary, &[0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0, 0.0]);

		let indices = append_fixture_bytes(&mut binary, &[0, 0, 1, 0, 2, 0]);

		let document = serde_json::json!({
			"asset": { "version": "2.0" },
			"scene": 0,
			"scenes": [{ "nodes": [0] }],
			"nodes": [{ "name": "Triangle", "mesh": 0 }],
			"meshes": [{
				"primitives": [{ "attributes": { "POSITION": 0 }, "indices": 1 }]
			}],
			"buffers": [{ "byteLength": binary.len() }],
			"bufferViews": [
				{ "buffer": 0, "byteOffset": positions.0, "byteLength": positions.1, "target": 34962 },
				{ "buffer": 0, "byteOffset": indices.0, "byteLength": indices.1, "target": 34963 }
			],
			"accessors": [
				{ "bufferView": 0, "componentType": 5126, "count": 3, "type": "VEC3", "min": [0.0, 0.0, 0.0], "max": [1.0, 1.0, 0.0] },
				{ "bufferView": 1, "componentType": 5123, "count": 3, "type": "SCALAR" }
			]
		});

		(document, binary)
	}

	/// Packages generated JSON and binary data into a valid GLB container.
	fn package_fixture_glb(document: &serde_json::Value, mut binary: Vec<u8>) -> Vec<u8> {
		let mut json = serde_json::to_vec(document).expect("fixture JSON should serialize");

		while !json.len().is_multiple_of(4) {
			json.push(b' ');
		}

		while !binary.len().is_multiple_of(4) {
			binary.push(0);
		}

		let total_length = 12 + 8 + json.len() + 8 + binary.len();

		let mut glb = Vec::with_capacity(total_length);

		glb.extend_from_slice(b"glTF");

		glb.extend_from_slice(&2u32.to_le_bytes());

		glb.extend_from_slice(&(total_length as u32).to_le_bytes());

		glb.extend_from_slice(&(json.len() as u32).to_le_bytes());

		glb.extend_from_slice(b"JSON");

		glb.extend_from_slice(&json);

		glb.extend_from_slice(&(binary.len() as u32).to_le_bytes());

		glb.extend_from_slice(b"BIN\0");

		glb.extend_from_slice(&binary);

		glb
	}

	/// Encodes the tiny image embedded in the textured GLB fixture.
	fn generated_rgba8_png() -> Vec<u8> {
		let mut png = Vec::new();

		{
			let mut encoder = png::Encoder::new(&mut png, 4, 4);

			encoder.set_color(png::ColorType::Rgba);

			encoder.set_depth(png::BitDepth::Eight);

			let mut writer = encoder.write_header().expect("generated PNG header should encode");

			writer
				.write_image_data(&[255, 64, 32, 255].repeat(16))
				.expect("generated PNG pixels should encode");
		}

		png
	}

	/// Builds a triangle GLB with one material and one PNG stored in its binary chunk.
	fn generated_textured_triangle_glb() -> Vec<u8> {
		let (mut document, mut binary) = generated_triangle_gltf();

		let image = append_fixture_bytes(&mut binary, &generated_rgba8_png());

		document["buffers"][0]["byteLength"] = binary.len().into();

		document["bufferViews"]
			.as_array_mut()
			.expect("fixture buffer views should be an array")
			.push(serde_json::json!({ "buffer": 0, "byteOffset": image.0, "byteLength": image.1 }));

		document["images"] = serde_json::json!([{ "name": "Test Texture", "bufferView": 2, "mimeType": "image/png" }]);

		document["textures"] = serde_json::json!([{ "source": 0 }]);

		document["materials"] = serde_json::json!([{
			"name": "Test Material",
			"pbrMetallicRoughness": { "baseColorTexture": { "index": 0 } }
		}]);

		document["meshes"][0]["primitives"][0]["material"] = 0.into();

		package_fixture_glb(&document, binary)
	}

	/// Builds a self-contained GLB that exercises hierarchy remapping, mixed instancing, two influence sets, and pose curves.
	fn generated_skeletal_glb() -> Vec<u8> {
		let mut binary = Vec::new();

		let positions = append_fixture_f32(&mut binary, &[0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0, 0.0]);

		let indices = append_fixture_bytes(&mut binary, &[0, 0, 1, 0, 2, 0]);

		let joints_0 = append_fixture_bytes(&mut binary, &[0, 1, 0, 1, 0, 1, 0, 1, 0, 1, 0, 1]);

		let weights_0 = append_fixture_f32(&mut binary, &[0.1, 0.2, 0.3, 0.4, 1.0, 0.0, 0.0, 0.0, 0.25, 0.25, 0.25, 0.25]);

		let joints_1 = append_fixture_bytes(&mut binary, &[1, 0, 1, 0, 0, 1, 0, 1, 1, 0, 1, 0]);

		let weights_1 = append_fixture_f32(&mut binary, &[0.8, 0.7, 0.6, 0.5, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0]);

		let identity = [1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0];

		let inverse_binds = append_fixture_f32(&mut binary, &identity.into_iter().chain(identity).collect::<Vec<_>>());

		let times = append_fixture_f32(&mut binary, &[0.0, 2.0]);

		let translations = append_fixture_f32(&mut binary, &[0.0, 0.0, 2.0, 1.0, 2.0, 3.0]);

		let rotations = append_fixture_f32(
			&mut binary,
			&[
				2.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 2.0, 4.0, 0.0, 0.0, 0.0, 6.0, 0.0, 0.0, 0.0, 0.0, 0.0, 2.0, 0.0, 8.0, 0.0,
				0.0, 0.0,
			],
		);

		let document = serde_json::json!({
			"asset": { "version": "2.0" },
			"scene": 0,
			"scenes": [{ "nodes": [2] }],
			"nodes": [
				{ "name": "Joint", "translation": [0.0, 0.0, 2.0] },
				{ "name": "SkinnedMesh", "mesh": 0, "skin": 0, "translation": [3.0, 0.0, 0.0] },
				{ "name": "Root", "children": [0, 1, 3], "translation": [0.0, 0.0, 1.0] },
				{ "name": "RigidMesh", "mesh": 0, "translation": [-3.0, 0.0, 0.0] }
			],
			"meshes": [{
				"primitives": [{
					"attributes": { "POSITION": 0, "JOINTS_0": 2, "WEIGHTS_0": 3, "JOINTS_1": 4, "WEIGHTS_1": 5 },
					"indices": 1,
					"material": 0
				}]
			}],
			"materials": [{ "name": "TestMaterial" }],
			"skins": [{ "inverseBindMatrices": 6, "joints": [0, 2], "skeleton": 2 }],
			"animations": [{
				"name": "Walk",
				"samplers": [
					{ "input": 7, "output": 8, "interpolation": "LINEAR" },
					{ "input": 7, "output": 9, "interpolation": "CUBICSPLINE" }
				],
				"channels": [
					{ "sampler": 0, "target": { "node": 0, "path": "translation" } },
					{ "sampler": 1, "target": { "node": 0, "path": "rotation" } }
				]
			}],
			"buffers": [{ "byteLength": binary.len() }],
			"bufferViews": [
				{ "buffer": 0, "byteOffset": positions.0, "byteLength": positions.1, "target": 34962 },
				{ "buffer": 0, "byteOffset": indices.0, "byteLength": indices.1, "target": 34963 },
				{ "buffer": 0, "byteOffset": joints_0.0, "byteLength": joints_0.1, "target": 34962 },
				{ "buffer": 0, "byteOffset": weights_0.0, "byteLength": weights_0.1, "target": 34962 },
				{ "buffer": 0, "byteOffset": joints_1.0, "byteLength": joints_1.1, "target": 34962 },
				{ "buffer": 0, "byteOffset": weights_1.0, "byteLength": weights_1.1, "target": 34962 },
				{ "buffer": 0, "byteOffset": inverse_binds.0, "byteLength": inverse_binds.1 },
				{ "buffer": 0, "byteOffset": times.0, "byteLength": times.1 },
				{ "buffer": 0, "byteOffset": translations.0, "byteLength": translations.1 },
				{ "buffer": 0, "byteOffset": rotations.0, "byteLength": rotations.1 }
			],
			"accessors": [
				{ "bufferView": 0, "componentType": 5126, "count": 3, "type": "VEC3", "min": [0.0, 0.0, 0.0], "max": [1.0, 1.0, 0.0] },
				{ "bufferView": 1, "componentType": 5123, "count": 3, "type": "SCALAR" },
				{ "bufferView": 2, "componentType": 5121, "count": 3, "type": "VEC4" },
				{ "bufferView": 3, "componentType": 5126, "count": 3, "type": "VEC4" },
				{ "bufferView": 4, "componentType": 5121, "count": 3, "type": "VEC4" },
				{ "bufferView": 5, "componentType": 5126, "count": 3, "type": "VEC4" },
				{ "bufferView": 6, "componentType": 5126, "count": 2, "type": "MAT4" },
				{ "bufferView": 7, "componentType": 5126, "count": 2, "type": "SCALAR", "min": [0.0], "max": [2.0] },
				{ "bufferView": 8, "componentType": 5126, "count": 2, "type": "VEC3" },
				{ "bufferView": 9, "componentType": 5126, "count": 6, "type": "VEC4" }
			]
		});

		let mut json = serde_json::to_vec(&document).expect("fixture JSON should serialize");

		while !json.len().is_multiple_of(4) {
			json.push(b' ');
		}

		while !binary.len().is_multiple_of(4) {
			binary.push(0);
		}

		let total_length = 12 + 8 + json.len() + 8 + binary.len();

		let mut glb = Vec::with_capacity(total_length);

		glb.extend_from_slice(b"glTF");

		glb.extend_from_slice(&2u32.to_le_bytes());

		glb.extend_from_slice(&(total_length as u32).to_le_bytes());

		glb.extend_from_slice(&(json.len() as u32).to_le_bytes());

		glb.extend_from_slice(b"JSON");

		glb.extend_from_slice(&json);

		glb.extend_from_slice(&(binary.len() as u32).to_le_bytes());

		glb.extend_from_slice(b"BIN\0");

		glb.extend_from_slice(&binary);

		glb
	}

	/// Builds a one-clip GLB with a node hierarchy but no renderable mesh geometry.
	fn generated_animation_only_glb() -> Vec<u8> {
		let mut binary = Vec::new();

		let times = append_fixture_f32(&mut binary, &[0.0, 1.0]);

		let translations = append_fixture_f32(&mut binary, &[0.0, 0.0, 0.0, 2.0, 0.0, 0.0]);

		let document = serde_json::json!({
			"asset": { "version": "2.0" },
			"scene": 0,
			"scenes": [{ "nodes": [0] }],
			"nodes": [{ "name": "AnimatedRoot" }],
			"animations": [{
				"name": "MoveX",
				"samplers": [{ "input": 0, "output": 1, "interpolation": "LINEAR" }],
				"channels": [{ "sampler": 0, "target": { "node": 0, "path": "translation" } }]
			}],
			"buffers": [{ "byteLength": binary.len() }],
			"bufferViews": [
				{ "buffer": 0, "byteOffset": times.0, "byteLength": times.1 },
				{ "buffer": 0, "byteOffset": translations.0, "byteLength": translations.1 }
			],
			"accessors": [
				{ "bufferView": 0, "componentType": 5126, "count": 2, "type": "SCALAR", "min": [0.0], "max": [1.0] },
				{ "bufferView": 1, "componentType": 5126, "count": 2, "type": "VEC3" }
			]
		});

		let mut json = serde_json::to_vec(&document).expect("fixture JSON should serialize");

		while !json.len().is_multiple_of(4) {
			json.push(b' ');
		}

		while !binary.len().is_multiple_of(4) {
			binary.push(0);
		}

		let total_length = 12 + 8 + json.len() + 8 + binary.len();

		let mut glb = Vec::with_capacity(total_length);

		glb.extend_from_slice(b"glTF");

		glb.extend_from_slice(&2u32.to_le_bytes());

		glb.extend_from_slice(&(total_length as u32).to_le_bytes());

		glb.extend_from_slice(&(json.len() as u32).to_le_bytes());

		glb.extend_from_slice(b"JSON");

		glb.extend_from_slice(&json);

		glb.extend_from_slice(&(binary.len() as u32).to_le_bytes());

		glb.extend_from_slice(b"BIN\0");

		glb.extend_from_slice(&binary);

		glb
	}

	/// Parses the generated GLB through the same glTF reader utilities used by the importer.
	fn parse_skeletal_fixture() -> (gltf::Gltf, Vec<gltf::buffer::Data>) {
		let gltf = gltf::Gltf::from_slice(&generated_skeletal_glb()).expect("generated skeletal GLB should parse");

		let buffers = gltf::import_buffers(&gltf, None, gltf.blob.clone()).expect("generated binary buffer should import");

		(gltf, buffers)
	}

	fn assert_near(actual: f32, expected: f32) {
		assert!((actual - expected).abs() < 1.0e-5, "expected {expected}, got {actual}");
	}

	#[test]
	fn imports_parent_before_child_skeleton_with_left_handed_rest_pose() {
		let (gltf, _) = parse_skeletal_fixture();

		let graph = import_gltf_node_graph(&gltf).expect("node graph should import");

		assert_eq!(graph.source_to_dense, vec![1, 2, 0, 3]);
		assert_eq!(
			graph
				.skeleton
				.nodes
				.iter()
				.map(|node| (node.name.as_deref(), node.parent))
				.collect::<Vec<_>>(),
			vec![
				(Some("Root"), None),
				(Some("Joint"), Some(0)),
				(Some("SkinnedMesh"), Some(0)),
				(Some("RigidMesh"), Some(0))
			]
		);
		assert_eq!(graph.skeleton.nodes[0].rest_local.translation, [0.0, 0.0, -1.0]);
		assert_eq!(graph.skeleton.nodes[1].rest_local.translation, [0.0, 0.0, -2.0]);
	}

	#[test]
	fn unfragmented_glb_with_geometry_remains_mesh_first() {
		let gltf = gltf::Gltf::from_slice(&generated_skeletal_glb()).unwrap();

		assert_eq!(
			select_unfragmented_gltf_resource(&gltf, None),
			Ok(ContainerDefaultResource::Mesh)
		);
	}

	#[test]
	fn transforms_normals_and_tangents_without_translation_contamination() {
		let transform = maths_rs::Mat4f::new(
			2.0, 0.0, 0.0, 10.0, 0.0, 3.0, 0.0, 20.0, 0.0, 0.0, -4.0, 30.0, 0.0, 0.0, 0.0, 1.0,
		);

		let normal_transform = gltf_normal_transform(transform).expect("normal transform should invert");

		let normal = transform_gltf_unit_direction(&normal_transform, [1.0, 1.0, 0.0]).unwrap();

		let orientation = gltf_transform_orientation(transform).unwrap();

		let tangent = transform_gltf_tangent(&transform, orientation, [1.0, 1.0, 0.0, 1.0]).unwrap();

		assert_near(normal[0], 0.8320503);

		assert_near(normal[1], 0.5547002);

		assert_near(normal[2], 0.0);

		assert_near(tangent[0], 0.5547002);

		assert_near(tangent[1], 0.8320503);

		assert_near(tangent[2], 0.0);

		assert_eq!(tangent[3], -1.0);
	}

	#[test]
	fn rejects_singular_bind_transforms_only_when_geometry_retains_an_animation_node() {
		let singular = maths_rs::Mat4f::new(
			0.0, 0.0, 0.0, 3.0, 0.0, 1.0, 0.0, 2.0, 0.0, 0.0, -1.0, 1.0, 0.0, 0.0, 0.0, 1.0,
		);

		assert!(validate_gltf_flattened_animation_transform(singular, None).is_ok());
		assert_eq!(
			validate_gltf_flattened_animation_transform(singular, Some(0)),
			Err(GltfSkeletalImportError::SingularMeshTransform)
		);
	}

	#[test]
	fn imports_adjusted_binding_and_merges_strongest_influences_for_only_skinned_instances() {
		let (gltf, buffers) = parse_skeletal_fixture();

		let graph = import_gltf_node_graph(&gltf).expect("node graph should import");

		let skinned_node = gltf.nodes().find(|node| node.name() == Some("SkinnedMesh")).unwrap();

		let rigid_node = gltf.nodes().find(|node| node.name() == Some("RigidMesh")).unwrap();

		let binding = import_gltf_skin_binding(&skinned_node, &buffers, &graph).expect("skin binding should import");

		assert_eq!(
			binding.entries.iter().map(|entry| entry.joint).collect::<Vec<_>>(),
			vec![SkinJoint::Node(1), SkinJoint::Node(0)]
		);

		for entry in &binding.entries {
			let inverse_bind = &entry.adjusted_inverse_bind_matrix;

			assert_near(inverse_bind[3][0], -3.0);

			assert_near(inverse_bind[3][1], 0.0);

			assert_near(inverse_bind[3][2], 1.0);
		}

		let primitive = gltf.meshes().next().unwrap().primitives().next().unwrap();

		validate_gltf_skin_attribute_sets(&primitive, true).expect("skinned instance should validate");

		validate_gltf_skin_attribute_sets(&primitive, false).expect("rigid instance should ignore skin streams");

		let reader = primitive.reader(|buffer| Some(&buffers[buffer.index()]));

		let (joints, weights) = import_gltf_vertex_skin(&reader, 3, binding.len()).expect("weights should import");

		assert_eq!(joints[0], [1, 0, 1, 0]);

		for (actual, expected) in weights[0].into_iter().zip([8.0 / 26.0, 7.0 / 26.0, 6.0 / 26.0, 5.0 / 26.0]) {
			assert_near(actual, expected);
		}

		assert!(skinned_node.skin().is_some());
		assert!(rigid_node.skin().is_none());
		assert_eq!(gltf_primitive_transform_node(&graph, &skinned_node, true), Some(2));
		assert_eq!(gltf_primitive_transform_node(&graph, &rigid_node, true), Some(3));
	}

	#[test]
	fn imports_pose_curves_with_dense_targets_and_preserved_cubic_derivatives() {
		let (gltf, buffers) = parse_skeletal_fixture();

		let graph = import_gltf_node_graph(&gltf).expect("node graph should import");

		let skeleton = ReferenceModel::<SkeletonModel>::new("fixture.glb#skeleton", 0, 0, &graph.skeleton, None);

		let animation = import_gltf_animation(&gltf, &buffers, "animations/Walk", &graph.source_to_dense, skeleton)
			.expect("animation should import");

		assert_eq!(animation.name.as_deref(), Some("Walk"));
		assert_eq!(animation.duration, 2.0);
		assert_eq!(animation.tracks.len(), 1);
		assert_eq!(animation.tracks[0].node, 1);

		match animation.tracks[0].translation.as_ref().unwrap() {
			Vector3Curve::Linear { times, values } => {
				assert_eq!(times, &[0.0, 2.0]);
				assert_eq!(values, &[[0.0, 0.0, -2.0], [1.0, 2.0, -3.0]]);
			}
			curve => panic!("expected linear translation curve, got {curve:?}"),
		}

		match animation.tracks[0].rotation.as_ref().unwrap() {
			QuaternionCurve::CubicSpline {
				times,
				values,
				in_tangents,
				out_tangents,
			} => {
				assert_eq!(times, &[0.0, 2.0]);
				assert_eq!(values, &[[0.0, 0.0, 0.0, 1.0], [0.0, 0.0, 1.0, 0.0]]);
				assert_eq!(in_tangents, &[[-2.0, 0.0, 0.0, 0.0], [-6.0, 0.0, 0.0, 0.0]]);
				assert_eq!(out_tangents, &[[-4.0, 0.0, 0.0, 0.0], [-8.0, 0.0, 0.0, 0.0]]);
			}
			curve => panic!("expected cubic rotation curve, got {curve:?}"),
		}
	}

	#[r#async::test]
	async fn bakes_generated_skeleton_fragment_from_the_base_glb() {
		let asset_storage_backend = AssetTestStorageBackend::new();

		asset_storage_backend.add_file("generated_skeletal.glb", &generated_skeletal_glb());

		let resource_storage_backend = ResourceTestStorageBackend::new();

		let mut asset_manager = AssetManager::new(asset_storage_backend, resource_storage_backend);

		asset_manager.add_asset_handler(GLTFAssetHandler::new());

		let skeleton: ReferenceModel<SkeletonModel> = asset_manager
			.bake_if_not_exists("generated_skeletal.glb#skeleton")
			.await
			.expect("generated skeleton fragment should bake");

		let skeleton = crate::from_slice::<SkeletonModel>(&skeleton.resource).expect("skeleton should deserialize");

		assert_eq!(skeleton.nodes.len(), 4);
		assert_eq!(skeleton.nodes[0].name.as_deref(), Some("Root"));
		assert_eq!(skeleton.nodes[1].parent, Some(0));
	}

	#[r#async::test]
	async fn bakes_named_animation_fragment_with_generated_skeleton_dependency() {
		let asset_storage_backend = AssetTestStorageBackend::new();

		asset_storage_backend.add_file("generated_skeletal.glb", &generated_skeletal_glb());

		let resource_storage_backend = ResourceTestStorageBackend::new();

		let mut asset_manager = AssetManager::new(asset_storage_backend, resource_storage_backend);

		asset_manager.add_asset_handler(GLTFAssetHandler::new());

		let animation: ReferenceModel<AnimationModel> = asset_manager
			.bake_if_not_exists("generated_skeletal.glb#animations/Walk")
			.await
			.expect("generated animation fragment should bake");

		let animation = crate::from_slice::<AnimationModel>(&animation.resource).expect("animation should deserialize");

		assert_eq!(animation.name.as_deref(), Some("Walk"));
		assert_eq!(animation.duration, 2.0);
		assert_eq!(animation.tracks.len(), 1);
		assert_eq!(animation.skeleton.id().as_ref(), "generated_skeletal.glb#skeleton");
	}

	#[r#async::test]
	async fn bakes_unfragmented_animation_only_glb_as_animation() {
		let asset_storage_backend = AssetTestStorageBackend::new();

		asset_storage_backend.add_file("animation_only.glb", &generated_animation_only_glb());

		let resource_storage_backend = ResourceTestStorageBackend::new();

		let mut asset_manager = AssetManager::new(asset_storage_backend, resource_storage_backend.clone());

		asset_manager.add_asset_handler(GLTFAssetHandler::new());

		asset_manager
			.bake("animation_only.glb")
			.await
			.expect("an unfragmented animation-only GLB should bake as Animation");

		let animation = resource_storage_backend
			.get_resource(ResourceId::new("animation_only.glb"))
			.expect("the bare GLB Animation resource should be stored");

		let animation = crate::from_slice::<AnimationModel>(&animation.resource).unwrap();

		assert_eq!(animation.name.as_deref(), Some("MoveX"));
		assert_eq!(animation.skeleton.id().as_ref(), "animation_only.glb#skeleton");
	}

	#[r#async::test]
	async fn bead_can_make_a_single_clip_glb_with_geometry_default_to_animation() {
		let asset_storage_backend = AssetTestStorageBackend::new();

		asset_storage_backend.add_file("generated_skeletal.glb", &generated_skeletal_glb());

		asset_storage_backend.add_file(
			"generated_skeletal.glb.bead",
			br#"{ // JSON5 BEAD sidecars can be commented and use unquoted keys.
				default_resource: 'animation',
			}"#,
		);

		let resource_storage_backend = ResourceTestStorageBackend::new();

		let mut asset_manager = AssetManager::new(asset_storage_backend, resource_storage_backend);

		asset_manager.add_asset_handler(GLTFAssetHandler::new());

		let animation: ReferenceModel<AnimationModel> = asset_manager
			.bake_if_not_exists("generated_skeletal.glb")
			.await
			.expect("the BEAD default should override mesh-first glTF dispatch");

		assert_eq!(animation.class(), "Animation");
	}

	#[r#async::test]
	async fn bakes_nested_gltf_animation_from_ordered_file_relative_buffers() {
		let glb_bytes = generated_skeletal_glb();

		let glb = gltf::Glb::from_slice(&glb_bytes).expect("generated skeletal GLB should parse");

		let mut document: serde_json::Value = serde_json::from_slice(&glb.json).expect("generated skeletal JSON should parse");

		let binary = glb
			.bin
			.expect("generated skeletal GLB should contain a BIN chunk")
			.into_owned();

		let times_offset = document["bufferViews"][7]["byteOffset"]
			.as_u64()
			.expect("animation times should have a byte offset") as usize;

		let values_offset = document["bufferViews"][8]["byteOffset"]
			.as_u64()
			.expect("animation values should have a byte offset") as usize;

		let times = &binary[times_offset..values_offset];

		let values = &binary[values_offset..];

		// Buffer zero is deliberately absent: skeletons need no binary data and selected clips load only their accessor buffers.
		document["buffers"] = serde_json::json!([
			{ "byteLength": times_offset, "uri": "missing_geometry" },
			{ "byteLength": times.len(), "uri": "timeline" },
			{ "byteLength": values.len(), "uri": "animation%20values" }
		]);

		document["bufferViews"][7]["buffer"] = 1.into();

		document["bufferViews"][7]["byteOffset"] = 0.into();

		for view_index in 8..=9 {
			let source_offset = document["bufferViews"][view_index]["byteOffset"]
				.as_u64()
				.expect("animation value view should have a byte offset") as usize;

			document["bufferViews"][view_index]["buffer"] = 2.into();

			document["bufferViews"][view_index]["byteOffset"] = (source_offset - values_offset).into();
		}

		let document = serde_json::to_vec(&document).expect("external-buffer glTF JSON should serialize");

		let asset_storage_backend = AssetTestStorageBackend::new();

		asset_storage_backend.add_file("characters/generated_skeletal.gltf", &document);

		let resource_storage_backend = ResourceTestStorageBackend::new();

		let mut asset_manager = AssetManager::new(asset_storage_backend.clone(), resource_storage_backend);

		asset_manager.add_asset_handler(GLTFAssetHandler::new());

		let skeleton: ReferenceModel<SkeletonModel> = asset_manager
			.bake_if_not_exists("characters/generated_skeletal.gltf#skeleton")
			.await
			.expect("nested glTF skeleton should not load unrelated buffers");

		assert_eq!(skeleton.id().as_ref(), "characters/generated_skeletal.gltf#skeleton");

		asset_storage_backend.add_file("characters/timeline", times);

		asset_storage_backend.add_file("characters/animation values", values);

		let animation: ReferenceModel<AnimationModel> = asset_manager
			.bake_if_not_exists("characters/generated_skeletal.gltf#animations/Walk")
			.await
			.expect("nested glTF animation should load its sibling buffer");

		let animation = crate::from_slice::<AnimationModel>(&animation.resource).expect("animation should deserialize");

		assert_eq!(animation.duration, 2.0);
		assert_eq!(
			animation.skeleton.id().as_ref(),
			"characters/generated_skeletal.gltf#skeleton"
		);
	}

	#[r#async::test]
	async fn rejects_a_truncated_data_uri_before_adding_alignment_padding() {
		let gltf = gltf::Gltf::from_slice(
			br#"{"asset":{"version":"2.0"},"buffers":[{"byteLength":4,"uri":"data:application/octet-stream;base64,AQID"}]}"#,
		)
		.expect("truncated data URI fixture should parse");

		let asset_storage_backend = AssetTestStorageBackend::new();

		let result = load_gltf_buffers(
			&asset_storage_backend,
			ResourceId::new("truncated.gltf"),
			&gltf,
			None,
			None,
			&std::alloc::Global,
		)
		.await;

		assert!(result.is_err());
	}

	#[r#async::test]
	async fn bakes_base_skeletal_mesh_with_primitive_node_and_skin_bindings() {
		let asset_storage_backend = AssetTestStorageBackend::new();

		asset_storage_backend.add_file("generated_skeletal.glb", &generated_skeletal_glb());

		let resource_storage_backend = ResourceTestStorageBackend::new();

		let mut asset_manager = AssetManager::new(asset_storage_backend, resource_storage_backend);

		let mut handler = GLTFAssetHandler::new();

		handler.set_shader_generator(MinimalTestShaderGenerator);

		asset_manager.add_asset_handler(handler);

		let mesh: ReferenceModel<MeshModel> = asset_manager
			.bake_if_not_exists("generated_skeletal.glb")
			.await
			.expect("generated skeletal mesh should bake");

		let mesh = crate::from_slice::<MeshModel>(&mesh.resource).expect("generated mesh should deserialize");

		assert_eq!(
			mesh.skeleton
				.as_ref()
				.expect("generated mesh should retain its skeleton")
				.id()
				.as_ref(),
			"generated_skeletal.glb#skeleton"
		);
		assert_eq!(mesh.skins.len(), 1);
		assert_eq!(mesh.primitives.len(), 2);
		assert_eq!(mesh.primitives[0].transform_node, Some(2));
		assert_eq!(mesh.primitives[0].skin, Some(0));
		assert_eq!(mesh.primitives[1].transform_node, Some(3));
		assert_eq!(mesh.primitives[1].skin, None);
	}

	#[test]
	fn normalizes_gltf_layouts_to_shared_supported_streams() {
		let normalized = normalize_vertex_layouts(&[
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
					semantic: VertexSemantics::BiTangent,
					format: "vec3f".to_string(),
					channel: 0,
				},
			],
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
			],
		]);

		assert_eq!(normalized.len(), 2);
		assert!(has_vertex_component(&normalized, VertexSemantics::Position, 0));
		assert!(has_vertex_component(&normalized, VertexSemantics::Normal, 0));
		assert!(!has_vertex_component(&normalized, VertexSemantics::BiTangent, 0));
	}

	#[test]
	fn maps_gltf_semantics_to_normalized_channels() {
		assert_eq!(gltf_vertex_component(gltf::Semantic::Normals).unwrap().channel, 0);
		assert_eq!(gltf_vertex_component(gltf::Semantic::TexCoords(0)).unwrap().channel, 0);
		assert!(gltf_vertex_component(gltf::Semantic::TexCoords(1)).is_none());
	}

	#[test]
	fn deduplicates_indexed_and_default_materials_in_primitive_order() {
		let gltf = gltf::Gltf::from_slice(
			r#"{
				"asset":{"version":"2.0"},
				"buffers":[{"byteLength":36}],
				"bufferViews":[{"buffer":0,"byteLength":36}],
				"accessors":[{
					"bufferView":0,"componentType":5126,"count":3,"type":"VEC3",
					"min":[0,0,0],"max":[1,1,0]
				}],
				"materials":[{},{}],
				"meshes":[{"primitives":[
					{"attributes":{"POSITION":0},"material":1},
					{"attributes":{"POSITION":0}},
					{"attributes":{"POSITION":0},"material":1},
					{"attributes":{"POSITION":0},"material":0},
					{"attributes":{"POSITION":0}},
					{"attributes":{"POSITION":0},"material":0}
				]}]
			}"#
			.as_bytes(),
		)
		.expect("test glTF should parse");

		let primitives = gltf.meshes().flat_map(|mesh| mesh.primitives()).collect::<Vec<_>>();

		let (materials, material_indices_per_primitive) = unique_gltf_materials(&primitives);

		assert_eq!(
			materials.iter().map(|material| material.index()).collect::<Vec<_>>(),
			vec![Some(1), None, Some(0)]
		);
		assert_eq!(material_indices_per_primitive, vec![0, 1, 0, 2, 1, 2]);
		assert_eq!(
			materials
				.iter()
				.map(|material| generated_material_base_id(ResourceId::new("models/drone.glb"), material))
				.collect::<Vec<_>>(),
			vec![
				"models/drone.glb#materials/material_1",
				"models/drone.glb#materials/material_default",
				"models/drone.glb#materials/material_0",
			]
		);
	}

	#[test]
	fn reads_bead_material_override_when_present() {
		let gltf = gltf::Gltf::from_slice(r#"{"asset":{"version":"2.0"},"materials":[{"name":"Paint"}]}"#.as_bytes())
			.expect("test glTF should parse");

		let material = gltf.materials().next().unwrap();

		let spec = crate::asset::parse_json(r#"{"asset":{"Paint":{"asset":"Paint.bema"}}}"#).unwrap();

		assert_eq!(material_override(Some(&spec), &material), Some("Paint.bema".to_string()));
	}

	#[test]
	fn misses_bead_material_override_when_absent() {
		let gltf = gltf::Gltf::from_slice(r#"{"asset":{"version":"2.0"},"materials":[{"name":"Paint"}]}"#.as_bytes())
			.expect("test glTF should parse");

		let material = gltf.materials().next().unwrap();

		assert_eq!(material_override(None, &material), None);
	}

	#[test]
	fn generated_material_ids_are_stable_and_sanitized() {
		let gltf = gltf::Gltf::from_slice(r#"{"asset":{"version":"2.0"},"materials":[{"name":"Red Paint/Gloss"}]}"#.as_bytes())
			.expect("test glTF should parse");

		let material = gltf.materials().next().unwrap();

		assert_eq!(sanitize_material_name("Red Paint/Gloss"), "Red_Paint_Gloss");
		assert_eq!(
			generated_material_base_id(ResourceId::new("models/car.glb"), &material),
			"models/car.glb#materials/Red_Paint_Gloss"
		);
	}

	#[test]
	fn generated_image_ids_use_stable_indices_and_optional_names() {
		assert_eq!(
			generated_gltf_image_id(ResourceId::new("models/robot.glb"), 0, None),
			"models/robot.glb#images/0"
		);
		assert_eq!(
			generated_gltf_image_id(ResourceId::new("models/robot.glb"), 12, Some("Base Color/PNG")),
			"models/robot.glb#images/12_Base_Color_PNG"
		);
		assert_eq!(generated_image_fragment_index("images/12_Base_Color_PNG"), Some(12));
		assert_eq!(generated_image_fragment_index("Base Color"), None);
	}

	#[test]
	fn collects_gltf_texture_dependencies_in_material_slot_order() {
		let mut builder = BrdfMaterialBuilder::new();

		let base_color = builder.texture(BrdfTexture {
			image_index: 2,
			texcoord_channel: 0,
		});

		let metallic_roughness = builder.texture(BrdfTexture {
			image_index: 5,
			texcoord_channel: 0,
		});

		let metallic = builder.extract_channel(metallic_roughness, BrdfChannel::Blue);

		let roughness = builder.extract_channel(metallic_roughness, BrdfChannel::Green);

		let normal_source = builder.texture(BrdfTexture {
			image_index: 8,
			texcoord_channel: 0,
		});

		let normal = builder.add(BrdfNode::NormalMap {
			source: normal_source,
			scale: 1.0,
		});

		let occlusion_source = builder.texture(BrdfTexture {
			image_index: 10,
			texcoord_channel: 0,
		});

		let occlusion = builder.add(BrdfNode::Occlusion {
			source: occlusion_source,
			strength: 0.75,
		});

		let emission_color = builder.constant(BrdfValue::Vector3([1.0, 0.25, 0.5]));

		let emission = builder.add(BrdfNode::Emission { color: emission_color });

		let surface = builder.add(BrdfNode::MetallicRoughness(BrdfMetallicRoughness {
			base_color,
			metallic,
			roughness,
			normal: Some(normal),
			occlusion: Some(occlusion),
			emission: Some(emission),
		}));

		let material = builder.finish(None, surface, false, BrdfAlphaMode::Opaque);

		let dependencies = collect_gltf_texture_dependencies(&material).expect("dependencies should collect");

		assert_eq!(
			dependencies,
			vec![
				GltfTextureDependency {
					image_index: 2,
					semantic: Semantic::Albedo,
				},
				GltfTextureDependency {
					image_index: 5,
					semantic: Semantic::Metallic,
				},
				GltfTextureDependency {
					image_index: 8,
					semantic: Semantic::Normal,
				},
				GltfTextureDependency {
					image_index: 10,
					semantic: Semantic::AO,
				},
			]
		);
	}

	#[test]
	fn defaults_to_clockwise_front_faces() {
		let asset_handler = GLTFAssetHandler::new();

		assert_eq!(
			asset_handler.triangle_front_face_winding(),
			TriangleFrontFaceWinding::Clockwise
		);
	}

	#[test]
	fn preserves_triangle_order_for_counter_clockwise_front_faces() {
		let indices = vec![0, 1, 2, 3, 4, 5];

		let oriented = orient_triangle_indices_for_front_face(indices, TriangleFrontFaceWinding::CounterClockwise);

		assert_eq!(oriented, vec![0, 1, 2, 3, 4, 5]);
	}

	#[test]
	fn rewinds_triangle_order_for_clockwise_front_faces() {
		let indices = vec![0, 1, 2, 3, 4, 5];

		let oriented = orient_triangle_indices_for_front_face(indices, TriangleFrontFaceWinding::Clockwise);

		assert_eq!(oriented, vec![0, 2, 1, 3, 5, 4]);
	}

	#[r#async::test]
	async fn bakes_skeleton_from_minimal_glb_bytes() {
		let (document, binary) = generated_triangle_gltf();

		let asset_storage_backend = AssetTestStorageBackend::new();

		asset_storage_backend.add_file("triangle.glb", &package_fixture_glb(&document, binary));

		let resource_storage_backend = ResourceTestStorageBackend::new();

		let mut asset_manager = AssetManager::new(asset_storage_backend, resource_storage_backend);

		asset_manager.add_asset_handler(GLTFAssetHandler::new());

		let skeleton: ReferenceModel<SkeletonModel> = asset_manager
			.bake_if_not_exists("triangle.glb#skeleton")
			.await
			.expect("generated triangle GLB skeleton should bake");

		let skeleton =
			crate::from_slice::<SkeletonModel>(&skeleton.resource).expect("generated GLB skeleton should deserialize");

		assert_eq!(skeleton.nodes.len(), 1);
		assert_eq!(skeleton.nodes[0].name.as_deref(), Some("Triangle"));
	}

	#[r#async::test]
	async fn loads_minimal_gltf_external_bin_from_in_memory_bytes() {
		let (mut document, binary) = generated_triangle_gltf();

		document["buffers"][0]["uri"] = "triangle.bin".into();

		let document = serde_json::to_vec(&document).expect("generated glTF JSON should serialize");

		let asset_storage_backend = AssetTestStorageBackend::new();

		asset_storage_backend.add_file("models/triangle.bin", &binary);

		let gltf = gltf::Gltf::from_slice(&document).expect("generated external-buffer glTF should parse");

		let buffers = load_gltf_buffers(
			&asset_storage_backend,
			ResourceId::new("models/triangle.gltf"),
			&gltf,
			None,
			None,
			&std::alloc::Global,
		)
		.await
		.expect("generated external binary should load");

		assert_eq!(buffers.len(), 1);
		assert_eq!(&buffers[0].0[..binary.len()], binary.as_slice());
	}

	#[r#async::test]
	async fn bakes_named_image_fragment_from_minimal_glb() {
		let asset_storage_backend = AssetTestStorageBackend::new();

		asset_storage_backend.add_file("named_image.glb", &generated_textured_triangle_glb());

		let resource_storage_backend = ResourceTestStorageBackend::new();

		let mut asset_manager = AssetManager::new(asset_storage_backend, resource_storage_backend.clone());

		asset_manager.add_asset_handler(GLTFAssetHandler::new());

		asset_manager
			.bake("named_image.glb#Test Texture")
			.await
			.expect("named image fragment should bake");

		let resource = resource_storage_backend
			.get_resource(ResourceId::new("named_image.glb#Test Texture"))
			.expect("named GLB image fragment should be stored");

		let image: Image = crate::from_slice(&resource.resource).expect("named image metadata should deserialize");

		assert_eq!(resource.class, "Image");
		assert_eq!(image.extent, [4, 4, 1]);
		assert_eq!(
			image.mip_count, 1,
			"explicit image fragments are not material-generated textures"
		);
	}

	#[r#async::test]
	async fn bakes_image_fragment_from_minimal_glb() {
		let asset_storage_backend = AssetTestStorageBackend::new();

		asset_storage_backend.add_file("image.glb", &generated_textured_triangle_glb());

		let resource_storage_backend = ResourceTestStorageBackend::new();

		let mut asset_manager = AssetManager::new(asset_storage_backend, resource_storage_backend.clone());

		asset_manager.add_asset_handler(GLTFAssetHandler::new());

		asset_manager
			.bake("image.glb#images/0_Test_Texture")
			.await
			.expect("generated GLB image fragment should bake");

		let resource = resource_storage_backend
			.get_resource(ResourceId::new("image.glb#images/0_Test_Texture"))
			.expect("baked GLB image fragment should be stored");

		let image: Image = crate::from_slice(&resource.resource).expect("GLB image metadata should deserialize");

		assert_eq!(resource.class, "Image");
		assert_eq!(image.extent, [4, 4, 1]);
		assert_eq!(
			image.mip_count, 1,
			"explicit image fragments are not material-generated textures"
		);
	}
}

use std::{collections::HashMap, path::Path, sync::Arc};

use maths_rs::{
	mat::{MatDeterminant, MatInverse, MatNew4, MatScale, MatTranspose},
	vec::Vec3,
};
use utils::{Extent, json, json::JsonValueTrait};

use super::{
	ContainerDefaultResource, ResourceId, container_default_resource,
	handler::{AssetHandler, BakeContext, LoadErrors},
	manager::AssetManager,
	sanitize_material_name, store_model, store_model_owned,
};
use crate::asset::handler::implementations::bema::{ProgramGenerator, compile_shader_program};
pub use crate::processors::processor::implementations::mesh::TriangleFrontFaceWinding;
use crate::{
	ProcessedAsset, ReferenceModel,
	asset::{self},
	r#async::spawn_cpu_task,
	pbr::{
		BrdfMaterialDescription, BrdfMaterialValidationError, BrdfNode, BrdfNodeId, BrdfValue, brdf_material_from_gltf,
		generate_textured_brdf_program, material_texture_variable_name,
	},
	processors::{
		processor::implementations::image::{
			ImageDescription, ImageSource, Semantic, SourceChannels, SourceEncoding, gamma_from_semantic,
			guess_semantic_from_name, process_image_with_mip_backend_in,
		},
		processor::implementations::mesh::{MeshPrimitiveProcessingError, MeshPrimitiveSource, MeshProcessor, VertexSkin},
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
