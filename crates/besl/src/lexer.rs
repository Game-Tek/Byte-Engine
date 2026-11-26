//! BESL analysis and lowering into executable VM instructions.

mod ast;
mod bools;
mod entry;
mod layers;
mod lowering;
mod matching;
mod resolution;
mod samplers;

pub use ast::{
	BindingTypes, BufferMemoryClass, Builtins, ElseBranch, Expressions, FixedArray, LexError, MAX_DESCRIPTOR_SETS, MatchArm,
	Name, Node, NodeId, NodeMap, NodeSet, Nodes, Operators, Placement, Program, UnaryOperators,
};
pub(crate) use ast::{UNARY_PRECEDENCE, lex_with_root};
pub use layers::place_descriptors;
pub use resolution::infer_expression_type;
pub use samplers::binding_sampler;

#[cfg(test)]
mod tests {
	use super::*;

	#[cfg(target_pointer_width = "64")]
	#[test]
	#[should_panic(expected = "resource array exceeds u32::MAX elements")]
	fn binding_array_rejects_count_larger_than_flat_metadata() {
		let builtins = crate::Builtins::new();
		let mut program = Program::new(&builtins);
		let textures_name = program.intern("textures");
		Node::binding_array(
			textures_name,
			BindingTypes::CombinedImageSampler {
				format: String::new(),
				sampler: None,
			},
			0,
			true,
			false,
			(u32::MAX as usize) + 1,
		);
	}

	#[test]
	fn source_descriptors_lower_to_existing_flat_binding_types() {
		let builtins = crate::Builtins::new();
		let source = r#"
			Data: struct {
				value: u32,
				weight: f32,
			}
			data: descriptor<{ type: Data, binding: 0, access: read_write, memory: device }>;
			texture: descriptor<{ type: Texture2D, binding: 1, access: read }>;
			texture_array: descriptor<{ type: Texture2DArray, binding: 2, access: read, count: 16 }>;
			volume: descriptor<{ type: Texture3D, binding: 3, access: read }>;
			result: descriptor<{ type: StorageImage<rgba16f>, binding: 4, access: write }>;
			unformatted_result: descriptor<{ type: StorageImage, binding: 5, access: write }>;
			main: fn () -> void {
				data.value = data.value;
			}
		"#;

		let program = crate::compile_to_besl(source, crate::Program::new(&builtins)).expect("resource descriptors should lex");
		let root = program.root();
		let data = program.get_child(root, "data").expect("data descriptor should exist");

		assert!(matches!(
			program[data].node(),
			Nodes::Binding {
				slot: 0,
				read: true,
				write: true,
				memory_class: BufferMemoryClass::Device,
				r#type: BindingTypes::Buffer { members },
				count: None,
				..
			} if members.iter().map(|member| program.name(*member).map(str::to_owned)).collect::<Vec<_>>()
				== vec![Some("value".to_string()), Some("weight".to_string())]
		));

		let texture = program.get_child(root, "texture").expect("texture descriptor should exist");

		assert!(matches!(
			program[texture].node(),
			Nodes::Binding {
				slot: 1,
				read: true,
				write: false,
				r#type: BindingTypes::CombinedImageSampler { format, .. },
				..
			} if format.is_empty()
		));

		let texture_array = program
			.get_child(root, "texture_array")
			.expect("texture array descriptor should exist");

		assert!(matches!(
			program[texture_array].node(),
			Nodes::Binding {
				slot: 2,
				r#type: BindingTypes::CombinedImageSampler { format, .. },
				count: Some(count),
				..
			} if format == "ArrayTexture2D" && count.get() == 16
		));

		let volume = program.get_child(root, "volume").expect("volume descriptor should exist");

		assert!(matches!(
			program[volume].node(),
			Nodes::Binding {
				r#type: BindingTypes::CombinedImageSampler { format, .. },
				..
			} if format == "Texture3D"
		));

		let result = program
			.get_child(root, "result")
			.expect("storage image descriptor should exist");

		assert!(matches!(
			program[result].node(),
			Nodes::Binding {
				slot: 4,
				read: false,
				write: true,
				r#type: BindingTypes::Image { format, layered: false },
				..
			} if format.as_deref() == Some("rgba16f")
		));

		let unformatted_result = program
			.get_child(root, "unformatted_result")
			.expect("unformatted storage image descriptor should exist");

		assert!(matches!(
			program[unformatted_result].node(),
			Nodes::Binding {
				slot: 5,
				read: false,
				write: true,
				r#type: BindingTypes::Image { format, layered: false },
				..
			} if format.is_none()
		));
	}

	#[test]
	fn structural_sprite_entry_points_lower_to_the_flat_semantic_abi() {
		let builtins = crate::Builtins::new();
		let vertex = crate::compile_to_besl(
			r#"
				main: fn (input: StageInput) -> interface { uv: vec2f, instance_index: u32 } {
					let uv: vec2f = vec2f(
						1.0 - f32(input.vertex_index & 1),
						f32(input.vertex_index >> 1),
					);

					return { uv, instance_index: input.instance_index };
				}
			"#,
			crate::Program::new(&builtins),
		)
		.expect("structural sprite vertex should link");
		let fragment = crate::compile_to_besl(
			r#"
				Instance: struct {
					position: vec3f,
					sprite_id: u32,
				}

				sprites: descriptor<{ type: Texture2DArray, binding: 0, access: read }>;
				instances: descriptor<{ type: Instance[], binding: 1, access: read }>;

				main: fn (input: StageInput, pipeline_input: interface { instance_index: u32, uv: vec2f }) -> output { color: vec4f } {
					let instance: Instance = instances[pipeline_input.instance_index];
					let color: vec4f = sample(sprites[instance.sprite_id], pipeline_input.uv);

					return { color };
				}
			"#,
			crate::Program::new(&builtins),
		)
		.expect("structural sprite fragment should link");

		fn io_location(program: &Program, name: &str) -> u8 {
			let node = program
				.get_child(program.root(), name)
				.expect("structural field should exist");
			match program[node].node() {
				Nodes::Input { location, .. } | Nodes::Output { location, .. } => *location,
				_ => panic!("structural field should be stage I/O"),
			}
		}

		assert_eq!(io_location(&vertex, "_besl_interface_instance_index"), 0);
		assert_eq!(io_location(&vertex, "_besl_interface_uv"), 1);
		assert_eq!(io_location(&fragment, "_besl_interface_instance_index"), 0);
		assert_eq!(io_location(&fragment, "_besl_interface_uv"), 1);
		assert!(matches!(
			fragment[fragment
				.get_child(fragment.root(), "instances")
				.expect("runtime buffer descriptor should exist")]
			.node(),
			Nodes::Binding {
				r#type: BindingTypes::BufferArray { element, .. },
				..
			} if fragment.name(*element) == Some("Instance")
		));
		assert_eq!(io_location(&fragment, "_besl_output_color"), 0);
	}

