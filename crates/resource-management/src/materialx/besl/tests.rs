use besl::parser::{Expressions, Node, Nodes};

use crate::materialx;

/// Wraps a document body and lowers its first material, handing the program to `check`.
fn with_program(body: &str, check: impl FnOnce(&super::Program<'_>)) {
	let source = format!("<?xml version=\"1.0\"?>\n<materialx version=\"1.39\">\n{body}\n</materialx>");
	let arena = bumpalo::Bump::new();
	let allocator = &&arena;

	let dag = materialx::parse(&source, allocator).expect("document should resolve");
	let program = super::lower(&dag).expect("material should lower");

	check(&program);
}

/// Wraps a document body and hands the lowering failure to `check`.
fn with_failure(body: &str, check: impl FnOnce(super::LowerError)) {
	let source = format!("<?xml version=\"1.0\"?>\n<materialx version=\"1.39\">\n{body}\n</materialx>");
	let arena = bumpalo::Bump::new();
	let allocator = &&arena;

	let dag = materialx::parse(&source, allocator).expect("document should resolve");

	check(super::lower(&dag).expect_err("material should not lower"));
}

/// Wraps a surface shader in the material that selects it.
fn material(shader: &str) -> String {
	format!(
		"{shader}\n<surfacematerial name=\"M\" type=\"material\">\n\t<input name=\"surfaceshader\" type=\"surfaceshader\" nodename=\"shader\"/>\n</surfacematerial>"
	)
}

/// Returns the statements of the program's `main` function.
fn statements<'a, 'p>(program: &'p super::Program<'a>) -> &'p [Node<'a>] {
	let Nodes::Scope { children, .. } = program.root.node() else {
		panic!("Lowered program should be a scope.");
	};

	let Nodes::Function { statements, .. } = children[0].node() else {
		panic!("Lowered program should hold a function.");
	};

	statements
}

/// Returns the expression assigned to one material property.
fn assignment<'a, 'p>(program: &'p super::Program<'a>, property: &str) -> &'p Node<'a> {
	statements(program)
		.iter()
		.find_map(|statement| match statement.node() {
			Nodes::Expression(Expressions::Operator { name: "=", left, right }) => match left.node() {
				Nodes::Expression(Expressions::Member { name }) if name == property => Some(&**right),
				_ => None,
			},
			_ => None,
		})
		.unwrap_or_else(|| panic!("Lowered program should write '{property}'."))
}

/// Collects every literal in an expression tree, in the order it holds them.
fn literals(node: &Node<'_>, found: &mut Vec<String>) {
	match node.node() {
		Nodes::Expression(Expressions::Literal { value }) => found.push(value.to_string()),
		Nodes::Expression(Expressions::Call { parameters, .. }) => {
			for parameter in parameters {
				literals(parameter, found);
			}
		}
		Nodes::Expression(Expressions::Operator { left, right, .. }) => {
			literals(left, found);
			literals(right, found);
		}
		Nodes::Expression(Expressions::Accessor { left, right }) => {
			literals(left, found);
			literals(right, found);
		}
		Nodes::Expression(Expressions::Expression(elements)) => {
			for element in elements {
				literals(element, found);
			}
		}
		_ => {}
	}
}

/// Reports whether an expression tree calls a named function anywhere.
fn calls(node: &Node<'_>, name: &str) -> bool {
	match node.node() {
		Nodes::Expression(Expressions::Call {
			name: called,
			parameters,
		}) => {
			matches!(called, besl::parser::TypeName::Named(called) if *called == name)
				|| parameters.iter().any(|parameter| calls(parameter, name))
		}
		Nodes::Expression(Expressions::Operator { left, right, .. })
		| Nodes::Expression(Expressions::Accessor { left, right }) => calls(left, name) || calls(right, name),
		Nodes::Expression(Expressions::Expression(elements)) => elements.iter().any(|element| calls(element, name)),
		Nodes::Expression(Expressions::Return { value: Some(value) }) => calls(value, name),
		_ => false,
	}
}

/// Collects every literal the program's body holds, in the order it computes them.
fn program_literals(program: &super::Program<'_>) -> Vec<String> {
	let mut found = Vec::new();

	for statement in statements(program) {
		literals(statement, &mut found);
	}

	found
}

/// Reports whether the program's body calls a named function anywhere.
fn program_calls(program: &super::Program<'_>, name: &str) -> bool {
	statements(program).iter().any(|statement| calls(statement, name))
}

