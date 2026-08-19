mod opacity;
mod reflection;

#[cfg(test)]
use std::collections::HashSet;

pub use opacity::OpacityEvaluation;
#[cfg(test)]
use reflection::{
	checked_align_up, primitive_storage_layout, reflected_storage_buffer_stride_for_target, reflected_storage_type_layout,
	StorageLayout, StorageLayoutTarget,
};
pub(crate) use reflection::{collect_bindings, BindingRecord};
pub use reflection::{BindingKind, BindingUsage, ProgramEvaluation, TextureView};

#[cfg(test)]
mod tests {
	use super::*;
	use crate::shader::generator;

	fn assert_builtin_layout(root: &besl::Node, target: StorageLayoutTarget, type_name: &str, expected: StorageLayout) {
		let r#type = root
			.get_child(type_name)
			.unwrap_or_else(|| panic!("Expected built-in type '{type_name}'"));
		let layout = reflected_storage_type_layout(&r#type, target, false, &mut HashSet::new())
			.unwrap_or_else(|error| panic!("Expected '{type_name}' layout for {target:?}: {error}"));

		assert_eq!(layout, expected, "Unexpected '{type_name}' layout for {target:?}");
	}

	#[test]
	fn binding_metadata_is_sorted_and_classified() {
		let main = generator::tests::bindings();

		let evaluation = ProgramEvaluation::from_main(&main).expect("Failed to evaluate program");
		let bindings = evaluation
			.bindings()
			.iter()
			.map(|binding| {
				(
					binding.name.as_str(),
					binding.kind,
					binding.count,
					binding.slot,
					binding.buffer_stride,
					binding.read,
					binding.write,
				)
			})
			.collect::<Vec<_>>();

		assert_eq!(
			bindings,
			vec![
				("buff", BindingKind::StorageBuffer, 1, 0, Some(4), true, true),
				("image", BindingKind::StorageImage, 1, 1, None, false, true),
				(
					"texture",
					BindingKind::CombinedImageSampler {
						view: TextureView::Texture2D,
					},
					1,
					2,
					None,
					true,
					false,
				),
			]
		);
	}

	#[test]
	fn storage_buffer_strides_cover_flattened_arrays_and_wrapper_structs() {
		let script = "main: fn () -> void { positions; indices; lighting; }";
		let mut root = besl::Node::root();
		let vec2f = root.get_child("vec2f").expect("Expected vec2f");
		let vec3f = root.get_child("vec3f").expect("Expected vec3f");
		let u8_type = root.get_child("u8").expect("Expected u8");
		let u16_type = root.get_child("u16").expect("Expected u16");
		let u32_type = root.get_child("u32").expect("Expected u32");
		let light = root.add_child(
			besl::Node::r#struct(
				"Light",
				vec![
					besl::Node::member("position", vec3f.clone()).into(),
					besl::Node::member("color", vec3f.clone()).into(),
					besl::Node::member("direction", vec3f.clone()).into(),
					besl::Node::member("cone_cosines", vec2f).into(),
					besl::Node::member("light_type", u8_type).into(),
					besl::Node::array("cascades", u32_type.clone(), 8),
				],
			)
			.into(),
		);
		root.add_children(vec![
			besl::Node::binding(
				"positions",
				besl::BindingTypes::Buffer {
					members: vec![besl::Node::array("positions", vec3f, 16)],
				},
				0,
				true,
				false,
			)
			.into(),
			besl::Node::binding(
				"indices",
				besl::BindingTypes::Buffer {
					members: vec![besl::Node::array("indices", u16_type, 16)],
				},
				1,
				true,
				false,
			)
			.into(),
			besl::Node::binding(
				"lighting",
				besl::BindingTypes::Buffer {
					members: vec![
						besl::Node::member("light_count", u32_type).into(),
						besl::Node::array("lights", light, 16),
					],
				},
				2,
				true,
				false,
			)
			.into(),
		]);

		let program = besl::compile_to_besl(script, Some(root)).expect("Expected stride-reflection shader to link");
		let evaluation = ProgramEvaluation::from_program(&program).expect("Expected storage-buffer strides to reflect");
		let strides = evaluation
			.bindings()
			.iter()
			.map(|binding| binding.buffer_stride)
			.collect::<Vec<_>>();

		let expected = if cfg!(target_vendor = "apple") {
			vec![Some(12), Some(2), Some(1552)]
		} else if cfg!(target_os = "windows") {
			vec![Some(12), Some(4), Some(1284)]
		} else {
			vec![Some(12), Some(2), Some(1284)]
		};

		assert_eq!(strides, expected);
	}

