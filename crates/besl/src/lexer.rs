//! BESL analysis and lowering into executable VM instructions.

mod ast;
mod entry;
mod lowering;
mod matching;
mod resolution;

pub use ast::{
	BindingTypes, BufferMemoryClass, CallTarget, ElseBranch, Expressions, FixedArray, LexError, MatchArm, Node, NodeReference,
	Nodes, Operators, UnaryOperators,
};
pub(crate) use ast::{TERNARY_PRECEDENCE, UNARY_PRECEDENCE, lex_with_root};
pub use resolution::infer_expression_type;

#[cfg(test)]
mod tests {
	use super::*;

	#[cfg(target_pointer_width = "64")]
	#[test]
	#[should_panic(expected = "resource array exceeds u32::MAX elements")]
	fn binding_array_rejects_count_larger_than_flat_metadata() {
		Node::binding_array(
			"textures",
			BindingTypes::CombinedImageSampler { format: String::new() },
			0,
			true,
			false,
			(u32::MAX as usize) + 1,
		);
	}

	#[test]
	fn source_descriptors_lower_to_existing_flat_binding_types() {
		let source = r#"
			Data: struct {
				value: u32,
				weight: f32,
			}
			data: descriptor<{ type: Data, binding: 2, access: read_write, memory: device }>;
			texture: descriptor<{ type: Texture2D, binding: 5, access: read }>;
			texture_array: descriptor<{ type: Texture2DArray, binding: 7, access: read, count: 16 }>;
			volume: descriptor<{ type: Texture3D, binding: 30, access: read }>;
			result: descriptor<{ type: StorageImage<rgba16f>, binding: 31, access: write }>;
			unformatted_result: descriptor<{ type: StorageImage, binding: 32, access: write }>;
			main: fn () -> void {
				data.value = data.value;
			}
		"#;

		let root = crate::compile_to_besl(source, None).expect("resource descriptors should lex");
		let data = root.borrow().get_child("data").expect("data descriptor should exist");

		assert!(matches!(
			data.borrow().node(),
			Nodes::Binding {
				slot: 2,
				read: true,
				write: true,
				memory_class: BufferMemoryClass::Device,
				r#type: BindingTypes::Buffer { members },
				count: None,
				..
			} if members.iter().map(|member| member.borrow().get_name().map(str::to_owned)).collect::<Vec<_>>()
				== vec![Some("value".to_string()), Some("weight".to_string())]
		));

		let texture = root.borrow().get_child("texture").expect("texture descriptor should exist");

		assert!(matches!(
			texture.borrow().node(),
			Nodes::Binding {
				slot: 5,
				read: true,
				write: false,
				r#type: BindingTypes::CombinedImageSampler { format },
				..
			} if format.is_empty()
		));

		let texture_array = root
			.borrow()
			.get_child("texture_array")
			.expect("texture array descriptor should exist");

		assert!(matches!(
			texture_array.borrow().node(),
			Nodes::Binding {
				slot: 7,
				r#type: BindingTypes::CombinedImageSampler { format },
				count: Some(count),
				..
			} if format == "ArrayTexture2D" && count.get() == 16
		));

		let volume = root.borrow().get_child("volume").expect("volume descriptor should exist");

		assert!(matches!(
			volume.borrow().node(),
			Nodes::Binding {
				r#type: BindingTypes::CombinedImageSampler { format },
				..
			} if format == "Texture3D"
		));

		let result = root
			.borrow()
			.get_child("result")
			.expect("storage image descriptor should exist");

		assert!(matches!(
			result.borrow().node(),
			Nodes::Binding {
				slot: 31,
				read: false,
				write: true,
				r#type: BindingTypes::Image { format },
				..
			} if format == "rgba16f"
		));

		let unformatted_result = root
			.borrow()
			.get_child("unformatted_result")
			.expect("unformatted storage image descriptor should exist");

		assert!(matches!(
			unformatted_result.borrow().node(),
			Nodes::Binding {
				slot: 32,
				read: false,
				write: true,
				r#type: BindingTypes::Image { format },
				..
			} if format == "unknown"
		));
	}

	#[test]
	fn structural_sprite_entry_points_lower_to_the_flat_semantic_abi() {
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
			None,
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
			None,
		)
		.expect("structural sprite fragment should link");

		fn io_location(root: &NodeReference, name: &str) -> u8 {
			let node = root.borrow().get_child(name).expect("structural field should exist");
			match node.borrow().node() {
				Nodes::Input { location, .. } | Nodes::Output { location, .. } => *location,
				_ => panic!("structural field should be stage I/O"),
			}
		}

