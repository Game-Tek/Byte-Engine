//! Checks the graph rules and the JSON form of BEMX.

use super::json::{self, ParseError};
use super::{BinaryOperator, GraphBuilder, GraphError, Node, Surface, Type, Value};
use crate::bemx::Expected;

#[test]
fn a_float_operand_applies_to_every_component_of_the_other() {
	let mut builder = GraphBuilder::new();
	let colour = builder.constant(Value::Vector3([1.0, 0.5, 0.25]));
	let half = builder.constant(Value::Float(0.5));

	let scaled = builder
		.add(Node::Binary {
			operator: BinaryOperator::Multiply,
			left: half,
			right: colour,
		})
		.expect("a float times a vector is a vector");

	assert_eq!(builder.r#type(scaled), Some(Type::Vector3));
}

#[test]
fn connections_of_different_widths_are_rejected() {
	let mut builder = GraphBuilder::new();
	let colour = builder.constant(Value::Vector3([1.0; 3]));
	let opaque = builder.constant(Value::Vector4([1.0; 4]));

	let error = builder
		.add(Node::Binary {
			operator: BinaryOperator::Add,
			left: colour,
			right: opaque,
		})
		.expect_err("a vector3 plus a vector4 has no type");

	assert!(matches!(
		error,
		GraphError::TypeMismatch {
			input: "in2",
			expected: Expected::TypeOrFloat(Type::Vector3),
			found: Type::Vector4,
			..
		}
	));
}

#[test]
fn a_surface_takes_inputs_of_its_declared_types_only() {
	let mut builder = GraphBuilder::new();
	let roughness = builder.constant(Value::Vector2([0.5, 0.5]));

	let error = builder
		.add(Node::Surface(Surface {
			roughness: Some(roughness),
			..Surface::default()
		}))
		.expect_err("roughness is a float");

	assert!(matches!(error, GraphError::TypeMismatch { input: "roughness", .. }));
}

#[test]
fn nodes_cannot_read_the_surface_or_nodes_added_after_them() {
	let mut builder = GraphBuilder::new();
	let surface = builder
		.add(Node::Surface(Surface::default()))
		.expect("an empty surface is valid");

	assert!(matches!(
		builder.add(Node::Magnitude { value: surface }),
		Err(GraphError::ReadsSurface { .. })
	));

	let later = super::NodeId::from_index(10);

	assert!(matches!(
		builder.add(Node::Magnitude { value: later }),
		Err(GraphError::UnknownInput { .. })
	));
}

#[test]
fn a_graph_finishes_with_a_surface() {
	let mut builder = GraphBuilder::new();
	let constant = builder.constant(Value::Float(1.0));

	assert!(matches!(builder.finish(constant), Err(GraphError::NotASurface { .. })));
}

#[test]
fn json_nodes_resolve_in_any_order_with_inline_constants_and_defaults() {
	let graph = json::parse(
		r#"{
			// The surface comes first and reads nodes written after it.
			nodes: [
				{ name: "surface", node: "surface", base_color: "tinted", roughness: 0.25 },
				{ name: "tinted", node: "multiply", in1: "albedo" },
				{ name: "albedo", node: "image", texture: "floor_texture", type: "color3" },
			],
		}"#,
	)
	.expect("graph should parse");

	let surface = graph.surface();
	let tinted = surface.base_color.expect("the base colour is written");

	assert_eq!(graph.r#type(tinted), Some(Type::Vector3));
	assert!(matches!(
		graph.node(tinted),
		Node::Binary { operator: BinaryOperator::Multiply, right, .. } if *graph.node(*right) == Node::Constant(Value::Float(1.0))
	));
	assert_eq!(
		surface.roughness.map(|roughness| graph.node(roughness)),
		Some(&Node::Constant(Value::Float(0.25)))
	);

	assert!(
		graph
			.nodes()
			.iter()
			.any(|node| matches!(node, Node::Image { texture, .. } if texture == "floor_texture"))
	);
}

#[test]
fn json_mistakes_name_the_node_they_are_in() {
	let parse = |nodes: &str| json::parse(&format!("{{ nodes: [{nodes}] }}")).expect_err("graph should not parse");

	assert_eq!(
		parse(r#"{ name: "surface", node: "surface", base_colour: 1.0 }"#),
		ParseError::UnknownField {
			node: Some("surface".to_string()),
			field: "base_colour".to_string(),
		}
	);
	assert_eq!(
		parse(r#"{ name: "x", node: "noise3d" }, { name: "surface", node: "surface" }"#),
		ParseError::UnknownKind {
			node: "x".to_string(),
			kind: "noise3d".to_string(),
		}
	);
	assert_eq!(
		parse(r#"{ name: "surface", node: "surface", roughness: "gloss" }"#),
		ParseError::UnknownReference {
			node: Some("surface".to_string()),
			field: "roughness".to_string(),
			reference: "gloss".to_string(),
		}
	);
	assert_eq!(
		parse(
			r#"{ name: "a", node: "sin", in: "b" }, { name: "b", node: "cos", in: "a" }, { name: "surface", node: "surface" }"#
		),
		ParseError::Cycle { node: "a".to_string() }
	);
	assert_eq!(
		parse(r#"{ name: "a", node: "position" }, { name: "a", node: "normal" }, { name: "surface", node: "surface" }"#),
		ParseError::DuplicateName { node: "a".to_string() }
	);
	assert_eq!(
		parse(r#"{ name: "pick", node: "switch", in1: 1.0, in3: 2.0 }, { name: "surface", node: "surface" }"#),
		ParseError::InvalidField {
			node: Some("pick".to_string()),
			field: "in3".to_string(),
			expected: "an input that follows every lower-numbered input",
		}
	);
	assert!(matches!(
		parse(r#"{ name: "surface", node: "surface", metalness: [1.0, 0.0] }"#),
		ParseError::Graph { node, error: GraphError::TypeMismatch { input: "metalness", .. } } if node == "surface"
	));
}

#[test]
fn json_documents_pick_their_surface() {
	assert_eq!(
		json::parse(r#"{ nodes: [{ name: "p", node: "position" }] }"#),
		Err(ParseError::MissingSurface)
	);
	assert_eq!(
		json::parse(r#"{ nodes: [{ name: "a", node: "surface" }, { name: "b", node: "surface" }] }"#),
		Err(ParseError::AmbiguousSurface)
	);

	let graph = json::parse(
		r#"{ surface: "b", nodes: [{ name: "a", node: "surface", roughness: 0.1 }, { name: "b", node: "surface" }] }"#,
	)
	.expect("a named surface resolves the choice");

	assert_eq!(graph.surface().roughness, None);
}

/// A graph is translucent when its surface sets `translucency`, whatever the input computes, and not otherwise.
#[test]
fn a_surface_that_sets_translucency_makes_a_translucent_graph() {
	let leaf = json::parse(
		r#"{ nodes: [
			{ name: "albedo", node: "image", texture: "base_color", type: "vector3" },
			{ name: "surface", node: "surface", base_color: "albedo", translucency: [0.0, 0.0, 0.0] },
		] }"#,
	)
	.expect("a surface with translucency should parse");
	assert!(leaf.is_translucent());

	let stone = json::parse(r#"{ nodes: [{ name: "surface", node: "surface", roughness: 0.9 }] }"#)
		.expect("a surface without translucency should parse");
	assert!(!stone.is_translucent());

	// `translucency` is a colour, as `base_color` is.
	json::parse(r#"{ nodes: [{ name: "surface", node: "surface", translucency: 0.5 }] }"#)
		.expect_err("a float translucency should not parse");
}

/// A graph scatters when its surface sets `subsurface_radius`, a colour of mean free paths, and `thickness` makes it
/// translucent only together with that radius and without `translucency`.
#[test]
fn subsurface_inputs_make_subsurface_and_translucent_graphs() {
	let skin = json::parse(
		r#"{ nodes: [
			{ name: "radius", node: "parameter", parameter: "radius", type: "vector3" },
			{ name: "surface", node: "surface", subsurface_radius: "radius" },
		] }"#,
	)
	.expect("a surface with a subsurface radius should parse");
	assert!(skin.is_subsurface());
	assert!(!skin.is_translucent());

	let ear = json::parse(
		r#"{ nodes: [{ name: "surface", node: "surface", subsurface_radius: [0.004, 0.0014, 0.0007], thickness: 0.01 }] }"#,
	)
	.expect("a subsurface surface with a thickness should parse");
	assert!(ear.is_subsurface());
	assert!(ear.is_translucent());

	let stone = json::parse(r#"{ nodes: [{ name: "surface", node: "surface", roughness: 0.9 }] }"#)
		.expect("a surface without subsurface inputs should parse");
	assert!(!stone.is_subsurface());

	// The radius is a colour, as `translucency` is.
	json::parse(r#"{ nodes: [{ name: "surface", node: "surface", subsurface_radius: 0.004 }] }"#)
		.expect_err("a float subsurface radius should not parse");
	let lone = json::parse(r#"{ nodes: [{ name: "surface", node: "surface", thickness: 0.01 }] }"#)
		.expect_err("a thickness without a subsurface radius should not parse");
	assert!(lone.to_string().contains("subsurface_radius"), "{lone}");
	json::parse(
		r#"{ nodes: [{ name: "surface", node: "surface", subsurface_radius: [0.004, 0.0014, 0.0007], thickness: 0.01, translucency: [0.5, 0.5, 0.5] }] }"#,
	)
	.expect_err("a thickness together with a translucency should not parse");
}

#[test]
fn json_error_messages_name_the_node_and_the_likely_cause() {
	let error = json::parse(r#"{ nodes: [{ name: "surface", node: "surface", metalness: [1.0, 0.0] }] }"#)
		.expect_err("graph should not parse");
	let message = error.to_string();

	assert!(message.contains("'surface'"), "{message}");
	assert!(message.contains("The most likely cause"), "{message}");
}

#[test]
fn written_text_reads_back_as_the_same_graph_with_names_first() {
	let source = r#"{
		// A comment, which a document doesn't keep.
		nodes: [
			{ type: "color3", texture: "color", node: "image", name: "base_color" },
			{ in2: [1.0, 0.9, 0.8], name: "tinted", node: "multiply", in1: "base_color" },
			{ name: "surface", node: "surface", roughness: 0.5, base_color: "tinted" },
		],
	}"#;
	let document = crate::asset::parse_json(source).unwrap();

	let written = json::write(&document);

	assert_eq!(crate::asset::parse_json(&written).unwrap(), document);
	assert_eq!(json::parse(&written).unwrap(), json::parse(source).unwrap());
	assert!(
		written.contains(r#"{ name: "tinted", node: "multiply", in1: "base_color", in2: [1.0, 0.9, 0.8] }"#),
		"{written}"
	);
}

#[test]
fn every_kind_accepts_the_fields_its_table_lists_and_no_other() {
	for kind in super::kinds::KINDS {
		// A graph needs one surface, so a surface under test is that one.
		let surface = if kind.produces {
			r#", { name: "surface", node: "surface" }"#
		} else {
			""
		};
		let node = |extra: &str| format!(r#"{{ nodes: [{{ name: "node", node: "{}"{extra} }}{surface}] }}"#, kind.name);
		// Other errors, such as a missing required input, are fine here: the fields themselves are accepted.
		for field in kind
			.inputs
			.iter()
			.map(|input| input.name)
			.chain(kind.settings.iter().map(|setting| setting.name))
		{
			let result = json::parse(&node(&format!(r#", {field}: 1"#)));
			assert!(
				!matches!(result, Err(ParseError::UnknownField { .. })),
				"{} rejects its own field {field}",
				kind.name
			);
		}
		assert!(
			matches!(json::parse(&node(r#", unlisted: 1"#)), Err(ParseError::UnknownField { .. })),
			"{} accepts a field its table doesn't list",
			kind.name
		);
	}
}

#[test]
fn an_input_left_out_reads_its_tables_default() {
	use super::kinds::{InputDefault, KINDS};

	// A constant every required input can read, and a setting value every kind takes.
	let mut compared = 0;
	for kind in KINDS.iter().filter(|kind| kind.produces) {
		let required = kind
			.inputs
			.iter()
			.filter(|input| input.default == InputDefault::Required)
			.map(|input| format!(", {}: [0.5, 0.5, 0.5]", input.name))
			.chain(kind.settings.iter().filter(|setting| setting.required).map(|setting| {
				let value = match setting.kind {
					super::kinds::SettingKind::Type => r#""vector3""#,
					super::kinds::SettingKind::Channels => r#""xyz""#,
					super::kinds::SettingKind::Value => "[0.5, 0.5, 0.5]",
					_ => r#""name""#,
				};
				format!(", {}: {value}", setting.name)
			}))
			.collect::<String>();
		for input in kind.inputs {
			let InputDefault::Value(default) = input.default else {
				continue;
			};
			let graph = |extra: &str| {
				json::parse(&format!(
					r#"{{ nodes: [{{ name: "node", node: "{}"{required}{extra} }}, {{ name: "surface", node: "surface" }}] }}"#,
					kind.name
				))
			};
			let written = default.iter().map(f32::to_string).collect::<Vec<_>>().join(", ");
			let written = if default.len() == 1 { written } else { format!("[{written}]") };
			// A node whose types don't fit these stand-in inputs is checked by its own tests instead.
			let (Ok(omitted), Ok(explicit)) = (graph(""), graph(&format!(", {}: {written}", input.name))) else {
				continue;
			};
			assert_eq!(omitted, explicit, "{}'s {} default", kind.name, input.name);
			compared += 1;
		}
	}
	assert!(compared > 20, "only {compared} defaults were compared");
}
