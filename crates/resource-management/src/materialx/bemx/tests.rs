use besl::parser::Node as Syntax;

use super::{Conversion, ConvertError};
use crate::bemx::besl::{Geometry, LowerError, Lowering, Stage, literal};
use crate::bemx::{BinaryOperator, Channel, Graph, GraphError, Node, NodeId, Type};
use crate::materialx;

/// Wraps a document body in a MaterialX document.
fn document(body: &str) -> String {
	format!("<?xml version=\"1.0\"?>\n<materialx version=\"1.39\">\n{body}\n</materialx>")
}

/// Wraps a document body and converts its first material, handing the conversion to `check`.
fn with_conversion(body: &str, check: impl FnOnce(&Conversion<'_>)) {
	let source = document(body);
	let arena = bumpalo::Bump::new();
	let allocator = &&arena;

	let dag = materialx::parse(&source, allocator).expect("document should resolve");
	let conversion = super::convert(&dag).expect("material should convert");

	check(&conversion);
}

/// Wraps a document body and hands the conversion failure to `check`.
fn with_failure(body: &str, check: impl FnOnce(ConvertError)) {
	let source = document(body);
	let arena = bumpalo::Bump::new();
	let allocator = &&arena;

	let dag = materialx::parse(&source, allocator).expect("document should resolve");

	check(super::convert(&dag).expect_err("material should not convert"));
}

/// Wraps a surface shader in the material that selects it.
fn material(shader: &str) -> String {
	format!(
		"{shader}\n<surfacematerial name=\"M\" type=\"material\">\n\t<input name=\"surfaceshader\" type=\"surfaceshader\" nodename=\"shader\"/>\n</surfacematerial>"
	)
}

/// Computes the value of a node that only reads constants, so a test can check what a surface input
/// resolves to without depending on how the conversion spelled it.
fn evaluate(graph: &Graph, id: NodeId) -> Vec<f32> {
	match graph.node(id) {
		Node::Constant(value) => value.components().to_vec(),
		Node::Binary { operator, left, right } => {
			let (left, right) = (evaluate(graph, *left), evaluate(graph, *right));
			// A float operand applies to every component of the other one.
			let lane = |values: &[f32], index: usize| values[index.min(values.len() - 1)];

			(0..left.len().max(right.len()))
				.map(|index| {
					let (left, right) = (lane(&left, index), lane(&right, index));

					match operator {
						BinaryOperator::Add => left + right,
						BinaryOperator::Subtract => left - right,
						BinaryOperator::Multiply => left * right,
						BinaryOperator::Divide => left / right,
						other => panic!("The test evaluator does not compute {other:?}."),
					}
				})
				.collect()
		}
		Node::Extract { value, index } => vec![evaluate(graph, *value)[usize::from(*index)]],
		Node::Combine { values } => values.iter().flat_map(|value| evaluate(graph, *value)).collect(),
		other => panic!("{other:?} does not compute a constant."),
	}
}

/// Asserts that a surface input holds the constant `expected`.
fn assert_constant(graph: &Graph, input: Option<NodeId>, expected: &[f32]) {
	let value = evaluate(graph, input.expect("surface input should be written"));

	assert_eq!(value.len(), expected.len(), "{value:?} should be {expected:?}");

	for (value, expected) in value.iter().zip(expected) {
		assert!((value - expected).abs() < 1.0e-6, "{value} should be {expected}");
	}
}

/// Reports whether `id` or anything it reads satisfies `predicate`.
fn reaches(graph: &Graph, id: NodeId, predicate: &impl Fn(&Node) -> bool) -> bool {
	let node = graph.node(id);
	let mut found = predicate(node);

	node.visit_inputs(|input| found = found || reaches(graph, input, predicate));

	found
}

/// Lists the texture parameters the graph samples, in first-sample order.
fn sampled_textures(graph: &Graph) -> Vec<String> {
	let mut textures = Vec::new();

	for node in graph.nodes() {
		if let Node::Image { texture, .. } = node
			&& !textures.contains(texture)
		{
			textures.push(texture.clone());
		}
	}

	textures
}

#[test]
fn constant_surface_fills_every_surface_input() {
	with_conversion(
		&material(
			r#"<standard_surface name="shader" type="surfaceshader">
				<input name="base_color" type="color3" value="0.9, 0.5, 0.1"/>
				<input name="base" type="float" value="0.5"/>
				<input name="metalness" type="float" value="1"/>
				<input name="specular_roughness" type="float" value="0.25"/>
				<input name="opacity" type="color3" value="0.75, 0.5, 0.25"/>
				<input name="emission" type="float" value="2"/>
				<input name="emission_color" type="color3" value="1, 0.5, 0"/>
			</standard_surface>"#,
		),
		|conversion| {
			let graph = &conversion.graph;
			let surface = graph.surface();

			// The base colour is scaled by the model's weight, and the opacity is the first channel of a colour.
			assert_constant(graph, surface.base_color, &[0.45, 0.25, 0.05]);
			assert_constant(graph, surface.metalness, &[1.0]);
			assert_constant(graph, surface.roughness, &[0.25]);
			assert_constant(graph, surface.opacity, &[0.75]);
			assert_constant(graph, surface.emission, &[2.0, 1.0, 0.0]);
			assert_eq!(surface.occlusion, None);
			assert_eq!(surface.normal, None);
			assert!(conversion.textures.is_empty());
		},
	);
}

#[test]
fn unwritten_inputs_take_the_shading_model_defaults() {
	with_conversion(&material(r#"<gltf_pbr name="shader" type="surfaceshader"/>"#), |conversion| {
		let graph = &conversion.graph;
		let surface = graph.surface();

		assert_constant(graph, surface.base_color, &[1.0, 1.0, 1.0]);
		assert_constant(graph, surface.metalness, &[1.0]);
		assert_constant(graph, surface.roughness, &[1.0]);
		assert_constant(graph, surface.occlusion, &[1.0]);
		assert_constant(graph, surface.emission, &[0.0, 0.0, 0.0]);
	});
}

#[test]
fn an_unlit_surface_reflects_nothing() {
	with_conversion(
		&material(
			r#"<surface_unlit name="shader" type="surfaceshader">
				<input name="emission_color" type="color3" value="0.2, 0.4, 0.6"/>
			</surface_unlit>"#,
		),
		|conversion| {
			let graph = &conversion.graph;

			assert_constant(graph, graph.surface().base_color, &[0.0, 0.0, 0.0]);
			assert_constant(graph, graph.surface().emission, &[0.2, 0.4, 0.6]);
		},
	);
}

#[test]
fn image_nodes_become_texture_slots() {
	with_conversion(
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
		|conversion| {
			let graph = &conversion.graph;

			// The two base images name one file, so they share one slot.
			assert_eq!(conversion.textures.len(), 2);
			assert_eq!(conversion.textures[0].file, "textures/base.png");
			assert_eq!(conversion.textures[0].colorspace, Some("srgb_texture"));
			assert_eq!(conversion.textures[1].file, "textures/rough.png");
			assert_eq!(sampled_textures(graph), ["material_texture_0", "material_texture_1"]);

			// An image keeps as many channels as the MaterialX node's type holds.
			assert_eq!(
				graph.node(graph.surface().roughness.unwrap()),
				&Node::Image {
					texture: "material_texture_1".to_string(),
					r#type: Type::Float,
					texcoord: None,
				}
			);
			assert!(reaches(graph, graph.surface().base_color.unwrap(), &|node| matches!(
				node,
				Node::Image { texture, r#type: Type::Vector3, texcoord: None } if texture == "material_texture_0"
			)));
		},
	);
}

#[test]
fn connection_channels_select_components() {
	with_conversion(
		&material(
			r#"<image name="reversed" type="color3">
				<input name="file" type="filename" value="textures/reversed.png"/>
			</image>
			<standard_surface name="shader" type="surfaceshader">
				<input name="base_color" type="color3" nodename="reversed" channels="bgr"/>
			</standard_surface>"#,
		),
		|conversion| {
			let graph = &conversion.graph;

			assert!(reaches(graph, graph.surface().base_color.unwrap(), &|node| matches!(
				node,
				Node::Swizzle { channels, .. } if channels == &[Channel::Z, Channel::Y, Channel::X]
			)));
		},
	);
}

#[test]
fn unread_nodes_are_left_out() {
	with_conversion(
		&material(
			r#"<image name="unused" type="color3">
				<input name="file" type="filename" value="textures/unused.png"/>
			</image>
			<standard_surface name="shader" type="surfaceshader">
				<input name="base_color" type="color3" value="1, 1, 1"/>
			</standard_surface>"#,
		),
		|conversion| {
			assert!(conversion.textures.is_empty());
			assert!(sampled_textures(&conversion.graph).is_empty());
		},
	);
}

#[test]
fn node_graph_outputs_are_expanded_in_place() {
	with_conversion(
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
		|conversion| {
			let graph = &conversion.graph;

			// The graph's interface value reaches the node inside it.
			assert_constant(graph, graph.surface().base_color, &[0.1, 0.15, 0.2]);
		},
	);
}

#[test]
fn a_constant_normal_leaves_the_geometric_normal_alone() {
	with_conversion(
		&material(
			r#"<standard_surface name="shader" type="surfaceshader">
				<input name="base_color" type="color3" value="1, 1, 1"/>
			</standard_surface>"#,
		),
		|conversion| {
			assert_eq!(conversion.graph.surface().normal, None);
		},
	);
}

#[test]
fn a_normal_map_drives_the_world_space_normal() {
	with_conversion(
		&material(
			r#"<image name="bumps" type="vector3">
				<input name="file" type="filename" value="textures/normal.png"/>
			</image>
			<normalmap name="mapped" type="vector3">
				<input name="in" type="vector3" nodename="bumps"/>
				<input name="scale" type="float" value="0.5"/>
			</normalmap>
			<standard_surface name="shader" type="surfaceshader">
				<input name="normal" type="vector3" nodename="mapped"/>
			</standard_surface>"#,
		),
		|conversion| {
			let graph = &conversion.graph;
			let Node::NormalMap { value, scale } = graph.node(graph.surface().normal.unwrap()) else {
				panic!("The surface normal should be the normal map's world-space normal.");
			};

			assert!(matches!(
				graph.node(*value),
				Node::Image {
					r#type: Type::Vector3,
					..
				}
			));
			assert_eq!(evaluate(graph, *scale), [0.5]);
		},
	);
}

#[test]
fn geometric_nodes_read_bemx_geometry() {
	with_conversion(
		&material(
			r#"<viewdirection name="view" type="vector3"/>
			<normal name="facing" type="vector3"/>
			<dotproduct name="grazing" type="float">
				<input name="in1" type="vector3" nodename="view"/>
				<input name="in2" type="vector3" nodename="facing"/>
			</dotproduct>
			<geompropvalue name="toward" type="vector3">
				<input name="geomprop" type="string" value="Vworld"/>
			</geompropvalue>
			<standard_surface name="shader" type="surfaceshader">
				<input name="specular_roughness" type="float" nodename="grazing"/>
				<input name="normal" type="vector3" nodename="toward"/>
			</standard_surface>"#,
		),
		|conversion| {
			let graph = &conversion.graph;
			let surface = graph.surface();
			let Node::DotProduct { left, right } = graph.node(surface.roughness.unwrap()) else {
				panic!("A dot product should convert to a dot product.");
			};

			// MaterialX and BEMX agree that a view direction points from the viewer at the surface.
			assert_eq!(graph.node(*left), &Node::ViewDirection);
			assert_eq!(graph.node(*right), &Node::Normal);

			// `Vworld` points the other way, toward the viewer.
			let Node::Binary {
				operator: BinaryOperator::Subtract,
				left,
				right,
			} = graph.node(surface.normal.unwrap())
			else {
				panic!("Vworld should negate the view direction.");
			};

			assert_eq!(evaluate(graph, *left), [0.0, 0.0, 0.0]);
			assert_eq!(graph.node(*right), &Node::ViewDirection);
		},
	);
}

#[test]
fn missing_switch_inputs_select_zero() {
	with_conversion(
		&material(
			r#"<switch name="pick" type="float">
				<input name="in1" type="float" value="0.25"/>
				<input name="in3" type="float" value="0.75"/>
				<input name="which" type="integer" value="2"/>
			</switch>
			<standard_surface name="shader" type="surfaceshader">
				<input name="specular_roughness" type="float" nodename="pick"/>
			</standard_surface>"#,
		),
		|conversion| {
			let graph = &conversion.graph;
			let Node::Switch { which, values } = graph.node(graph.surface().roughness.unwrap()) else {
				panic!("A switch should convert to a switch.");
			};

			// The integer selector becomes a float, and the gap before the last written input becomes a zero.
			assert_eq!(evaluate(graph, *which), [2.0]);
			assert_eq!(
				values.iter().map(|value| evaluate(graph, *value)).collect::<Vec<_>>(),
				[[0.25], [0.0], [0.75]]
			);
		},
	);
}

#[test]
fn a_document_without_a_material_is_reported() {
	let arena = bumpalo::Bump::new();
	let allocator = &&arena;
	let source = document("<constant name=\"c\" type=\"color3\"/>");

	let dag = materialx::parse(&source, allocator).expect("document should resolve");

	assert_eq!(super::convert(&dag).expect_err("no material"), ConvertError::NoMaterial);
}

#[test]
fn an_unsupported_shading_model_is_reported() {
	with_failure(&material(r#"<surface name="shader" type="surfaceshader"/>"#), |error| {
		assert_eq!(
			error,
			ConvertError::UnsupportedShader {
				material: "M".to_string(),
				category: "surface".to_string(),
			}
		);
	});
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
				ConvertError::UnsupportedNode {
					node: "grain".to_string(),
					category: "noise2d".to_string(),
				}
			);
		},
	);
}

