mod byte_windows;
mod opacity;
mod push_constant_layout;
mod reflection;

#[cfg(test)]
use std::collections::HashSet;

pub(crate) use byte_windows::{ByteWindow, byte_windows};
pub use opacity::OpacityEvaluation;
pub use reflection::{BindingKind, BindingUsage, ProgramEvaluation, RayPayload, SetUsage, TextureView};
pub use reflection::{FNV1A_OFFSET_BASIS, fnv1a};
pub(crate) use reflection::{collect_bindings, collect_set_nodes};

/// Reflects the descriptor set the layer `layer` declares, such as `::byte_engine::layouts::scene::Scene`, without a
/// shader that takes it.
///
/// `read` returns the source of a module file by its path from the asset root, such as `byte_engine/layouts/scene.besl`,
/// or `None` when it doesn't exist. The set has every binding marked as used.
///
/// A renderer uses it to build a descriptor set it shares with its shaders from the same source the shaders are baked
/// from. Next, persist the set with [`crate::resources::material::SetInterface::new`]; its identity equals the
/// identity of the set every shader that takes the layer was baked with.
pub fn reflect_layer(layer: &str, read: impl Fn(&str) -> Option<String>) -> Result<SetUsage, String> {
	// Reflecting a layer is a one-off link while a renderer builds its layouts, so this link owns its built-ins.
	let builtins = besl::Builtins::new();
	let failed = |error: besl::CompilationError| {
		format!(
			"Failed to load layer `{layer}` ({error:?}). The most likely cause is a misspelled layer path or invalid module source."
		)
	};

	// A program that only names the layer loads its module and the modules that one names.
	let source = format!("use {layer};");
	let modules = besl::ModuleLoader::load(&source, read).map_err(failed)?;

	let mut program = besl::parse(&source).map_err(failed)?;
	besl::attach_modules(&mut program, &modules).map_err(failed)?;
	let program = besl::lex(&builtins, program).map_err(failed)?;

	reflection::collect_sets(&program, &program.root())?
		.into_iter()
		.find(|set| set.layout.as_deref() == Some(layer))
		.ok_or_else(|| {
			format!(
				"Layer `{layer}` declares no descriptors. The most likely cause is a layer that holds only a push constant block or tile targets, which form no descriptor set. Reflect a layer that declares descriptors."
			)
		})
}

#[cfg(test)]
use reflection::{
	StorageLayout, StorageLayoutTarget, checked_align_up, primitive_storage_layout, reflected_array_buffer_stride_for_target,
	reflected_storage_buffer_stride_for_target, reflected_storage_type_layout,
};

#[cfg(test)]
mod tests {
	use super::*;
	use crate::shader::generator;
	use crate::shader::generator::tests::builtin;