	#[test]
	fn storage_layout_target_matches_the_compiled_backend() {
		#[cfg(target_vendor = "apple")]

		assert_eq!(StorageLayoutTarget::current(), StorageLayoutTarget::Msl);

		#[cfg(all(not(target_vendor = "apple"), target_os = "windows"))]

		assert_eq!(StorageLayoutTarget::current(), StorageLayoutTarget::Hlsl);

		#[cfg(all(not(target_vendor = "apple"), not(target_os = "windows")))]

		assert_eq!(StorageLayoutTarget::current(), StorageLayoutTarget::GlslScalar);
	}

	#[test]
	fn primitive_storage_layouts_follow_each_emitted_backend_type() {
		let root = besl::Node::root();

		for (type_name, size, alignment) in [
			("u8", 4, 4),
			("u16", 4, 4),
			("u32", 4, 4),
			("f16", 2, 2),
			("vec2u16", 4, 2),
			("vec4u16", 8, 2),
			("vec2f16", 4, 2),
			("vec3f16", 6, 2),
			("vec4f16", 8, 2),
			("vec3f", 12, 4),
			("packed_vec4f", 16, 4),
		] {
			assert_builtin_layout(&root, StorageLayoutTarget::Hlsl, type_name, StorageLayout { size, alignment });
		}

		for (type_name, size, alignment) in [
			("u8", 1, 1),
			("u16", 2, 2),
			("u32", 4, 4),
			("f16", 2, 2),
			("vec2u16", 4, 4),
			("vec4u16", 8, 8),
			("vec2f16", 4, 4),
			("vec3f16", 8, 8),
			("vec4f16", 8, 8),
			("vec3f", 16, 16),
			("packed_vec4f", 16, 4),
		] {
			assert_builtin_layout(&root, StorageLayoutTarget::Msl, type_name, StorageLayout { size, alignment });
		}

		for (type_name, size, alignment) in [
			("u8", 1, 1),
			("u16", 2, 2),
			("u32", 4, 4),
			("f16", 2, 2),
			("vec2u16", 4, 2),
			("vec4u16", 8, 2),
			("vec2f16", 4, 2),
			("vec3f16", 6, 2),
			("vec4f16", 8, 2),
			("vec3f", 12, 4),
			("packed_vec4f", 16, 4),
		] {
			assert_builtin_layout(
				&root,
				StorageLayoutTarget::GlslScalar,
				type_name,
				StorageLayout { size, alignment },
			);
		}
	}

	#[test]
	fn direct_f16_storage_members_preserve_the_packed_vm_layout() {
		let root = besl::Node::root();
		let members = vec![
			besl::Node::member("scalar", root.get_child("f16").expect("Expected f16")).into(),
			besl::Node::member("uv", root.get_child("vec2f16").expect("Expected vec2f16")).into(),
			besl::Node::member("normal", root.get_child("vec3f16").expect("Expected vec3f16")).into(),
			besl::Node::member("color", root.get_child("vec4f16").expect("Expected vec4f16")).into(),
		];

		for target in [
			StorageLayoutTarget::Hlsl,
			StorageLayoutTarget::Msl,
			StorageLayoutTarget::GlslScalar,
		] {

			assert_eq!(reflected_storage_buffer_stride_for_target(&members, target), Ok(20));
		}

		let uv_array = vec![besl::Node::array(
			"uvs",
			root.get_child("vec2f16").expect("Expected vec2f16"),
			2,
		)];
		for target in [
			StorageLayoutTarget::Hlsl,
			StorageLayoutTarget::Msl,
			StorageLayoutTarget::GlslScalar,
		] {

			assert_eq!(reflected_storage_buffer_stride_for_target(&uv_array, target), Ok(4));
		}
	}

