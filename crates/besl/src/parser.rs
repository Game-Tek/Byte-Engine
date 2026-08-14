//! Parses BESL tokens into syntax nodes that preserve the source structure.
//!
//! Use [`crate::parse`] as the entry point. The parser records cross-references by name.
//! The [`crate::lexer`] module resolves those names later.

mod declarations;
mod expressions;
mod iterator;

pub(crate) use declarations::parse;
pub use declarations::{Expressions, Node, Nodes, ParsingFailReasons, TypeName};
#[cfg(test)]
use expressions::*;
pub use iterator::ProgramState;
#[cfg(test)]
#[cfg(test)]
mod tests {
	use super::*;
	use crate::tokenizer::tokenize;

	#[test]
	#[should_panic(expected = "Invalid binding array count")]
	fn binding_array_rejects_zero_elements() {
		Node::binding_array("textures", Node::combined_image_sampler(), 0, true, false, 0);
	}

	#[test]
	fn parse_stage_interface_and_task_storage_declarations() {
		let tokens = tokenize(
			r#"
				instance_index: input<u32, 0>;
				primitive_index: output<u32, 1>;
				meshlet_indices: output<u32, 2, 126>;
				visible_meshlets: task_payload<u32, 32>;
				visible_count: workgroup<atomicu32>;
				scratch: workgroup<f32, 64>;
			"#,
		)
		.expect("stage-interface source should tokenize");
		let root = parse(&tokens).expect("stage-interface source should parse");

		assert!(matches!(
			root["instance_index"].node(),
			Nodes::Input {
				format: "u32",
				location: 0,
				..
			}
		));
		assert!(matches!(
			root["primitive_index"].node(),
			Nodes::Output {
				format: "u32",
				location: 1,
				count: None,
				..
			}
		));
		assert!(matches!(
			root["meshlet_indices"].node(),
			Nodes::Output {
				format: "u32",
				location: 2,
				count: Some(count),
				..
			} if count.get() == 126
		));
		assert!(matches!(
			root["visible_meshlets"].node(),
			Nodes::TaskPayload {
				format: "u32",
				count,
				..
			} if count.get() == 32
		));
		assert!(matches!(
			root["visible_count"].node(),
			Nodes::Workgroup { format: "atomicu32", .. }
		));
		assert!(matches!(
			root["scratch"].node(),
			Nodes::Workgroup {
				format: "f32",
				count: Some(count),
				..
			} if count.get() == 64
		));
	}

	#[test]
	fn workgroup_array_rejects_zero_elements() {
		let tokens = tokenize("scratch: workgroup<f32, 0>;").expect("workgroup array source should tokenize");
		parse(&tokens).expect_err("zero-length workgroup array should fail");
	}

	#[test]
	fn stage_interface_declarations_reject_invalid_locations_and_counts() {
		for source in [
			"value: input<u32, 256>;",
			"value: output<u32, 0, 0>;",
			"value: task_payload<u32, 0>;",
			"value: workgroup<u32>",
		] {
			let tokens = tokenize(source).expect("invalid declaration should still tokenize");
			assert!(parse(&tokens).is_err(), "expected `{source}` to be rejected");
		}
	}

	#[test]
	fn parse_resource_descriptors_with_flat_slots_access_and_count() {
		let tokens = tokenize(
			r#"
				source: descriptor<Texture2D, 3, read>;
				result: descriptor<StorageImage<rgba16f>, 7, write, 4>;
				unformatted_result: descriptor<StorageImage, 8, write>;
				data: descriptor<Data, 11, read_write>;
				textures: descriptor<Texture2DArray, 20, read, 16>;
			"#,
		)
		.expect("descriptor source should tokenize");
		let root = parse(&tokens).expect("descriptor source should parse");

		let Nodes::Descriptor {
			resource_type,
			slot,
			read,
			write,
			count,
			..
		} = root["source"].node()
		else {
			panic!("expected source descriptor");
		};
		assert_eq!(*resource_type, "Texture2D");
		assert_eq!(*slot, 3);
		assert!(*read);
		assert!(!*write);
		assert_eq!(*count, None);

		assert!(matches!(
			root["result"].node(),
			Nodes::Descriptor {
				format: Some("rgba16f"),
				slot: 7,
				read: false,
				write: true,
				count: Some(count),
				..
			} if count.get() == 4
		));
		assert!(matches!(
			root["unformatted_result"].node(),
			Nodes::Descriptor {
				format: None,
				slot: 8,
				..
			}
		));
		assert!(matches!(
			root["data"].node(),
			Nodes::Descriptor {
				resource_type: "Data",
				slot: 11,
				read: true,
				write: true,
				..
			}
		));
		assert!(matches!(
			root["textures"].node(),
			Nodes::Descriptor { resource_type: "Texture2DArray", slot: 20, count: Some(count), .. }
				if count.get() == 16
		));
	}

