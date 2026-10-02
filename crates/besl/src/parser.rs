//! Parses BESL tokens into syntax nodes that preserve the source structure.
//!
//! Use [`crate::parse`] as the entry point. The parser records cross-references by name.
//! The [`crate::lexer`] module resolves those names later.

mod declarations;
mod expressions;
mod iterator;

pub(crate) use declarations::parse;
pub use declarations::{
	BindingResource, ElseBranch, Expressions, MatchArm, MatchPattern, Node, Nodes, ParsingFailReasons, RecordField, RecordRole,
	TypeField, TypeName,
};
#[cfg(test)]
use expressions::*;
#[cfg(test)]
mod tests {
	use super::*;
	use crate::Operators;
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
				uvs: vertex_output<vec2f, 3, 64>;
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
				per_vertex: false,
				..
			} if count.get() == 126
		));
		assert!(matches!(
			root["uvs"].node(),
			Nodes::Output {
				format: "vec2f",
				location: 3,
				count: Some(count),
				per_vertex: true,
				..
			} if count.get() == 64
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
			"value: vertex_output<u32, 0, 0>;",
			"value: vertex_output<u32, 0>;",
			"value: task_payload<u32, 0>;",
			"value: workgroup<u32>",
		] {
			let tokens = tokenize(source).expect("invalid declaration should still tokenize");

			assert!(parse(&tokens).is_err(), "expected `{source}` to be rejected");
		}
	}

	#[test]
	fn parse_resource_descriptors_with_named_properties() {
		let tokens = tokenize(
			r#"
				source: descriptor<{ access: read, type: Texture2D, binding: 3, }>;
				result: descriptor<{ type: StorageImage<rgba16f>, binding: 7, access: write, count: 4 }>;
				unformatted_result: descriptor<{ type: StorageImage, binding: 8, access: write }>;
				data: descriptor<{ type: Data, binding: 11, access: read_write }>;
				textures: descriptor<{ type: Texture2DArray, binding: 20, access: read, count: 16 }>;
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
		assert!(!matches!(
			root["source"].node(),
			Nodes::Descriptor { runtime_array: true, .. }
		));
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
	fn runtime_array_descriptor_rejects_fixed_element_or_resource_counts() {
		for source in [
			"instances: descriptor<{ type: Instance[4], binding: 1, access: read }>;",
			"instances: descriptor<{ type: Instance[], binding: 1, access: read, count: 4 }>;",
			"instances: descriptor<{ type: Instance[], binding: 1, access: read, memory: device, count: 4 }>;",
		] {
			let tokens = tokenize(source).expect("invalid runtime-array descriptor source should tokenize");

			assert!(
				parse(&tokens).is_err(),
				"invalid runtime-array descriptor should be rejected: {source}"
			);
		}
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
	fn descriptor_rejects_invalid_properties() {
		for source in [
			"texture: descriptor<{ type: Texture2D, binding: 0, access: execute }>;",
			"textures: descriptor<{ type: Texture2D, binding: 0, access: read, count: 0 }>;",
			"texture: descriptor<{ binding: 0, access: read }>;",
			"texture: descriptor<{ type: Texture2D, access: read }>;",
			"texture: descriptor<{ type: Texture2D, binding: 0 }>;",
			"texture: descriptor<{ type: Texture2D, binding: first, access: read }>;",
			"texture: descriptor<{ type: Texture2D, binding: 0, access: read, count: many }>;",
			"texture: descriptor<{ type: Texture2D, binding: 0, access: read, memory: shared }>;",
			"texture: descriptor<{ type: Texture2D, type: Texture3D, binding: 0, access: read }>;",
			"texture: descriptor<{ type: Texture2D, binding: 0, binding: 1, access: read }>;",
			"texture: descriptor<{ type: Texture2D, binding: 0, access: read, access: write }>;",
			"texture: descriptor<{ type: Texture2D, binding: 0, access: read, memory: device, memory: constant }>;",
			"texture: descriptor<{ type: Texture2D, binding: 0, access: read, count: 1, count: 2 }>;",
			"texture: descriptor<{ type: Texture2D, binding: 0, access: read, group: 1 }>;",
			"texture: descriptor<{ type: Texture2D; binding: 0; access: read }>;",
		] {
			let tokens = tokenize(source).expect("descriptor source should tokenize");

			assert!(parse(&tokens).is_err(), "malformed descriptor should be rejected: {source}");
		}
	}

	#[test]
	fn descriptor_rejects_formats_on_non_storage_image_resources() {
		for source in [
			"texture: descriptor<{ type: Texture2D<rgba16f>, binding: 0, access: read }>;",
			"data: descriptor<{ type: Data<rgba16f>, binding: 0, access: read }>;",
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

	#[test]
	fn parse_structural_entry_types_and_record_values() {
		let tokens = tokenize(
			"main: fn (input: StageInput, pipeline_input: interface { uv: vec2f, }) -> output { color: vec4f, } { return { color, }; }",
		)
		.expect("trailing-comma record source should tokenize");
		let root = parse(&tokens).expect("trailing-comma record source should parse");
		let Nodes::Function {
			params,
			return_type,
			statements,
			..
		} = root["main"].node()
		else {
			panic!("expected main function");
		};

		assert!(matches!(
			params[1].node(),
			Nodes::Parameter {
				r#type: TypeName::Record {
					role: RecordRole::Interface,
					fields,
				},
				..
			} if fields.len() == 1 && fields[0].name == "uv"
		));
		assert!(matches!(
			return_type,
			TypeName::Record {
				role: RecordRole::Output,
				fields,
			} if fields.len() == 1 && fields[0].name == "color"
		));
		assert!(matches!(
			statements[0].node(),
			Nodes::Expression(Expressions::Return { value: Some(value) })
				if matches!(value.node(), Nodes::Expression(Expressions::RecordLiteral { fields }) if fields.len() == 1 && fields[0].name == "color")
		));
	}

	#[test]
	fn record_and_struct_fields_require_comma_separators() {
		for source in [
			"main: fn (input: interface { uv: vec2f; }) -> void {}",
			"main: fn () -> output { color: vec4f; } { return { color: vec4f(1.0); }; }",
			"Instance: struct { position: vec3f; }",
			"Instance: struct { position: vec3f sprite_id: u32 }",
		] {
			let tokens = tokenize(source).expect("invalid field separator source should tokenize");

			assert!(
				parse(&tokens).is_err(),
				"non-comma field separators should be rejected: {source}"
			);
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
				operator,
				left: var_decl,
				right: function_call,
			}) = &statement.node
			{
				assert_eq!(*operator, Operators::Assignment);

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
	fn parse_match_rejects_malformed_arms() {
		for source in [
			"main: fn () -> void { match n { 0 => break 1 => break } }",
			"main: fn () -> void { match n { x => break, } }",
			"main: fn () -> void { match n { 0 if n => break, } }",
			"main: fn () -> void { match n { 0 break, } }",
		] {
			let tokens = tokenize(source).expect("Failed to tokenize");
			assert!(parse(&tokens).is_err(), "`{source}` should not parse");
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

			if let Nodes::Expression(Expressions::Operator { operator, .. }) = &value.node {
				assert_eq!(*operator, Operators::Multiply);
			} else {
				panic!("Expected an operator expression, got: {:?}", value.node);
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

		let Nodes::Expression(Expressions::Operator { operator, right, .. }) = &statements[0].node else {
			panic!("Expected assignment expression");
		};

		assert_eq!(*operator, Operators::Assignment);

		let Nodes::Expression(Expressions::Operator { operator, left, right }) = &right.node else {
			panic!("Expected bitwise or expression");
		};

		assert_eq!(*operator, Operators::BitwiseOr);
		assert!(matches!(
			left.node,
			Nodes::Expression(Expressions::Operator { operator, .. }) if operator == Operators::ShiftLeft
		));
		assert!(matches!(
			right.node,
			Nodes::Expression(Expressions::Operator { operator, .. }) if operator == Operators::BitwiseAnd
		));
	}

	#[test]
	fn parse_bitwise_xor_between_or_and_and() {
		let source = "
main: fn () -> void {
	let packed: u32 = 1 | 2 ^ 3 & 255;
}";

		let tokens = tokenize(source).expect("Failed to tokenize");
		let node = parse(&tokens).expect("Failed to parse");

		let main_node = &node["main"];
		let Nodes::Function { statements, .. } = &main_node.node else {
			panic!("Expected main function");
		};

		let Nodes::Expression(Expressions::Operator { right, .. }) = &statements[0].node else {
			panic!("Expected assignment expression");
		};

		let Nodes::Expression(Expressions::Operator { operator, right, .. }) = &right.node else {
			panic!("Expected bitwise or expression");
		};

		assert_eq!(*operator, Operators::BitwiseOr);

		let Nodes::Expression(Expressions::Operator { operator, right, .. }) = &right.node else {
			panic!("Expected bitwise xor expression");
		};

		assert_eq!(*operator, Operators::BitwiseXor);
		assert!(matches!(
			right.node,
			Nodes::Expression(Expressions::Operator { operator, .. }) if operator == Operators::BitwiseAnd
		));
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
	fn truncated_function_returns_an_error() {
		let tokens = tokenize("main: fn () -> void {").expect("Failed to tokenize");

		assert!(matches!(parse(&tokens), Err(ParsingFailReasons::BadSyntax { .. })));
	}
}