	fn assert_builtin_layout(program: &besl::Program, target: StorageLayoutTarget, type_name: &str, expected: StorageLayout) {
		let r#type = builtin(program, type_name);
		let layout = reflected_storage_type_layout(program, &r#type, target, &mut besl::NodeSet::default())
			.unwrap_or_else(|error| panic!("Expected '{type_name}' layout for {target:?}: {error}"));

		assert_eq!(layout, expected, "Unexpected '{type_name}' layout for {target:?}");
	}

	/// Adds a struct named `name` to the root of `program` whose members have the given names and types.
	fn add_struct(program: &mut besl::Program, name: &str, members: &[(&str, besl::NodeId)]) -> besl::NodeId {
		let fields = members
			.iter()
			.map(|(member, r#type)| {
				let member = program.intern(member);
				program.add(besl::Node::member(member, *r#type))
			})
			.collect();
		let name = program.intern(name);
		program.add_to_root(besl::Node::r#struct(name, fields))
	}

	#[test]
	fn binding_metadata_is_sorted_and_classified() {
		let builtins = besl::Builtins::new();
		let mut program = generator::tests::bindings(&builtins);
		let main = program.get_main().expect("Expected main function");

		let evaluation = ProgramEvaluation::from_main(&mut program, &main).expect("Failed to evaluate program");
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
						sampler: None,
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
		let builtins = besl::Builtins::new();
		let script = "main: fn () -> void { positions; indices; lighting; }";
		let mut program = besl::Program::new(&builtins);
		let vec2f = builtin(&program, "vec2f");
		let vec3f = builtin(&program, "vec3f");
		let u8_type = builtin(&program, "u8");
		let u16_type = builtin(&program, "u16");
		let u32_type = builtin(&program, "u32");
		let mut light_fields = [
			("position", vec3f),
			("color", vec3f),
			("direction", vec3f),
			("cone_cosines", vec2f),
			("light_type", u8_type),
		]
		.map(|(name, r#type)| {
			let name = program.intern(name);
			program.add(besl::Node::member(name, r#type))
		})
		.to_vec();
		let cascades_name = program.intern("cascades");
		light_fields.push(program.add(besl::Node::array(cascades_name, u32_type, 8)));
		let light_name = program.intern("Light");
		let light = program.add_to_root(besl::Node::r#struct(light_name, light_fields));

		let positions_name = program.intern("positions");
		let positions = program.add(besl::Node::array(positions_name, vec3f, 16));
		program.add_to_root(besl::Node::binding(
			positions_name,
			besl::BindingTypes::Buffer {
				members: vec![positions],
			},
			0,
			true,
			false,
		));
		let indices_name = program.intern("indices");
		let indices = program.add(besl::Node::array(indices_name, u16_type, 16));
		program.add_to_root(besl::Node::binding(
			indices_name,
			besl::BindingTypes::Buffer { members: vec![indices] },
			1,
			true,
			false,
		));
		let light_count_name = program.intern("light_count");
		let lights_name = program.intern("lights");
		let lighting = vec![
			program.add(besl::Node::member(light_count_name, u32_type)),
			program.add(besl::Node::array(lights_name, light, 16)),
		];
		let lighting_name = program.intern("lighting");
		program.add_to_root(besl::Node::binding(
			lighting_name,
			besl::BindingTypes::Buffer { members: lighting },
			2,
			true,
			false,
		));

		let mut program = besl::compile_to_besl(script, program).expect("Expected stride-reflection shader to link");
		let evaluation = ProgramEvaluation::from_program(&mut program).expect("Expected storage-buffer strides to reflect");
		let strides = evaluation
			.bindings()
			.iter()
			.map(|binding| binding.buffer_stride)
			.collect::<Vec<_>>();

		let expected = if cfg!(target_os = "windows") {
			vec![Some(12), Some(4), Some(1284)]
		} else {
			vec![Some(12), Some(2), Some(1284)]
		};

		assert_eq!(strides, expected);
	}

	#[test]
	fn reflection_rejects_a_push_constant_that_dx12_would_read_at_other_offsets() {
		let builtins = besl::Builtins::new();
		let mut program = besl::compile_to_besl(
			r#"
			push_constant: push_constant {
				scale: f32,
				bias: f32,
				offset: vec3f,
			}
			main: fn () -> void { push_constant.offset; }
			"#,
			besl::Program::new(&builtins),
		)
		.expect("Expected the push-constant program to link");

		let error =
			ProgramEvaluation::from_program(&mut program).expect_err("Expected the push-constant layout to be rejected");

		assert!(
			error.contains("`offset` starts at byte 8 on the CPU and Vulkan, 8 on Metal, and 16 on DX12"),
			"{error}"
		);
	}

	#[test]
	fn packed_bool_and_byte_records_reflect_one_stride_on_every_backend() {
		let builtins = besl::Builtins::new();
		let program = besl::compile_to_besl(
			r#"
			Mixed: struct { visible: bool, selected: bool, id: u32, hovered: bool }
			Many: struct {
				f0: bool, f1: bool, f2: bool, f3: bool, f4: bool, f5: bool, f6: bool, f7: bool, f8: bool,
				f9: bool, f10: bool, f11: bool, f12: bool, f13: bool, f14: bool, f15: bool, f16: bool,
				tail: u16,
			}
			Bytes: struct { mode: u8, flag: bool, layer: u16, count: u32 }
			main: fn () -> void {}
			"#,
			besl::Program::new(&builtins),
		)
		.expect("Expected the bool records to link");

		for (name, stride) in [("Mixed", 12), ("Many", 6), ("Bytes", 8)] {
			let record = program.get_child(program.root(), name).expect("Expected the record struct");
			for target in [
				StorageLayoutTarget::Hlsl,
				StorageLayoutTarget::Msl,
				StorageLayoutTarget::GlslScalar,
			] {
				assert_eq!(
					reflected_array_buffer_stride_for_target(&program, &record, target),
					Ok(stride),
					"{name} on {target:?}"
				);
			}
		}
	}

	#[test]
	fn reflection_rejects_bytes_dx12_cannot_window() {
		let builtins = besl::Builtins::new();
		for (records, expected) in [
			(
				"Record: struct { a: bool, mode: u8, layer: u8 }",
				"`u8` member `layer` ends a layout of 3 bytes",
			),
			(
				"Inner: struct { x: u8, y: u8 } Record: struct { flag: u8, inner: Inner, tail: u8, pad: u8 }",
				"`u8` member `flag` shares 2 bytes with member `inner`",
			),
			(
				"Record: struct { flag: u8, values: u8[3] }",
				"`u8` member `flag` shares 2 bytes with member `values`",
			),
		] {
			let source = format!(
				"{records} records: descriptor<{{ type: Record[], binding: 0, access: read }}>; main: fn () -> void {{ records; }}"
			);
			let mut program = besl::compile_to_besl(&source, besl::Program::new(&builtins)).expect("Expected the record program to link");

			let error = ProgramEvaluation::from_program(&mut program).expect_err("Expected the record layout to be rejected");

			assert!(error.contains(expected), "{records}: {error}");
			assert!(error.contains("reference/besl/interface#stored-records"), "{error}");
		}
	}

	#[test]
	fn reflection_accepts_packed_bools_in_a_push_constant() {
		let builtins = besl::Builtins::new();
		let mut program = besl::compile_to_besl(
			r#"
			push_constant: push_constant {
				first_slot: u32,
				show_normals: bool,
				show_albedo: bool,
				mode: u8,
			}
			main: fn () -> void { push_constant.show_albedo; }
			"#,
			besl::Program::new(&builtins),
		)
		.expect("Expected the push-constant program to link");

		ProgramEvaluation::from_program(&mut program).expect("Expected packed bools to reflect in a push constant");
	}

	#[test]
	fn runtime_storage_buffer_reflects_its_element_stride() {
		let builtins = besl::Builtins::new();
		let mut program = besl::Program::new(&builtins);
		let vec3f = builtin(&program, "vec3f");
		let u32_type = builtin(&program, "u32");
		let instance = add_struct(&mut program, "Instance", &[("position", vec3f), ("sprite_id", u32_type)]);

		assert_eq!(
			reflected_array_buffer_stride_for_target(&program, &instance, StorageLayoutTarget::Hlsl),
			Ok(16)
		);
		assert_eq!(
			reflected_array_buffer_stride_for_target(&program, &instance, StorageLayoutTarget::Msl),
			Ok(16)
		);
		assert_eq!(
			reflected_array_buffer_stride_for_target(&program, &instance, StorageLayoutTarget::GlslScalar),
			Ok(16)
		);

		let instances_name = program.intern("instances");
		program.add_to_root(besl::Node::binding(
			instances_name,
			besl::BindingTypes::BufferArray {
				element: instance,
				fixed: None,
			},
			0,
			true,
			false,
		));
		let mut program = besl::compile_to_besl("main: fn () -> void { return; }", program)
			.expect("Expected runtime storage-buffer reflection fixture to link");
		let bindings = ProgramEvaluation::from_program(&mut program)
			.expect("Expected runtime storage-buffer reflection")
			.into_bindings();

		assert_eq!(bindings.len(), 1);
		assert_eq!(bindings[0].kind, BindingKind::StorageBuffer);
		assert_eq!(bindings[0].count, 1);
		assert_eq!(bindings[0].buffer_stride, Some(16));
	}

	#[test]
	fn primitive_storage_layouts_follow_each_emitted_backend_type() {
		let builtins = besl::Builtins::new();
		let program = besl::Program::new(&builtins);

		// Metal stores vectors as packed types, so every backend shares their scalar-aligned layout.
		for target in [
			StorageLayoutTarget::Hlsl,
			StorageLayoutTarget::Msl,
			StorageLayoutTarget::GlslScalar,
		] {
			for (type_name, size, alignment) in [
				// DX12 stores `u8` members in 2-byte windows, so a `u8` takes one byte everywhere.
				("u8", 1, 1),
				("u16", 2, 2),
				("u32", 4, 4),
				("f16", 2, 2),
				("vec2u16", 4, 2),
				("vec4u16", 8, 2),
				("vec2f16", 4, 2),
				("vec3f16", 6, 2),
				("vec4f16", 8, 2),
				("vec2f", 8, 4),
				("vec3f", 12, 4),
				("vec4f", 16, 4),
			] {
				assert_builtin_layout(&program, target, type_name, StorageLayout { size, alignment });
			}
		}
	}

	#[test]
	fn direct_f16_storage_members_preserve_the_packed_vm_layout() {
		let builtins = besl::Builtins::new();
		let mut program = besl::Program::new(&builtins);
		let members = [
			("scalar", "f16"),
			("uv", "vec2f16"),
			("normal", "vec3f16"),
			("color", "vec4f16"),
		]
		.map(|(name, type_name)| {
			let r#type = builtin(&program, type_name);
			let name = program.intern(name);
			program.add(besl::Node::member(name, r#type))
		});

		for target in [
			StorageLayoutTarget::Hlsl,
			StorageLayoutTarget::Msl,
			StorageLayoutTarget::GlslScalar,
		] {
			assert_eq!(reflected_storage_buffer_stride_for_target(&program, &members, target), Ok(20));
		}

		let uv = builtin(&program, "vec2f16");
		for target in [
			StorageLayoutTarget::Hlsl,
			StorageLayoutTarget::Msl,
			StorageLayoutTarget::GlslScalar,
		] {
			assert_eq!(reflected_array_buffer_stride_for_target(&program, &uv, target), Ok(4));
		}
	}

	#[test]
	fn scalar_and_vector_array_buffers_use_the_emitted_element_width() {
		let builtins = besl::Builtins::new();
		let program = besl::Program::new(&builtins);
		let element = |name| builtin(&program, name);

		for (target, byte, word, position) in [
			(StorageLayoutTarget::Hlsl, 4, 4, 12),
			(StorageLayoutTarget::Msl, 1, 2, 12),
			(StorageLayoutTarget::GlslScalar, 1, 2, 12),
		] {
			for (name, stride) in [("u8", byte), ("u16", word), ("vec3f", position)] {
				assert_eq!(
					reflected_array_buffer_stride_for_target(&program, &element(name), target),
					Ok(stride),
					"{name} array stride for {target:?}"
				);
			}
		}
	}

	#[test]
	fn hlsl_native_u16_record_stride_matches_the_emitted_struct() {
		let builtins = besl::Builtins::new();
		let mut program = besl::Program::new(&builtins);
		let u16_type = builtin(&program, "u16");
		let pair = add_struct(&mut program, "Pair", &[("low", u16_type), ("high", u16_type)]);
		assert_eq!(
			reflected_array_buffer_stride_for_target(&program, &pair, StorageLayoutTarget::Hlsl),
			Ok(4)
		);
	}

	#[test]
	fn matrix_storage_layouts_cover_all_besl_matrix_types() {
		let builtins = besl::Builtins::new();
		let mut program = besl::Program::new(&builtins);
		let vec2f = builtin(&program, "vec2f");
		let vec3f = builtin(&program, "vec3f");
		let mat2f = add_struct(&mut program, "mat2f", &[("x", vec2f), ("y", vec2f)]);
		let mat3f = add_struct(&mut program, "mat3f", &[("x", vec3f), ("y", vec3f), ("z", vec3f)]);
		let matrices = [
			("mat2f", mat2f),
			("mat3f", mat3f),
			("mat4f", builtin(&program, "mat4f")),
			("mat4x3f", builtin(&program, "mat4x3f")),
		];

		for (target, expected) in [
			(StorageLayoutTarget::Hlsl, [(16, 4), (36, 4), (64, 4), (48, 4)]),
			(StorageLayoutTarget::Msl, [(16, 8), (48, 16), (64, 16), (48, 4)]),
			(StorageLayoutTarget::GlslScalar, [(16, 4), (36, 4), (64, 4), (48, 4)]),
		] {
			for ((type_name, matrix), (size, alignment)) in matrices.iter().zip(expected) {
				let layout = reflected_storage_type_layout(&program, matrix, target, &mut besl::NodeSet::default())
					.unwrap_or_else(|error| panic!("Expected '{type_name}' layout for {target:?}: {error}"));

				assert_eq!(layout, StorageLayout { size, alignment });
			}
		}
	}

	#[test]
	fn nested_struct_arrays_apply_member_and_tail_alignment() {
		let builtins = besl::Builtins::new();
		let mut program = besl::Program::new(&builtins);
		let vec3f = builtin(&program, "vec3f");
		let u8_type = builtin(&program, "u8");
		let u32_type = builtin(&program, "u32");
		let mixed = add_struct(&mut program, "Mixed", &[("position", vec3f), ("tag", u8_type)]);
		let items_name = program.intern("items");
		let tail_name = program.intern("tail");
		let wrapper = vec![
			program.add(besl::Node::array(items_name, mixed, 2)),
			program.add(besl::Node::member(tail_name, u32_type)),
		];
		let position_name = program.intern("position");
		let scalar_position = vec![program.add(besl::Node::member(position_name, vec3f))];

		assert_eq!(
			reflected_storage_buffer_stride_for_target(&program, &wrapper, StorageLayoutTarget::Hlsl),
			Ok(36)
		);
		assert_eq!(
			reflected_storage_buffer_stride_for_target(&program, &wrapper, StorageLayoutTarget::Msl),
			Ok(36)
		);
		assert_eq!(
			reflected_storage_buffer_stride_for_target(&program, &wrapper, StorageLayoutTarget::GlslScalar),
			Ok(36)
		);

		// Metal emits packed_float3 for every vec3f member, so direct members and fields nested inside Mixed keep
		// the 12-byte layout of the other backends.
		assert_eq!(
			reflected_storage_buffer_stride_for_target(&program, &scalar_position, StorageLayoutTarget::Hlsl),
			Ok(12)
		);
		assert_eq!(
			reflected_storage_buffer_stride_for_target(&program, &scalar_position, StorageLayoutTarget::Msl),
			Ok(12)
		);
		assert_eq!(
			reflected_storage_buffer_stride_for_target(&program, &scalar_position, StorageLayoutTarget::GlslScalar),
			Ok(12)
		);
	}

	#[test]
	fn visibility_storage_structs_match_the_backend_abi() {
		let builtins = besl::Builtins::new();
		let mut program = besl::Program::new(&builtins);
		let f32_type = builtin(&program, "f32");
		let u32_type = builtin(&program, "u32");
		let vec2f = builtin(&program, "vec2f");
		let vec3f = builtin(&program, "vec3f");
		let vec4f = builtin(&program, "vec4f");
		let vec2u16 = builtin(&program, "vec2u16");
		let mat4f = builtin(&program, "mat4f");
		let mat4x3f = builtin(&program, "mat4x3f");

		let mesh = add_struct(
			&mut program,
			"Mesh",
			&[
				("model", mat4x3f),
				("material_index", u32_type),
				("base_vertex_index", u32_type),
				("base_primitive_index", u32_type),
				("base_triangle_index", u32_type),
				("base_meshlet_index", u32_type),
				("meshlet_count", u32_type),
				("skinned_base_vertex_index", u32_type),
				("flags", u32_type),
			],
		);
		let view = add_struct(
			&mut program,
			"View",
			&[
				("view", mat4x3f),
				("view_projection", mat4f),
				("inverse_view", mat4x3f),
				("fov", vec2f),
				("near", f32_type),
				("far", f32_type),
			],
		);
		let meshlet = add_struct(
			&mut program,
			"Meshlet",
			&[
				("primitive_offset", u32_type),
				("triangle_offset", u32_type),
				("primitive_count", u32_type),
				("triangle_count", u32_type),
				("center_radius", vec4f),
				("cone_apex_cutoff", vec4f),
				("cone_axis", vec2u16),
			],
		);
		let mut light_fields = [
			("position", vec3f),
			("color", vec3f),
			("direction", vec3f),
			("cone_cosines", vec2f),
			("type", u32_type),
		]
		.map(|(name, r#type)| {
			let name = program.intern(name);
			program.add(besl::Node::member(name, r#type))
		})
		.to_vec();
		let cascades_name = program.intern("cascades");
		light_fields.push(program.add(besl::Node::array(cascades_name, u32_type, 8)));
		let light_name = program.intern("Light");
		let light = program.add_to_root(besl::Node::r#struct(light_name, light_fields));

		let light_count_name = program.intern("light_count");
		let lights_name = program.intern("lights");
		let lighting_buffer = vec![
			program.add(besl::Node::member(light_count_name, u32_type)),
			program.add(besl::Node::array(lights_name, light, 16)),
		];

		for (target, mesh_stride, view_stride) in [
			(StorageLayoutTarget::Hlsl, 80, 176),
			(StorageLayoutTarget::Msl, 80, 176),
			(StorageLayoutTarget::GlslScalar, 80, 176),
		] {
			assert_eq!(
				reflected_array_buffer_stride_for_target(&program, &mesh, target),
				Ok(mesh_stride)
			);
			assert_eq!(
				reflected_array_buffer_stride_for_target(&program, &view, target),
				Ok(view_stride)
			);
			assert_eq!(reflected_array_buffer_stride_for_target(&program, &meshlet, target), Ok(52));
			assert_eq!(
				reflected_storage_buffer_stride_for_target(&program, &lighting_buffer, target),
				Ok(1284)
			);
		}
	}

	#[test]
	fn sampled_texture_shapes_and_descriptor_counts_are_preserved() {
		let builtins = besl::Builtins::new();
		let mut program = besl::Program::new(&builtins);
		let void = builtin(&program, "void");
		let volumes_name = program.intern("volumes");
		let volumes = program.add(besl::Node::binding_array(
			volumes_name,
			besl::BindingTypes::CombinedImageSampler {
				format: "Texture3D".to_string(),
				sampler: None,
			},
			0,
			true,
			false,
			3,
		));
		let main_name = program.intern("main");
		let main = program.add(besl::Node::function(main_name, Vec::new(), void, vec![volumes]));

		let bindings = ProgramEvaluation::from_main(&mut program, &main)
			.expect("Expected sampled binding metadata to evaluate")
			.into_bindings();

		assert_eq!(bindings[0].count, 3);
		assert_eq!(
			bindings[0].kind,
			BindingKind::CombinedImageSampler {
				view: TextureView::Texture3D,
				sampler: None,
			}
		);
	}

	#[test]
	fn sets_follow_the_layers_main_takes_and_skip_the_others() {
		let builtins = besl::Builtins::new();
		let program = besl::compile_to_besl(
			r#"
				A: layer { a: descriptor<{ type: f32[], binding: 0, access: read }>; }
				B: layer { b: descriptor<{ type: f32[], binding: 0, access: read }>; }
				Unused: layer { c: descriptor<{ type: f32[], binding: 0, access: read }>; }
				own: descriptor<{ type: f32[], binding: 0, access: write }>;
				Shader: fn (input: StageInput, b: B, a: A) -> void;
				main: Shader { own[0] = a.a[0] + b.b[0]; }
			"#,
			besl::Program::new(&builtins),
		)
		.expect("Expected the program to link");

		let layouts = reflection::collect_sets(&program, &program.root())
			.expect("Expected the sets to reflect")
			.into_iter()
			.map(|set| set.layout)
			.collect::<Vec<_>>();

		assert_eq!(layouts, [Some("B".to_string()), Some("A".to_string()), None]);
	}

	#[test]
	fn a_layer_reflects_from_the_modules_that_declare_it() {
		let files = [
			("shared/records.besl", "Light: struct { color: vec3f, intensity: f32 }"),
			(
				"shared/lighting.besl",
				"use ::shared::records::Light;\nLighting: layer { lights: descriptor<{ type: Light[], binding: 0, access: read }>; }",
			),
		];
		let read = |file: &str| {
			files
				.iter()
				.find(|(name, _)| *name == file)
				.map(|(_, source)| source.to_string())
		};

		let set = reflect_layer("::shared::lighting::Lighting", read).expect("Expected the layer to reflect");

		assert_eq!(set.layout.as_deref(), Some("::shared::lighting::Lighting"));
		assert_eq!(set.bindings.len(), 1);
		assert_eq!(set.bindings[0].name, "lights");
		assert_eq!(set.used, 1);
	}

	#[test]
	fn layer_reflection_carries_the_sampler_a_texture_declares() {
		let pyramids = r#"
			farthest_depth: sampler<{ mip_filter: nearest, reduction: min }>;
			Pyramids: layer {
				pyramid: descriptor<{ type: Texture2D, binding: 0, access: read, sampler: farthest_depth }>;
				albedo: descriptor<{ type: Texture2D, binding: 1, access: read }>;
			}
		"#;
		let set = reflect_layer("::shared::pyramids::Pyramids", |file| {
			(file == "shared/pyramids.besl").then(|| pyramids.to_string())
		})
		.expect("Expected the sampled layer to reflect");

		let kind = |name: &str| {
			set.bindings
				.iter()
				.find(|binding| binding.name == name)
				.expect("Expected the layer binding")
				.kind
		};
		assert_eq!(
			kind("pyramid"),
			BindingKind::CombinedImageSampler {
				view: TextureView::Texture2D,
				sampler: Some(besl::SamplerState {
					mip_filter: besl::sampler::Filter::Nearest,
					reduction: besl::sampler::Reduction::Min,
					..Default::default()
				}),
			}
		);
		assert_eq!(
			kind("albedo"),
			BindingKind::CombinedImageSampler {
				view: TextureView::Texture2D,
				sampler: None,
			}
		);
	}

	/// Returns a program whose root declares a read-only buffer binding `name` with one `f32` member named `value`.
	fn f32_buffer_program<'b>(builtins: &'b besl::Builtins, name: &str) -> besl::Program<'b> {
		let mut program = besl::Program::new(builtins);
		let f32_type = builtin(&program, "f32");
		let value_name = program.intern("value");
		let value = program.add(besl::Node::member(value_name, f32_type));
		let name = program.intern(name);
		program.add_to_root(besl::Node::binding(
			name,
			besl::BindingTypes::Buffer { members: vec![value] },
			0,
			true,
			false,
		));
		program
	}

	#[test]
	fn program_reflection_keeps_an_unreachable_declared_binding() {
		let builtins = besl::Builtins::new();
		let mut program = besl::compile_to_besl("main: fn() -> void { return; }", f32_buffer_program(&builtins, "unreachable"))
			.expect("Expected unreachable binding fixture to link");
		let main = program
			.get_main()
			.expect("Expected unreachable binding fixture main function");

		let program_bindings = ProgramEvaluation::from_program(&mut program)
			.expect("Expected full-program reflection")
			.into_bindings();
		let main_bindings = ProgramEvaluation::from_main(&mut program, &main)
			.expect("Expected reachable-main reflection")
			.into_bindings();

		assert_eq!(program_bindings.len(), 1);
		assert_eq!(program_bindings[0].name, "unreachable");
		assert!(main_bindings.is_empty());
	}

	#[test]
	fn reflection_culls_a_binding_used_only_by_a_dead_local() {
		let builtins = besl::Builtins::new();
		let mut program = besl::compile_to_besl(
			r#"
			main: fn() -> void {
				let ignored: f32 = values.value;
				return;
			}
		"#,
			f32_buffer_program(&builtins, "values"),
		)
		.expect("Expected dead binding fixture to link");
		let main = program.get_main().expect("Expected dead binding fixture main function");

		let evaluation = ProgramEvaluation::from_main(&mut program, &main).expect("Expected optimized reflection");

		assert!(evaluation.bindings().is_empty(), "Dead local binding reached reflection");
	}

	/// Links `script` against a root that declares one stage output named `output` of the built-in type `format`, and
	/// evaluates the whole program.
	fn evaluate_with_output(script: &str, format: &str) -> ProgramEvaluation {
		let builtins = besl::Builtins::new();
		let mut program = besl::Program::new(&builtins);
		let format = builtin(&program, format);
		let output_name = program.intern("output");
		program.add_to_root(besl::Node::output(output_name, format, 0));

		let mut program = besl::compile_to_besl(script, program).unwrap();
		ProgramEvaluation::from_program(&mut program).expect("Failed to evaluate program")
	}

	#[test]
	fn opacity_is_opaque_when_non_local_output_is_referenced() {
		let script = r#"
		main: fn () -> void {
			output;
		}
		"#;

		let evaluation = evaluate_with_output(script, "vec3f");

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

		let evaluation = evaluate_with_output(script, "vec3f");

		assert_eq!(evaluation.opacity(), OpacityEvaluation::Unknown);
	}

	#[test]
	fn opacity_is_non_opaque_when_output_vec4f_w_is_not_one() {
		let script = r#"
		main: fn () -> void {
			output = vec4f(1.0, 0.0, 0.0, 0.5);
		}
		"#;

		let evaluation = evaluate_with_output(script, "vec4f");

		assert_eq!(evaluation.opacity(), OpacityEvaluation::NonOpaque);
	}

	#[test]
	fn opacity_vec4f_with_vec3f_first_param_uses_w_for_opacity() {
		fn evaluate(w: &str) -> OpacityEvaluation {
			let builtins = besl::Builtins::new();
			let mut program = besl::Program::new(&builtins);
			let void_type = builtin(&program, "void");
			let vec3f_type = builtin(&program, "vec3f");
			let vec4f_type = builtin(&program, "vec4f");

			let output_name = program.intern("output");
			let output_node = program.add(besl::Node::output(output_name, vec4f_type, 0));

			let literals = ["1.0", "0.0", "0.0"].map(|value| {
				program.add(besl::Node::expression(besl::Expressions::Literal {
					value: value.to_string(),
				}))
			});
			let vec3f_call = program.add(besl::Node::expression(besl::Expressions::FunctionCall {
				function: vec3f_type,
				parameters: literals.to_vec(),
			}));

			let w = program.add(besl::Node::expression(besl::Expressions::Literal { value: w.to_string() }));
			let vec4f_call = program.add(besl::Node::expression(besl::Expressions::FunctionCall {
				function: vec4f_type,
				parameters: vec![vec3f_call, w],
			}));

			let output_member = program.add(besl::Node::expression(besl::Expressions::Member {
				name: output_name,
				source: output_node,
			}));

			let assignment = program.add(besl::Node::expression(besl::Expressions::Operator {
				operator: besl::Operators::Assignment,
				left: output_member,
				right: vec4f_call,
			}));

			let main_name = program.intern("main");
			let main = program.add(besl::Node::function(main_name, Vec::new(), void_type, vec![assignment]));

			let root = program.root();
			program[root].add_children(vec![output_node, main]);

			let evaluation = ProgramEvaluation::from_program(&mut program).expect("Failed to evaluate program");
			evaluation.opacity()
		}

		assert_eq!(evaluate("1.0"), OpacityEvaluation::Opaque);
		assert_eq!(evaluate("0.5"), OpacityEvaluation::NonOpaque);
	}

	#[test]
	fn reflected_ray_payloads_are_equal_exactly_when_their_layouts_are() {
		/// Reflects the payload of a linked shader.
		fn payload(source: &str) -> Option<RayPayload> {
			let builtins = besl::Builtins::new();
			let mut program =
				besl::compile_to_besl(source, besl::Program::new(&builtins)).expect("Expected the shader to link");
			let evaluation = ProgramEvaluation::from_program(&mut program).expect("Expected the shader to reflect");
			evaluation.ray_payload().cloned()
		}
		/// Returns a closest hit whose payload declares `members`.
		fn closest_hit(members: &str) -> String {
			format!("SurfaceHit: struct {{ {members} }} main: fn (payload: SurfaceHit) -> SurfaceHit {{ return payload; }}")
		}

		let ray_generation = payload(
			r#"
			SurfaceHit: struct { albedo: vec3f, distance: f32, }
			scene: descriptor<{ type: AccelerationStructure, binding: 0, access: read }>;
			surfaces: descriptor<{ type: SurfaceHit[], binding: 1, access: write }>;
			main: fn () -> void {
				let hit: Hit = intersect(scene, 255, vec3f(0.0, 0.0, 0.0), vec3f(0.0, 0.0, 1.0), 0.0, 100.0);
				surfaces[0] = invoke(hit, SurfaceHit(vec3f(0.0, 0.0, 0.0), -1.0));
			}
			"#,
		)
		.expect("Expected the payload that `invoke` passes");
		let matching = payload(&closest_hit("albedo: vec3f, distance: f32,")).expect("Expected the closest hit's payload");
		let reordered = payload(&closest_hit("distance: f32, albedo: vec3f,")).expect("Expected the closest hit's payload");
		let retyped = payload(&closest_hit("albedo: vec3f, distance: f16,")).expect("Expected the closest hit's payload");

		assert_eq!(ray_generation, matching);
		assert_eq!(ray_generation.name, "SurfaceHit");
		assert_eq!(ray_generation.size, 16);
		assert_ne!(ray_generation.layout_hash, reordered.layout_hash);
		assert_ne!(ray_generation.layout_hash, retyped.layout_hash);
		assert_eq!(payload("main: fn () -> void {}"), None);
	}
}
