mod byte_windows;
mod push_constant_layout;
mod reflection;

#[cfg(test)]
use std::collections::HashSet;

pub(crate) use byte_windows::{ByteWindow, byte_windows};
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

	/// Links `source` and reflects every binding it declares.
	fn reflect(source: &str) -> Vec<BindingUsage> {
		let builtins = besl::Builtins::new();
		let mut program =
			besl::compile_to_besl(source, besl::Program::new(&builtins)).expect("Expected the reflection fixture to link");
		ProgramEvaluation::from_program(&mut program)
			.expect("Expected the reflection fixture to reflect")
			.into_bindings()
	}

	#[test]
	fn binding_metadata_is_sorted_and_classified() {
		let builtins = besl::Builtins::new();
		let mut program = generator::tests::bindings(&builtins);

		let evaluation = ProgramEvaluation::from_program(&mut program).expect("Failed to evaluate program");
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
				("image", BindingKind::StorageImage { layered: false }, 1, 1, None, false, true),
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
		let strides = reflect(
			r#"
			Light: struct { position: vec3f, color: vec3f, direction: vec3f, cone_cosines: vec2f, light_type: u8, cascades: u32[8] }
			Positions: struct { positions: vec3f[16] }
			positions: descriptor<{ type: Positions, binding: 0, access: read }>;
			Indices: struct { indices: u16[16] }
			indices: descriptor<{ type: Indices, binding: 1, access: read }>;
			Lighting: struct { light_count: u32, lights: Light[16] }
			lighting: descriptor<{ type: Lighting, binding: 2, access: read }>;
			main: fn () -> void { positions; indices; lighting; }
			"#,
		)
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

	/// Verifies each array element reflects the stride its emitted type has on HLSL, MSL, and GLSL, so the CPU, the
	/// VM, and every backend agree on where each element starts.
	#[test]
	fn array_elements_reflect_the_stride_each_backend_emits() {
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
			Instance: struct { position: vec3f, sprite_id: u32 }
			Halves: struct { scalar: f16, uv: vec2f16, normal: vec3f16, color: vec4f16 }
			Pair: struct { low: u16, high: u16 }
			Tagged: struct { position: vec3f, tag: u8 }
			TaggedItems: struct { items: Tagged[2], tail: u32 }
			Position: struct { position: vec3f }
			Mesh: struct {
				model: mat4x3f, material_index: u32, base_vertex_index: u32, base_primitive_index: u32,
				base_triangle_index: u32, base_meshlet_index: u32, meshlet_count: u32, skinned_base_vertex_index: u32,
				flags: u32,
			}
			View: struct { view: mat4x3f, view_projection: mat4f, inverse_view: mat4x3f, fov: vec2f, near: f32, far: f32 }
			Meshlet: struct {
				primitive_offset: u32, triangle_offset: u32, primitive_count: u32, triangle_count: u32,
				center_radius: vec4f, cone_apex_cutoff: vec4f, cone_axis: vec2u16,
			}
			Light: struct { position: vec3f, color: vec3f, direction: vec3f, cone_cosines: vec2f, light_type: u32, cascades: u32[8] }
			Lighting: struct { light_count: u32, lights: Light[16] }
			main: fn () -> void {}
			"#,
			besl::Program::new(&builtins),
		)
		.expect("Expected the records to link");

		// Each element's stride on HLSL, MSL, and GLSL. DX12 stores narrow scalars in 32-bit words, and Metal stores
		// vectors as packed types, so they keep the scalar-aligned layout of the other backends.
		for (name, strides) in [
			("Mixed", [12, 12, 12]),
			("Many", [6, 6, 6]),
			("Bytes", [8, 8, 8]),
			("Instance", [16, 16, 16]),
			("Halves", [20, 20, 20]),
			("vec2f16", [4, 4, 4]),
			("u8", [4, 1, 1]),
			("u16", [4, 2, 2]),
			("vec3f", [12, 12, 12]),
			("Pair", [4, 4, 4]),
			("TaggedItems", [36, 36, 36]),
			("Position", [12, 12, 12]),
			("Mesh", [80, 80, 80]),
			("View", [176, 176, 176]),
			("Meshlet", [52, 52, 52]),
			("Lighting", [1284, 1284, 1284]),
		] {
			let element = builtin(&program, name);
			for (target, stride) in [
				StorageLayoutTarget::Hlsl,
				StorageLayoutTarget::Msl,
				StorageLayoutTarget::GlslScalar,
			]
			.into_iter()
			.zip(strides)
			{
				assert_eq!(
					reflected_array_buffer_stride_for_target(&program, &element, target),
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
			let mut program =
				besl::compile_to_besl(&source, besl::Program::new(&builtins)).expect("Expected the record program to link");

			let error = ProgramEvaluation::from_program(&mut program).expect_err("Expected the record layout to be rejected");

			assert!(error.contains(expected), "{records}: {error}");
			assert!(error.contains("reference/besl/interface#stored-records"), "{error}");
		}
	}

	#[test]
	fn runtime_storage_buffer_reflects_its_element_stride() {
		let bindings = reflect(
			"Instance: struct { position: vec3f, sprite_id: u32 } instances: descriptor<{ type: Instance[], binding: 0, access: read }>; main: fn () -> void { return; }",
		);

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
	fn matrix_storage_layouts_cover_all_besl_matrix_types() {
		let builtins = besl::Builtins::new();
		// BESL declares no two- or three-column matrix, so the fixture declares them as structs of their columns.
		let program = besl::compile_to_besl(
			"mat2f: struct { x: vec2f, y: vec2f } mat3f: struct { x: vec3f, y: vec3f, z: vec3f } main: fn () -> void {}",
			besl::Program::new(&builtins),
		)
		.expect("Expected the matrix fixture to link");
		let matrices = ["mat2f", "mat3f", "mat4f", "mat4x3f"].map(|name| (name, builtin(&program, name)));

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
	fn sampled_texture_shapes_and_descriptor_counts_are_preserved() {
		let bindings = reflect(
			"volumes: descriptor<{ type: Texture3D, binding: 0, access: read, count: 3 }>; main: fn () -> void { volumes; }",
		);

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

	/// Verifies reflection keeps a binding `main` never reads, or reads only into a dead local the optimizer removes,
	/// and marks it unused.
	#[test]
	fn declared_bindings_main_never_reads_stay_unused() {
		for main in [
			"main: fn () -> void { return; }",
			"main: fn () -> void { let ignored: f32 = values.value; return; }",
		] {
			let builtins = besl::Builtins::new();
			let mut program = besl::compile_to_besl(
				&format!(
					"Value: struct {{ value: f32 }} values: descriptor<{{ type: Value, binding: 0, access: read }}>; {main}"
				),
				besl::Program::new(&builtins),
			)
			.expect("Expected the unused binding fixture to link");
			let sets = ProgramEvaluation::from_program(&mut program)
				.expect("Expected full-program reflection")
				.into_sets();

			assert_eq!(sets.len(), 1, "{main}");
			assert_eq!(sets[0].bindings[0].name, "values", "{main}");
			assert_eq!(sets[0].used, 0, "{main}");
		}
	}

	/// Builds a sampled-image binding at `slot`, for collector tests whose programs the BESL source can't express.
	fn binding(name: besl::Name, slot: u32, read: bool, write: bool) -> besl::Node {
		besl::Node::binding(
			name,
			besl::BindingTypes::CombinedImageSampler {
				format: String::new(),
				sampler: None,
			},
			slot,
			read,
			write,
		)
	}

	fn usage(bindings: &[BindingUsage]) -> Vec<(u32, bool, bool)> {
		bindings
			.iter()
			.map(|binding| (binding.slot, binding.read, binding.write))
			.collect()
	}

	#[test]
	fn binding_collector_uses_only_instantiated_intrinsic_elements() {
		let builtins = besl::Builtins::new();
		let mut program = besl::Program::new(&builtins);
		let void_type = builtin(&program, "void");
		let definition_first_name = program.intern("definition_first");
		let definition_only_name = program.intern("definition_only");
		let elements = vec![
			program.add(binding(definition_first_name, 0, true, false)),
			program.add(binding(definition_only_name, 2, true, true)),
		];
		let binding_order_fixture_name = program.intern("binding_order_fixture");
		let intrinsic = program.add(besl::Node::intrinsic(binding_order_fixture_name, elements, void_type));
		// The intrinsic definition is only a template; emitted bindings come from the instantiated elements.
		let instantiated_name = program.intern("instantiated");
		let instantiated = program.add(binding(instantiated_name, 100, true, false));
		let call = program.add(besl::Node::expression(besl::Expressions::IntrinsicCall {
			intrinsic,
			arguments: Vec::new(),
			elements: vec![instantiated],
		}));
		let main_name = program.intern("main");
		let main = program.add(besl::Node::function(main_name, Vec::new(), void_type, vec![call]));

		let bindings = collect_bindings(&program, &main).expect("Expected instantiated flat resource metadata");

		assert_eq!(usage(&bindings), vec![(100, true, false)]);
	}

	#[test]
	fn binding_collector_deduplicates_shared_binding_references() {
		let builtins = besl::Builtins::new();
		let mut program = besl::Program::new(&builtins);
		let void_type = builtin(&program, "void");
		let shared_name = program.intern("shared");
		let shared = program.add(binding(shared_name, 3, true, false));
		let main_name = program.intern("main");
		let main = program.add(besl::Node::function(main_name, Vec::new(), void_type, vec![shared, shared]));

		let bindings = collect_bindings(&program, &main).expect("Expected one shared flat resource declaration");

		assert_eq!(usage(&bindings), vec![(3, true, false)]);
	}

	#[test]
	fn binding_collector_rejects_distinct_same_slot_declarations() {
		let builtins = besl::Builtins::new();
		let mut program = besl::Program::new(&builtins);
		let void_type = builtin(&program, "void");
		let first_name = program.intern("first");
		let second_name = program.intern("second");
		let statements = vec![
			program.add(binding(first_name, 3, true, false)),
			program.add(binding(second_name, 3, false, true)),
		];
		let main_name = program.intern("main");
		let main = program.add(besl::Node::function(main_name, Vec::new(), void_type, statements));

		let error = collect_bindings(&program, &main).expect_err("Expected distinct same-slot declarations to fail");

		assert!(error.contains("Duplicate resource declaration at binding 3"), "{error}");
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