	#[test]
	fn flattened_narrow_scalar_arrays_use_the_emitted_element_width() {
		let root = besl::Node::root();
		let u8_type = root.get_child("u8").expect("Expected u8");
		let u16_type = root.get_child("u16").expect("Expected u16");
		let bytes = vec![besl::Node::array("bytes", u8_type, 8)];
		let words = vec![besl::Node::array("words", u16_type, 8)];

		for (target, byte_stride, word_stride) in [
			(StorageLayoutTarget::Hlsl, 4, 4),
			(StorageLayoutTarget::Msl, 1, 2),
			(StorageLayoutTarget::GlslScalar, 1, 2),
		] {

			assert_eq!(reflected_storage_buffer_stride_for_target(&bytes, target), Ok(byte_stride));
			assert_eq!(reflected_storage_buffer_stride_for_target(&words, target), Ok(word_stride));
		}
	}

	#[test]
	fn matrix_storage_layouts_cover_all_besl_matrix_types() {
		let mut root = besl::Node::root();
		let vec2f = root.get_child("vec2f").expect("Expected vec2f");
		let vec3f = root.get_child("vec3f").expect("Expected vec3f");
		let mat2f = root.add_child(
			besl::Node::r#struct(
				"mat2f",
				vec![
					besl::Node::member("x", vec2f.clone()).into(),
					besl::Node::member("y", vec2f).into(),
				],
			)
			.into(),
		);
		let mat3f = root.add_child(
			besl::Node::r#struct(
				"mat3f",
				vec![
					besl::Node::member("x", vec3f.clone()).into(),
					besl::Node::member("y", vec3f.clone()).into(),
					besl::Node::member("z", vec3f).into(),
				],
			)
			.into(),
		);
		let matrices = [
			("mat2f", mat2f),
			("mat3f", mat3f),
			("mat4f", root.get_child("mat4f").expect("Expected mat4f")),
			("mat4x3f", root.get_child("mat4x3f").expect("Expected mat4x3f")),
		];

		for (target, expected) in [
			(StorageLayoutTarget::Hlsl, [(16, 4), (36, 4), (64, 4), (48, 4)]),
			(StorageLayoutTarget::Msl, [(16, 8), (48, 16), (64, 16), (48, 4)]),
			(StorageLayoutTarget::GlslScalar, [(16, 4), (36, 4), (64, 4), (48, 4)]),
		] {
			for ((type_name, matrix), (size, alignment)) in matrices.iter().zip(expected) {
				let layout = reflected_storage_type_layout(matrix, target, false, &mut HashSet::new())
					.unwrap_or_else(|error| panic!("Expected '{type_name}' layout for {target:?}: {error}"));

				assert_eq!(layout, StorageLayout { size, alignment });
			}
		}
	}

	#[test]
	fn nested_struct_arrays_apply_member_and_tail_alignment() {
		let mut root = besl::Node::root();
		let vec3f = root.get_child("vec3f").expect("Expected vec3f");
		let u8_type = root.get_child("u8").expect("Expected u8");
		let u32_type = root.get_child("u32").expect("Expected u32");
		let mixed = root.add_child(
			besl::Node::r#struct(
				"Mixed",
				vec![
					besl::Node::member("position", vec3f.clone()).into(),
					besl::Node::member("tag", u8_type).into(),
				],
			)
			.into(),
		);
		let wrapper = vec![
			besl::Node::array("items", mixed, 2),
			besl::Node::member("tail", u32_type).into(),
		];
		let scalar_position = vec![besl::Node::member("position", vec3f.clone()).into()];
		let flattened_positions = vec![besl::Node::array("positions", vec3f, 8)];

		assert_eq!(
			reflected_storage_buffer_stride_for_target(&wrapper, StorageLayoutTarget::Hlsl),
			Ok(36)
		);
		assert_eq!(
			reflected_storage_buffer_stride_for_target(&wrapper, StorageLayoutTarget::Msl),
			Ok(80)
		);
		assert_eq!(
			reflected_storage_buffer_stride_for_target(&wrapper, StorageLayoutTarget::GlslScalar),
			Ok(36)
		);

		// Metal emits packed_float3 only for the direct array member. Direct
		// scalar members and fields nested inside Mixed retain native float3.

		assert_eq!(
			reflected_storage_buffer_stride_for_target(&scalar_position, StorageLayoutTarget::Hlsl),
			Ok(12)
		);
		assert_eq!(
			reflected_storage_buffer_stride_for_target(&scalar_position, StorageLayoutTarget::Msl),
			Ok(16)
		);
		assert_eq!(
			reflected_storage_buffer_stride_for_target(&scalar_position, StorageLayoutTarget::GlslScalar),
			Ok(12)
		);
		for target in [
			StorageLayoutTarget::Hlsl,
			StorageLayoutTarget::Msl,
			StorageLayoutTarget::GlslScalar,
		] {

			assert_eq!(
				reflected_storage_buffer_stride_for_target(&flattened_positions, target),
				Ok(12)
			);
		}
	}

	#[test]
	fn visibility_storage_structs_match_the_backend_abi() {
		let mut root = besl::Node::root();
		let f32_type = root.get_child("f32").expect("Expected f32");
		let u32_type = root.get_child("u32").expect("Expected u32");
		let vec2f = root.get_child("vec2f").expect("Expected vec2f");
		let vec4f = root.get_child("vec4f").expect("Expected vec4f");
		let packed_vec4f = root.get_child("packed_vec4f").expect("Expected packed_vec4f");
		let vec2u16 = root.get_child("vec2u16").expect("Expected vec2u16");
		let mat4f = root.get_child("mat4f").expect("Expected mat4f");
		let mat4x3f = root.get_child("mat4x3f").expect("Expected mat4x3f");

		let mesh = root.add_child(
			besl::Node::r#struct(
				"Mesh",
				vec![
					besl::Node::member("model", mat4x3f.clone()).into(),
					besl::Node::member("material_index", u32_type.clone()).into(),
					besl::Node::member("base_vertex_index", u32_type.clone()).into(),
					besl::Node::member("base_primitive_index", u32_type.clone()).into(),
					besl::Node::member("base_triangle_index", u32_type.clone()).into(),
					besl::Node::member("base_meshlet_index", u32_type.clone()).into(),
					besl::Node::member("meshlet_count", u32_type.clone()).into(),
					besl::Node::member("skinned_base_vertex_index", u32_type.clone()).into(),
					besl::Node::member("padding0", u32_type.clone()).into(),
				],
			)
			.into(),
		);
		let view = root.add_child(
			besl::Node::r#struct(
				"View",
				vec![
					besl::Node::member("view", mat4x3f.clone()).into(),
					besl::Node::member("view_projection", mat4f.clone()).into(),
					besl::Node::member("inverse_view", mat4x3f).into(),
					besl::Node::member("fov", vec2f.clone()).into(),
					besl::Node::member("near", f32_type.clone()).into(),
					besl::Node::member("far", f32_type).into(),
				],
			)
			.into(),
		);
		let meshlet = root.add_child(
			besl::Node::r#struct(
				"Meshlet",
				vec![
					besl::Node::member("primitive_offset", u32_type.clone()).into(),
					besl::Node::member("triangle_offset", u32_type.clone()).into(),
					besl::Node::member("primitive_count", u32_type.clone()).into(),
					besl::Node::member("triangle_count", u32_type.clone()).into(),
					besl::Node::member("center_radius", packed_vec4f.clone()).into(),
					besl::Node::member("cone_apex_cutoff", packed_vec4f).into(),
					besl::Node::member("cone_axis", vec2u16).into(),
				],
			)
			.into(),
		);
		let light = root.add_child(
			besl::Node::r#struct(
				"Light",
				vec![
					besl::Node::member("position", vec4f.clone()).into(),
					besl::Node::member("color", vec4f.clone()).into(),
					besl::Node::member("direction", vec4f).into(),
					besl::Node::member("cone_cosines", vec2f).into(),
					besl::Node::member("type", u32_type.clone()).into(),
					besl::Node::array("cascades", u32_type.clone(), 8),
					besl::Node::member("_padding", u32_type.clone()).into(),
				],
			)
			.into(),
		);

		let mesh_buffer = vec![besl::Node::array("meshes", mesh, 1024)];
		let view_buffer = vec![besl::Node::array("views", view, 8)];
		let meshlet_buffer = vec![besl::Node::array("meshlets", meshlet, 1024)];
		let lighting_buffer = vec![
			besl::Node::member("light_count", u32_type.clone()).into(),
			besl::Node::array("_light_count_padding", u32_type, 3),
			besl::Node::array("lights", light, 16),
		];

		for (target, mesh_stride, view_stride) in [
			(StorageLayoutTarget::Hlsl, 80, 176),
			(StorageLayoutTarget::Msl, 80, 176),
			(StorageLayoutTarget::GlslScalar, 80, 176),
		] {

			assert_eq!(
				reflected_storage_buffer_stride_for_target(&mesh_buffer, target),
				Ok(mesh_stride)
			);
			assert_eq!(
				reflected_storage_buffer_stride_for_target(&view_buffer, target),
				Ok(view_stride)
			);
			assert_eq!(reflected_storage_buffer_stride_for_target(&meshlet_buffer, target), Ok(52));
			assert_eq!(reflected_storage_buffer_stride_for_target(&lighting_buffer, target), Ok(1552));
		}
	}

	#[test]
	fn sampled_texture_shapes_and_descriptor_counts_are_preserved() {
		let root = besl::Node::root();
		let void = root.get_child("void").expect("Expected the built-in void type");
		let main: besl::NodeReference = besl::Node::function(
			"main",
			Vec::new(),
			void,
			vec![besl::Node::binding_array(
				"volumes",
				besl::BindingTypes::CombinedImageSampler {
					format: "Texture3D".to_string(),
				},
				0,
				true,
				false,
				3,
			)
			.into()],
		)
		.into();

		let bindings = ProgramEvaluation::from_main(&main)
			.expect("Expected sampled binding metadata to evaluate")
			.into_bindings();

		assert_eq!(bindings[0].count, 3);
		assert_eq!(
			bindings[0].kind,
			BindingKind::CombinedImageSampler {
				view: TextureView::Texture3D
			}
		);
	}

	#[test]
	fn bindings_from_program() {
		let script = r#"
		main: fn () -> void {
			buff;
			image;
			texture;
		}
		"#;

		let mut root_node = besl::Node::root();

		let float_type = root_node.get_child("f32").unwrap();

		root_node.add_children(vec![
			besl::Node::binding(
				"buff",
				besl::BindingTypes::Buffer {
					members: vec![besl::Node::member("member", float_type).into()],
				},
				0,
				true,
				true,
			)
			.into(),
			besl::Node::binding(
				"image",
				besl::BindingTypes::Image {
					format: "r8".to_string(),
				},
				1,
				false,
				true,
			)
			.into(),
			besl::Node::binding(
				"texture",
				besl::BindingTypes::CombinedImageSampler { format: "".to_string() },
				2,
				true,
				false,
			)
			.into(),
		]);

		let program_node = besl::compile_to_besl(&script, Some(root_node)).unwrap();
		let evaluation = ProgramEvaluation::from_program(&program_node).expect("Failed to evaluate program");
		let bindings = evaluation.bindings();

		assert_eq!(bindings.len(), 3);
	}

	#[test]
	fn program_reflection_keeps_an_unreachable_declared_binding() {
		let mut root = besl::Node::root();
		let f32_type = root.get_child("f32").expect("Expected f32 type");
		root.add_child(
			besl::Node::binding(
				"unreachable",
				besl::BindingTypes::Buffer {
					members: vec![besl::Node::member("value", f32_type).into()],
				},
				0,
				true,
				false,
			)
			.into(),
		);
		let program = besl::compile_to_besl("main: fn() -> void { return; }", Some(root))
			.expect("Expected unreachable binding fixture to link");
		let main = program
			.get_main()
			.expect("Expected unreachable binding fixture main function");

		let program_bindings = ProgramEvaluation::from_program(&program)
			.expect("Expected full-program reflection")
			.into_bindings();
		let main_bindings = ProgramEvaluation::from_main(&main)
			.expect("Expected reachable-main reflection")
			.into_bindings();

		assert_eq!(program_bindings.len(), 1);
		assert_eq!(program_bindings[0].name, "unreachable");
		assert!(main_bindings.is_empty());
	}

	#[test]
	fn reflection_culls_a_binding_used_only_by_a_dead_local() {
		let mut root = besl::Node::root();
		let f32_type = root.get_child("f32").expect("Expected f32 type");
		root.add_child(
			besl::Node::binding(
				"values",
				besl::BindingTypes::Buffer {
					members: vec![besl::Node::member("value", f32_type).into()],
				},
				0,
				true,
				false,
			)
			.into(),
		);

		let program = besl::compile_to_besl(
			r#"
			main: fn() -> void {
				let ignored: f32 = values.value;
				return;
			}
		"#,
			Some(root),
		)
		.expect("Expected dead binding fixture to link");
		let main = program.get_main().expect("Expected dead binding fixture main function");

		let evaluation = ProgramEvaluation::from_main(&main).expect("Expected optimized reflection");

		assert!(evaluation.bindings().is_empty(), "Dead local binding reached reflection");
	}

	#[test]
	fn opacity_is_opaque_when_non_local_output_is_referenced() {
		let script = r#"
		main: fn () -> void {
			output;
		}
		"#;

		let mut root_node = besl::Node::root();
		let vec3f_type = root_node.get_child("vec3f").unwrap();
		root_node.add_child(besl::Node::output("output", vec3f_type, 0).into());

		let program_node = besl::compile_to_besl(script, Some(root_node)).unwrap();
		let evaluation = ProgramEvaluation::from_program(&program_node).expect("Failed to evaluate program");

		assert_eq!(evaluation.opacity(), OpacityEvaluation::Opaque);
	}

	#[test]
	fn opacity_is_unknown_when_output_is_shadowed_locally() {
		let script = r#"
		main: fn () -> void {
			let output: vec3f = vec3f(1.0, 0.0, 0.0);
			output;
		}
		"#;

		let mut root_node = besl::Node::root();
		let vec3f_type = root_node.get_child("vec3f").unwrap();
		root_node.add_child(besl::Node::output("output", vec3f_type, 0).into());

		let program_node = besl::compile_to_besl(script, Some(root_node)).unwrap();
		let evaluation = ProgramEvaluation::from_program(&program_node).expect("Failed to evaluate program");

		assert_eq!(evaluation.opacity(), OpacityEvaluation::Unknown);
	}

	#[test]
	fn opacity_is_unknown_when_main_contains_raw_code() {
		let mut root_node = besl::Node::root();
		let return_type = root_node.get_child("void").unwrap();
		let main = besl::Node::function(
			"main",
			Vec::new(),
			return_type,
			vec![besl::Node::glsl("output = vec3f(1.0, 0.0, 0.0);".to_string(), Vec::new(), Vec::new()).into()],
		);
		root_node.add_child(main.into());

		let program_node: besl::NodeReference = root_node.into();
		let evaluation = ProgramEvaluation::from_program(&program_node).expect("Failed to evaluate program");

		assert_eq!(evaluation.opacity(), OpacityEvaluation::Unknown);
	}

	#[test]
	fn opacity_is_non_opaque_when_output_vec4f_w_is_not_one() {
		let script = r#"
		main: fn () -> void {
			output = vec4f(1.0, 0.0, 0.0, 0.5);
		}
		"#;

		let mut root_node = besl::Node::root();
		let vec4f_type = root_node.get_child("vec4f").unwrap();
		root_node.add_child(besl::Node::output("output", vec4f_type, 0).into());

		let program_node = besl::compile_to_besl(script, Some(root_node)).unwrap();
		let evaluation = ProgramEvaluation::from_program(&program_node).expect("Failed to evaluate program");

		assert_eq!(evaluation.opacity(), OpacityEvaluation::NonOpaque);
	}

	#[test]
	fn opacity_is_opaque_when_output_vec4f_w_is_one() {
		let script = r#"
		main: fn () -> void {
			output = vec4f(1.0, 0.0, 0.0, 1.0);
		}
		"#;

		let mut root_node = besl::Node::root();
		let vec4f_type = root_node.get_child("vec4f").unwrap();
		root_node.add_child(besl::Node::output("output", vec4f_type, 0).into());

		let program_node = besl::compile_to_besl(script, Some(root_node)).unwrap();
		let evaluation = ProgramEvaluation::from_program(&program_node).expect("Failed to evaluate program");

		assert_eq!(evaluation.opacity(), OpacityEvaluation::Opaque);
	}

	#[test]
	fn opacity_vec4f_with_vec3f_first_param_uses_w_for_opacity() {
		fn evaluate(w: &str) -> OpacityEvaluation {
			let mut root_node = besl::Node::root();
			let void_type = root_node.get_child("void").unwrap();
			let vec3f_type = root_node.get_child("vec3f").unwrap();
			let vec4f_type = root_node.get_child("vec4f").unwrap();

			let output_node: besl::NodeReference = besl::Node::output("output", vec4f_type.clone(), 0).into();

			let vec3f_call = besl::Node::expression(besl::Expressions::FunctionCall {
				function: vec3f_type,
				parameters: vec![
					besl::Node::expression(besl::Expressions::Literal {
						value: "1.0".to_string(),
					})
					.into(),
					besl::Node::expression(besl::Expressions::Literal {
						value: "0.0".to_string(),
					})
					.into(),
					besl::Node::expression(besl::Expressions::Literal {
						value: "0.0".to_string(),
					})
					.into(),
				],
			})
			.into();

			let vec4f_call = besl::Node::expression(besl::Expressions::FunctionCall {
				function: vec4f_type,
				parameters: vec![
					vec3f_call,
					besl::Node::expression(besl::Expressions::Literal { value: w.to_string() }).into(),
				],
			})
			.into();

			let output_member = besl::Node::expression(besl::Expressions::Member {
				name: "output".to_string(),
				source: output_node.clone(),
			})
			.into();

			let assignment = besl::Node::expression(besl::Expressions::Operator {
				operator: besl::Operators::Assignment,
				left: output_member,
				right: vec4f_call,
			})
			.into();

			let main = besl::Node::function("main", Vec::new(), void_type, vec![assignment]).into();

			root_node.add_children(vec![output_node, main]);

			let program_node: besl::NodeReference = root_node.into();
			let evaluation = ProgramEvaluation::from_program(&program_node).expect("Failed to evaluate program");
			evaluation.opacity()
		}

		assert_eq!(evaluate("1.0"), OpacityEvaluation::Opaque);
		assert_eq!(evaluate("0.5"), OpacityEvaluation::NonOpaque);
	}
}