	/// Verifies `primitive<T>` interface fields become per-primitive fragment inputs of type `T`, and keep the
	/// locations sorted field names give, so they still link to the mesh stage's outputs.
	#[test]
	fn primitive_interface_fields_lower_to_per_primitive_inputs() {
		let builtins = crate::Builtins::new();
		let fragment = crate::compile_to_besl(
			r#"
				main: fn (pipeline_input: interface { uv: vec2f, instance_index: primitive<u32> }) -> output { color: vec4f } {
					return { color: vec4f(pipeline_input.uv.x, f32(pipeline_input.instance_index), 0.0, 1.0) };
				}
			"#,
			crate::Program::new(&builtins),
		)
		.expect("a fragment with a per-primitive input should link");

		let input = |name: &str| {
			let node = fragment
				.get_child(fragment.root(), name)
				.expect("the interface field should exist");
			let Nodes::Input {
				format,
				location,
				per_primitive,
				..
			} = fragment[node].node()
			else {
				panic!("the interface field should be a fragment input");
			};
			(fragment.name(*format).map(str::to_owned), *location, *per_primitive)
		};
		assert_eq!(input("_besl_interface_instance_index"), (Some("u32".to_owned()), 0, true));
		assert_eq!(input("_besl_interface_uv"), (Some("vec2f".to_owned()), 1, false));
	}

	/// Verifies a vertex `render_target_array_index` field lowers to the reserved `u32` layer output, and that it and
	/// `position` leave the varying locations as a fragment stage that declares only the varyings sees them.
	#[test]
	fn render_target_array_index_lowers_to_a_layer_output_that_takes_no_varying_location() {
		let builtins = crate::Builtins::new();
		let vertex = crate::compile_to_besl(
			r#"
				main: fn (input: StageInput) -> interface { position: vec4f, render_target_array_index: u32, uv: vec2f } {
					let layer: u32 = input.instance_index;
					return { position: vec4f(0.0, 0.0, 0.0, 1.0), render_target_array_index: layer, uv: vec2f(0.0, 1.0) };
				}
			"#,
			crate::Program::new(&builtins),
		)
		.expect("a vertex stage that picks its layer should link");
		let fragment = crate::compile_to_besl(
			r#"
				main: fn (pipeline_input: interface { uv: vec2f }) -> output { color: vec4f } {
					return { color: vec4f(pipeline_input.uv.x, pipeline_input.uv.y, 0.0, 1.0) };
				}
			"#,
			crate::Program::new(&builtins),
		)
		.expect("a fragment stage that reads only the varyings should link");

		let output = |program: &Program, name: &str| {
			let node = program
				.get_child(program.root(), name)
				.expect("the interface field should exist");
			match program[node].node() {
				Nodes::Input { format, location, .. } | Nodes::Output { format, location, .. } => {
					(program.name(*format).map(str::to_owned), *location)
				}
				_ => panic!("the interface field should be stage I/O"),
			}
		};
		assert_eq!(
			output(&vertex, crate::STRUCTURAL_RENDER_TARGET_ARRAY_INDEX_OUTPUT).0,
			Some("u32".to_owned())
		);
		assert_eq!(output(&vertex, "_besl_interface_uv"), (Some("vec2f".to_owned()), 0));
		assert_eq!(output(&fragment, "_besl_interface_uv"), (Some("vec2f".to_owned()), 0));
	}

	/// Verifies a `StorageImageArray` is one layered storage image, whose layer a `u32` index selects, and that it
	/// can't be an array of descriptors.
	#[test]
	fn storage_image_arrays_are_one_layered_image() {
		let builtins = crate::Builtins::new();
		let root = crate::compile_to_besl(
			"faces: descriptor<{ type: StorageImageArray<rgba16f>, binding: 0, access: write }>; main: fn (input: StageInput) -> void { write(faces[input.thread_layer], input.thread_id, vec4f(1.0, 1.0, 1.0, 1.0)); }",
			Program::new(&builtins),
		)
		.expect("A StorageImageArray written through a u32 layer should compile.");
		let faces = root.get_child(root.root(), "faces").expect("faces descriptor should exist");
		assert!(matches!(
			root[faces].node(),
			Nodes::Binding {
				r#type: BindingTypes::Image { format, layered: true },
				count: None,
				..
			} if format.as_deref() == Some("rgba16f")
		));
	}

	#[test]
	fn texture_descriptors_take_the_state_of_the_sampler_they_name() {
		let builtins = crate::Builtins::new();
		let source = r#"
			farthest_depth: sampler<{ filter: linear, mip_filter: nearest, reduction: min }>;
			pyramid: descriptor<{ type: Texture2D, binding: 0, access: read, sampler: farthest_depth }>;
			layers: descriptor<{ type: Texture2DArray, binding: 1, access: read, sampler: farthest_depth }>;
			albedo: descriptor<{ type: Texture2D, binding: 2, access: read }>;
			main: fn () -> void {
				let depth: f32 = texture_lod(pyramid, vec2f(0.5, 0.5), 1.0).x;
				let layer_depth: f32 = downsample_min(pyramid, vec2f(0.5, 0.5), 0.0);
				let color: vec4f = sample(albedo, vec2f(0.5, 0.5));
				depth;
				layer_depth;
				color;
			}
		"#;

		let program = crate::compile_to_besl(source, crate::Program::new(&builtins)).expect("sampled descriptors should lex");
		let sampler =
			|name: &str| match program[program.get_child(program.root(), name).expect("descriptor should exist")].node() {
				Nodes::Binding {
					r#type: BindingTypes::CombinedImageSampler { sampler, .. },
					..
				} => *sampler,
				_ => panic!("expected texture binding {name}"),
			};
		let farthest_depth = crate::SamplerState {
			mip_filter: crate::sampler::Filter::Nearest,
			reduction: crate::sampler::Reduction::Min,
			..Default::default()
		};
		assert_eq!(sampler("pyramid"), Some(farthest_depth));
		assert_eq!(sampler("layers"), Some(farthest_depth));
		assert_eq!(sampler("albedo"), None);
		assert!(
			program.get_child(program.root(), "farthest_depth").is_none(),
			"a sampler is part of the bindings that name it, not a declaration of its own"
		);
	}