		assert_eq!(io_location(&vertex, "_besl_interface_instance_index"), 0);
		assert_eq!(io_location(&vertex, "_besl_interface_uv"), 1);
		assert_eq!(io_location(&fragment, "_besl_interface_instance_index"), 0);
		assert_eq!(io_location(&fragment, "_besl_interface_uv"), 1);
		assert!(matches!(
			fragment
				.borrow()
				.get_child("instances")
				.expect("runtime buffer descriptor should exist")
				.borrow()
				.node(),
			Nodes::Binding {
				r#type: BindingTypes::BufferArray { element, .. },
				..
			} if element.borrow().get_name() == Some("Instance")
		));
		assert_eq!(io_location(&fragment, "_besl_output_color"), 0);
	}

	#[test]
	fn structural_entry_rejects_invalid_record_shapes() {
		for (source, expected) in [
			(
				r#"main: fn () -> output { color: vec4f, depth: f32 } {
					let color: vec4f = vec4f(1.0, 1.0, 1.0, 1.0);
					return { color };
				}"#,
				"every declared field",
			),
			(
				r#"main: fn (input: interface { position: vec4f }) -> output { color: vec4f } {
					return { color: input.position };
				}"#,
				"cannot be an interface parameter",
			),
		] {
			let error = crate::compile_to_besl(source, None).expect_err("invalid structural entry should fail while linking");
			assert!(matches!(
				error,
				crate::CompilationError::Lex(LexError::Invalid { message })
					if message.contains(expected)
			));
		}
	}

	#[test]
	fn runtime_buffer_arrays_accept_numeric_scalar_and_vector_elements() {
		for element_type in ["u8", "u16", "u32", "i32", "f16", "f32", "vec2u16", "vec3f"] {
			let source = format!(
				"values: descriptor<{{ type: {element_type}[], binding: 0, access: read }}>; main: fn () -> void {{ let value: {element_type} = values[7]; value; }}"
			);
			let root = crate::compile_to_besl(&source, None)
				.unwrap_or_else(|error| panic!("{element_type} runtime buffer should link: {error:?}"));
			let values = root
				.borrow()
				.get_child("values")
				.expect("runtime buffer descriptor should exist");
			assert!(matches!(
				values.borrow().node(),
				Nodes::Binding {
					r#type: BindingTypes::BufferArray { element, .. },
					..
				} if element.borrow().get_name() == Some(element_type)
			));
		}
	}

	#[test]
	fn runtime_buffer_arrays_reject_resource_handle_elements() {
		for resource_type in [
			"void",
			"bool",
			"Texture2D",
			"Texture2DArray",
			"Texture3D",
			"TextureCube",
			"TextureCubeArray",
			"StorageImage",
		] {
			let source = format!(
				"values: descriptor<{{ type: {resource_type}[], binding: 0, access: read }}>; main: fn () -> void {{ values; }}"
			);
			assert!(
				crate::compile_to_besl(&source, None).is_err(),
				"{resource_type} should not become a runtime buffer element"
			);
		}
	}

	#[test]
	fn texture_2d_array_layer_indices_must_be_u32() {
		let source = r#"
			sprites: descriptor<{ type: Texture2DArray, binding: 0, access: read }>;
			main: fn () -> void {
				let color: vec4f = sample(sprites[1.0], vec2f(0.0, 0.0));
				color;
			}
		"#;

		let error = crate::compile_to_besl(source, None).expect_err("f32 layer index should fail while linking");
		assert!(matches!(
			error,
			crate::CompilationError::Lex(LexError::Invalid { message })
				if message.contains("layer index must be u32")
		));
	}

	#[test]
	fn source_descriptor_rejects_writable_constant_buffers() {
		let source = r#"
			Counters: struct { values: u32[8], }
			counters: descriptor<{ type: Counters, binding: 0, access: write, memory: constant }>;
			main: fn () -> void { counters.values[0] = 1; }
		"#;

		assert!(
			crate::compile_to_besl(source, None).is_err(),
			"Writable buffers must select the device memory class"
		);
	}

	/// Builds one statement for each supported scalar buffer or workgroup atomic intrinsic.
	fn atomic_statement(name: &str, target: &str) -> String {
		match name {
			"atomic_load" => format!("{name}({target});"),
			"atomic_compare_exchange" => format!("{name}({target}, 1, 2);"),
			_ => format!("{name}({target}, 1);"),
		}
	}

	/// Compiles invalid source and returns the plain-language linker diagnostic.
	fn atomic_link_error(source: &str) -> String {
		match crate::compile_to_besl(source, None).expect_err("invalid atomic source should fail while linking") {
			crate::CompilationError::Lex(LexError::Invalid { message }) => message,
			error => panic!("Expected a detailed atomic linker error, found {error:?}"),
		}
	}

	#[test]
	fn functions_return_only_short_scalar_arrays() {
		for return_type in ["vec4f[16]", "u32[5]", "vec2u[2]"] {
			let source =
				format!("make: fn (values: {return_type}) -> {return_type} {{ return values; }} main: fn () -> void {{}}");
			let message = match crate::compile_to_besl(&source, None).expect_err("a large array return should fail to link") {
				crate::CompilationError::Lex(LexError::Invalid { message }) => message,
				error => panic!("Expected a detailed return-type error, found {error:?}"),
			};

			assert!(
				message.contains("can't return an array"),
				"Unexpected `{return_type}` error: {message}"
			);
			assert!(message.contains("/docs/reference/besl/language#pass-and-return-arrays"));
		}

		// Short scalar arrays lower to vectors, so they stay valid return types, and any array stays a valid parameter.
		crate::compile_to_besl(
			"pass: fn (values: vec4f[16]) -> u32[4] { return u32[4](1, 2, 3, 4); } main: fn () -> void {}",
			None,
		)
		.expect("a large array parameter with a short array return should link");
	}

	#[test]
	fn value_returning_atomics_require_read_write_buffers() {
		for operation in [
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
		] {
			let statement = atomic_statement(operation, "counters.value");
			for access in ["read", "write"] {
				let source = format!(
					r#"
						Counters: struct {{ value: atomicu32, }}
						counters: descriptor<{{ type: Counters, binding: 3, access: {access} }}>;
						main: fn () -> void {{ {statement} }}
					"#
				);
				let message = atomic_link_error(&source);
				assert!(
					message.contains("requires a read-write buffer"),
					"Unexpected `{operation}` error: {message}"
				);
				assert!(
					message.contains("/docs/reference/besl/intrinsics#buffer-and-workgroup-atomics"),
					"Atomic access diagnostics should link to the focused recovery documentation"
				);
			}

			let source = format!(
				r#"
					Counters: struct {{ value: atomicu32, }}
					counters: descriptor<{{ type: Counters, binding: 3, access: read_write }}>;
					main: fn () -> void {{ {statement} }}
				"#
			);
			crate::compile_to_besl(&source, None)
				.unwrap_or_else(|error| panic!("`{operation}` should accept a read-write descriptor: {error:?}"));
		}
	}

	#[test]
	fn atomic_store_accepts_write_access_and_rejects_read_only_buffers() {
		for access in ["write", "read_write"] {
			let source = format!(
				r#"
					Counters: struct {{ value: atomicu32, }}
					counters: descriptor<{{ type: Counters, binding: 3, access: {access} }}>;
					main: fn () -> void {{ atomic_store(counters.value, 1); }}
				"#
			);
			crate::compile_to_besl(&source, None)
				.unwrap_or_else(|error| panic!("Atomic store should accept `{access}` access: {error:?}"));
		}

		let source = r#"
			Counters: struct { value: atomicu32, }
			counters: descriptor<{ type: Counters, binding: 3, access: read }>;
			main: fn () -> void { atomic_store(counters.value, 1); }
		"#;
		let message = atomic_link_error(source);
		assert!(
			message.contains("requires a writable buffer"),
			"Unexpected atomic store error: {message}"
		);
		assert!(message.contains("/docs/reference/besl/intrinsics#buffer-and-workgroup-atomics"));
	}

	#[test]
	fn every_atomic_intrinsic_rejects_by_value_function_parameters() {
		for operation in [
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
		] {
			let statement = atomic_statement(operation, "value");
			let source = format!(
				r#"
					apply: fn (value: atomicu32) -> void {{ {statement} }}
					main: fn () -> void {{}}
				"#
			);
			let message = atomic_link_error(&source);
			assert!(
				message.contains("must come directly from a buffer or workgroup"),
				"Unexpected `{operation}` target error: {message}"
			);
			assert!(
				message.contains(operation),
				"Atomic target diagnostic should identify `{operation}`"
			);
		}
	}

	#[test]
	fn atomic_intrinsics_reject_local_values_and_function_results() {
		for source in [
			r#"
				counter: workgroup<atomicu32>;
				main: fn () -> void {
					let copied: atomicu32 = counter;
					atomic_load(copied);
				}
			"#,
			r#"
				counter: workgroup<atomicu32>;
				copy: fn () -> atomicu32 { return counter; }
				main: fn () -> void { atomic_load(copy()); }
			"#,
		] {
			let message = atomic_link_error(source);
			assert!(
				message.contains("must come directly from a buffer or workgroup"),
				"Unexpected target error: {message}"
			);
			assert!(message.contains("/docs/reference/besl/intrinsics#buffer-and-workgroup-atomics"));
		}
	}

	#[test]
	fn source_task_storage_and_stage_interfaces_link_without_injected_rust_nodes() {
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

		let root = crate::compile_to_besl(source, None).expect("standalone task shader should link");
		let payload = root
			.borrow()
			.get_child("visible_meshlets")
			.expect("task payload declaration should be linked");

		assert!(matches!(
			payload.borrow().node(),
			Nodes::TaskPayload { count, format, .. }
				if count.get() == 32 && format.borrow().get_name() == Some("u32")
		));
		assert!(payload.borrow().node().is_indexable());

		let workgroup = root
			.borrow()
			.get_child("visible_count")
			.expect("workgroup declaration should be linked");

		assert!(matches!(
			workgroup.borrow().node(),
			Nodes::Workgroup { format, .. } if format.borrow().get_name() == Some("atomicu32")
		));
		let scratch = root
			.borrow()
			.get_child("scratch")
			.expect("counted workgroup declaration should be linked");

		assert!(matches!(
			scratch.borrow().node(),
			Nodes::Workgroup {
				format,
				count: Some(count),
				..
			} if count.get() == 64 && format.borrow().get_name() == Some("f32")
		));
		assert!(scratch.borrow().node().is_indexable());
		assert!(root.get_main().is_some());
	}

	#[test]
	fn source_buffer_descriptor_requires_a_declared_type() {
		let parsed =
			crate::parse("data: descriptor<{ type: Missing, binding: 0, access: read }>;").expect("descriptor should parse");

		assert_eq!(
			lex_with_root(Node::root(), parsed),
			Err(LexError::ReferenceToUndefinedType {
				type_name: "Missing".to_string(),
			})
		);
	}

	fn assert_type(node: &Node, type_name: &str) {
		match &node.node {
			Nodes::Struct { name, .. } => {
				assert_eq!(name, type_name);
			}
			_ => {
				panic!("Expected type");
			}
		}
	}

	#[test]
	fn lex_non_existant_function_struct_member_type() {
		let source = "
Foo: struct {
	bar: NonExistantType
}";

		let node = crate::parse(source).expect("Failed to parse");
		lex_with_root(Node::root(), node)
			.err()
			.filter(|e| {
				e == &LexError::ReferenceToUndefinedType {
					type_name: "NonExistantType".to_string(),
				}
			})
			.expect("Expected error");
	}

	#[test]
	fn lex_rejects_non_type_names_as_types() {
		// `root` names the program scope and `main` names the function itself. Neither declares a type.
		for (source, type_name) in [("main: fn () -> root {}", "root"), ("main: fn () -> main {}", "main")] {
			let node = crate::parse(source).expect("Failed to parse");
			assert_eq!(
				lex_with_root(Node::root(), node).err(),
				Some(LexError::ReferenceToUndefinedType {
					type_name: type_name.to_string(),
				})
			);
		}
	}

	#[test]
	fn lex_rejects_functions_used_as_values() {
		let source = "main: fn () -> void { normalize(vec3f(main)); }";
		let node = crate::parse(source).expect("Failed to parse");

		assert!(matches!(lex_with_root(Node::root(), node), Err(LexError::Invalid { .. })));
	}

	#[test]
	fn recursive_function_calls_link_to_their_function() {
		let program = crate::compile_to_besl("count: fn (n: u32) -> u32 { return count(n); }", None)
			.expect("Recursive functions should link");
		let count = program.borrow().get_child("count").expect("Expected count function");
		let count_ref = count.borrow();
		let Nodes::Function { statements, .. } = count_ref.node() else {
			panic!("Expected count function");
		};
		let statement = statements[0].borrow();
		let Nodes::Expression(Expressions::Return { value: Some(value) }) = statement.node() else {
			panic!("Expected return statement");
		};
		let value = value.borrow();
		let Nodes::Expression(Expressions::FunctionCall { function, .. }) = value.node() else {
			panic!("Expected recursive call");
		};

		assert_eq!(function.get(), count);
	}

	#[test]
	fn lex_non_existant_function_return_type() {
		let source = "
main: fn () -> NonExistantType {}";

		let node = crate::parse(source).expect("Failed to parse");
		lex_with_root(Node::root(), node)
			.err()
			.filter(|e| {
				e == &LexError::ReferenceToUndefinedType {
					type_name: "NonExistantType".to_string(),
				}
			})
			.expect("Expected error");
	}

	#[test]
	fn lex_wrong_parameter_count() {
		let source = "
function: fn () -> void {}
main: fn () -> void {
	function(vec3f(1.0, 1.0, 1.0), vec3f(0.0, 0.0, 0.0));
}";

		let node = crate::parse(source).expect("Failed to parse");
		lex_with_root(Node::root(), node)
			.err()
			.filter(|e| e == &LexError::FunctionCallParametersDoNotMatchFunctionParameters)
			.expect("Expected error");
	}

	#[test]
	fn mesh_render_target_array_index_requires_unsigned_indices() {
		let source = "
main: fn () -> void {
	set_mesh_primitive_render_target_array_index(0, 1.0);
}";

		let node = crate::parse(source).expect("Failed to parse");

		assert_eq!(
			lex_with_root(Node::root(), node).expect_err("The mesh primitive and array indices must both be u32"),
			LexError::FunctionCallParametersDoNotMatchFunctionParameters
		);
	}

	/// Asserts that `node` references a buffer binding lowered from a lone fixed-array member.
	fn assert_lowered_array_reference(node: &NodeReference, binding: &str, alias: &str, count: usize) {
		let node = node.borrow();
		let Nodes::Expression(Expressions::Member { name, source }) = node.node() else {
			panic!("Expected a binding reference");
		};
		assert_eq!(name, binding);
		assert!(matches!(
			source.borrow().node(),
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
		let script = r#"
		main: fn () -> void {
			let material_index: u32 = meshes.meshes[0].material_index;
			let mapped: u32 = pixel_mapping.pixel_mapping[1];
		}
		"#;

		let mut root = Node::root();
		let u32_type = root.get_child("u32").expect("Expected u32");
		let mesh = root.add_child(Node::r#struct("Mesh", vec![Node::member("material_index", u32_type.clone()).into()]).into());

		root.add_children(vec![
			Node::binding(
				"meshes",
				BindingTypes::Buffer {
					members: vec![Node::array("meshes", mesh, 4)],
				},
				0,
				true,
				false,
			)
			.into(),
			Node::binding(
				"pixel_mapping",
				BindingTypes::Buffer {
					members: vec![Node::array("pixel_mapping", u32_type, 4)],
				},
				1,
				true,
				true,
			)
			.into(),
		]);

		let node = crate::compile_to_besl(script, Some(root)).expect("Failed to lex");
		let main = node.get_descendant("main").expect("Expected main");
		let main = main.borrow();

		let Nodes::Function { statements, .. } = main.node() else {
			panic!("Expected function");
		};

		let material_index_access = match statements[0].borrow().node() {
			Nodes::Expression(Expressions::Operator { right, .. }) => right.clone(),
			_ => panic!("Expected assignment"),
		};
		let (indexed_meshes, material_index_member) = match material_index_access.borrow().node() {
			Nodes::Expression(Expressions::Accessor { left, right }) => (left.clone(), right.clone()),
			_ => panic!("Expected struct member accessor"),
		};
		match material_index_member.borrow().node() {
			Nodes::Expression(Expressions::Member { name, source }) => {
				assert_eq!(name, "material_index");
				assert!(matches!(
					source.borrow().node(),
					Nodes::Member { name, count, .. } if name == "material_index" && count.is_none()
				));
			}
			_ => panic!("Expected material_index member expression"),
		}

		match indexed_meshes.borrow().node() {
			Nodes::Expression(Expressions::Accessor { left, .. }) => {
				assert_lowered_array_reference(left, "meshes", "meshes", 4)
			}
			_ => panic!("Expected indexed meshes accessor"),
		}

		let pixel_mapping_access = match statements[1].borrow().node() {
			Nodes::Expression(Expressions::Operator { right, .. }) => right.clone(),
			_ => panic!("Expected assignment"),
		};
		match pixel_mapping_access.borrow().node() {
			Nodes::Expression(Expressions::Accessor { left, .. }) => {
				assert_lowered_array_reference(left, "pixel_mapping", "pixel_mapping", 4);
			}
			_ => panic!("Expected indexed pixel_mapping accessor"),
		}
	}

	#[test]
	fn lex_local_named_like_its_field_resolves_access_to_the_field() {
		let script = r#"
		Transform: struct { model: mat4f, }
		transforms: descriptor<{ type: Transform[], binding: 0, access: read }>;
		main: fn () -> void {
			let model: Transform = transforms[0];
			model.model[0];
		}
		"#;

		let node = crate::compile_to_besl(script, None).expect("Failed to lex");
		let main = node.get_descendant("main").expect("Expected main");
		let main = main.borrow();
		let Nodes::Function { statements, .. } = main.node() else {
			panic!("Expected function");
		};

		let statement = statements[1].borrow();
		let Nodes::Expression(Expressions::Accessor { left: field_access, .. }) = statement.node() else {
			panic!("Expected indexed field access");
		};
		let field_access = field_access.borrow();
		let Nodes::Expression(Expressions::Accessor { right: field, .. }) = field_access.node() else {
			panic!("Expected field access");
		};
		let field = field.borrow();
		let Nodes::Expression(Expressions::Member { source, .. }) = field.node() else {
			panic!("Expected field member expression");
		};

		assert!(
			matches!(source.borrow().node(), Nodes::Member { name, .. } if name == "model"),
			"Expected `model.model` to resolve to the `Transform.model` field instead of the local"
		);
	}

	// #[test]
	// fn push_constant() {
	// }

	// TODO: test function with body with missing close brace

	#[test]
	fn lex_builtin_texture_intrinsics_validate_parameter_count() {
		let source = r#"
		main: fn () -> void {
			let color: vec4f = sample(texture_sampler);
		}
		"#;

		let parsed = crate::parse(source).expect("Failed to parse");

		let mut root = Node::root();
		root.add_child(
			Node::binding(
				"texture_sampler",
				BindingTypes::CombinedImageSampler { format: String::new() },
				0,
				true,
				false,
			)
			.into(),
		);

		lex_with_root(root, parsed)
			.err()
			.filter(|error| error == &LexError::FunctionCallParametersDoNotMatchFunctionParameters)
			.expect("Expected parameter count validation error");
	}

	#[test]
	fn lex_const_variable() {
		let script = r#"
		PI: const f32 = 3.14;

		main: fn () -> void {
			PI;
		}
		"#;

		let node = crate::compile_to_besl(script, None).expect("Failed to lex");

		let pi = node.get_descendant("PI").expect("Expected PI const");
		let pi = pi.borrow();

		match pi.node() {
			Nodes::Const { name, r#type, value } => {
				assert_eq!(name, "PI");
				assert_eq!(r#type.borrow().get_name().unwrap(), "f32");
				match value.borrow().node() {
					Nodes::Expression(Expressions::Literal { value }) => {
						assert_eq!(value, "3.14");
					}
					_ => panic!("Expected a literal expression value"),
				}
			}
			_ => panic!("Expected Const node"),
		}
	}

	/// Declarations inside a block stay in that block, so statements after it can't reference them.
	#[test]
	fn block_declarations_are_not_visible_after_the_block() {
		for block in [
			"",
			"if (true) { let leaked: u32 = 1; }",
			"if (true) {} else { let leaked: u32 = 1; }",
			"if (true) {} else if (true) { let leaked: u32 = 1; }",
			"match 1 { 0 => { let leaked: u32 = 1; } _ => {} }",
			"match 1 { 0 => {} _ => { let leaked: u32 = 1; } }",
			"for (let leaked: u32 = 0; leaked < 1; leaked = leaked + 1) {}",
			"for (let i: u32 = 0; i < 1; i = i + 1) { let leaked: u32 = 1; }",
		] {
			let source = format!("main: fn () -> void {{ {block} leaked = 2; }}");
			assert!(
				crate::compile_to_besl(&source, None).is_err(),
				"`leaked` should not resolve after `{block}`"
			);
		}
	}

	/// `break` and `continue` need an enclosing loop, even in a match arm that can never run.
	#[test]
	fn loop_control_outside_a_loop_is_rejected() {
		for statement in [
			"break;",
			"if (true) { continue; }",
			"match true { _ => {} false => break }",
			"match 1 { 0 => {} _ => { match 2 { _ => continue } } }",
		] {
			let source = format!("main: fn () -> void {{ {statement} }}");
			assert!(crate::compile_to_besl(&source, None).is_err(), "`{statement}` should not lex");
		}

		let source = "main: fn () -> void { for (let i: u32 = 0; i < 1; i = i + 1) { match i { _ => {} 1 => break } } }";
		assert!(crate::compile_to_besl(source, None).is_ok());
	}

	/// A match lexes to distinct labels and one default, following Rust's first-match-wins rule.
	#[test]
	fn match_resolves_unreachable_patterns() {
		let root = crate::compile_to_besl(
			"main: fn () -> void { let n: u32 = 0; match n { 0 | 1 => n = 1, 1 | 2 => n = 2, 0 => n = 3, 3 | _ => n = 4, 4 => n = 5 } }",
			None,
		)
		.expect("Expected the match to lex");
		let main = root.borrow().get_child("main").expect("Expected main");
		let main = main.borrow();
		let Nodes::Function { statements, .. } = main.node() else {
			panic!("Expected function");
		};
		let statement = statements[1].borrow();
		let Nodes::Match {
			r#type, arms, default, ..
		} = statement.node()
		else {
			panic!("Expected match");
		};

		assert_eq!(r#type.borrow().get_name(), Some("u32"));
		// `1` already belongs to the first arm, and the `0` arm can never run.
		let labels: Vec<&[i64]> = arms.iter().map(|arm| arm.values.as_slice()).collect();
		assert_eq!(labels, [&[0, 1][..], &[2]]);
		// The `3 | _` arm catches every other value, so the `4` arm after it is unreachable.
		assert_eq!(default.len(), 1);
	}

	/// A match must cover every scrutinee value with patterns of the scrutinee's type, as in Rust.
	#[test]
	fn match_rejects_invalid_patterns() {
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
			let source = format!("main: fn () -> void {{ match {scrutinee} {{ {arms} }} }}");
			assert!(crate::compile_to_besl(&source, None).is_err(), "`{source}` should not lex");
		}

		for (scrutinee, arms) in [
			("true", "true => {} false => {}"),
			("u16(0)", "65535 => {} _ => {}"),
			("0 < 1", "false => {} _ => {}"),
		] {
			let source = format!("main: fn () -> void {{ match {scrutinee} {{ {arms} }} }}");
			assert!(crate::compile_to_besl(&source, None).is_ok(), "`{source}` should lex");
		}
	}

	/// Verifies an indexed array selects overloads by its element type, not by the type of the index.
	#[test]
	fn lex_indexed_array_arguments_select_element_overloads() {
		let script = r#"
		shared_depth: workgroup<f32, 4>;
		WEIGHTS: const f32[2] = f32[2](0.25, 0.75);
		main: fn () -> void {
			let index: u32 = 1;
			let shared_maximum: f32 = max(shared_depth[index], shared_depth[index + 1]);
			let constant_minimum: f32 = min(WEIGHTS[index], WEIGHTS[0]);
		}
		"#;

		let node = crate::compile_to_besl(script, None).expect("Failed to lex");
		let main = node.get_descendant("main").expect("Expected main");
		let main = main.borrow();

		let Nodes::Function { statements, .. } = main.node() else {
			panic!("Expected function");
		};

		for (statement, expected_name) in [(&statements[1], "max"), (&statements[2], "min")] {
			match statement.borrow().node() {
				Nodes::Expression(Expressions::Operator { right, .. }) => match right.borrow().node() {
					Nodes::Expression(Expressions::IntrinsicCall { intrinsic, .. }) => match intrinsic.borrow().node() {
						Nodes::Intrinsic { name, r#return, .. } => {
							assert_eq!(name, expected_name);
							assert_type(&r#return.borrow(), "f32");
						}
						_ => panic!("Expected intrinsic"),
					},
					_ => panic!("Expected intrinsic call"),
				},
				_ => panic!("Expected assignment"),
			}
		}
	}

	#[test]
	fn lex_vector_intrinsic_overloads_still_resolve() {
		let script = r#"
		main: fn () -> void {
			let maximum: vec3f = max(vec3f(1.0, 2.0, 3.0), vec3f(4.0, 5.0, 6.0));
			let clamped: vec3f = clamp(vec3f(1.5, 0.5, 0.0), vec3f(0.0, 0.0, 0.0), vec3f(1.0, 1.0, 1.0));
		}
		"#;

		let node = crate::compile_to_besl(script, None).expect("Failed to lex");
		let main = node.get_descendant("main").expect("Expected main");
		let main = main.borrow();

		let Nodes::Function { statements, .. } = main.node() else {
			panic!("Expected function");
		};

		for (statement, expected_name, expected_type) in [(&statements[0], "max", "vec3f"), (&statements[1], "clamp", "vec3f")]
		{
			match statement.borrow().node() {
				Nodes::Expression(Expressions::Operator { right, .. }) => match right.borrow().node() {
					Nodes::Expression(Expressions::IntrinsicCall { intrinsic, .. }) => match intrinsic.borrow().node() {
						Nodes::Intrinsic { name, r#return, .. } => {
							assert_eq!(name, expected_name);
							assert_type(&r#return.borrow(), expected_type);
						}
						_ => panic!("Expected intrinsic"),
					},
					_ => panic!("Expected intrinsic call"),
				},
				_ => panic!("Expected assignment"),
			}
		}
	}

	/// Verifies matrix products expose their vector result to subsequent intrinsic overload resolution.
	#[test]
	fn lex_matrix_vector_and_scalar_vector_expression_results() {
		let script = r#"
		main: fn () -> void {
			let model: mat4x3f = mat4x3f(
				vec3f(1.0, 0.0, 0.0),
				vec3f(0.0, 1.0, 0.0),
				vec3f(0.0, 0.0, 1.0),
				vec3f(0.0, 0.0, 0.0)
			);
			let transformed: vec3f = normalize(model * vec4f(1.0, 2.0, 3.0, 1.0));
			let scaled: vec3f = normalize(2.0 * transformed);
		}
		"#;

		crate::compile_to_besl(script, None).expect(
			"Failed to resolve matrix-vector arithmetic. The most likely cause is incorrect BESL operator result typing.",
		);
	}
	/// Verifies a bare name resolves only to declarations in scope, never to fields of a struct type.
	#[test]
	fn bare_names_do_not_resolve_to_struct_fields() {
		for source in [
			"main: fn () -> void { x = 1; }",
			"Light: struct { intensity: f32, } main: fn () -> void { intensity = 1.0; }",
		] {
			let error = crate::compile_to_besl(source, None).expect_err("an undeclared bare name should fail to link");
			assert!(
				matches!(
					error,
					crate::CompilationError::Lex(LexError::AccessingUndeclaredMember { .. })
				),
				"{source} linked as {error:?}"
			);
		}
	}

	/// Verifies buffer and push constant members are reached only through their resource.
	#[test]
	fn bare_names_do_not_resolve_to_resource_members() {
		for source in [
			"Data: struct { count: u32, } data: descriptor<{ type: Data, binding: 0, access: read }>; main: fn () -> void { count; }",
			"push_constant: push_constant { count: u32 } main: fn () -> void { count; }",
		] {
			let error = crate::compile_to_besl(source, None).expect_err("a bare resource member should fail to link");
			assert!(
				matches!(
					error,
					crate::CompilationError::Lex(LexError::AccessingUndeclaredMember { .. })
				),
				"{source} linked as {error:?}"
			);
		}
	}

	/// Verifies a function's locals and parameters stay private to it.
	#[test]
	fn bare_names_do_not_resolve_to_other_function_locals() {
		for source in [
			"helper: fn () -> void { let hidden: f32 = 1.0; } main: fn () -> void { hidden; }",
			"helper: fn (hidden: f32) -> void { } main: fn () -> void { hidden; }",
		] {
			let error = crate::compile_to_besl(source, None).expect_err("another function's local should not be visible");
			assert!(
				matches!(
					error,
					crate::CompilationError::Lex(LexError::AccessingUndeclaredMember { .. })
				),
				"{source} linked as {error:?}"
			);
		}
	}

	/// Verifies `get_main` returns the entry-point function even when a struct declares a `main` member first.
	#[test]
	fn get_main_returns_the_entry_point_function() {
		let source = r#"
			Config: struct { main: u32, }
			main: fn () -> void { }
		"#;

		let root = crate::compile_to_besl(source, None).expect("source should link");
		let main = root.get_main().expect("main function should be found");
		assert!(matches!(main.borrow().node(), Nodes::Function { name, .. } if name == "main"));

		let root = crate::compile_to_besl("Config: struct { main: u32, }", None).expect("source should link");
		assert!(root.get_main().is_none(), "a struct member is not an entry point");
	}
}