#[test]
fn constant_surface_writes_every_material_property() {
	with_program(
		&material(
			r#"<standard_surface name="shader" type="surfaceshader">
				<input name="base_color" type="color3" value="0.9, 0.5, 0.1"/>
				<input name="metalness" type="float" value="1"/>
				<input name="specular_roughness" type="float" value="0.25"/>
			</standard_surface>"#,
		),
		|program| {
			let written = program_literals(program);

			for value in ["0.9", "0.5", "0.1", "0.25"] {
				assert!(written.contains(&value.to_string()), "{written:?} should hold {value}");
			}

			// The albedo carries the base colour's three channels next to the surface's opacity.
			let Nodes::Expression(Expressions::Call { name, parameters }) = assignment(program, "albedo").node() else {
				panic!("An albedo write should construct a value.");
			};
			assert_eq!(name.to_string(), "vec4f");
			assert_eq!(parameters.len(), 4);

			let mut metalness = Vec::new();
			literals(assignment(program, "metalness"), &mut metalness);
			assert_eq!(metalness, ["1.0"]);

			let mut roughness = Vec::new();
			literals(assignment(program, "roughness"), &mut roughness);
			assert_eq!(roughness, ["0.25"]);
		},
	);
}

#[test]
fn lowered_program_links() {
	with_program(
		&material(
			r#"<multiply name="tint" type="color3">
				<input name="in1" type="color3" value="0.8, 0.7, 0.6"/>
				<input name="in2" type="float" value="0.5"/>
			</multiply>
			<standard_surface name="shader" type="surfaceshader">
				<input name="base_color" type="color3" nodename="tint"/>
				<input name="specular_roughness" type="float" value="0.4"/>
			</standard_surface>"#,
		),
		|program| {
			// A material program is linked with the renderer's own declarations, so it links here as a
			// standalone program only once its material property writes are turned into locals.
			let mut root = program.root.clone();
			let Nodes::Scope { children, .. } = root.node_mut() else {
				panic!("Lowered program should be a scope.");
			};
			let Nodes::Function { statements, .. } = children[0].node_mut() else {
				panic!("Lowered program should hold a function.");
			};

			for statement in statements.iter_mut() {
				let Nodes::Expression(Expressions::Operator { name: "=", left, right }) = statement.node() else {
					continue;
				};
				let Nodes::Expression(Expressions::Member { name }) = left.node() else {
					continue;
				};
				let declared = match name.as_ref() {
					"albedo" => "vec4f",
					"metalness" | "roughness" | "occlusion" => "f32",
					_ => "vec3f",
				};

				*statement = Node::let_assignment(name.to_string(), declared, (**right).clone());
			}

			besl::lex(root).expect("lowered material program should link");
		},
	);
}

#[test]
fn image_nodes_become_texture_slots() {
	with_program(
		&material(
			r#"<image name="base" type="color3">
				<input name="file" type="filename" value="textures/base.png" colorspace="srgb_texture"/>
			</image>
			<image name="again" type="color3">
				<input name="file" type="filename" value="textures/base.png" colorspace="srgb_texture"/>
			</image>
			<image name="rough" type="float">
				<input name="file" type="filename" value="textures/rough.png"/>
			</image>
			<multiply name="tinted" type="color3">
				<input name="in1" type="color3" nodename="base"/>
				<input name="in2" type="color3" nodename="again"/>
			</multiply>
			<standard_surface name="shader" type="surfaceshader">
				<input name="base_color" type="color3" nodename="tinted"/>
				<input name="specular_roughness" type="float" nodename="rough"/>
			</standard_surface>"#,
		),
		|program| {
			// The two base images name one file, so they share one slot.
			assert_eq!(program.textures.len(), 2);
			assert_eq!(program.textures[0].file, "textures/base.png");
			assert_eq!(program.textures[0].colorspace, Some("srgb_texture"));
			assert_eq!(program.textures[1].file, "textures/rough.png");
			assert!(program_calls(program, "sample_material"));
		},
	);
}

#[test]
fn unread_nodes_are_left_out() {
	with_program(
		&material(
			r#"<image name="unused" type="color3">
				<input name="file" type="filename" value="textures/unused.png"/>
			</image>
			<standard_surface name="shader" type="surfaceshader">
				<input name="base_color" type="color3" value="1, 1, 1"/>
			</standard_surface>"#,
		),
		|program| {
			assert!(program.textures.is_empty());
			assert!(!program_calls(program, "sample_material"));
		},
	);
}