	/// Compiles invalid source and returns the plain-language linker diagnostic.
	fn link_error(source: &str) -> String {
		let builtins = crate::Builtins::new();
		match crate::compile_to_besl(source, crate::Program::new(&builtins))
			.expect_err("invalid source should fail while linking")
		{
			crate::CompilationError::Lex(LexError::Invalid { message }) => message,
			error => panic!("Expected a detailed linker error, found {error:?}"),
		}
	}

	#[test]
	fn source_task_storage_and_stage_interfaces_link_without_injected_rust_nodes() {
		let builtins = crate::Builtins::new();
		let source = r#"
			instance_index: input<u32, 0>;
			primitive_index: output<u32, 1>;
			visible_meshlets: task_payload<u32, 32>;
			visible_count: workgroup<atomicu32>;
			scratch: workgroup<f32, 64>;
			main: fn () -> void {
				let position: u32 = thread_position();
				visible_meshlets[thread_idx()] = position;
				atomic_store(visible_count, position);
				workgroup_barrier();
				set_task_mesh_output_count(atomic_load(visible_count));
				primitive_index = instance_index;
			}
		"#;

		let program =
			crate::compile_to_besl(source, crate::Program::new(&builtins)).expect("standalone task shader should link");
		let root = program.root();
		let payload = program
			.get_child(root, "visible_meshlets")
			.expect("task payload declaration should be linked");

		assert!(matches!(
			program[payload].node(),
			Nodes::TaskPayload { count, format, .. }
				if count.get() == 32 && program.name(*format) == Some("u32")
		));
		assert!(program[payload].node().is_indexable(&program));

		let workgroup = program
			.get_child(root, "visible_count")
			.expect("workgroup declaration should be linked");

		assert!(matches!(
			program[workgroup].node(),
			Nodes::Workgroup { format, .. } if program.name(*format) == Some("atomicu32")
		));
		let scratch = program
			.get_child(root, "scratch")
			.expect("counted workgroup declaration should be linked");

		assert!(matches!(
			program[scratch].node(),
			Nodes::Workgroup {
				format,
				count: Some(count),
				..
			} if count.get() == 64 && program.name(*format) == Some("f32")
		));
		assert!(program[scratch].node().is_indexable(&program));
		assert!(program.get_main().is_some());
	}

	#[test]
	fn recursive_function_calls_link_to_their_function() {
		let builtins = crate::Builtins::new();
		let program = crate::compile_to_besl(
			"count: fn (n: u32) -> u32 { return count(n); }",
			crate::Program::new(&builtins),
		)
		.expect("Recursive functions should link");
		let count = program.get_child(program.root(), "count").expect("Expected count function");
		let Nodes::Function { statements, .. } = program[count].node() else {
			panic!("Expected count function");
		};
		let Nodes::Expression(Expressions::Return { value: Some(value) }) = program[statements[0]].node() else {
			panic!("Expected return statement");
		};
		let Nodes::Expression(Expressions::FunctionCall { function, .. }) = program[value].node() else {
			panic!("Expected recursive call");
		};

		assert_eq!(*function, count);
	}

	#[test]
	fn declarations_use_only_the_declarations_before_them() {
		let builtins = crate::Builtins::new();
		let link = |declarations: &[&str]| crate::compile_to_besl(&declarations.join("\n"), crate::Program::new(&builtins));
		let data = "Data: struct { count: u32, }\ndata: descriptor<{ type: Data, binding: 0, access: read }>;";

		// Each case holds a declaration, the declarations it uses, and how the error starts when they come after it.
		let cases = [
			(
				"caller: fn () -> f32 { return helper(); }",
				"helper: fn () -> f32 { return 1.0; }",
				"`caller` uses `helper`, which is declared after it.",
			),
			(
				"scaled: fn () -> f32 { return SCALE * 2.0; }",
				"SCALE: const f32 = 0.5;",
				"`scaled` uses `SCALE`, which is declared after it.",
			),
			(
				"Light: struct { falloff: Falloff, }",
				"Falloff: struct { radius: f32, }",
				"`Light` uses `Falloff`, which is declared after it.",
			),
			(
				"lights: descriptor<{ type: Light[], binding: 0, access: read }>;",
				"Light: struct { radius: f32, }",
				"`lights` uses `Light`, which is declared after it.",
			),
			(
				"total: fn () -> u32 { return data.count; }",
				data,
				"`total` uses `data`, which is declared after it.",
			),
			(
				"index: fn () -> u32 { return push_constant.index; }",
				"push_constant: push_constant { index: u32 }",
				"`index` uses `push_constant`, which is declared after it.",
			),
		];
		for (user, used, expected) in cases {
			link(&[used, user]).unwrap_or_else(|error| panic!("`{user}` should link after `{used}`: {error:?}"));
			let message = link_error(&[user, used].join("\n"));
			assert!(message.starts_with(expected), "{message}");
		}
		let (caller, helper, _) = cases[0];
		assert!(link_error(&[caller, helper].join("\n")).contains("Move `helper` above `caller`."));

		// `main` links after every other declaration.
		link(&["main: fn () -> f32 { return caller(); }", helper, caller]).expect("`main` should use later declarations");

		// A name that nothing declares is undeclared, not out of order.
		assert!(matches!(
			link(&[caller]),
			Err(crate::CompilationError::Lex(LexError::UndeclaredName { name })) if name == "helper"
		));
		// A type names only structs, so a later function by its name isn't one the type uses.
		assert!(matches!(
			link(&["make: fn () -> helper {}", helper]),
			Err(crate::CompilationError::Lex(LexError::ReferenceToUndefinedType { type_name })) if type_name == "helper"
		));
		// A member its owner lacks is misspelled, even when a later declaration has the member's name.
		let message = link_error(&[data, "total: fn () -> u32 { return data.helper; }", helper].join("\n"));
		assert!(message.starts_with("`helper` isn't a member of `data`."), "{message}");
	}

