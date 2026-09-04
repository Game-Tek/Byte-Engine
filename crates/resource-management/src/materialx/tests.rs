use bumpalo::Bump;

use super::{
	DataType, Document, Error, ParseError, ResolveError, Source, Value,
	dag::{Dag, GraphId},
};

/// Wraps a document body in the root element every test needs.
fn document(body: &str) -> String {
	format!("<?xml version=\"1.0\"?>\n<materialx version=\"1.39\">\n{body}\n</materialx>")
}

/// Reads a document body and hands the resolved graph to `check`.
///
/// The graph borrows its text from the source and its storage from the arena, so both are kept alive
/// around the check rather than returned.
fn with_dag(body: &str, check: impl FnOnce(&Dag<'_>)) {
	with_source(&document(body), check);
}

/// Reads a whole document, for the tests that need to write the root element themselves.
fn with_source(source: &str, check: impl FnOnce(&Dag<'_>)) {
	let arena = Bump::new();
	let allocator = &&arena;
	let dag = super::parse(source, allocator).expect("The document should resolve");

	check(&dag);
}

/// Reads a document body that should be rejected, handing the failure to `check`.
fn with_failure(body: &str, check: impl FnOnce(Error)) {
	failure(&document(body), check);
}

fn failure(source: &str, check: impl FnOnce(Error)) {
	let arena = Bump::new();
	let allocator = &&arena;

	check(super::parse(source, allocator).expect_err("The document should be rejected"));
}

#[test]
fn resolves_a_material_to_its_surface_shader() {
	with_dag(
		r#"
		<standard_surface name="gold" type="surfaceshader">
			<input name="base_color" type="color3" value="0.944, 0.776, 0.373"/>
			<input name="metalness" type="float" value="1"/>
		</standard_surface>
		<surfacematerial name="Mgold" type="material">
			<input name="surfaceshader" type="surfaceshader" nodename="gold"/>
		</surfacematerial>"#,
		|dag| {
			let material = dag.node(*dag.materials().first().expect("The material should be found"));

			assert_eq!(material.name, "Mgold");
			assert_eq!(material.category, "surfacematerial");

			let Some(Source::Node { node, .. }) = material.input("surfaceshader").map(|input| input.source.clone()) else {
				panic!("The material should connect to a shader node");
			};

			let shader = dag.node(node);

			assert_eq!(shader.name, "gold");
			assert_eq!(
				shader.input("base_color").map(|input| &input.source),
				Some(&Source::Value(Value::Color3([0.944, 0.776, 0.373])))
			);
		},
	);
}

#[test]
fn resolves_connections_into_a_nodegraph_output() {
	with_dag(
		r#"
		<nodegraph name="NG_brass">
			<tiledimage name="color" type="color3">
				<input name="file" type="filename" value="brass_color.jpg" colorspace="srgb_rec709_scene"/>
			</tiledimage>
			<tiledimage name="roughness" type="float">
				<input name="file" type="filename" value="brass_roughness.jpg"/>
			</tiledimage>
			<output name="out_color" type="color3" nodename="color"/>
			<output name="out_roughness" type="float" nodename="roughness"/>
		</nodegraph>
		<standard_surface name="SR_brass" type="surfaceshader">
			<input name="base_color" type="color3" nodegraph="NG_brass" output="out_color"/>
			<input name="specular_roughness" type="float" nodegraph="NG_brass" output="out_roughness"/>
		</standard_surface>"#,
		|dag| {
			let shader = dag
				.find(GraphId::ROOT, "SR_brass")
				.map(|id| dag.node(id))
				.expect("The shader should be found");

			let Some(Source::Graph { graph, output }) = shader.input("base_color").map(|input| input.source.clone()) else {
				panic!("The shader should read a nodegraph output");
			};

			let (node, _) = dag.trace(graph, output).expect("The output should trace to a node");

			assert_eq!(dag.node(node).name, "color");
			assert_eq!(
				dag.node(node).input("file").map(|input| input.colorspace),
				Some(Some("srgb_rec709_scene"))
			);
		},
	);
}

#[test]
fn resolves_interface_references_in_a_functional_nodegraph() {
	with_dag(
		r#"
		<nodedef name="ND_blendadd_color4" node="blend_add">
			<input name="fg" type="color4" value="0,0,0,0"/>
			<input name="bg" type="color4" value="0,0,0,0"/>
			<input name="amount" type="float" value="1.0"/>
			<output name="out" type="color4" defaultinput="bg"/>
		</nodedef>
		<nodegraph name="NG_blendadd_color4" nodedef="ND_blendadd_color4">
			<multiply name="n1" type="color4">
				<input name="in1" type="color4" interfacename="fg"/>
				<input name="in2" type="float" interfacename="amount"/>
			</multiply>
			<add name="n2" type="color4">
				<input name="in1" type="color4" nodename="n1"/>
				<input name="in2" type="color4" interfacename="bg"/>
			</add>
			<output name="out" type="color4" nodename="n2"/>
		</nodegraph>"#,
		|dag| {
			let id = dag.find_graph("NG_blendadd_color4").expect("The graph should be found");
			let graph = dag.graph(id);

			assert_eq!(graph.interface.len(), 3);
			assert_eq!(graph.interface[2].source, Source::Value(Value::Float(1.0)));

			let multiply = dag.find(id, "n1").expect("The multiply node should be found");

			let Some(Source::Interface { graph: owner, input }) =
				dag.node(multiply).input("in1").map(|input| input.source.clone())
			else {
				panic!("The node should read the graph interface");
			};

			assert_eq!(owner, id);
			assert_eq!(graph.interface[input.index()].name, "fg");
		},
	);
}

#[test]
fn selects_the_declaration_matching_an_instance_signature() {
	with_dag(
		r#"
		<nodedef name="ND_mix_float" node="mix">
			<input name="fg" type="float" value="0"/>
			<input name="bg" type="float" value="0"/>
			<output name="out" type="float"/>
		</nodedef>
		<nodedef name="ND_mix_color3" node="mix">
			<input name="fg" type="color3" value="0,0,0"/>
			<input name="bg" type="color3" value="0,0,0"/>
			<output name="out" type="color3"/>
		</nodedef>
		<mix name="m1" type="color3">
			<input name="fg" type="color3" value="1,0,0"/>
		</mix>"#,
		|dag| {
			let node = dag.find(GraphId::ROOT, "m1").map(|id| dag.node(id)).expect("m1 should exist");
			let declaration = node.declaration.map(|id| dag.declaration(id)).expect("m1 should be declared");

			assert_eq!(declaration.name, "ND_mix_color3");
			assert_eq!(node.outputs.len(), 1);
			assert_eq!(node.outputs[0].data_type, DataType::Color3);
		},
	);
}

#[test]
fn folds_inherited_declaration_ports_into_one_interface() {
	with_dag(
		r#"
		<nodedef name="ND_base" node="surf">
			<input name="base" type="float" value="1.0"/>
			<input name="roughness" type="float" value="0.3"/>
			<output name="out" type="surfaceshader"/>
		</nodedef>
		<nodedef name="ND_derived" node="surf" inherit="ND_base">
			<input name="roughness" type="float" value="0.1"/>
			<input name="coat" type="float" value="0.0"/>
			<output name="out" type="surfaceshader"/>
		</nodedef>
		<surf name="s1" type="surfaceshader" nodedef="ND_derived"/>"#,
		|dag| {
			let node = dag.find(GraphId::ROOT, "s1").map(|id| dag.node(id)).expect("s1 should exist");
			let declaration = node.declaration.map(|id| dag.declaration(id)).expect("s1 should be declared");

			let names: Vec<&str> = declaration.inputs.iter().map(|port| port.name).collect();

			assert_eq!(names, ["roughness", "coat", "base"]);
			// The nearest definition of an inherited input wins.
			assert_eq!(declaration.inputs[0].default, Some(Value::Float(0.1)));
		},
	);
}

#[test]
fn orders_nodes_so_dependencies_come_first() {
	with_dag(
		r#"
		<nodegraph name="NG">
			<constant name="a" type="float">
				<input name="value" type="float" value="1"/>
			</constant>
			<add name="b" type="float">
				<input name="in1" type="float" nodename="a"/>
				<input name="in2" type="float" value="2"/>
			</add>
			<multiply name="c" type="float">
				<input name="in1" type="float" nodename="b"/>
				<input name="in2" type="float" nodename="a"/>
			</multiply>
			<output name="out" type="float" nodename="c"/>
		</nodegraph>"#,
		|dag| {
			let order: Vec<&str> = dag.topological_order().iter().map(|id| dag.node(*id).name).collect();

			assert_eq!(order, ["a", "b", "c"]);
		},
	);
}

#[test]
fn selects_an_output_of_a_multi_output_node() {
	with_dag(
		r#"
		<nodedef name="ND_doublecolor" node="doublecolor">
			<input name="in1" type="color3" value="0,0,0"/>
			<output name="c1" type="color3"/>
			<output name="c2" type="color3"/>
		</nodedef>
		<nodegraph name="NG">
			<doublecolor name="dc1" type="multioutput"/>
			<add name="n2" type="color3">
				<input name="in1" type="color3" nodename="dc1" output="c2"/>
				<input name="in2" type="color3" value="0,0,0"/>
			</add>
			<output name="out" type="color3" nodename="n2"/>
		</nodegraph>"#,
		|dag| {
			let graph = dag.find_graph("NG").expect("NG should exist");
			let add = dag.find(graph, "n2").expect("n2 should exist");

			let Some(Source::Node { node, output }) = dag.node(add).input("in1").map(|input| input.source.clone()) else {
				panic!("n2 should read a node output");
			};

			assert_eq!(dag.node(node).outputs[output.index()].name, "c2");
		},
	);
}

#[test]
fn merges_an_included_document_before_resolving() {
	let arena = Bump::new();
	let allocator = &&arena;

	let library_source = document(
		r#"
		<nodedef name="ND_tint" node="tint">
			<input name="in" type="color3" value="0,0,0"/>
			<output name="out" type="color3"/>
		</nodedef>"#,
	);
	let main_source = document(
		r#"
		<xi:include href="library.mtlx"/>
		<tint name="t1" type="color3">
			<input name="in" type="color3" value="1,1,1"/>
		</tint>"#,
	);

	let library = Document::parse(&library_source, allocator).expect("The library should parse");
	let mut main = Document::parse(&main_source, allocator).expect("The document should parse");

	assert_eq!(main.includes.len(), 1);
	assert_eq!(main.includes[0].href, "library.mtlx");

	main.merge(library);

	let dag = Dag::resolve(&main, allocator).expect("The merged document should resolve");
	let node = dag.find(GraphId::ROOT, "t1").map(|id| dag.node(id)).expect("t1 should exist");

	assert_eq!(node.declaration.map(|id| dag.declaration(id).name), Some("ND_tint"));
}

#[test]
fn resolves_namespaced_references_from_a_qualified_name() {
	let arena = Bump::new();
	let allocator = &&arena;

	let library_source = format!(
		"<materialx version=\"1.39\" namespace=\"site_ops\">{}</materialx>",
		r#"<nodedef name="ND_mynoise" node="mynoise">
			<input name="f" type="float" value="0.3"/>
			<output name="out" type="color3"/>
		</nodedef>"#
	);
	let main_source = document(
		r#"<mynoise name="mn1" type="color3" nodedef="site_ops:ND_mynoise">
			<input name="f" type="float" value="0.5"/>
		</mynoise>"#,
	);

	let library = Document::parse(&library_source, allocator).expect("The library should parse");
	let mut main = Document::parse(&main_source, allocator).expect("The document should parse");

	main.merge(library);

	let dag = Dag::resolve(&main, allocator).expect("The merged document should resolve");
	let node = dag
		.find(GraphId::ROOT, "mn1")
		.map(|id| dag.node(id))
		.expect("mn1 should exist");
	let declaration = node
		.declaration
		.map(|id| dag.declaration(id))
		.expect("mn1 should be declared");

	assert_eq!(declaration.name, "ND_mynoise");
	assert_eq!(declaration.namespace, Some("site_ops"));
}

#[test]
fn merging_the_same_library_twice_changes_nothing() {
	let arena = Bump::new();
	let allocator = &&arena;

	let library_source = document(
		r#"<nodedef name="ND_tint" node="tint">
			<output name="out" type="color3"/>
		</nodedef>"#,
	);
	let main_source = document(r#"<tint name="t1" type="color3"/>"#);

	let mut main = Document::parse(&main_source, allocator).expect("The document should parse");

	main.merge(Document::parse(&library_source, allocator).expect("The library should parse"));
	main.merge(Document::parse(&library_source, allocator).expect("The library should parse"));

	assert_eq!(main.node_defs.len(), 1);
	assert!(Dag::resolve(&main, allocator).is_ok());
}

#[test]
fn keeps_a_non_numeric_output_default_as_written() {
	// The standard library writes geometric property names in `default`, which is only a hint for
	// readers that cannot evaluate the node.
	let arena = Bump::new();
	let allocator = &&arena;
	let source = document(
		r#"<nodedef name="ND_flake" node="flake">
			<output name="flakenormal" type="vector3" default="Nworld"/>
		</nodedef>"#,
	);

	let document = Document::parse(&source, allocator).expect("The document should parse");

	assert_eq!(document.node_defs[0].outputs[0].default_value, Some(Value::Opaque("Nworld")));
}

#[test]
fn keeps_nodes_without_a_declaration_usable() {
	with_dag(
		r#"<image name="i1" type="color3">
			<input name="file" type="filename" value="albedo.png"/>
		</image>"#,
		|dag| {
			let node = dag.find(GraphId::ROOT, "i1").map(|id| dag.node(id)).expect("i1 should exist");

			assert_eq!(node.declaration, None);
			// Without a declaration the instance type still says what the single output carries.
			assert_eq!(node.outputs.len(), 1);
			assert_eq!(node.outputs[0].data_type, DataType::Color3);
			assert_eq!(
				node.input("file").map(|input| &input.source),
				Some(&Source::Value(Value::Filename("albedo.png")))
			);
		},
	);
}

#[test]
fn applies_the_enclosing_file_prefix_and_color_space() {
	with_source(
		r#"<materialx version="1.39" colorspace="lin_rec709_scene" fileprefix="textures/">
			<nodegraph name="NG" fileprefix="textures/brass/">
				<image name="i1" type="color3">
					<input name="file" type="filename" value="color.png"/>
				</image>
				<output name="out" type="color3" nodename="i1"/>
			</nodegraph>
		</materialx>"#,
		|dag| {
			let node = dag
				.find_graph("NG")
				.and_then(|graph| dag.find(graph, "i1"))
				.map(|id| dag.node(id))
				.expect("The image node should exist");

			assert_eq!(node.file_prefix, Some("textures/brass/"));
			assert_eq!(
				node.input("file").map(|input| input.colorspace),
				Some(Some("lin_rec709_scene"))
			);
		},
	);
}

#[test]
fn links_a_functional_nodegraph_through_an_implementation() {
	// The standard library links most nodegraphs to their declaration this way rather than with a
	// `nodedef` attribute on the graph.
	with_dag(
		r#"
		<nodedef name="ND_tint" node="tint">
			<input name="in" type="color3" value="0,0,0"/>
			<input name="amount" type="float" value="1.0"/>
			<output name="out" type="color3"/>
		</nodedef>
		<implementation name="IM_tint" nodedef="ND_tint" nodegraph="NG_tint"/>
		<nodegraph name="NG_tint">
			<multiply name="n1" type="color3">
				<input name="in1" type="color3" interfacename="in"/>
				<input name="in2" type="float" interfacename="amount"/>
			</multiply>
			<output name="out" type="color3" nodename="n1"/>
		</nodegraph>"#,
		|dag| {
			let id = dag.find_graph("NG_tint").expect("The graph should be found");

			assert_eq!(dag.graph(id).declaration.map(|id| dag.declaration(id).name), Some("ND_tint"));
			assert_eq!(dag.graph(id).interface.len(), 2);
		},
	);
}

#[test]
fn resolves_an_interface_declared_at_document_scope() {
	with_dag(
		r#"
		<input name="input_color3" type="color3" value="0, 0, 1"/>
		<surface_unlit name="surface_unlit" type="surfaceshader">
			<input name="emission_color" type="color3" interfacename="input_color3"/>
		</surface_unlit>"#,
		|dag| {
			let node = dag
				.find(GraphId::ROOT, "surface_unlit")
				.map(|id| dag.node(id))
				.expect("The shader should exist");

			let Some(Source::Interface { graph, input }) = node.input("emission_color").map(|input| input.source.clone())
			else {
				panic!("The shader should read the document interface");
			};

			assert_eq!(graph, GraphId::ROOT);
			assert_eq!(
				dag.root().interface[input.index()].source,
				Source::Value(Value::Color3([0.0, 0.0, 1.0]))
			);
		},
	);
}

#[test]
fn reads_values_separated_by_whitespace_alone() {
	// The specification writes components with commas, but documents in the wild use spaces.
	with_dag(
		r#"<constant name="c" type="color3">
			<input name="value" type="color3" value="0.944 0.776 0.373"/>
		</constant>"#,
		|dag| {
			let node = dag.find(GraphId::ROOT, "c").map(|id| dag.node(id)).expect("c should exist");

			assert_eq!(
				node.input("value").map(|input| &input.source),
				Some(&Source::Value(Value::Color3([0.944, 0.776, 0.373])))
			);
		},
	);
}

#[test]
fn follows_a_nodename_that_reaches_a_nodegraph() {
	with_dag(
		r#"
		<nodegraph name="NG">
			<constant name="a" type="float">
				<input name="value" type="float" value="1"/>
			</constant>
			<output name="out1" type="float" nodename="a"/>
			<output name="out2" type="float" nodename="a"/>
		</nodegraph>
		<output name="top" type="float" nodename="NG" output="out2"/>"#,
		|dag| {
			let index = dag.root().output("top").expect("The document output should exist");

			let Source::Graph { graph, output } = dag.root().outputs[index.index()].source.clone() else {
				panic!("The document output should read a nodegraph output");
			};

			assert_eq!(dag.graph(graph).outputs[output.index()].name, "out2");
		},
	);
}

#[test]
fn skips_looks_and_interface_layout_elements() {
	with_dag(
		r#"
		<backdrop name="b1" contains="a" width="4" height="2"/>
		<constant name="a" type="float">
			<input name="value" type="float" value="1"/>
		</constant>
		<look name="hero">
			<materialassign name="m1" material="Mgold" geom="/a/b"/>
		</look>"#,
		|dag| {
			assert_eq!(dag.nodes().len(), 1);
			assert_eq!(dag.nodes()[0].name, "a");
		},
	);
}

#[test]
fn reads_an_unplugged_shader_input_as_unconnected() {
	with_dag(
		r#"<surfacematerial name="M" type="material">
			<input name="surfaceshader" type="surfaceshader" value=""/>
		</surfacematerial>"#,
		|dag| {
			let node = dag.node(dag.materials()[0]);

			assert_eq!(
				node.input("surfaceshader").map(|input| &input.source),
				Some(&Source::Unconnected)
			);
		},
	);
}

#[test]
fn rejects_a_cycle_between_nodes() {
	with_failure(
		r#"
		<nodegraph name="NG">
			<add name="a" type="float">
				<input name="in1" type="float" nodename="b"/>
			</add>
			<add name="b" type="float">
				<input name="in1" type="float" nodename="a"/>
			</add>
			<output name="out" type="float" nodename="a"/>
		</nodegraph>"#,
		|error| assert!(matches!(error, Error::Resolve(ResolveError::Cycle { .. }))),
	);
}

#[test]
fn rejects_a_connection_between_mismatched_types() {
	with_failure(
		r#"
		<nodegraph name="NG">
			<constant name="a" type="float">
				<input name="value" type="float" value="1"/>
			</constant>
			<add name="b" type="color3">
				<input name="in1" type="color3" nodename="a"/>
			</add>
			<output name="out" type="color3" nodename="b"/>
		</nodegraph>"#,
		|error| assert!(matches!(error, Error::Resolve(ResolveError::TypeMismatch { .. }))),
	);
}

#[test]
fn rejects_a_reference_to_a_node_outside_its_scope() {
	with_failure(
		r#"
		<constant name="outer" type="float">
			<input name="value" type="float" value="1"/>
		</constant>
		<nodegraph name="NG">
			<add name="a" type="float">
				<input name="in1" type="float" nodename="outer"/>
			</add>
			<output name="out" type="float" nodename="a"/>
		</nodegraph>"#,
		|error| assert!(matches!(error, Error::Resolve(ResolveError::UnknownNode { .. }))),
	);
}

#[test]
fn rejects_documents_older_than_the_supported_schema() {
	failure(
		r#"<materialx version="1.37"><image name="i" type="color3"/></materialx>"#,
		|error| assert!(matches!(error, Error::Parse(ParseError::UnsupportedVersion { .. }))),
	);
}

#[test]
fn rejects_elements_the_specification_removed() {
	with_failure(
		r#"<image name="i1" type="color3">
			<parameter name="file" type="filename" value="albedo.png"/>
		</image>"#,
		|error| assert!(matches!(error, Error::Parse(ParseError::RemovedElement { .. }))),
	);
}

#[test]
fn rejects_inputs_carrying_more_than_one_upstream_reference() {
	with_failure(
		r#"
		<nodegraph name="NG">
			<constant name="a" type="float">
				<input name="value" type="float" value="1"/>
			</constant>
			<add name="b" type="float">
				<input name="in1" type="float" nodename="a" nodegraph="NG"/>
			</add>
			<output name="out" type="float" nodename="b"/>
		</nodegraph>"#,
		|error| assert!(matches!(error, Error::Parse(ParseError::ConflictingConnection { .. }))),
	);
}

#[test]
fn rejects_two_nodes_sharing_a_name_in_one_scope() {
	with_failure(
		r#"
		<constant name="a" type="float"/>
		<constant name="a" type="float"/>"#,
		|error| assert!(matches!(error, Error::Resolve(ResolveError::DuplicateName { .. }))),
	);
}

#[test]
fn rejects_a_node_instance_naming_a_declaration_the_document_does_not_carry() {
	with_failure(r#"<tint name="t1" type="color3" nodedef="ND_tint"/>"#, |error| {
		assert!(matches!(error, Error::Resolve(ResolveError::UnknownDeclaration { .. })));
	});
}

#[test]
fn reports_where_malformed_values_are() {
	failure(
		"<materialx version=\"1.39\">\n\t<constant name=\"a\" type=\"color3\">\n\t\t<input name=\"value\" type=\"color3\" value=\"1,2\"/>\n\t</constant>\n</materialx>",
		|error| {
			let Error::Parse(error) = error else {
				panic!("The failure should come from parsing");
			};

			assert_eq!(error.position().line, 3);
		},
	);
}
