//! BESL analysis and lowering into executable VM instructions.

mod ast;
mod lowering;
mod resolution;

#[cfg(test)]
use std::{cell::RefCell, num::NonZeroUsize};

pub(crate) use ast::{lex, lex_with_root};
pub use ast::{
	BindingTypes, BufferMemoryClass, Expressions, LexError, Node, NodeReference, Nodes, Operators, ParentNodeReference,
};
#[cfg(test)]
use resolution::*;

#[cfg(test)]
use crate::parser;
#[cfg(test)]
#[cfg(test)]
mod tests {
	use super::*;
	use crate::tokenizer;

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
			data: descriptor<Data, 2, read_write, device>;
			texture: descriptor<Texture2D, 5, read>;
			texture_array: descriptor<Texture2DArray, 7, read, 16>;
			volume: descriptor<Texture3D, 30, read>;
			result: descriptor<StorageImage<rgba16f>, 31, write>;
			unformatted_result: descriptor<StorageImage, 32, write>;
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
	fn source_descriptor_rejects_writable_constant_buffers() {
		let source = r#"
			Counters: struct { values: u32[8], }
			counters: descriptor<Counters, 0, write, constant>;
			main: fn () -> void { counters.values[0] = 1; }
		"#;

		assert!(
			crate::compile_to_besl(source, None).is_err(),
			"Writable buffers must select the device memory class"
		);
	}