#[test]
fn mismatched_input_types_name_the_node() {
	with_failure(
		&material(
			r#"<dotproduct name="facing" type="float">
				<input name="in1" type="vector3" value="0, 0, 1"/>
				<input name="in2" type="vector2" value="0, 1"/>
			</dotproduct>
			<standard_surface name="shader" type="surfaceshader">
				<input name="specular_roughness" type="float" nodename="facing"/>
			</standard_surface>"#,
		),
		|error| {
			assert!(
				matches!(
					&error,
					ConvertError::Graph { node, error: GraphError::TypeMismatch { input: "in2", found: Type::Vector2, .. } }
						if node == "facing"
				),
				"{error:?}"
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
				ConvertError::UnsupportedTextureCoordinates {
					node: "base".to_string(),
				}
			);
		},
	);
}

#[test]
fn a_node_definition_built_from_a_graph_is_expanded() {
	with_conversion(
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
		|conversion| {
			let graph = &conversion.graph;

			assert_constant(graph, graph.surface().base_color, &[0.2, 0.3, 0.4]);
		},
	);
}

#[test]
fn a_shading_model_built_from_a_graph_is_expanded() {
	with_conversion(
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
		|conversion| {
			let graph = &conversion.graph;

			assert_constant(graph, graph.surface().base_color, &[0.2, 0.3, 0.4]);
			assert_constant(graph, graph.surface().roughness, &[0.75]);
		},
	);
}