#[test]
fn node_graph_outputs_are_expanded_in_place() {
	with_program(
		&material(
			r#"<nodegraph name="tint">
				<input name="factor" type="float" value="0.25"/>
				<multiply name="scaled" type="color3">
					<input name="in1" type="color3" value="0.4, 0.6, 0.8"/>
					<input name="in2" type="float" interfacename="factor"/>
				</multiply>
				<output name="out" type="color3" nodename="scaled"/>
			</nodegraph>
			<standard_surface name="shader" type="surfaceshader">
				<input name="base_color" type="color3" nodegraph="tint"/>
			</standard_surface>"#,
		),
		|program| {
			let written = program_literals(program);

			// The graph's interface value reaches the node inside it.
			assert!(written.contains(&"0.25".to_string()), "{written:?}");
			assert!(written.contains(&"0.4".to_string()), "{written:?}");
		},
	);
}

#[test]
fn geometric_nodes_read_the_material_stage() {
	with_program(
		&material(
			r#"<texcoord name="uv" type="vector2"/>
			<convert name="tinted" type="color3">
				<input name="in" type="vector2" nodename="uv"/>
			</convert>
			<standard_surface name="shader" type="surfaceshader">
				<input name="base_color" type="color3" nodename="tinted"/>
			</standard_surface>"#,
		),
		|program| {
			let mut found = false;

			for statement in statements(program) {
				let text = format!("{statement:?}");
				found |= text.contains("vertex_uv");
			}

			assert!(found, "Lowered program should read the stage's texture coordinates.");
		},
	);
}

#[test]
fn world_space_normals_are_projected_onto_the_tangent_frame() {
	with_program(
		&material(
			r#"<image name="encoded" type="vector3">
				<input name="file" type="filename" value="textures/normal.png"/>
			</image>
			<normalmap name="mapped" type="vector3">
				<input name="in" type="vector3" nodename="encoded"/>
			</normalmap>
			<standard_surface name="shader" type="surfaceshader">
				<input name="normal" type="vector3" nodename="mapped"/>
			</standard_surface>"#,
		),
		|program| {
			let normal = assignment(program, "normal");

			assert!(calls(normal, "dot"), "A shading normal should project onto the frame.");
			assert!(program_calls(program, "normalize"));
		},
	);
}

#[test]
fn a_constant_normal_leaves_the_geometric_normal_alone() {
	with_program(
		&material(
			r#"<standard_surface name="shader" type="surfaceshader">
				<input name="base_color" type="color3" value="1, 1, 1"/>
			</standard_surface>"#,
		),
		|program| {
			assert!(
				!statements(program).iter().any(|statement| {
					matches!(statement.node(), Nodes::Expression(Expressions::Operator { name: "=", left, .. })
						if matches!(left.node(), Nodes::Expression(Expressions::Member { name }) if name == "normal"))
				}),
				"An unconnected normal should not be written."
			);
		},
	);
}

#[test]
fn a_document_without_a_material_is_reported() {
	let arena = bumpalo::Bump::new();
	let allocator = &&arena;
	let source = "<?xml version=\"1.0\"?>\n<materialx version=\"1.39\">\n<constant name=\"c\" type=\"color3\"/>\n</materialx>";

	let dag = materialx::parse(source, allocator).expect("document should resolve");

	assert_eq!(super::lower(&dag).expect_err("no material"), super::LowerError::NoMaterial);
}