	#[test]
	fn source_atomic_buffers_and_push_constants_link_without_injected_rust_nodes() {
		let source = r#"
			Counters: struct {
				values: atomicu32[8],
			}
			counters: descriptor<Counters, 3, read_write>;
			push_constant: push_constant {
				index: u32,
			}
			main: fn () -> void {
				let old: u32 = atomic_add(counters.values[push_constant.index], 1);
				atomic_store(counters.values[push_constant.index], atomic_load(counters.values[old]));
			}
		"#;

		let root = crate::compile_to_besl(source, None).expect("standalone atomic shader should link");
		root.get_main().expect("standalone atomic shader should have main");

		assert!(root.borrow().get_child("push_constant").is_some());
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
	fn source_boolean_literals_link_as_bool_values() {
		let source = r#"
			main: fn () -> void {
				let enabled: bool = true;
				let disabled: bool = false;
			}
		"#;

		let root = crate::compile_to_besl(source, None).expect("boolean literals should link");
		root.get_main().expect("boolean literal shader should have main");

		assert_eq!(infer_literal_type("true").unwrap().borrow().get_name(), Some("bool"));
		assert_eq!(infer_literal_type("false").unwrap().borrow().get_name(), Some("bool"));
	}

	#[test]
	fn source_buffer_descriptor_requires_a_declared_type() {
		let tokens = tokenizer::tokenize("data: descriptor<Missing, 0, read>;").expect("descriptor should tokenize");
		let parsed = parser::parse(&tokens).expect("descriptor should parse");

		assert_eq!(
			lex(parsed),
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
	fn raw_code_constructors_select_only_the_requested_backend() {
		const EXPECTED: [(Option<&str>, Option<&str>, Option<&str>); 3] =
			[(Some("g"), None, None), (None, Some("h"), None), (None, None, Some("m"))];

		let parser_nodes = [
			parser::Node::glsl("g", &[], &[]),
			parser::Node::hlsl("h", &[], &[]),
			parser::Node::msl("m", &[], &[]),
		];
		let linked_nodes = [
			Node::glsl("g".into(), Vec::new(), Vec::new()),
			Node::hlsl("h".into(), Vec::new(), Vec::new()),
			Node::msl("m".into(), Vec::new(), Vec::new()),
		];

		for ((parser_node, linked_node), expected) in parser_nodes.into_iter().zip(linked_nodes).zip(EXPECTED) {
			let parser::Nodes::RawCode { glsl, hlsl, msl, .. } = parser_node.node() else {
				panic!("Expected parser raw-code node. The constructor returned a different node variant.");
			};

			assert_eq!((glsl.as_deref(), hlsl.as_deref(), msl.as_deref()), expected);

			let Nodes::Raw { glsl, hlsl, msl, .. } = linked_node.node() else {
				panic!("Expected linked raw-code node. The constructor returned a different node variant.");
			};

			assert_eq!((glsl.as_deref(), hlsl.as_deref(), msl.as_deref()), expected);
		}
	}

	#[test]
	fn lex_non_existant_function_struct_member_type() {
		let source = "
Foo: struct {
	bar: NonExistantType
}";

		let tokens = tokenizer::tokenize(source).expect("Failed to tokenize");
		let node = parser::parse(&tokens).expect("Failed to parse");
		lex(node)
			.err()
			.filter(|e| {
				e == &LexError::ReferenceToUndefinedType {
					type_name: "NonExistantType".to_string(),
				}
			})
			.expect("Expected error");
	}

	#[test]
	fn lex_non_existant_function_return_type() {
		let source = "
main: fn () -> NonExistantType {}";

		let tokens = tokenizer::tokenize(source).expect("Failed to tokenize");
		let node = parser::parse(&tokens).expect("Failed to parse");
		lex(node)
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

		let tokens = tokenizer::tokenize(source).expect("Failed to tokenize");
		let node = parser::parse(&tokens).expect("Failed to parse");
		lex(node)
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

		let tokens = tokenizer::tokenize(source).expect("Failed to tokenize");
		let node = parser::parse(&tokens).expect("Failed to parse");

		assert_eq!(
			lex(node).expect_err("The mesh primitive and array indices must both be u32"),
			LexError::FunctionCallParametersDoNotMatchFunctionParameters
		);
	}

	#[test]
	fn lex_function() {
		let source = "
main: fn () -> void {
	let position: vec4f = vec4f(0.0, 0.0, 0.0, 1.0);
	position = position;
}";

		let tokens = tokenizer::tokenize(source).expect("Failed to tokenize");
		let node = parser::parse(&tokens).expect("Failed to parse");
		let node = lex(node).expect("Failed to lex");

		let vec4f = node.get_descendant("vec4f").expect("Expected vec4f");

		let nb = node.borrow();

		match &nb.node {
			Nodes::Scope { .. } => {
				let main = node.get_descendant("main").expect("Expected main");
				let main = RefCell::borrow(&main.0);

				match main.node() {
					Nodes::Function {
						name,
						return_type,
						statements,
						..
					} => {
						assert_eq!(name, "main");
						assert_type(&return_type.borrow(), "void");

						let position = statements[0].borrow();

						match position.node() {
							Nodes::Expression(Expressions::Operator { operator, left, right }) => {
								let position = left.borrow();

								assert_eq!(operator, &Operators::Assignment);

								match position.node() {
									Nodes::Expression(Expressions::VariableDeclaration { name, r#type }) => {
										assert_eq!(name, "position");
										assert_eq!(r#type, &vec4f);
									}
									_ => {
										panic!("Expected expression");
									}
								}

								let constructor = right.borrow();

								match constructor.node() {
									Nodes::Expression(Expressions::FunctionCall {
										function, parameters, ..
									}) => {
										let function = RefCell::borrow(&function.0);
										let name = function.get_name().expect("Expected name");

										assert_eq!(name, "vec4f");
										assert_eq!(parameters.len(), 4);
									}
									_ => {
										panic!("Expected expression");
									}
								}
							}
							_ => {
								panic!("Expected variable declaration");
							}
						}
					}
					_ => {
						panic!("Expected function.");
					}
				}
			}
			_ => {
				panic!("Expected scope");
			}
		}
	}

	#[test]
	fn parse_script() {
		let script = r#"
		used: fn () -> void {
			return;
		}

		not_used: fn () -> void {
			return;
		}

		main: fn () -> void {
			used();
		}
		"#;

		let tokens = tokenizer::tokenize(script).expect("Failed to tokenize");
		let node = parser::parse(&tokens).expect("Failed to parse");
		lex(node).expect("Failed to lex");
	}

	#[test]
	fn lex_struct() {
		let script = r#"
		Vertex: struct {
			array: u32[3],
			position: vec3f,
			normal: vec3f,
		}
		"#;

		let tokens = tokenizer::tokenize(script).expect("Failed to tokenize");
		let node = parser::parse(&tokens).expect("Failed to parse");
		let node = lex(node).expect("Failed to lex");

		let nb = node.borrow();

		match nb.node() {
			Nodes::Scope { name, .. } => {
				assert_eq!(name, "root");

				let vertex = node.get_descendant("Vertex").expect("Expected Vertex");
				let vertex = RefCell::borrow(&vertex.0);

				match vertex.node() {
					Nodes::Struct { name, fields, .. } => {
						assert_eq!(name, "Vertex");
						assert_eq!(fields.len(), 3);

						let array = fields[0].borrow();

						match array.node() {
							Nodes::Member { name, r#type, count } => {
								assert_eq!(name, "array");
								assert_type(&r#type.borrow(), "u32");

								assert_eq!(count, &Some(NonZeroUsize::new(3).expect("Invalid count")));
							}
							_ => {
								panic!("Expected member");
							}
						}
					}
					_ => {
						panic!("Expected struct");
					}
				}
			}
			_ => {
				panic!("Expected scope");
			}
		}
	}

	#[test]
	fn lex_array_index_accessor() {
		let script = r#"
		main: fn () -> void {
			let value: f32 = buff.values[1];
		}
		"#;

		let mut root = Node::root();
		let float_type = root.get_child("f32").expect("Expected f32");
		root.add_child(
			Node::binding(
				"buff",
				BindingTypes::Buffer {
					members: vec![Node::array("values", float_type, 3)],
				},
				0,
				true,
				false,
			)
			.into(),
		);

		let node = crate::compile_to_besl(script, Some(root)).expect("Failed to lex");
		let main = node.get_descendant("main").expect("Expected main");
		let main = main.borrow();

		let Nodes::Function { statements, .. } = main.node() else {
			panic!("Expected function");
		};

		let statement = statements[0].borrow();
		let Nodes::Expression(Expressions::Operator { right, .. }) = statement.node() else {
			panic!("Expected assignment");
		};
		let right = right.borrow();
		let Nodes::Expression(Expressions::Accessor { left, right }) = right.node() else {
			panic!("Expected outer accessor");
		};

		assert!(matches!(
			right.borrow().node(),
			Nodes::Expression(Expressions::Expression { elements })
				if elements.len() == 1
					&& matches!(elements[0].borrow().node(), Nodes::Expression(Expressions::Literal { value }) if value == "1")
		));
		assert!(matches!(
			left.borrow().node(),
			Nodes::Expression(Expressions::Accessor { .. })
		));
	}

	#[test]
	fn lex_same_named_buffer_members_resolve_to_member_declarations() {
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

		let meshes_member = match indexed_meshes.borrow().node() {
			Nodes::Expression(Expressions::Accessor { left, .. }) => match left.borrow().node() {
				Nodes::Expression(Expressions::Accessor { left, right }) => {
					assert_eq!(left.borrow().get_name(), Some("meshes"));
					assert!(
						right.borrow().node().is_indexable(),
						"Expected meshes.meshes to stay indexable"
					);
					right.clone()
				}
				_ => panic!("Expected meshes accessor"),
			},
			_ => panic!("Expected indexed meshes accessor"),
		};
		match meshes_member.borrow().node() {
			Nodes::Expression(Expressions::Member { name, source }) => {
				assert_eq!(name, "meshes");
				assert!(matches!(
					source.borrow().node(),
					Nodes::Member { name, count, .. } if name == "meshes" && count == &Some(NonZeroUsize::new(4).expect("Expected valid count"))
				));
			}
			_ => panic!("Expected meshes member expression"),
		}

		let pixel_mapping_access = match statements[1].borrow().node() {
			Nodes::Expression(Expressions::Operator { right, .. }) => right.clone(),
			_ => panic!("Expected assignment"),
		};
		let pixel_mapping_member = match pixel_mapping_access.borrow().node() {
			Nodes::Expression(Expressions::Accessor { left, .. }) => {
				assert!(left.borrow().node().is_indexable());
				match left.borrow().node() {
					Nodes::Expression(Expressions::Accessor { right, .. }) => right.clone(),
					_ => panic!("Expected pixel_mapping accessor"),
				}
			}
			_ => panic!("Expected indexed pixel_mapping accessor"),
		};
		match pixel_mapping_member.borrow().node() {
			Nodes::Expression(Expressions::Member { name, source }) => {
				assert_eq!(name, "pixel_mapping");
				assert!(matches!(
					source.borrow().node(),
					Nodes::Member { name, count, .. } if name == "pixel_mapping" && count == &Some(NonZeroUsize::new(4).expect("Expected valid count"))
				));
			}
			_ => panic!("Expected pixel_mapping member expression"),
		};
	}

	// #[test]
	// fn push_constant() {
	// }

	#[test]
	fn fragment_shader() {
		let source = r#"
		main: fn () -> void {
			let albedo: vec3f = vec3f(1.0, 0.0, 0.0);
		}
		"#;

		let tokens = tokenizer::tokenize(source).expect("Failed to tokenize");
		let node = parser::parse(&tokens).expect("Failed to parse");
		let node = lex(node).expect("Failed to lex");

		let nb = node.borrow();

		let vec3f = node.get_descendant("vec3f").expect("Expected vec3f");

		match nb.node() {
			Nodes::Scope { name, .. } => {
				assert_eq!(name, "root");

				let main = node.get_descendant("main").expect("Expected main");
				let main = RefCell::borrow(&main.0);

				match main.node() {
					Nodes::Function {
						name,
						return_type,
						statements,
						..
					} => {
						assert_eq!(name, "main");
						assert_type(&return_type.borrow(), "void");

						let albedo = statements[0].borrow();

						match albedo.node() {
							Nodes::Expression(Expressions::Operator { operator, left, right }) => {
								let albedo = left.borrow();

								assert_eq!(operator, &Operators::Assignment);

								match albedo.node() {
									Nodes::Expression(Expressions::VariableDeclaration { name, r#type }) => {
										assert_eq!(name, "albedo");
										assert_eq!(r#type, &vec3f);
									}
									_ => {
										panic!("Expected expression");
									}
								}

								let constructor = right.borrow();

								match constructor.node() {
									Nodes::Expression(Expressions::FunctionCall {
										function, parameters, ..
									}) => {
										let function = RefCell::borrow(&function.0);
										let name = function.get_name().expect("Expected name");

										assert_eq!(name, "vec3f");
										assert_eq!(parameters.len(), 3);
									}
									_ => {
										panic!("Expected expression");
									}
								}
							}
							_ => {
								panic!("Expected variable declaration");
							}
						}
					}
					_ => {
						panic!("Expected function.");
					}
				}
			}
			_ => {
				panic!("Expected scope");
			}
		}
	}

	// TODO: test function with body with missing close brace

	#[test]
	fn lex_intrinsic() {
		let source = "
main: fn () -> void {
	let n: f32 = intrinsic(0).y;
}";

		let tokens = tokenizer::tokenize(source).expect("Failed to tokenize");
		let mut node = parser::parse(&tokens).expect("Failed to parse");

		let intrinsic = parser::Node::intrinsic(
			"intrinsic",
			parser::Node::parameter("num", "u32"),
			parser::Node::sentence(vec![
				parser::Node::glsl("vec3(", &[], &[]),
				parser::Node::member_expression("num"),
				parser::Node::glsl(")", &[], &[]),
			]),
			"vec3f",
		);

		node.add(vec![intrinsic]);

		let node = lex(node).expect("Failed to lex");

		let nb = node.borrow();

		match nb.node() {
			Nodes::Scope { name, .. } => {
				assert_eq!(name, "root");

				let main = node.get_descendant("main").unwrap();
				let main = main.borrow();

				match main.node() {
					Nodes::Function { name, statements, .. } => {
						assert_eq!(name, "main");

						let n = statements[0].borrow();

						match n.node() {
							Nodes::Expression(Expressions::Operator { operator, left, right }) => {
								assert_eq!(operator, &Operators::Assignment);

								let n = left.borrow();

								match n.node() {
									Nodes::Expression(Expressions::VariableDeclaration { name, r#type }) => {
										assert_eq!(name, "n");
										assert_type(&r#type.borrow(), "f32");
									}
									_ => {
										panic!("Expected variable declaration");
									}
								}

								let intrinsic = right.borrow();

								match intrinsic.node() {
									Nodes::Expression(Expressions::Accessor { left, right }) => {
										let left = left.borrow();

										match left.node() {
											Nodes::Expression(Expressions::IntrinsicCall { intrinsic, .. }) => {
												let intrinsic = intrinsic.borrow();

												match intrinsic.node() {
													Nodes::Intrinsic { name, elements, .. } => {
														assert_eq!(name, "intrinsic");
														assert_eq!(elements.len(), 2);
													}
													_ => {
														panic!("Expected intrinsic");
													}
												}
											}
											_ => {
												panic!("Expected intrinsic call");
											}
										}

										let right = right.borrow();

										match right.node() {
											Nodes::Expression(Expressions::Member { name, .. }) => {
												assert_eq!(name, "y");
											}
											_ => {
												panic!("Expected member");
											}
										}
									}
									_ => {
										panic!("Expected accessor");
									}
								}
							}
							_ => {
								panic!("Expected assignment");
							}
						}
					}
					_ => {
						panic!("Expected feature");
					}
				}
			}
			_ => {
				panic!("Expected scope");
			}
		}
	}

	#[test]
	fn lex_builtin_texture_intrinsics() {
		let script = r#"
		main: fn () -> void {
			let uv: vec2f = vec2f(0.5, 0.5);
			let coord: vec2u = vec2u(1, 2);
			let color: vec4f = sample(texture_sampler, uv);
			let texel: vec4f = fetch(texture, coord);
		}
		"#;

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
		root.add_child(
			Node::binding(
				"texture",
				BindingTypes::CombinedImageSampler { format: String::new() },
				1,
				true,
				false,
			)
			.into(),
		);

		let node = crate::compile_to_besl(script, Some(root)).expect("Failed to lex");
		let main = node.get_descendant("main").expect("Expected main");
		let main = main.borrow();

		let Nodes::Function { statements, .. } = main.node() else {
			panic!("Expected function");
		};

		let sample_statement = statements[2].borrow();
		let fetch_statement = statements[3].borrow();

		let assert_intrinsic_call = |statement: &Node, expected_name: &str| match statement.node() {
			Nodes::Expression(Expressions::Operator { right, .. }) => {
				let right = right.borrow();
				match right.node() {
					Nodes::Expression(Expressions::IntrinsicCall {
						intrinsic,
						arguments,
						elements,
					}) => {
						assert_eq!(arguments.len(), 2);
						assert_eq!(elements.len(), 2);

						let intrinsic = intrinsic.borrow();
						match intrinsic.node() {
							Nodes::Intrinsic {
								name,
								r#return,
								elements,
							} => {
								assert_eq!(name, expected_name);
								assert_type(&r#return.borrow(), "vec4f");

								assert_eq!(elements.len(), 2);
							}
							_ => panic!("Expected intrinsic"),
						}
					}
					_ => panic!("Expected intrinsic call"),
				}
			}
			_ => panic!("Expected assignment"),
		};

		assert_intrinsic_call(&sample_statement, "sample");
		assert_intrinsic_call(&fetch_statement, "fetch");
	}

	#[test]
	fn lex_builtin_texture_intrinsics_validate_parameter_count() {
		let source = r#"
		main: fn () -> void {
			let color: vec4f = sample(texture_sampler);
		}
		"#;

		let tokens = tokenizer::tokenize(source).expect("Failed to tokenize");
		let parsed = parser::parse(&tokens).expect("Failed to parse");

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
	fn lex_builtin_image_write_intrinsic() {
		let script = r#"
		main: fn () -> void {
			write(image, vec2u(1, 2), vec4f(1.0, 0.0, 0.0, 1.0));
		}
		"#;

		let mut root = Node::root();
		root.add_child(
			Node::binding(
				"image",
				BindingTypes::Image {
					format: "rgba8".to_string(),
				},
				0,
				false,
				true,
			)
			.into(),
		);

		let node = crate::compile_to_besl(script, Some(root)).expect("Failed to lex");
		let main = node.get_descendant("main").expect("Expected main");
		let main = main.borrow();

		let Nodes::Function { statements, .. } = main.node() else {
			panic!("Expected function");
		};

		let write_statement = statements[0].borrow();
		match write_statement.node() {
			Nodes::Expression(Expressions::IntrinsicCall {
				intrinsic,
				arguments,
				elements,
			}) => {
				assert_eq!(arguments.len(), 3);
				assert_eq!(elements.len(), 3);

				let intrinsic = intrinsic.borrow();
				match intrinsic.node() {
					Nodes::Intrinsic { name, r#return, .. } => {
						assert_eq!(name, "write");
						assert_type(&r#return.borrow(), "void");
					}
					_ => panic!("Expected intrinsic"),
				}
			}
			_ => panic!("Expected intrinsic call"),
		}
	}

	#[test]
	fn lex_builtin_dot_intrinsic() {
		let script = r#"
		main: fn () -> void {
			let strength: f32 = dot(vec3f(1.0, 0.0, 0.0), vec3f(0.5, 0.5, 0.0));
		}
		"#;

		let node = crate::compile_to_besl(script, None).expect("Failed to lex");
		let main = node.get_descendant("main").expect("Expected main");
		let main = main.borrow();

		let Nodes::Function { statements, .. } = main.node() else {
			panic!("Expected function");
		};

		let statement = statements[0].borrow();
		match statement.node() {
			Nodes::Expression(Expressions::Operator { right, .. }) => match right.borrow().node() {
				Nodes::Expression(Expressions::IntrinsicCall {
					intrinsic, arguments, ..
				}) => {
					assert_eq!(arguments.len(), 2);
					match intrinsic.borrow().node() {
						Nodes::Intrinsic { name, r#return, .. } => {
							assert_eq!(name, "dot");
							assert_type(&r#return.borrow(), "f32");
						}
						_ => panic!("Expected intrinsic"),
					}
				}
				_ => panic!("Expected intrinsic call"),
			},
			_ => panic!("Expected assignment"),
		}
	}

	#[test]
	fn lex_builtin_cross_intrinsic() {
		let script = r#"
		main: fn () -> void {
			let normal: vec3f = cross(vec3f(1.0, 0.0, 0.0), vec3f(0.0, 1.0, 0.0));
		}
		"#;

		let node = crate::compile_to_besl(script, None).expect("Failed to lex");
		let main = node.get_descendant("main").expect("Expected main");
		let main = main.borrow();

		let Nodes::Function { statements, .. } = main.node() else {
			panic!("Expected function");
		};

		let statement = statements[0].borrow();
		match statement.node() {
			Nodes::Expression(Expressions::Operator { right, .. }) => match right.borrow().node() {
				Nodes::Expression(Expressions::IntrinsicCall {
					intrinsic, arguments, ..
				}) => {
					assert_eq!(arguments.len(), 2);
					match intrinsic.borrow().node() {
						Nodes::Intrinsic { name, r#return, .. } => {
							assert_eq!(name, "cross");
							assert_type(&r#return.borrow(), "vec3f");
						}
						_ => panic!("Expected intrinsic"),
					}
				}
				_ => panic!("Expected intrinsic call"),
			},
			_ => panic!("Expected assignment"),
		}
	}

	#[test]
	fn lex_builtin_length_and_normalize_intrinsics() {
		let script = r#"
		main: fn () -> void {
			let magnitude: f32 = length(vec3f(3.0, 4.0, 0.0));
			let direction: vec3f = normalize(vec3f(3.0, 4.0, 0.0));
		}
		"#;

		let node = crate::compile_to_besl(script, None).expect("Failed to lex");
		let main = node.get_descendant("main").expect("Expected main");
		let main = main.borrow();

		let Nodes::Function { statements, .. } = main.node() else {
			panic!("Expected function");
		};

		let magnitude = statements[0].borrow();
		let direction = statements[1].borrow();

		match magnitude.node() {
			Nodes::Expression(Expressions::Operator { right, .. }) => match right.borrow().node() {
				Nodes::Expression(Expressions::IntrinsicCall { intrinsic, .. }) => match intrinsic.borrow().node() {
					Nodes::Intrinsic { name, r#return, .. } => {
						assert_eq!(name, "length");
						assert_type(&r#return.borrow(), "f32");
					}
					_ => panic!("Expected intrinsic"),
				},
				_ => panic!("Expected intrinsic call"),
			},
			_ => panic!("Expected assignment"),
		}

		match direction.node() {
			Nodes::Expression(Expressions::Operator { right, .. }) => match right.borrow().node() {
				Nodes::Expression(Expressions::IntrinsicCall { intrinsic, .. }) => match intrinsic.borrow().node() {
					Nodes::Intrinsic { name, r#return, .. } => {
						assert_eq!(name, "normalize");
						assert_type(&r#return.borrow(), "vec3f");
					}
					_ => panic!("Expected intrinsic"),
				},
				_ => panic!("Expected intrinsic call"),
			},
			_ => panic!("Expected assignment"),
		}
	}

	#[test]
	fn lex_builtin_reflect_intrinsic() {
		let root = Node::root();
		let reflect = root.get_child("reflect").expect("Expected reflect builtin");
		match reflect.borrow().node() {
			Nodes::Intrinsic {
				name,
				elements,
				r#return,
			} => {
				assert_eq!(name, "reflect");
				assert_eq!(elements.len(), 2);
				assert_type(&r#return.borrow(), "vec4f");
			}
			_ => panic!("Expected intrinsic"),
		};
	}

	#[test]
	fn lex_builtin_thread_idx_intrinsic() {
		let script = r#"
		main: fn () -> void {
			let index: u32 = thread_idx();
		}
		"#;

		let node = crate::compile_to_besl(script, None).expect("Failed to lex");
		let main = node.get_descendant("main").expect("Expected main");
		let main = main.borrow();

		let Nodes::Function { statements, .. } = main.node() else {
			panic!("Expected function");
		};

		let statement = statements[0].borrow();
		match statement.node() {
			Nodes::Expression(Expressions::Operator { right, .. }) => match right.borrow().node() {
				Nodes::Expression(Expressions::IntrinsicCall {
					intrinsic, arguments, ..
				}) => {
					assert!(arguments.is_empty());
					match intrinsic.borrow().node() {
						Nodes::Intrinsic { name, r#return, .. } => {
							assert_eq!(name, "thread_idx");
							assert_type(&r#return.borrow(), "u32");
						}
						_ => panic!("Expected intrinsic"),
					}
				}
				_ => panic!("Expected intrinsic call"),
			},
			_ => panic!("Expected assignment"),
		}
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

	#[test]
	fn lex_const_array_variable() {
		let script = r#"
		WEIGHTS: const f32[3] = f32[3](0.5, 0.25, 0.125);

		main: fn () -> void {
			let value: f32 = WEIGHTS[1];
		}
		"#;

		let node = crate::compile_to_besl(script, None).expect("Failed to lex");

		let weights = node.get_descendant("WEIGHTS").expect("Expected WEIGHTS const");
		let weights = weights.borrow();

		match weights.node() {
			Nodes::Const { name, r#type, value } => {
				assert_eq!(name, "WEIGHTS");
				assert_eq!(r#type.borrow().get_name().unwrap(), "f32[3]");
				assert!(weights.node().is_indexable());
				{
					let value = value.borrow();

					assert!(matches!(value.node(), Nodes::Expression(Expressions::FunctionCall { .. })));
				}
			}
			_ => panic!("Expected Const node"),
		}

		let main = node.get_descendant("main").expect("Expected main");
		let statements = {
			let main = main.borrow();
			let Nodes::Function { statements, .. } = main.node() else {
				panic!("Expected function");
			};
			statements.clone()
		};

		let statement = statements[0].clone();
		{
			let statement = statement.borrow();
			match statement.node() {
				Nodes::Expression(Expressions::Operator { right, .. }) => {
					let right = right.borrow();

					assert!(matches!(right.node(), Nodes::Expression(Expressions::Accessor { .. })));
				}
				_ => panic!("Expected assignment"),
			}
		};
	}

	#[test]
	fn lex_array_constructor_call() {
		let script = r#"
		main: fn () -> void {
			let weights: f32[3] = f32[3](0.5, 0.25, 0.125);
		}
		"#;

		let node = crate::compile_to_besl(script, None).expect("Failed to lex");
		let main = node.get_descendant("main").expect("Expected main");
		let statements = {
			let main = main.borrow();
			let Nodes::Function { statements, .. } = main.node() else {
				panic!("Expected function");
			};
			statements.clone()
		};

		let statement = statements[0].clone();
		{
			let statement = statement.borrow();
			match statement.node() {
				Nodes::Expression(Expressions::Operator { left, right, .. }) => {
					match left.borrow().node() {
						Nodes::Expression(Expressions::VariableDeclaration { r#type, .. }) => {
							assert_eq!(r#type.borrow().get_name().unwrap(), "f32[3]");
						}
						_ => panic!("Expected variable declaration"),
					}

					match right.borrow().node() {
						Nodes::Expression(Expressions::FunctionCall { function, parameters }) => {
							assert_eq!(parameters.len(), 3);
							assert_eq!(function.borrow().get_name().unwrap(), "f32[3]");
						}
						_ => panic!("Expected function call"),
					}
				}
				_ => panic!("Expected assignment"),
			}
		};
	}

	#[test]
	fn lex_conditional_block() {
		let script = r#"
		main: fn () -> void {
			let n: u32 = 0;
			if (n < 1) {
				n = 2;
			}
		}
		"#;

		let node = crate::compile_to_besl(script, None).expect("Failed to lex");
		let main = node.get_descendant("main").expect("Expected main");
		let main = main.borrow();

		let Nodes::Function { statements, .. } = main.node() else {
			panic!("Expected function");
		};

		let conditional = statements[1].borrow();
		match conditional.node() {
			Nodes::Conditional { condition, statements } => {
				assert_eq!(statements.len(), 1);

				match condition.borrow().node() {
					Nodes::Expression(Expressions::Operator { operator, .. }) => {
						assert_eq!(operator, &Operators::LessThan);
					}
					_ => panic!("Expected less-than condition"),
				}
			}
			_ => panic!("Expected conditional node"),
		}
	}

	#[test]
	fn lex_for_loop_block() {
		let script = r#"
		main: fn () -> void {
			let sum: u32 = 0;
			for (let i: u32 = 0; i < 4; i = i + 1) {
				sum = sum + i;
			}
		}
		"#;

		let node = crate::compile_to_besl(script, None).expect("Failed to lex");
		let main = node.get_descendant("main").expect("Expected main");
		let main = main.borrow();

		let Nodes::Function { statements, .. } = main.node() else {
			panic!("Expected function");
		};

		let for_loop = statements[1].borrow();
		match for_loop.node() {
			Nodes::ForLoop {
				initializer,
				condition,
				update,
				statements,
			} => {
				assert_eq!(statements.len(), 1);
				assert!(matches!(
					initializer.borrow().node(),
					Nodes::Expression(Expressions::Operator { operator, .. }) if operator == &Operators::Assignment
				));
				assert!(matches!(
					condition.borrow().node(),
					Nodes::Expression(Expressions::Operator { operator, .. }) if operator == &Operators::LessThan
				));
				assert!(matches!(
					update.borrow().node(),
					Nodes::Expression(Expressions::Operator { operator, .. }) if operator == &Operators::Assignment
				));
			}
			_ => panic!("Expected for loop node"),
		}
	}

	#[test]
	fn lex_bitwise_expression() {
		let script = r#"
		main: fn () -> void {
			let packed: u32 = 1 << 8 | 2 & 255;
		}
		"#;

		let node = crate::compile_to_besl(script, None).expect("Failed to lex");
		let main = node.get_descendant("main").expect("Expected main");
		let main = main.borrow();

		let Nodes::Function { statements, .. } = main.node() else {
			panic!("Expected function");
		};

		let statement = statements[0].borrow();
		match statement.node() {
			Nodes::Expression(Expressions::Operator { right, .. }) => match right.borrow().node() {
				Nodes::Expression(Expressions::Operator { operator, left, right }) => {
					assert_eq!(operator, &Operators::BitwiseOr);
					assert!(matches!(
						left.borrow().node(),
						Nodes::Expression(Expressions::Operator { operator, .. }) if operator == &Operators::ShiftLeft
					));
					assert!(matches!(
						right.borrow().node(),
						Nodes::Expression(Expressions::Operator { operator, .. }) if operator == &Operators::BitwiseAnd
					));
				}
				_ => panic!("Expected bitwise or expression"),
			},
			_ => panic!("Expected assignment"),
		}
	}

	#[test]
	fn lex_comparison_and_continue() {
		let script = r#"
		main: fn () -> void {
			for (let i: u32 = 0; i <= 4; i = i + 1) {
				if (i >= 2) {
					continue;
				}
			}
		}
		"#;

		let node = crate::compile_to_besl(script, None).expect("Failed to lex");
		let main = node.get_descendant("main").expect("Expected main");
		let main = main.borrow();

		let Nodes::Function { statements, .. } = main.node() else {
			panic!("Expected function");
		};

		let for_loop = statements[0].borrow();
		let Nodes::ForLoop {
			condition, statements, ..
		} = for_loop.node()
		else {
			panic!("Expected for loop");
		};

		assert!(matches!(
			condition.borrow().node(),
			Nodes::Expression(Expressions::Operator { operator, .. }) if operator == &Operators::LessThanOrEqual
		));

		let conditional = statements[0].borrow();
		let Nodes::Conditional { condition, statements } = conditional.node() else {
			panic!("Expected conditional");
		};

		assert!(matches!(
			condition.borrow().node(),
			Nodes::Expression(Expressions::Operator { operator, .. }) if operator == &Operators::GreaterThanOrEqual
		));
		assert!(matches!(
			statements[0].borrow().node(),
			Nodes::Expression(Expressions::Continue)
		));
	}

	#[test]
	fn lex_scalar_intrinsic_overloads() {
		let script = r#"
		main: fn () -> void {
			let maximum: f32 = max(1.0, 2.0);
			let clamped: f32 = clamp(1.5, 0.0, 1.0);
		}
		"#;

		let node = crate::compile_to_besl(script, None).expect("Failed to lex");
		let main = node.get_descendant("main").expect("Expected main");
		let main = main.borrow();

		let Nodes::Function { statements, .. } = main.node() else {
			panic!("Expected function");
		};

		for (statement, expected_name, expected_type) in [(&statements[0], "max", "f32"), (&statements[1], "clamp", "f32")] {
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

	/// Verifies mesh index helpers can widen packed byte and word values through portable BESL.
	#[test]
	fn lex_u32_widening_intrinsic_overloads() {
		let script = r#"
		main: fn () -> void {
			let byte: u8 = 7;
			let word: u16 = 513;
			let byte_wide: u32 = u32(byte);
			let word_wide: u32 = u32(word);
		}
		"#;

		crate::compile_to_besl(script, None)
			.expect("Failed to resolve u32 widening calls. The most likely cause is a missing narrow-integer overload.");
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

	#[test]
	fn lex_packed_vec4f_construction_and_conversion() {
		let script = r#"
		main: fn () -> void {
			let packed: packed_vec4f = packed_vec4f(vec4f(1.0, 2.0, 3.0, 4.0));
			let ordinary: vec4f = vec4f(packed);
			ordinary.w;
		}
		"#;

		crate::compile_to_besl(script, None).expect(
			"Failed to resolve packed_vec4f conversions. The most likely cause is a missing packed-vector intrinsic overload.",
		);
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
}