	/// Asserts that `node` references a buffer binding lowered from a lone fixed-array member.
	fn assert_lowered_array_reference(program: &Program, node: NodeId, binding: &str, alias: &str, count: usize) {
		let Nodes::Expression(Expressions::Member { name, source }) = program[node].node() else {
			panic!("Expected a binding reference");
		};
		assert_eq!(program.text(*name), binding);
		assert!(matches!(
			program[source].node(),
			Nodes::Binding {
				r#type: BindingTypes::BufferArray { fixed: Some(fixed), .. },
				..
			} if fixed.count.get() == count && fixed.alias == alias
		));
	}

	#[test]
	// This AST identity test keeps both same-named buffer scopes in one contiguous assertion tree.
	#[allow(clippy::cognitive_complexity)]
	fn lex_same_named_buffer_members_resolve_to_lowered_arrays() {
		let builtins = crate::Builtins::new();
		let script = r#"
		main: fn () -> void {
			let material_index: u32 = meshes.meshes[0].material_index;
			let mapped: u32 = pixel_mapping.pixel_mapping[1];
		}
		"#;

		let mut program = Program::new(&builtins);
		let u32_type = program.get_child(program.root(), "u32").expect("Expected u32");
		let material_index_name = program.intern("material_index");
		let material_index = program.add(Node::member(material_index_name, u32_type));
		let mesh_name = program.intern("Mesh");
		let mesh = program.add_to_root(Node::r#struct(mesh_name, vec![material_index]));

		let meshes_name = program.intern("meshes");
		let meshes = program.add(Node::array(meshes_name, mesh, 4));
		program.add_to_root(Node::binding(
			meshes_name,
			BindingTypes::Buffer { members: vec![meshes] },
			0,
			true,
			false,
		));
		let pixel_mapping_name = program.intern("pixel_mapping");
		let pixel_mapping = program.add(Node::array(pixel_mapping_name, u32_type, 4));
		program.add_to_root(Node::binding(
			pixel_mapping_name,
			BindingTypes::Buffer {
				members: vec![pixel_mapping],
			},
			1,
			true,
			true,
		));

		let program = crate::compile_to_besl(script, program).expect("Failed to lex");
		let main = program.get_descendant(program.root(), "main").expect("Expected main");

		let Nodes::Function { statements, .. } = program[main].node() else {
			panic!("Expected function");
		};

		let material_index_access = match program[statements[0]].node() {
			Nodes::Expression(Expressions::Operator { right, .. }) => *right,
			_ => panic!("Expected assignment"),
		};
		let (indexed_meshes, material_index_member) = match program[material_index_access].node() {
			Nodes::Expression(Expressions::Accessor { left, right }) => (*left, *right),
			_ => panic!("Expected struct member accessor"),
		};
		match program[material_index_member].node() {
			Nodes::Expression(Expressions::Member { name, source }) => {
				assert_eq!(program.text(*name), "material_index");
				assert!(matches!(
					program[source].node(),
					Nodes::Member { name, count, .. } if program.text(*name) == "material_index" && count.is_none()
				));
			}
			_ => panic!("Expected material_index member expression"),
		}

		match program[indexed_meshes].node() {
			Nodes::Expression(Expressions::Accessor { left, .. }) => {
				assert_lowered_array_reference(&program, *left, "meshes", "meshes", 4)
			}
			_ => panic!("Expected indexed meshes accessor"),
		}

		let pixel_mapping_access = match program[statements[1]].node() {
			Nodes::Expression(Expressions::Operator { right, .. }) => *right,
			_ => panic!("Expected assignment"),
		};
		match program[pixel_mapping_access].node() {
			Nodes::Expression(Expressions::Accessor { left, .. }) => {
				assert_lowered_array_reference(&program, *left, "pixel_mapping", "pixel_mapping", 4);
			}
			_ => panic!("Expected indexed pixel_mapping accessor"),
		}
	}

	#[test]
	fn lex_local_named_like_its_field_resolves_access_to_the_field() {
		let builtins = crate::Builtins::new();
		let script = r#"
		Transform: struct { model: mat4f, }
		transforms: descriptor<{ type: Transform[], binding: 0, access: read }>;
		main: fn () -> void {
			let model: Transform = transforms[0];
			model.model[0];
		}
		"#;

		let program = crate::compile_to_besl(script, crate::Program::new(&builtins)).expect("Failed to lex");
		let main = program.get_descendant(program.root(), "main").expect("Expected main");
		let Nodes::Function { statements, .. } = program[main].node() else {
			panic!("Expected function");
		};

		let Nodes::Expression(Expressions::Accessor { left: field_access, .. }) = program[statements[1]].node() else {
			panic!("Expected indexed field access");
		};
		let Nodes::Expression(Expressions::Accessor { right: field, .. }) = program[field_access].node() else {
			panic!("Expected field access");
		};
		let Nodes::Expression(Expressions::Member { source, .. }) = program[field].node() else {
			panic!("Expected field member expression");
		};

		assert!(
			matches!(program[source].node(), Nodes::Member { name, .. } if program.text(*name) == "model"),
			"Expected `model.model` to resolve to the `Transform.model` field instead of the local"
		);
	}

	// #[test]
	// fn push_constant() {
	// }

	// TODO: test function with body with missing close brace

	/// A match lexes to distinct labels and one default, following Rust's first-match-wins rule.
	#[test]
	fn match_resolves_unreachable_patterns() {
		let builtins = crate::Builtins::new();
		let program = crate::compile_to_besl(
			"main: fn () -> void { let n: u32 = 0; match n { 0 | 1 => n = 1, 1 | 2 => n = 2, 0 => n = 3, 3 | _ => n = 4, 4 => n = 5 } }",
			crate::Program::new(&builtins),
		)
		.expect("Expected the match to lex");
		let main = program.get_child(program.root(), "main").expect("Expected main");
		let Nodes::Function { statements, .. } = program[main].node() else {
			panic!("Expected function");
		};
		let Nodes::Match {
			r#type, arms, default, ..
		} = program[statements[1]].node()
		else {
			panic!("Expected match");
		};

		assert_eq!(program.name(*r#type), Some("u32"));
		// `1` already belongs to the first arm, and the `0` arm can never run.
		let labels: Vec<&[i64]> = arms.iter().map(|arm| arm.values.as_slice()).collect();
		assert_eq!(labels, [&[0, 1][..], &[2]]);
		// The `3 | _` arm catches every other value, so the `4` arm after it is unreachable.
		assert_eq!(default.len(), 1);
	}

	/// Verifies `get_main` returns the entry-point function even when a struct declares a `main` member first.
	#[test]
	fn get_main_returns_the_entry_point_function() {
		let builtins = crate::Builtins::new();
		let source = r#"
			Config: struct { main: u32, }
			main: fn () -> void { }
		"#;

		let program = crate::compile_to_besl(source, crate::Program::new(&builtins)).expect("source should link");
		let main = program.get_main().expect("main function should be found");
		assert!(matches!(program[main].node(), Nodes::Function { name, .. } if program.text(*name) == "main"));

		let program = crate::compile_to_besl("Config: struct { main: u32, }", crate::Program::new(&builtins))
			.expect("source should link");
		assert!(program.get_main().is_none(), "a struct member is not an entry point");
	}

	/// The atomics every buffer and workgroup atomic rule applies to.
	const ATOMICS: [&str; 11] = [
		"atomic_store",
		"atomic_load",
		"atomic_exchange",
		"atomic_compare_exchange",
		"atomic_add",
		"atomic_sub",
		"atomic_min",
		"atomic_max",
		"atomic_and",
		"atomic_or",
		"atomic_xor",
	];

	/// Builds one statement for each supported scalar buffer or workgroup atomic intrinsic.
	fn atomic_statement(name: &str, target: &str) -> String {
		match name {
			"atomic_load" => format!("{name}({target});"),
			"atomic_compare_exchange" => format!("{name}({target}, 1, 2);"),
			_ => format!("{name}({target}, 1);"),
		}
	}

	/// Builds a program that runs `statement` against an atomic counter in a buffer of `access`.
	fn atomic_counter_source(access: &str, statement: &str) -> String {
		format!(
			"Counters: struct {{ value: atomicu32, }} counters: descriptor<{{ type: Counters, binding: 0, access: {access} }}>; main: fn () -> void {{ {statement} }}"
		)
	}

	/// The resource types a runtime buffer array can't hold, and the numeric types it can.
	const RUNTIME_ARRAY_HANDLES: [&str; 8] = [
		"void",
		"bool",
		"Texture2D",
		"Texture2DArray",
		"Texture3D",
		"TextureCube",
		"TextureCubeArray",
		"StorageImage",
	];
	const RUNTIME_ARRAY_ELEMENTS: [&str; 8] = ["u8", "u16", "u32", "i32", "f16", "f32", "vec2u16", "vec3f"];

	fn runtime_array_source(element_type: &str) -> String {
		format!(
			"values: descriptor<{{ type: {element_type}[], binding: 0, access: read }}>; main: fn () -> void {{ let value: {element_type} = values[0]; value; }}"
		)
	}

	/// Verifies every source that breaks a linking rule fails, with a diagnostic that holds each expected part. An
	/// empty part list accepts any diagnostic.
	#[test]
	fn sources_that_break_a_linking_rule_are_rejected() {
		const ATOMIC_DOCS: &str = "/docs/reference/besl/intrinsics#buffer-and-workgroup-atomics";
		const BOOL_DOCS: &str = "reference/besl/language#bool-members";
		const NEXT: &str = "next: fn () -> u32 { return 0; }";
		let builtins = crate::Builtins::new();
		let sampler_program = |declarations: &str, read: &str| {
			format!(
				"farthest_depth: sampler<{{ mip_filter: nearest, reduction: min }}>;\n{declarations}\nmain: fn () -> void {{ let value: f32 = {read}; value; }}"
			)
		};
		let pyramid = "pyramid: descriptor<{ type: Texture2D, binding: 0, access: read, sampler: farthest_depth }>;";
		let mut cases: Vec<(String, Vec<&str>)> = vec![
			// Structural entry points.
			(
				"main: fn () -> output { color: vec4f, depth: f32 } { let color: vec4f = vec4f(1.0, 1.0, 1.0, 1.0); return { color }; }".into(),
				vec!["every declared field"],
			),
			(
				"main: fn (input: interface { position: vec4f }) -> output { color: vec4f } { return { color: input.position }; }".into(),
				vec!["cannot be an interface parameter"],
			),
			(
				"main: fn (input: interface { render_target_array_index: u32 }) -> output { color: vec4f } { return { color: vec4f(f32(input.render_target_array_index), 0.0, 0.0, 1.0) }; }".into(),
				vec!["cannot be an interface parameter"],
			),
			(
				"main: fn () -> interface { position: vec4f, render_target_array_index: f32 } { return { position: vec4f(0.0, 0.0, 0.0, 1.0), render_target_array_index: 1.0 }; }".into(),
				vec!["every backend reads as a `u32`"],
			),
			(
				"main: fn () -> interface { position: vec4f, instance_index: primitive<u32> } { return { position: vec4f(0.0, 0.0, 0.0, 1.0), instance_index: 0 }; }".into(),
				vec!["`primitive<T>` marks a fragment input"],
			),
			// Layered images and their layer indices.
			(
				"faces: descriptor<{ type: StorageImageArray<rgba16f>, binding: 0, access: write, count: 6 }>; main: fn () -> void { faces; }".into(),
				vec!["declares a count"],
			),
			(
				"faces: descriptor<{ type: StorageImageArray<rgba16f>, binding: 0, access: write }>; main: fn (input: StageInput) -> void { let layer: i32 = 1; write(faces[layer], input.thread_id, vec4f(1.0, 1.0, 1.0, 1.0)); }".into(),
				vec!["layer index must be u32"],
			),
			(
				"sprites: descriptor<{ type: Texture2DArray, binding: 0, access: read }>; main: fn () -> void { let color: vec4f = sample(sprites[1.0], vec2f(0.0, 0.0)); color; }".into(),
				vec!["layer index must be u32"],
			),
			// Buffers.
			(
				"Counters: struct { values: u32[8], } counters: descriptor<{ type: Counters, binding: 0, access: write, memory: constant }>; main: fn () -> void { counters.values[0] = 1; }".into(),
				vec!["uses constant memory"],
			),
			// Samplers.
			(
				sampler_program(
					"pyramid: descriptor<{ type: Texture2D, binding: 0, access: read, sampler: nearest_occluder }>;",
					"0.0",
				),
				vec!["isn't declared before it"],
			),
			(
				"pyramid: descriptor<{ type: Texture2D, binding: 0, access: read, sampler: late }>;\nlate: sampler<{}>;\nmain: fn () -> void {}".into(),
				vec!["isn't declared before it"],
			),
			(
				sampler_program(
					"Data: struct { value: f32, }\ndata: descriptor<{ type: Data, binding: 0, access: read, sampler: farthest_depth }>;",
					"data.value",
				),
				vec!["only texture descriptors take a sampler"],
			),
			(
				"blended: sampler<{ reduction: max }>;\npyramid: descriptor<{ type: Texture2D, binding: 0, access: read, sampler: blended }>;\nmain: fn () -> void {}".into(),
				vec!["blends levels"],
			),
			(
				sampler_program(
					"cube: descriptor<{ type: TextureCube, binding: 0, access: read, sampler: farthest_depth }>;",
					"0.0",
				),
				vec!["isn't 2D"],
			),
			(
				"wrapped: sampler<{ mip_filter: nearest, reduction: min, address: repeat }>;\npyramid: descriptor<{ type: Texture2D, binding: 0, access: read, sampler: wrapped }>;\nmain: fn () -> void {}".into(),
				vec!["doesn't clamp coordinates"],
			),
			(
				"pixelated: sampler<{ filter: nearest, anisotropy: 16 }>;\npyramid: descriptor<{ type: Texture2D, binding: 0, access: read, sampler: pixelated }>;\nmain: fn () -> void {}".into(),
				vec!["filters anisotropically but doesn't filter linearly"],
			),
			(
				format!(
					"farthest_depth: sampler<{{ mip_filter: nearest, reduction: min }}>;\n{pyramid}\nread: fn (texture: Texture2D) -> f32 {{ return texture_lod(texture, vec2f(0.5, 0.5), 0.0).x; }}\nmain: fn () -> void {{ let value: f32 = read(pyramid); value; }}"
				),
				vec!["Function `read` takes a texture whose sampler reduces"],
			),
			(
				sampler_program(pyramid, "sample(pyramid, vec2f(0.5, 0.5)).x"),
				vec!["`sample` reads a texture whose sampler reduces"],
			),
			(
				sampler_program(pyramid, "downsample_max(pyramid, vec2f(0.5, 0.5), 0.0)"),
				vec!["`downsample_max` reads a texture whose sampler reduces with `min`"],
			),
			// Atomic targets must be addressable storage.
			(
				"counter: workgroup<atomicu32>; main: fn () -> void { let copied: atomicu32 = counter; atomic_load(copied); }".into(),
				vec!["must come directly from a buffer or workgroup", ATOMIC_DOCS],
			),
			(
				"counter: workgroup<atomicu32>; copy: fn () -> atomicu32 { return counter; } main: fn () -> void { atomic_load(copy()); }".into(),
				vec!["must come directly from a buffer or workgroup", ATOMIC_DOCS],
			),
			// Types must name declared types, and structs must be finite and unique.
			(
				"data: descriptor<{ type: Missing, binding: 0, access: read }>;".into(),
				vec![r#"ReferenceToUndefinedType { type_name: "Missing" }"#],
			),
			(
				"Foo: struct {\n\tbar: NonExistantType\n}".into(),
				vec![r#"ReferenceToUndefinedType { type_name: "NonExistantType" }"#],
			),
			(
				"main: fn () -> NonExistantType {}".into(),
				vec![r#"ReferenceToUndefinedType { type_name: "NonExistantType" }"#],
			),
			// `root` names the program scope and `main` the function itself. Neither declares a type.
			("main: fn () -> root {}".into(), vec![r#"ReferenceToUndefinedType { type_name: "root" }"#]),
			("main: fn () -> main {}".into(), vec![r#"ReferenceToUndefinedType { type_name: "main" }"#]),
			("Light: struct { next: Light }".into(), vec!["Invalid {"]),
			("Light: struct { next: Light[2] }".into(), vec!["Invalid {"]),
			("Hit: struct { value: u32 }".into(), vec!["built-in type"]),
			("vec3f: struct { value: u32 }".into(), vec!["built-in type"]),
			// Functions and scopes aren't values.
			("main: fn () -> void { normalize(vec3f(main)); }".into(), vec!["Invalid {"]),
			("main: fn () -> void { let x: f32 = root; }".into(), vec!["Invalid {"]),
			(
				"noise: mod { SEED: const f32 = 1.0; } main: fn () -> void { let x: f32 = noise; }".into(),
				vec!["Invalid {"],
			),
			// Calls must match a signature.
			(
				"function: fn () -> void {}\nmain: fn () -> void { function(vec3f(1.0, 1.0, 1.0), vec3f(0.0, 0.0, 0.0)); }".into(),
				vec!["FunctionCallParametersDoNotMatchFunctionParameters"],
			),
			(
				"main: fn () -> void { set_mesh_primitive_render_target_array_index(0, 1.0); }".into(),
				vec!["FunctionCallParametersDoNotMatchFunctionParameters"],
			),
			(
				"texture_sampler: descriptor<{ type: Texture2D, binding: 0, access: read }>; main: fn () -> void { let color: vec4f = sample(texture_sampler); }".into(),
				vec!["FunctionCallParametersDoNotMatchFunctionParameters"],
			),
			// A bare name resolves only to declarations in scope: not to struct fields, resource members, or another
			// function's locals and parameters.
			("main: fn () -> void { x = 1; }".into(), vec!["UndeclaredName"]),
			(
				"Light: struct { intensity: f32, } main: fn () -> void { intensity = 1.0; }".into(),
				vec!["UndeclaredName"],
			),
			(
				"Data: struct { count: u32, } data: descriptor<{ type: Data, binding: 0, access: read }>; main: fn () -> void { count; }".into(),
				vec!["UndeclaredName"],
			),
			(
				"push_constant: push_constant { count: u32 } main: fn () -> void { count; }".into(),
				vec!["UndeclaredName"],
			),
			(
				"helper: fn () -> void { let hidden: f32 = 1.0; } main: fn () -> void { hidden; }".into(),
				vec!["UndeclaredName"],
			),
			(
				"helper: fn (hidden: f32) -> void { } main: fn () -> void { hidden; }".into(),
				vec!["UndeclaredName"],
			),
			// A statement block's last expression without `;` must not produce a value that would be silently lost.
			("main: fn () -> void { let x: u32 = 1; if (true) { x } }".into(), vec![]),
			("get: fn () -> u32 { let x: u32 = 1; x } main: fn () -> void {}".into(), vec![]),
			(
				"one: fn () -> u32 { return 1; } get: fn () -> u32 { one() } main: fn () -> void {}".into(),
				vec![],
			),
			("main: fn () -> void { let x: u32 = 1; match x { 0 => x, _ => {} } }".into(), vec![]),
			// A value whose branches run statements needs a statement to run them before. A `for` condition or update
			// runs on every iteration and a constant runs nowhere.
			(
				"main: fn () -> void { for (let i: u32 = 0; i < match i { _ => 2 }; i = i + 1) {} }".into(),
				vec![],
			),
			(
				"main: fn () -> void { for (let i: u32 = 0; i < 2; i = i + if (true) { let one: u32 = 1; one } else { 2 }) {} }"
					.into(),
				vec![],
			),
			("LIMIT: const u32 = u32(match 1 { _ => 2 }); main: fn () -> void {}".into(), vec![]),
			// Bool members.
			(
				"Flags: struct { values: bool[4] } main: fn () -> void {}".into(),
				vec!["member `values` is a `bool` array", BOOL_DOCS],
			),
			(
				"Flags: struct { _besl_bits0: u8, ready: bool } main: fn () -> void {}".into(),
				vec!["declares `_besl_bits0`", BOOL_DOCS],
			),
			(
				format!(
					"Flags: struct {{ ready: bool }} flags: descriptor<{{ type: Flags[], binding: 0, access: read_write }}>; {NEXT} main: fn () -> void {{ flags[next()].ready = true; }}"
				),
				vec!["written through a place that calls `next`", BOOL_DOCS],
			),
			(
				format!(
					"Bytes: struct {{ value: u8 }} bytes: descriptor<{{ type: Bytes[], binding: 0, access: read_write }}>; {NEXT} main: fn () -> void {{ bytes[next()].value = u8(1); }}"
				),
				vec!["written through a place that calls `next`", BOOL_DOCS],
			),
			(
				"Flags: struct { ready: bool } flags: descriptor<{ type: Flags, binding: 0, access: write }>; main: fn () -> void { flags.ready = true; }".into(),
				vec!["which is write-only", BOOL_DOCS],
			),
		];
		// Only short scalar arrays lower to the vectors backends return.
		for return_type in ["vec4f[16]", "u32[5]", "vec2u[2]"] {
			cases.push((
				format!("make: fn (values: {return_type}) -> {return_type} {{ return values; }} main: fn () -> void {{}}"),
				vec![
					"can't return an array",
					"/docs/reference/besl/language#pass-and-return-arrays",
				],
			));
		}
		for resource_type in RUNTIME_ARRAY_HANDLES {
			cases.push((runtime_array_source(resource_type), vec![]));
		}
		// Every atomic needs addressable storage, value-returning atomics a read-write buffer, and a store a writable
		// one.
		for operation in ATOMICS {
			cases.push((
				format!(
					"apply: fn (value: atomicu32) -> void {{ {} }} main: fn () -> void {{}}",
					atomic_statement(operation, "value")
				),
				vec!["must come directly from a buffer or workgroup", operation],
			));
			let statement = atomic_statement(operation, "counters.value");
			if operation == "atomic_store" {
				cases.push((
					atomic_counter_source("read", &statement),
					vec!["requires a writable buffer", ATOMIC_DOCS],
				));
			} else {
				for access in ["read", "write"] {
					cases.push((
						atomic_counter_source(access, &statement),
						vec!["requires a read-write buffer", ATOMIC_DOCS],
					));
				}
			}
		}
		// Declarations inside a block stay in that block.
		for block in [
			"",
			"if (true) { let leaked: u32 = 1; }",
			"if (true) {} else { let leaked: u32 = 1; }",
			"if (true) {} else if (true) { let leaked: u32 = 1; }",
			"match 1 { 0 => { let leaked: u32 = 1; } _ => {} }",
			"match 1 { 0 => {} _ => { let leaked: u32 = 1; } }",
			"for (let leaked: u32 = 0; leaked < 1; leaked = leaked + 1) {}",
			"for (let i: u32 = 0; i < 1; i = i + 1) { let leaked: u32 = 1; }",
			"let x: u32 = if (true) { let leaked: u32 = 1; leaked } else { 2 };",
			"let x: u32 = match 1 { 0 => { let leaked: u32 = 1; leaked } _ => 2 };",
		] {
			cases.push((format!("main: fn () -> void {{ {block} leaked = 2; }}"), vec![]));
		}
		// `break` and `continue` need an enclosing loop, even in a match arm that can never run, and each branch of an
		// `if` or `match` value must end in a value or leave with control flow, and an `if` value needs an `else`.
		for statement in [
			"break;",
			"if (true) { continue; }",
			"match true { _ => {} false => break }",
			"match 1 { 0 => {} _ => { match 2 { _ => continue } } }",
			"let x: u32 = if (true) { break; } else { 1 };",
			"let x: u32 = match 1 { 0 => { continue; } _ => 1 };",
			"let x: u32 = if (true) { 1 };",
			"let x: u32 = if (true) { 1; } else { 2 };",
			"let x: u32 = match 1 { 0 => {} _ => 2 };",
			"let x: u32 = if (true) { return; } else { return; };",
		] {
			cases.push((format!("main: fn () -> void {{ {statement} }}"), vec![]));
		}
		// A match must cover every scrutinee value with patterns of the scrutinee's type.
		for (scrutinee, arms) in [
			("0", "0 => {} 1 => {}"),
			("0", ""),
			("true", "true => {}"),
			("0", "true => {} _ => {}"),
			("0", "-1 => {} _ => {}"),
			("0", "1.0 => {} _ => {}"),
			("u16(0)", "65536 => {} _ => {}"),
			("1.0", "_ => {}"),
		] {
			cases.push((format!("main: fn () -> void {{ match {scrutinee} {{ {arms} }} }}"), vec![]));
		}

		for (source, expected) in cases {
			let error = crate::compile_to_besl(&source, crate::Program::new(&builtins))
				.expect_err(&format!("`{source}` should not link"));
			let error = format!("{error:?}");
			for part in expected {
				assert!(error.contains(part), "`{source}` failed without `{part}`: {error}");
			}
		}
	}

	/// Verifies sources at the edges of the linking rules link.
	#[test]
	fn sources_within_the_linking_rules_link() {
		let builtins = crate::Builtins::new();
		let mut sources: Vec<String> = [
			// A fragment stage reads the index of the primitive it shades through `StageInput`.
			"main: fn (input: StageInput) -> output { id: u32 } { return { id: input.primitive_index }; }",
			// Short scalar arrays lower to vectors, so they stay valid return types, and any array stays a valid parameter.
			"pass: fn (values: vec4f[16]) -> u32[4] { return u32[4](1, 2, 3, 4); } main: fn () -> void {}",
			"main: fn () -> void { let x: u32 = 1; if (true) { x = 2 } }",
			"main: fn () -> void { let x: u32 = 1; match x { 0 => x = 2, _ => {} } }",
			"get: fn () -> u32 { let x: u32 = 1; if (true) { return x } return 0; } main: fn () -> void {}",
			// A plain `if` value can run in a loop condition.
			"main: fn () -> void { for (let i: u32 = 0; i < if (true) { 2 } else { 3 }; i = i + 1) {} }",
			// A structural `main` can return an `if` value whose branches read its contextual inputs.
			"main: fn (pipeline_input: interface { uv: vec2f }) -> output { color: vec4f } { return { color: if (pipeline_input.uv.x > 0.5) { let red: f32 = pipeline_input.uv.y; vec4f(red, 0.0, 0.0, 1.0) } else { vec4f(pipeline_input.uv.x, 0.0, 0.0, 1.0) } }; }",
			// Matrix products expose their vector result to later overload resolution.
			"main: fn () -> void { let model: mat4x3f = mat4x3f(vec3f(1.0, 0.0, 0.0), vec3f(0.0, 1.0, 0.0), vec3f(0.0, 0.0, 1.0), vec3f(0.0, 0.0, 0.0)); let transformed: vec3f = normalize(model * vec4f(1.0, 2.0, 3.0, 1.0)); let scaled: vec3f = normalize(2.0 * transformed); }",
			// Bool members link in locals, constructors, and indexed places.
			"NormalDelta: struct { value: vec3f, valid: bool } Flags: struct { ready: bool, done: bool } flags: descriptor<{ type: Flags[], binding: 0, access: read_write }>; make: fn (valid: bool) -> NormalDelta { return NormalDelta(vec3f(0.0, 0.0, 0.0), valid); } main: fn () -> void { let index: u32 = 1; let delta: NormalDelta = make(true); flags[index].ready = delta.valid; flags[index + 1].done = !flags[index].ready; }",
		]
		.map(String::from)
		.to_vec();
		for body in [
			"match i { _ => {} 1 => break }",
			"let x: u32 = if (i == 0) { continue; } else { i };",
		] {
			sources.push(format!(
				"main: fn () -> void {{ for (let i: u32 = 0; i < 1; i = i + 1) {{ {body} }} }}"
			));
		}
		for (scrutinee, arms) in [
			("true", "true => {} false => {}"),
			("u16(0)", "65535 => {} _ => {}"),
			("0 < 1", "false => {} _ => {}"),
		] {
			sources.push(format!("main: fn () -> void {{ match {scrutinee} {{ {arms} }} }}"));
		}
		sources.extend(RUNTIME_ARRAY_ELEMENTS.map(runtime_array_source));
		for operation in ATOMICS {
			let statement = atomic_statement(operation, "counters.value");
			sources.push(atomic_counter_source("read_write", &statement));
			if operation == "atomic_store" {
				sources.push(atomic_counter_source("write", &statement));
			}
		}

		for source in sources {
			crate::compile_to_besl(&source, crate::Program::new(&builtins))
				.unwrap_or_else(|error| panic!("`{source}` should link: {error:?}"));
		}
	}

	/// Verifies intrinsic calls pick the overload their arguments' types select: an indexed array by its element type,
	/// not by the type of the index, and vectors by their vector type.
	#[test]
	fn intrinsic_calls_select_the_overload_of_their_argument_types() {
		let builtins = crate::Builtins::new();
		let script = r#"
		shared_depth: workgroup<f32, 4>;
		WEIGHTS: const f32[2] = f32[2](0.25, 0.75);
		main: fn () -> void {
			let index: u32 = 1;
			let shared_maximum: f32 = max(shared_depth[index], shared_depth[index + 1]);
			let constant_minimum: f32 = min(WEIGHTS[index], WEIGHTS[0]);
			let maximum: vec3f = max(vec3f(1.0, 2.0, 3.0), vec3f(4.0, 5.0, 6.0));
			let clamped: vec3f = clamp(vec3f(1.5, 0.5, 0.0), vec3f(0.0, 0.0, 0.0), vec3f(1.0, 1.0, 1.0));
		}
		"#;

		let program = crate::compile_to_besl(script, crate::Program::new(&builtins)).expect("Failed to lex");
		let main = program.get_main().expect("Expected main");
		let Nodes::Function { statements, .. } = program[main].node() else {
			panic!("Expected function");
		};

		let expected = [("max", "f32"), ("min", "f32"), ("max", "vec3f"), ("clamp", "vec3f")];
		for (statement, (expected_name, expected_type)) in statements[1..].iter().zip(expected) {
			let Nodes::Expression(Expressions::Operator { right, .. }) = program[statement].node() else {
				panic!("Expected assignment");
			};
			let Nodes::Expression(Expressions::IntrinsicCall { intrinsic, .. }) = program[right].node() else {
				panic!("Expected intrinsic call");
			};
			let Nodes::Intrinsic { name, r#return, .. } = program[intrinsic].node() else {
				panic!("Expected intrinsic");
			};
			assert_eq!(
				(program.text(*name), program.name(*r#return)),
				(expected_name, Some(expected_type))
			);
		}
	}
}