	#[test]
	fn parse_buffer_memory_classes_after_descriptor_access() {
		let tokens = tokenize(
			r#"
				view: descriptor<View, 0, read, constant>;
				vertices: descriptor<Vertices, 1, read, device>;
				counters: descriptor<Counters, 2, read_write, device, 4>;
			"#,
		)
		.expect("buffer memory class source should tokenize");
		let root = parse(&tokens).expect("buffer memory class source should parse");

		assert!(matches!(
			root["view"].node(),
			Nodes::Descriptor {
				memory_class: Some("constant"),
				count: None,
				..
			}
		));
		assert!(matches!(
			root["vertices"].node(),
			Nodes::Descriptor {
				memory_class: Some("device"),
				count: None,
				..
			}
		));
		assert!(matches!(
			root["counters"].node(),
			Nodes::Descriptor {
				memory_class: Some("device"),
				count: Some(count),
				..
			} if count.get() == 4
		));
	}

	#[test]
	fn parse_source_push_constant_block() {
		let tokens = tokenize(
			r#"
				push_constant: push_constant {
					source_vertex_base: u32,
					destination_vertex_base: u32,
					vertex_count: u32,
				}
			"#,
		)
		.expect("push-constant source should tokenize");
		let root = parse(&tokens).expect("push-constant source should parse");
		let Nodes::Scope { children, .. } = root.node() else {
			panic!("expected root scope");
		};
		assert!(matches!(
			children.as_slice(),
			[Node {
				node: Nodes::PushConstant { members },
				..
			}] if members.len() == 3
		));
	}

	#[test]
	fn descriptor_rejects_invalid_access_count_and_arguments() {
		for source in [
			"texture: descriptor<Texture2D, 0, execute>;",
			"textures: descriptor<Texture2D, 0, read, 0>;",
			"texture: descriptor<Texture2D>;",
		] {
			let tokens = tokenize(source).expect("descriptor source should tokenize");
			assert!(parse(&tokens).is_err(), "malformed descriptor should be rejected: {source}");
		}
	}

	#[test]
	fn descriptor_rejects_formats_on_non_storage_image_resources() {
		for source in [
			"texture: descriptor<Texture2D<rgba16f>, 0, read>;",
			"data: descriptor<Data<rgba16f>, 0, read>;",
		] {
			let tokens = tokenize(source).expect("formatted descriptor source should tokenize");
			assert!(
				parse(&tokens).is_err(),
				"non-storage image descriptor format should be rejected: {source}"
			);
		}
	}

	fn assert_named_type(type_name: &TypeName<'_>, expected: &str) {
		assert!(matches!(type_name, TypeName::Named(name) if *name == expected));
	}

	fn print_tree(node: &Node) {
		match &node.node {
			Nodes::Scope { name, children } => {
				println!("{}", name,);
				for child in children {
					print_tree(child);
				}
			}
			Nodes::Struct { name, fields } => {
				println!("{}", name,);
				for field in fields {
					print_tree(field);
				}
			}
			_ => {}
		}
	}