#[test]
fn an_unsupported_shading_model_is_reported() {
	with_failure(
		&material(r#"<surface name="shader" type="surfaceshader"/>"#),
		|error| {
			assert_eq!(
				error,
				super::LowerError::UnsupportedShader {
					material: "M".to_string(),
					category: "surface".to_string(),
				}
			);
		},
	);
}

#[test]
fn an_unsupported_node_names_itself() {
	with_failure(
		&material(
			r#"<noise2d name="grain" type="color3"/>
			<standard_surface name="shader" type="surfaceshader">
				<input name="base_color" type="color3" nodename="grain"/>
			</standard_surface>"#,
		),
		|error| {
			assert_eq!(
				error,
				super::LowerError::UnsupportedNode {
					node: "grain".to_string(),
					category: "noise2d".to_string(),
				}
			);
		},
	);
}

#[test]
fn transformed_texture_coordinates_are_reported() {
	with_failure(
		&material(
			r#"<texcoord name="uv" type="vector2"/>
			<multiply name="scaled" type="vector2">
				<input name="in1" type="vector2" nodename="uv"/>
				<input name="in2" type="float" value="2"/>
			</multiply>
			<image name="base" type="color3">
				<input name="file" type="filename" value="textures/base.png"/>
				<input name="texcoord" type="vector2" nodename="scaled"/>
			</image>
			<standard_surface name="shader" type="surfaceshader">
				<input name="base_color" type="color3" nodename="base"/>
			</standard_surface>"#,
		),
		|error| {
			assert_eq!(
				error,
				super::LowerError::UnsupportedTextureCoordinates {
					node: "base".to_string(),
				}
			);
		},
	);
}

#[test]
fn a_node_definition_built_from_a_graph_is_expanded() {
	with_program(
		&material(
			r#"<nodedef name="ND_tint" node="tint" nodegroup="math">
				<input name="in" type="color3" value="1, 1, 1"/>
				<input name="factor" type="float" value="1"/>
				<output name="out" type="color3"/>
			</nodedef>
			<nodegraph name="NG_tint" nodedef="ND_tint">
				<multiply name="scaled" type="color3">
					<input name="in1" type="color3" interfacename="in"/>
					<input name="in2" type="float" interfacename="factor"/>
				</multiply>
				<output name="out" type="color3" nodename="scaled"/>
			</nodegraph>
			<tint name="tinted" type="color3">
				<input name="in" type="color3" value="0.4, 0.6, 0.8"/>
				<input name="factor" type="float" value="0.5"/>
			</tint>
			<standard_surface name="shader" type="surfaceshader">
				<input name="base_color" type="color3" nodename="tinted"/>
			</standard_surface>"#,
		),
		|program| {
			let written = program_literals(program);

			assert!(written.contains(&"0.4".to_string()), "{written:?}");
			assert!(written.contains(&"0.5".to_string()), "{written:?}");
		},
	);
}

#[test]
fn a_shading_model_built_from_a_graph_is_expanded() {
	with_program(
		&material(
			r#"<nodedef name="ND_house_surface" node="house_surface" nodegroup="pbr">
				<input name="tint" type="color3" value="1, 1, 1"/>
				<output name="out" type="surfaceshader"/>
			</nodedef>
			<nodegraph name="NG_house_surface" nodedef="ND_house_surface">
				<standard_surface name="inner" type="surfaceshader">
					<input name="base_color" type="color3" interfacename="tint"/>
					<input name="specular_roughness" type="float" value="0.75"/>
				</standard_surface>
				<output name="out" type="surfaceshader" nodename="inner"/>
			</nodegraph>
			<house_surface name="shader" type="surfaceshader">
				<input name="tint" type="color3" value="0.2, 0.3, 0.4"/>
			</house_surface>"#,
		),
		|program| {
			let written = program_literals(program);

			assert!(written.contains(&"0.2".to_string()), "{written:?}");

			let mut roughness = Vec::new();
			literals(assignment(program, "roughness"), &mut roughness);
			assert_eq!(roughness, ["0.75"]);
		},
	);
}

#[test]
fn a_material_written_inside_a_node_graph_is_reached() {
	with_program(
		r#"<nodegraph name="wrapper">
			<standard_surface name="inner" type="surfaceshader">
				<input name="specular_roughness" type="float" value="0.6"/>
			</standard_surface>
			<surfacematerial name="M" type="material">
				<input name="surfaceshader" type="surfaceshader" nodename="inner"/>
			</surfacematerial>
			<output name="out" type="material" nodename="M"/>
		</nodegraph>"#,
		|program| {
			let mut roughness = Vec::new();
			literals(assignment(program, "roughness"), &mut roughness);
			assert_eq!(roughness, ["0.6"]);
		},
	);
}

#[test]
fn an_unlit_surface_carries_its_colour_as_emission() {
	with_program(
		&material(
			r#"<surface_unlit name="shader" type="surfaceshader">
				<input name="emission" type="float" value="2"/>
				<input name="emission_color" type="color3" value="1, 0.5, 0.25"/>
			</surface_unlit>"#,
		),
		|program| {
			let written = program_literals(program);

			assert!(written.contains(&"0.25".to_string()), "{written:?}");
			assert!(written.contains(&"2.0".to_string()), "{written:?}");
		},
	);
}

#[test]
fn comparisons_carry_their_result_without_a_conditional() {
	with_program(
		&material(
			r#"<ifgreater name="pick" type="color3">
				<input name="value1" type="float" value="2"/>
				<input name="value2" type="float" value="1"/>
				<input name="in1" type="color3" value="1, 0, 0"/>
				<input name="in2" type="color3" value="0, 1, 0"/>
			</ifgreater>
			<standard_surface name="shader" type="surfaceshader">
				<input name="base_color" type="color3" nodename="pick"/>
			</standard_surface>"#,
		),
		|program| {
			// BESL has no conditional expression, so the branch is weighted by a step.
			assert!(program_calls(program, "step"));
		},
	);
}