#[test]
fn a_material_written_inside_a_node_graph_is_reached() {
	with_conversion(
		r#"<nodegraph name="wrapper">
			<standard_surface name="inner" type="surfaceshader">
				<input name="specular_roughness" type="float" value="0.6"/>
			</standard_surface>
			<surfacematerial name="M" type="material">
				<input name="surfaceshader" type="surfaceshader" nodename="inner"/>
			</surfacematerial>
			<output name="out" type="material" nodename="M"/>
		</nodegraph>"#,
		|conversion| {
			let graph = &conversion.graph;

			assert_constant(graph, graph.surface().roughness, &[0.6]);
		},
	);
}

/// The `Locals` struct is a lowering stage whose geometry and texture samples are locals the test
/// program declares, so the lowered statements lex without a renderer's interface around them.
struct Locals;

impl<'a> Stage<'a> for Locals {
	fn geometry(&mut self, geometry: Geometry) -> Syntax<'a> {
		Syntax::member_expression(match geometry {
			Geometry::Position => "position",
			Geometry::Normal => "normal",
			Geometry::Tangent => "tangent",
			Geometry::Bitangent => "bitangent",
			Geometry::View => "view",
			Geometry::TexCoord => "uv",
		})
	}

	fn sample(&mut self, _: &'a str) -> Result<Syntax<'a>, LowerError> {
		Ok(Syntax::member_expression("texel"))
	}

	fn parameter(&mut self, name: &'a str, r#type: Type) -> Result<Syntax<'a>, LowerError> {
		Err(LowerError::UnknownParameter {
			name: name.to_string(),
			r#type,
		})
	}
}

#[test]
fn converted_textured_material_lowers_to_besl_that_lexes() {
	with_conversion(
		&material(
			r#"<image name="albedo" type="color3">
				<input name="file" type="filename" value="textures/albedo.png" colorspace="srgb_texture"/>
			</image>
			<image name="packed" type="color3">
				<input name="file" type="filename" value="textures/packed.png"/>
			</image>
			<extract name="rough" type="float">
				<input name="in" type="color3" nodename="packed"/>
				<input name="index" type="integer" value="1"/>
			</extract>
			<image name="bumps" type="vector3">
				<input name="file" type="filename" value="textures/normal.png"/>
			</image>
			<normalmap name="mapped" type="vector3">
				<input name="in" type="vector3" nodename="bumps"/>
			</normalmap>
			<multiply name="tinted" type="color3">
				<input name="in1" type="color3" nodename="albedo"/>
				<input name="in2" type="color3" value="0.8, 0.7, 0.6"/>
			</multiply>
			<standard_surface name="shader" type="surfaceshader">
				<input name="base_color" type="color3" nodename="tinted"/>
				<input name="specular_roughness" type="float" nodename="rough"/>
				<input name="normal" type="vector3" nodename="mapped"/>
			</standard_surface>"#,
		),
		|conversion| {
			let graph = &conversion.graph;
			let mut lowering = Lowering::new(graph, Locals);
			let mut outputs = Vec::new();

			for (name, input, r#type, _) in graph.surface().inputs() {
				if let Some(input) = input {
					let value = lowering.value(input).expect("surface input should lower");
					let declared = if r#type == Type::Float { "f32" } else { "vec3f" };

					outputs.push(Syntax::let_assignment(format!("output_{name}"), declared, value.syntax));
				}
			}

			let (statements, _) = lowering.finish();

			// The stage's values are plain locals here, declared before the statements that read them.
			let mut body = vec![
				Syntax::let_assignment(
					"position",
					"vec3f",
					Syntax::call("vec3f", vec![literal(0.0), literal(0.0), literal(0.0)]),
				),
				Syntax::let_assignment(
					"normal",
					"vec3f",
					Syntax::call("vec3f", vec![literal(0.0), literal(0.0), literal(1.0)]),
				),
				Syntax::let_assignment(
					"tangent",
					"vec3f",
					Syntax::call("vec3f", vec![literal(1.0), literal(0.0), literal(0.0)]),
				),
				Syntax::let_assignment(
					"bitangent",
					"vec3f",
					Syntax::call("vec3f", vec![literal(0.0), literal(1.0), literal(0.0)]),
				),
				Syntax::let_assignment(
					"view",
					"vec3f",
					Syntax::call("vec3f", vec![literal(0.0), literal(0.0), literal(1.0)]),
				),
				Syntax::let_assignment("uv", "vec2f", Syntax::call("vec2f", vec![literal(0.5), literal(0.5)])),
				Syntax::let_assignment(
					"texel",
					"vec4f",
					Syntax::call("vec4f", vec![literal(0.5), literal(0.5), literal(1.0), literal(1.0)]),
				),
			];

			body.extend(statements);
			body.extend(outputs);

			besl::lex(Syntax::root_with_children(vec![Syntax::main_function(body)])).expect("lowered material should lex");
		},
	);
}