	fn assert_struct(node: &Node) {
		if let Nodes::Struct { name, fields } = &node.node {
			assert_eq!(*name, "Light");
			assert_eq!(fields.len(), 2);

			let position = &fields[0];

			if let Nodes::Member { name, r#type } = &position.node {
				assert_eq!(*name, "position");
				assert_eq!(r#type, "vec3f");
			} else {
				panic!("Not a member");
			}

			let color = &fields[1];

			if let Nodes::Member { name, r#type } = &color.node {
				assert_eq!(*name, "color");
				assert_eq!(r#type, "vec3f");
			} else {
				panic!("Not a member");
			}
		} else {
			panic!("Not a struct");
		}
	}

	#[test]
	fn test_parse_struct() {
		let source = "
Light: struct {
	array: u32[3],
	position: vec3f,
	color: vec3f
}";

		let tokens = tokenize(source).unwrap();
		let node = parse(&tokens).expect("Failed to parse");

		// program.types.get("Light").expect("Failed to get Light type");

		if let Nodes::Struct { name, .. } = node.node {
			assert_eq!(name, "root");
			assert_struct(&node["Light"]);
		}
	}

	fn assert_function(node: &Node) {
		if let Nodes::Function {
			name,
			params,
			return_type,
			statements,
			..
		} = &node.node
		{
			assert_eq!(*name, "main");
			assert_eq!(params.len(), 0);
			assert_eq!(*return_type, TypeName::Named("void"));
			assert_eq!(statements.len(), 2);

			let statement = &statements[0];

			if let Nodes::Expression(Expressions::Operator {
				name,
				left: var_decl,
				right: function_call,
			}) = &statement.node
			{
				assert_eq!(*name, "=");

				if let Nodes::Expression(Expressions::VariableDeclaration { name, r#type, .. }) = &var_decl.node {
					assert_eq!(*name, "position");
					assert_named_type(r#type, "vec4f");
				} else {
					panic!("Not an variable declaration");
				}

				if let Nodes::Expression(Expressions::Call { name, parameters, .. }) = &function_call.node {
					assert_named_type(name, "vec4");
					assert_eq!(parameters.len(), 4);

					let x_param = &parameters[0];

					if let Nodes::Expression(Expressions::Literal { value }) = &x_param.node {
						assert_eq!(value, "0.0");
					} else {
						panic!("Not a literal");
					}
				} else {
					panic!("Not a function call");
				}
			} else {
				panic!("Not an assignment");
			}
		} else {
			panic!("Not a function");
		}
	}

	#[test]
	fn test_parse_function() {
		let source = "
main: fn () -> void {
	let position: vec4f = vec4(0.0, 0.0, 0.0, 1.0);
	gl_Position = position;
}";

		let tokens = tokenize(source).unwrap();
		let node = parse(&tokens).expect("Failed to parse");

		if let Nodes::Scope { name, .. } = node.node {
			assert_eq!(name, "root");
			assert_function(&node["main"]);
		} else {
			panic!("Not root node")
		}
	}

	#[test]
	fn test_parse_function_with_parameters_and_return_value() {
		let source = "
		add: fn (lhs: f32, rhs: f32) -> f32 {
			return lhs + rhs;
		}";

		let tokens = tokenize(source).unwrap();
		let node = parse(&tokens).expect("Failed to parse");

		let function = &node["add"];
		if let Nodes::Function {
			name,
			params,
			return_type,
			statements,
			..
		} = &function.node
		{
			assert_eq!(*name, "add");
			assert_eq!(params.len(), 2);
			assert_eq!(*return_type, TypeName::Named("f32"));
			assert_eq!(statements.len(), 1);

			if let Nodes::Parameter { name, r#type } = &params[0].node {
				assert_eq!(*name, "lhs");
				assert_eq!(*r#type, TypeName::Named("f32"));
			} else {
				panic!("Expected parameter");
			}

			if let Nodes::Expression(Expressions::Return { value }) = &statements[0].node {
				let value = value.as_ref().expect("Expected return value");
				if let Nodes::Expression(Expressions::Operator { name, .. }) = &value.node {
					assert_eq!(*name, "+");
				} else {
					panic!("Expected return operator");
				}
			} else {
				panic!("Expected return statement");
			}
		} else {
			panic!("Expected function");
		}
	}

	#[test]
	fn parse_function_array_signature() {
		let source = "
		copy_indices: fn (indices: u32[3]) -> u32[3] {
			return indices;
		}";
		let tokens = tokenize(source).expect("Failed to tokenize");
		let node = parse(&tokens).expect("Failed to parse");

		let function = &node["copy_indices"];
		let Nodes::Function { params, return_type, .. } = &function.node else {
			panic!("Expected function");
		};
		assert_eq!(
			*return_type,
			TypeName::Array {
				element: Box::new(TypeName::Named("u32")),
				count: 3,
			}
		);
		let Nodes::Parameter { r#type, .. } = &params[0].node else {
			panic!("Expected parameter");
		};
		assert_eq!(
			*r#type,
			TypeName::Array {
				element: Box::new(TypeName::Named("u32")),
				count: 3,
			}
		);
	}

	#[test]
	fn parse_operators() {
		let source = "
main: fn () -> void {
	let position: vec4f = vec4(0.0, 0.0, 0.0, 1.0) * 2.0;
	gl_Position = position;
}";

		let tokens = tokenize(source).unwrap();
		let node = parse(&tokens).expect("Failed to parse");

		let main_node = &node["main"];

		if let Nodes::Function {
			name,
			statements,
			return_type,
			params,
			..
		} = &main_node.node
		{
			assert_eq!(*name, "main");
			assert_eq!(statements.len(), 2);
			assert_eq!(*return_type, TypeName::Named("void"));
			assert_eq!(params.len(), 0);

			assert_eq!(statements.len(), 2);

			let statement0 = &statements[0];

			if let Nodes::Expression(Expressions::Operator {
				name,
				left: var_decl,
				right: multiply,
			}) = &statement0.node
			{
				assert_eq!(*name, "=");

				if let Nodes::Expression(Expressions::VariableDeclaration { .. }) = var_decl.node {
				} else {
					panic!("Not a variable declaration");
				}

				if let Nodes::Expression(Expressions::Operator {
					name,
					left: vec4,
					right: literal,
				}) = &multiply.node
				{
					assert_eq!(*name, "*");

					if let Nodes::Expression(Expressions::Call { name, .. }) = &vec4.node {
						assert_named_type(name, "vec4");
					} else {
						panic!("Not a function call");
					}

					if let Nodes::Expression(Expressions::Literal { value }) = &literal.node {
						assert_eq!(value, "2.0");
					} else {
						panic!("Not a literal");
					}
				} else {
					panic!("Not an operator");
				}
			} else {
				panic!("Not an expression");
			}
		} else {
			panic!("Not a feature");
		}
	}

	#[test]
	fn builder_creates_assignment_expression() {
		let node = Node::assignment(Node::member_expression("albedo"), Node::literal_expression("1.0"));

		let Nodes::Expression(Expressions::Operator { name, left, right }) = node.node else {
			panic!("Expected assignment operator");
		};

		assert_eq!(name, "=");
		assert!(matches!(left.node, Nodes::Expression(Expressions::Member { name }) if name == "albedo"));
		assert!(matches!(right.node, Nodes::Expression(Expressions::Literal { value }) if value == "1.0"));
	}

	#[test]
	fn builder_creates_call_expression() {
		let node = Node::call(
			"vec4f",
			vec![
				Node::literal_expression("1.0"),
				Node::literal_expression("0.0"),
				Node::literal_expression("0.0"),
				Node::literal_expression("1.0"),
			],
		);

		let Nodes::Expression(Expressions::Call { name, parameters, .. }) = node.node else {
			panic!("Expected call expression");
		};

		assert_named_type(&name, "vec4f");
		assert_eq!(parameters.len(), 4);
	}

	#[test]
	fn builder_creates_variable_declaration_assignment() {
		let node = Node::let_assignment("roughness", "f32", Node::literal_expression("0.5"));

		let Nodes::Expression(Expressions::Operator { name, left, right }) = node.node else {
			panic!("Expected assignment operator");
		};

		assert_eq!(name, "=");
		assert!(matches!(
			left.node,
			Nodes::Expression(Expressions::VariableDeclaration { name, r#type, .. })
				if name == "roughness" && matches!(r#type, TypeName::Named("f32")),
		));
		assert!(matches!(right.node, Nodes::Expression(Expressions::Literal { value }) if value == "0.5"));
	}

	#[test]
	fn builder_program_lexes() {
		let program = Node::root_with_children(vec![Node::main_function(vec![Node::let_assignment(
			"albedo",
			"vec4f",
			Node::call(
				"vec4f",
				vec![
					Node::literal_expression("1.0"),
					Node::literal_expression("0.0"),
					Node::literal_expression("0.0"),
					Node::literal_expression("1.0"),
				],
			),
		)])]);

		crate::lex(program).expect("builder generated program should lex");
	}

	#[test]
	fn parse_accessor() {
		let source = "
main: fn () -> void {
	let position: vec4f = vec4(0.0, 0.0, 0.0, 1.0) * 2.0;
	position.y = 2.0;
	gl_Position = position;
}";

		let tokens = tokenize(source).unwrap();
		let node = parse(&tokens).expect("Failed to parse");

		print_tree(&node);

		if let Nodes::Scope { children, .. } = &node.node {
			assert_eq!(children.len(), 1);

			let main_node = &node["main"];

			if let Nodes::Function { name, statements, .. } = &main_node.node {
				assert_eq!(*name, "main");
				assert_eq!(statements.len(), 3);

				let statement1 = &statements[1];

				if let Nodes::Expression(Expressions::Operator {
					name,
					left: accessor,
					right: literal,
				}) = &statement1.node
				{
					assert_eq!(*name, "=");

					if let Nodes::Expression(Expressions::Accessor {
						left: position,
						right: y,
					}) = &accessor.node
					{
						if let Nodes::Expression(Expressions::Member { name }) = &position.node {
							assert_eq!(name, "position");
						} else {
							panic!("Not a member");
						}

						if let Nodes::Expression(Expressions::Member { name }) = &y.node {
							assert_eq!(name, "y");
						} else {
							panic!("Not a member");
						}
					} else {
						panic!("Not an accessor");
					}

					if let Nodes::Expression(Expressions::Literal { value }) = &literal.node {
						assert_eq!(value, "2.0");
					} else {
						panic!("Not a literal");
					}
				} else {
					panic!("Not an operator");
				}
			} else {
				panic!("Not a function");
			}
		} else {
			panic!("Not root node")
		}
	}

	#[test]
	fn test_parse_struct_and_function() {
		let source = "
Light: struct {
	position: vec3f,
	color: vec3f
}

#[vertex]
main: fn () -> void {
	let position: vec4f = vec4(0.0, 0.0, 0.0, 1.0);
	gl_Position = position;
}";

		let tokens = tokenize(source).expect("Failed to tokenize");
		let node = parse(&tokens).expect("Failed to parse");

		if let Nodes::Scope { .. } = &node.node {
			assert_struct(&node["Light"]);
			assert_function(&node["main"]);
		} else {
			panic!("Not root node")
		}
	}

	#[test]
	fn test_parse_member() {
		let source = "color: In<vec4f>;";

		let tokens = tokenize(source).expect("Failed to tokenize");
		let node = parse(&tokens).expect("Failed to parse");

		if let Nodes::Scope { .. } = &node.node {
			let member_node = &node["color"];

			if let Nodes::Member { name, r#type } = &member_node.node {
				assert_eq!(*name, "color");
				assert_eq!(r#type, "In<vec4f>");
			} else {
				panic!("Not a feature");
			}
		}
	}

	#[test]
	fn test_parse_multiple_functions() {
		let source = "
used: fn () -> void {}
not_used: fn () -> void {}

main: fn () -> void {
	used();
}";

		let tokens = tokenize(source).expect("Failed to tokenize");
		let node = parse(&tokens).expect("Failed to parse");

		if let Nodes::Scope { children, .. } = node.node {
			assert_eq!(children.len(), 3);
		}
	}

	#[test]
	fn fragment_shader() {
		let source = r#"
		main: fn () -> void {
			let albedo: vec3f = vec3f(1.0, 0.0, 0.0);
		}
		"#;

		let tokens = tokenize(source).expect("Failed to tokenize");
		let node = parse(&tokens).expect("Failed to parse");

		if let Nodes::Scope { children, .. } = node.node {
			assert_eq!(children.len(), 1);
		}
	}

	#[test]
	fn test_parse_accessor_and_assignment() {
		let source = "
main: fn () -> void {
	let n: f32 = intrinsic(0).y;
}";

		let tokens = tokenize(source).expect("Failed to tokenize");
		let node = parse(&tokens).expect("Failed to parse");

		if let Nodes::Scope { children, .. } = &node.node {
			assert_eq!(children.len(), 1);

			let main_node = &node["main"];

			if let Nodes::Function { name, statements, .. } = &main_node.node {
				assert_eq!(*name, "main");
				assert_eq!(statements.len(), 1);

				let statement = &statements[0];

				if let Nodes::Expression(Expressions::Operator { name, left, right }) = &statement.node {
					assert_eq!(*name, "=");

					if let Nodes::Expression(Expressions::VariableDeclaration { name, r#type, .. }) = &left.node {
						assert_eq!(*name, "n");
						assert_named_type(r#type, "f32");
					} else {
						panic!("Not a variable declaration");
					}

					if let Nodes::Expression(Expressions::Accessor { left, right }) = &right.node {
						if let Nodes::Expression(Expressions::Call { name, parameters, .. }) = &left.node {
							assert_named_type(name, "intrinsic");
							assert_eq!(parameters.len(), 1);

							if let Nodes::Expression(Expressions::Literal { value }) = &parameters[0].node {
								assert_eq!(value, "0");
							} else {
								panic!("Not a literal");
							}
						} else {
							panic!("Not a function call");
						}

						if let Nodes::Expression(Expressions::Member { name }) = &right.node {
							assert_eq!(name, "y");
						} else {
							panic!("Not a member");
						}
					} else {
						panic!("Not an accessor");
					}
				} else {
					panic!("Not an operator");
				}
			} else {
				panic!("Not a function");
			}
		} else {
			panic!("Not root node")
		}
	}

	#[test]
	fn parse_array_index_accessor() {
		let source = "
main: fn () -> void {
	let n: u32 = values[1];
}";

		let tokens = tokenize(source).expect("Failed to tokenize");
		let node = parse(&tokens).expect("Failed to parse");

		let main_node = &node["main"];
		if let Nodes::Function { statements, .. } = &main_node.node {
			let statement = &statements[0];
			if let Nodes::Expression(Expressions::Operator { right, .. }) = &statement.node {
				if let Nodes::Expression(Expressions::Accessor { left, right }) = &right.node {
					assert!(matches!(&left.node, Nodes::Expression(Expressions::Member { name }) if name == "values"));
					assert!(matches!(
						right.node,
						Nodes::Expression(Expressions::Expression(ref elements))
							if elements.len() == 1
								&& matches!(&elements[0].node, Nodes::Expression(Expressions::Literal { value }) if value == "1")
					));
				} else {
					panic!("Not an accessor");
				}
			} else {
				panic!("Not an operator");
			}
		} else {
			panic!("Not a function");
		}
	}

	#[test]
	fn parse_comparison_and_continue() {
		let source = r#"
		main: fn () -> void {
			for (let i: u32 = 0; i <= 4; i = i + 1) {
				if (i >= 2) {
					continue;
				}
			}
		}
		"#;

		let tokens = tokenize(source).expect("Failed to tokenize");
		let node = parse(&tokens).expect("Failed to parse");
		let main_node = &node["main"];

		let Nodes::Function { statements, .. } = &main_node.node else {
			panic!("Expected function");
		};

		let Nodes::ForLoop {
			condition, statements, ..
		} = &statements[0].node
		else {
			panic!("Expected for loop");
		};

		assert!(matches!(
			&condition.node,
			Nodes::Expression(Expressions::Operator { name, .. }) if *name == "<="
		));

		let Nodes::Conditional { condition, statements } = &statements[0].node else {
			panic!("Expected conditional");
		};

		assert!(matches!(
			&condition.node,
			Nodes::Expression(Expressions::Operator { name, .. }) if *name == ">="
		));
		assert!(matches!(statements[0].node, Nodes::Expression(Expressions::Continue)));
	}

	#[test]
	fn parse_discard_in_conditional() {
		let tokens = tokenize("main: fn () -> void { if (true) { discard; } }").expect("Failed to tokenize");
		let node = parse(&tokens).expect("Failed to parse");
		let Nodes::Function { statements, .. } = &node["main"].node else {
			panic!("Expected function");
		};
		let Nodes::Conditional { statements, .. } = &statements[0].node else {
			panic!("Expected conditional");
		};
		assert!(matches!(statements[0].node, Nodes::Expression(Expressions::Discard)));
	}

	#[test]
	fn test_parse_const() {
		let source = "
PI: const f32 = 3.14;
";

		let tokens = tokenize(source).expect("Failed to tokenize");
		let node = parse(&tokens).expect("Failed to parse");

		if let Nodes::Scope { children, .. } = &node.node {
			assert_eq!(children.len(), 1);

			let const_node = &node["PI"];

			if let Nodes::Const { name, r#type, value, .. } = &const_node.node {
				assert_eq!(*name, "PI");
				assert_named_type(r#type, "f32");

				if let Nodes::Expression(Expressions::Literal { value }) = &value.node {
					assert_eq!(*value, "3.14");
				} else {
					panic!("Expected a literal value, got: {:?}", value.node);
				}
			} else {
				panic!("Expected a const node, got: {:?}", const_node.node);
			}
		} else {
			panic!("Not root node");
		}
	}

	#[test]
	fn test_parse_const_with_expression() {
		let source = "
TAU: const f32 = 3.14 * 2.0;
";

		let tokens = tokenize(source).expect("Failed to tokenize");
		let node = parse(&tokens).expect("Failed to parse");

		let const_node = &node["TAU"];

		if let Nodes::Const { name, r#type, value, .. } = &const_node.node {
			assert_eq!(*name, "TAU");
			assert_named_type(r#type, "f32");

			if let Nodes::Expression(Expressions::Operator { name, .. }) = &value.node {
				assert_eq!(*name, "*");
			} else {
				panic!("Expected an operator expression, got: {:?}", value.node);
			}
		} else {
			panic!("Expected a const node");
		}
	}

	#[test]
	fn test_parse_const_array() {
		let source = "
		WEIGHTS: const f32 [ 3 ] = f32 [ 3 ](0.5, 0.25, 0.125);
";

		let tokens = tokenize(source).expect("Failed to tokenize");
		let node = parse(&tokens).expect("Failed to parse");

		let const_node = &node["WEIGHTS"];

		if let Nodes::Const { name, r#type, value } = &const_node.node {
			assert_eq!(*name, "WEIGHTS");
			assert_eq!(
				r#type,
				&TypeName::Array {
					element: Box::new(TypeName::Named("f32")),
					count: 3,
				}
			);

			if let Nodes::Expression(Expressions::Call { name, parameters }) = &value.node {
				assert_eq!(
					name,
					&TypeName::Array {
						element: Box::new(TypeName::Named("f32")),
						count: 3,
					}
				);
				assert_eq!(parameters.len(), 3);
			} else {
				panic!("Expected an array constructor call, got: {:?}", value.node);
			}
		} else {
			panic!("Expected a const node");
		}
	}

	#[test]
	fn parse_nested_array_type_without_flattening() {
		let tokens = tokenize("f32 [ 3 ] [ 4 ]").expect("Failed to tokenize");
		let mut tokens = tokens.tokens.iter();
		let base_type = tokens.next().expect("Expected a base type");
		let (type_name, mut iterator) = parse_type_name(tokens, base_type).expect("Failed to parse type");

		assert_eq!(
			type_name,
			TypeName::Array {
				element: Box::new(TypeName::Array {
					element: Box::new(TypeName::Named("f32")),
					count: 3,
				}),
				count: 4,
			}
		);
		assert!(iterator.next().is_none());
	}

	#[test]
	fn parse_conditional_block() {
		let source = "
main: fn () -> void {
	let n: u32 = 0;
	if (n < 1) {
		n = 2;
	}
}";

		let tokens = tokenize(source).expect("Failed to tokenize");
		let node = parse(&tokens).expect("Failed to parse");

		let main_node = &node["main"];
		if let Nodes::Function { statements, .. } = &main_node.node {
			assert_eq!(statements.len(), 2);

			let conditional = &statements[1];
			if let Nodes::Conditional { condition, statements } = &conditional.node {
				assert_eq!(statements.len(), 1);

				assert!(matches!(
					condition.node,
					Nodes::Expression(Expressions::Operator { name, .. }) if name == "<"
				));

				assert!(matches!(
					statements[0].node,
					Nodes::Expression(Expressions::Operator { name, .. }) if name == "="
				));
			} else {
				panic!("Expected conditional block");
			}
		} else {
			panic!("Expected main function");
		}
	}

	#[test]
	fn parse_for_loop_block() {
		let source = "
main: fn () -> void {
	let sum: u32 = 0;
	for (let i: u32 = 0; i < 4; i = i + 1) {
		sum = sum + i;
	}
}";

		let tokens = tokenize(source).expect("Failed to tokenize");
		let node = parse(&tokens).expect("Failed to parse");

		let main_node = &node["main"];
		let Nodes::Function { statements, .. } = &main_node.node else {
			panic!("Expected main function");
		};

		assert_eq!(statements.len(), 2);

		let for_loop = &statements[1];
		let Nodes::ForLoop {
			initializer,
			condition,
			update,
			statements,
		} = &for_loop.node
		else {
			panic!("Expected for loop block");
		};

		assert!(matches!(
			initializer.node,
			Nodes::Expression(Expressions::Operator { name, .. }) if name == "="
		));
		assert!(matches!(
			condition.node,
			Nodes::Expression(Expressions::Operator { name, .. }) if name == "<"
		));
		assert!(matches!(
			update.node,
			Nodes::Expression(Expressions::Operator { name, .. }) if name == "="
		));
		assert_eq!(statements.len(), 1);
	}

	#[test]
	fn parse_bitwise_expression() {
		let source = "
main: fn () -> void {
	let packed: u32 = 1 << 8 | 2 & 255;
}";

		let tokens = tokenize(source).expect("Failed to tokenize");
		let node = parse(&tokens).expect("Failed to parse");

		let main_node = &node["main"];
		let Nodes::Function { statements, .. } = &main_node.node else {
			panic!("Expected main function");
		};

		let Nodes::Expression(Expressions::Operator { name, right, .. }) = &statements[0].node else {
			panic!("Expected assignment expression");
		};
		assert_eq!(*name, "=");

		let Nodes::Expression(Expressions::Operator { name, left, right }) = &right.node else {
			panic!("Expected bitwise or expression");
		};
		assert_eq!(*name, "|");

		assert!(matches!(
			left.node,
			Nodes::Expression(Expressions::Operator { name, .. }) if name == "<<"
		));
		assert!(matches!(
			right.node,
			Nodes::Expression(Expressions::Operator { name, .. }) if name == "&"
		));
	}

	#[test]
	fn parse_compute_vertex_position() {
		let source = r#"
compute_vertex_position: fn (mesh: Mesh, meshlet: Meshlet, primitive_index: u32) -> vec4f {
	let vertex_index: u32 = compute_vertex_index(mesh, meshlet, primitive_index);
	return vec4f(
		vertex_positions.positions[vertex_index].x,
		vertex_positions.positions[vertex_index].y,
		vertex_positions.positions[vertex_index].z,
		1.0
	);
}
"#;
		let tokens = tokenize(source).expect("Failed to tokenize");
		let node = parse(&tokens).expect("Failed to parse");
		let func = &node["compute_vertex_position"];
		assert!(matches!(&func.node, Nodes::Function { .. }));
	}

	#[test]
	fn parse_compute_triangle() {
		let source = r#"
compute_triangle: fn (mesh: Mesh, meshlet: Meshlet, primitive_index: u32) -> vec3u {
	return vec3u(
		primitive_indices.primitive_indices[(mesh.base_triangle_index + u16_to_u32(meshlet.triangle_offset) + primitive_index) * 3 + 0],
		primitive_indices.primitive_indices[(mesh.base_triangle_index + u16_to_u32(meshlet.triangle_offset) + primitive_index) * 3 + 1],
		primitive_indices.primitive_indices[(mesh.base_triangle_index + u16_to_u32(meshlet.triangle_offset) + primitive_index) * 3 + 2]
	);
}
"#;
		let tokens = tokenize(source).expect("Failed to tokenize");
		println!("Tokens: {:?}", tokens.tokens);
		let node = parse(&tokens).expect("Failed to parse");
		let func = &node["compute_triangle"];
		assert!(matches!(&func.node, Nodes::Function { .. }));
	}

	#[test]
	fn parse_grouping_parentheses() {
		// Minimal repro: grouping parentheses inside a function call
		let source = r#"
main: fn () -> void {
	foo((a + b) * 3);
}
"#;
		let tokens = tokenize(source).expect("Failed to tokenize");
		println!("Tokens: {:?}", tokens.tokens);
		let node = parse(&tokens).expect("Failed to parse");
		let func = &node["main"];
		assert!(matches!(&func.node, Nodes::Function { .. }));
	}

	#[test]
	fn parse_conditional_comparing_a_push_constant_member() {
		let source = r#"
main: fn () -> void {
	let local_vertex_index: u32 = thread_id().x;
	if (local_vertex_index >= push_constant.vertex_count) {
		return;
	}
}
"#;
		let tokens = tokenize(source).expect("Failed to tokenize");
		let node = parse(&tokens).expect("Failed to parse push-constant comparison");
		let func = &node["main"];
		assert!(matches!(&func.node, Nodes::Function { .. }));
	}

	#[test]
	fn parse_grouped_arithmetic_inside_a_conditional() {
		let source = r#"
main: fn () -> void {
	if (total_weight > 0.00000001) {
		let column0: vec4f = (
			matrix0.column0 * weights.x
			+ matrix1.column0 * weights.y
		) * inverse_total_weight;
	}
}
"#;
		let tokens = tokenize(source).expect("Failed to tokenize");
		let node = parse(&tokens).expect("Failed to parse grouped conditional arithmetic");
		let func = &node["main"];
		assert!(matches!(&func.node, Nodes::Function { .. }));
	}

	#[test]
	fn parse_process_meshlet() {
		let source = r#"
process_meshlet: fn (instance_index: u32, matrix: mat4f) -> void {
	let mesh: Mesh = meshes.meshes[instance_index];
	let meshlet_index: u32 = threadgroup_position() + mesh.base_meshlet_index;
	let meshlet: Meshlet = meshlets.meshlets[meshlet_index];
	let primitive_index: u32 = thread_idx();

	set_mesh_output_counts(u8_to_u32(meshlet.primitive_count), u8_to_u32(meshlet.triangle_count));

	if (primitive_index < u8_to_u32(meshlet.primitive_count)) {
		set_mesh_vertex_position(
			primitive_index,
			matrix * mesh.model * compute_vertex_position(mesh, meshlet, primitive_index)
		);
	}

	if (primitive_index < u8_to_u32(meshlet.triangle_count)) {
		set_mesh_triangle(primitive_index, compute_triangle(mesh, meshlet, primitive_index));
		out_instance_index[primitive_index] = instance_index;
		out_primitive_index[primitive_index] = meshlet_index << 8 | primitive_index & 255;
	}
}
"#;
		let tokens = tokenize(source).expect("Failed to tokenize");
		let node = parse(&tokens).expect("Failed to parse");
		let func = &node["process_meshlet"];
		assert!(matches!(&func.node, Nodes::Function { .. }));
	}

	#[test]
	fn truncated_function_returns_an_error() {
		let tokens = tokenize("main: fn () -> void {").expect("Failed to tokenize");

		assert!(matches!(parse(&tokens), Err(ParsingFailReasons::BadSyntax { .. })));
	}
}
