//! Resolve a parsed MaterialX document into the directed acyclic graph it describes.
//!
//! Parsing leaves every reference as a name. This layer looks those names up, works out which
//! declaration each node instance uses, checks that connected ports agree on their type, and proves
//! the result really is acyclic. What comes out is an arena of nodes joined by [`Source`] edges,
//! which is the form a shader generator wants.
//!
//! Resolution runs in two passes over the document. The first declares every scope, node and port,
//! which is everything that can be read without following a reference. The second turns each
//! authored reference into an edge, now that every name it could point at exists.
//!
//! Like the document it comes from, the graph borrows every name from the source text and draws
//! every collection from the caller's allocator.
//!
//! Next, walk the graph from [`Dag::materials`] in [`Dag::topological_order`].

use std::{collections::HashMap, hash::RandomState};

use super::{
	Alloc,
	document::{self, Document, Named, merge_by_name},
	error::ResolveError,
	types::{DataType, TypeSemantic, Value},
};

/// A map drawing its storage from the same allocator as the graph it belongs to.
type Map<'a, K, V> = HashMap<K, V, RandomState, Alloc<'a>>;

/// A namespace and a name, which is how MaterialX addresses an element across files.
///
/// Keeping the two apart lets a reference be resolved without joining them into a new string.
type Key<'a> = (Option<&'a str>, &'a str);

/// Defines one of the arena indices a [`Dag`] addresses its elements with.
macro_rules! index {
	($($(#[$documentation:meta])* $name:ident),* $(,)?) => {
		$(
			$(#[$documentation])*
			#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
			pub struct $name(u32);

			impl $name {
				fn new(index: usize) -> Self {
					$name(index as u32)
				}

				/// Returns this index as a position in the list it addresses.
				pub fn index(self) -> usize {
					self.0 as usize
				}
			}
		)*
	};
}

index!(
	/// The `NodeId` struct identifies one node inside a [`Dag`].
	NodeId,
	/// The `GraphId` struct identifies one node scope inside a [`Dag`]: the document root, or a nodegraph.
	GraphId,
	/// The `DeclarationId` struct identifies one node declaration inside a [`Dag`].
	DeclarationId,
	/// The `PortIndex` struct identifies one port in the list it was found on.
	PortIndex,
);

impl GraphId {
	/// The scope holding the nodes written directly inside `<materialx>`.
	pub const ROOT: GraphId = GraphId(0);
}

/// The `Port` struct describes one declared connection point, with whatever the declaration says it
/// falls back to.
#[derive(Clone, Debug, PartialEq)]
pub struct Port<'a> {
	pub name: &'a str,
	pub data_type: DataType<'a>,
	/// The constant used when nothing drives this port.
	pub default: Option<Value<'a>>,
	/// On an output, the input passed straight through by readers that cannot evaluate the node.
	pub default_input: Option<&'a str>,
	/// Whether the port only carries values that do not vary across a surface.
	pub uniform: bool,
	/// On an input, the geometric property read when nothing else drives it.
	pub default_geom_prop: Option<&'a str>,
}

impl<'a> Port<'a> {
	fn from_input(input: &document::Input<'a>) -> Self {
		Port {
			name: input.name,
			data_type: input.data_type,
			default: input.value.clone(),
			default_input: None,
			uniform: input.uniform,
			default_geom_prop: input.default_geom_prop,
		}
	}

	fn from_output(output: &document::Output<'a>) -> Self {
		Port {
			name: output.name,
			data_type: output.data_type,
			default: output.default_value.clone(),
			default_input: output.default_input,
			uniform: output.uniform,
			default_geom_prop: None,
		}
	}
}

impl Named for Port<'_> {
	fn name(&self) -> &str {
		self.name
	}
}

/// The `Source` enum records what drives one input: a constant, an edge to another node, or nothing.
///
/// [`Source::Node`] and [`Source::Graph`] are the graph's edges; everything else terminates a branch.
#[derive(Clone, Debug, PartialEq)]
pub enum Source<'a> {
	/// A constant, either written on the input or taken from the node's declaration.
	Value(Value<'a>),
	/// One output of another node in the same scope.
	Node { node: NodeId, output: PortIndex },
	/// One output of a nodegraph, which stands for whatever that output is connected to.
	Graph { graph: GraphId, output: PortIndex },
	/// One input of the enclosing graph's interface, which the graph's caller supplies.
	Interface { graph: GraphId, input: PortIndex },
	/// A geometric property of the surface being shaded, named by `defaultgeomprop`.
	GeomProp(&'a str),
	/// Nothing drives this input, which is how MaterialX spells an unplugged shader or closure input.
	Unconnected,
}

/// The `Input` struct holds one resolved port: its type, and what drives it.
///
/// Node inputs, graph interfaces, graph outputs and filename substitution tokens are all ports in
/// this sense, so they all arrive as an `Input`.
#[derive(Clone, Debug, PartialEq)]
pub struct Input<'a> {
	pub name: &'a str,
	pub data_type: DataType<'a>,
	pub source: Source<'a>,
	/// The channel selection of pre-1.39 documents; 1.39 replaces it with explicit channel nodes.
	pub channels: Option<&'a str>,
	/// The color space this input's value is written in, taken from the nearest enclosing scope.
	pub colorspace: Option<&'a str>,
	/// The path prepended to this input's value when it names a file, taken from the nearest enclosing scope.
	pub file_prefix: Option<&'a str>,
	pub unit: Option<&'a str>,
	pub unit_type: Option<&'a str>,
}

impl<'a> Input<'a> {
	/// Declares one port with nothing driving it yet.
	fn declared(name: &'a str, data_type: DataType<'a>) -> Self {
		Input {
			name,
			data_type,
			source: Source::Unconnected,
			channels: None,
			colorspace: None,
			file_prefix: None,
			unit: None,
			unit_type: None,
		}
	}
}

impl Named for Input<'_> {
	fn name(&self) -> &str {
		self.name
	}
}

/// The `Declaration` struct holds one node declaration with its inherited ports already folded in.
///
/// Look one up with [`Dag::declaration`] to find the defaults and port names of a node whose inputs
/// the document left unwritten.
#[derive(Clone, Debug, PartialEq)]
pub struct Declaration<'a> {
	pub name: &'a str,
	/// The namespace qualifying this declaration's name, when it carries one.
	pub namespace: Option<&'a str>,
	/// The node category this declaration defines.
	pub category: &'a str,
	pub version: Option<&'a str>,
	pub is_default_version: bool,
	pub targets: Vec<&'a str, Alloc<'a>>,
	pub node_group: Option<&'a str>,
	pub inputs: Vec<Port<'a>, Alloc<'a>>,
	/// The filename substitution ports this declaration accepts.
	pub tokens: Vec<Port<'a>, Alloc<'a>>,
	pub outputs: Vec<Port<'a>, Alloc<'a>>,
}

/// The `Node` struct holds one resolved node instance: a graph vertex and the edges into it.
#[derive(Clone, Debug, PartialEq)]
pub struct Node<'a> {
	/// The node category, such as `image`, `multiply` or `standard_surface`.
	pub category: &'a str,
	pub name: &'a str,
	/// The scope this node was written in.
	pub graph: GraphId,
	/// The type this instance produces, or [`DataType::MultiOutput`] when it produces several.
	pub data_type: DataType<'a>,
	/// The declaration this instance uses, when the document carries one for it.
	pub declaration: Option<DeclarationId>,
	/// The inputs the document wrote, in source order; unwritten inputs live on the declaration.
	pub inputs: Vec<Input<'a>, Alloc<'a>>,
	pub outputs: Vec<Port<'a>, Alloc<'a>>,
	/// The filename substitution tokens the document wrote on this instance.
	pub tokens: Vec<Input<'a>, Alloc<'a>>,
	/// The color space in force for this node, taken from the nearest enclosing scope.
	pub colorspace: Option<&'a str>,
	/// The file prefix in force for this node, taken from the nearest enclosing scope.
	pub file_prefix: Option<&'a str>,
}

impl<'a> Node<'a> {
	/// Returns one input by name, or `None` when the document did not write it.
	pub fn input(&self, name: &str) -> Option<&Input<'a>> {
		self.inputs.iter().find(|input| input.name == name)
	}

	/// Returns the index of one output by name.
	pub fn output(&self, name: &str) -> Option<PortIndex> {
		find_port(&self.outputs, name)
	}
}

/// The `Graph` struct holds one node scope: the document root, or a `<nodegraph>`.
///
/// A graph that names a `declaration` implements it and takes its interface from it; a graph that
/// declares its own interface wraps the nodes it contains.
#[derive(Clone, Debug, PartialEq)]
pub struct Graph<'a> {
	/// The graph's name; the root scope is unnamed.
	pub name: &'a str,
	/// The namespace qualifying this graph's name, when it carries one.
	pub namespace: Option<&'a str>,
	pub parent: Option<GraphId>,
	/// The declaration this graph implements, when it is a functional nodegraph.
	pub declaration: Option<DeclarationId>,
	/// The ports the graph presents to its caller, and what feeds each of them.
	pub interface: Vec<Input<'a>, Alloc<'a>>,
	/// The filename substitution tokens the graph presents to its caller.
	pub interface_tokens: Vec<Input<'a>, Alloc<'a>>,
	/// The results this graph publishes.
	pub outputs: Vec<Input<'a>, Alloc<'a>>,
	pub nodes: Vec<NodeId, Alloc<'a>>,
	pub children: Vec<GraphId, Alloc<'a>>,
	pub colorspace: Option<&'a str>,
	pub file_prefix: Option<&'a str>,
}

impl Graph<'_> {
	/// Returns the index of one published output by name.
	pub fn output(&self, name: &str) -> Option<PortIndex> {
		find_port(&self.outputs, name)
	}
}

/// The `Dag` struct holds a MaterialX document as one resolved directed acyclic graph.
///
/// Build it with [`Dag::resolve`], find the entry points with [`Dag::materials`], and walk the whole
/// graph in dependency order with [`Dag::topological_order`].
#[derive(Clone, Debug, PartialEq)]
pub struct Dag<'a> {
	nodes: Vec<Node<'a>, Alloc<'a>>,
	graphs: Vec<Graph<'a>, Alloc<'a>>,
	declarations: Vec<Declaration<'a>, Alloc<'a>>,
	materials: Vec<NodeId, Alloc<'a>>,
	order: Vec<NodeId, Alloc<'a>>,
}

impl<'a> Dag<'a> {
	/// Resolves a parsed document into its graph, drawing every collection from `allocator`.
	///
	/// Fold in every `<xi:include>` with [`Document::merge`] first, otherwise nodes whose
	/// declarations live in an included library resolve without one.
	pub fn resolve(document: &Document<'a>, allocator: Alloc<'a>) -> Result<Self, ResolveError> {
		Resolver::new(document, allocator).run()
	}

	/// Returns every node in the document, across all scopes.
	pub fn nodes(&self) -> &[Node<'a>] {
		&self.nodes
	}

	pub fn node(&self, id: NodeId) -> &Node<'a> {
		&self.nodes[id.index()]
	}

	/// Returns every scope, starting with [`GraphId::ROOT`].
	pub fn graphs(&self) -> &[Graph<'a>] {
		&self.graphs
	}

	pub fn graph(&self, id: GraphId) -> &Graph<'a> {
		&self.graphs[id.index()]
	}

	/// Returns the scope holding the nodes written directly inside `<materialx>`.
	pub fn root(&self) -> &Graph<'a> {
		&self.graphs[GraphId::ROOT.index()]
	}

	/// Returns every node declaration the document carries, with inheritance already folded in.
	pub fn declarations(&self) -> &[Declaration<'a>] {
		&self.declarations
	}

	pub fn declaration(&self, id: DeclarationId) -> &Declaration<'a> {
		&self.declarations[id.index()]
	}

	/// Returns the material nodes, which are the graph's entry points.
	pub fn materials(&self) -> &[NodeId] {
		&self.materials
	}

	/// Returns every node in dependency order, so a node always appears after the nodes it reads.
	///
	/// This is the order a shader generator emits statements in.
	pub fn topological_order(&self) -> &[NodeId] {
		&self.order
	}

	/// Returns one scope by name, which may be written with or without its namespace.
	pub fn find_graph(&self, name: &str) -> Option<GraphId> {
		let (namespace, name) = split_name(name);

		self.graphs
			.iter()
			.position(|graph| graph.name == name && (namespace.is_none() || graph.namespace == namespace))
			.map(GraphId::new)
	}

	/// Returns one node by name within a scope.
	pub fn find(&self, graph: GraphId, name: &str) -> Option<NodeId> {
		self.graphs[graph.index()]
			.nodes
			.iter()
			.copied()
			.find(|id| self.nodes[id.index()].name == name)
	}

	/// Follows a graph output to the node that actually produces it.
	///
	/// Use this to step across a nodegraph boundary when a consumer wants a flat view of the graph.
	/// Returns `None` when the output is driven by a constant or by the graph's own interface.
	pub fn trace(&self, graph: GraphId, output: PortIndex) -> Option<(NodeId, PortIndex)> {
		let mut source = &self.graphs[graph.index()].outputs.get(output.index())?.source;

		// A graph output may name another graph's output, so follow the chain; the graph count bounds it.
		for _ in 0..=self.graphs.len() {
			match source {
				Source::Node { node, output } => return Some((*node, *output)),
				Source::Graph { graph, output } => {
					source = &self.graphs[graph.index()].outputs.get(output.index())?.source;
				}
				_ => return None,
			}
		}

		None
	}

	/// Collects, for every node, the nodes it reads from, following nodegraph outputs across scopes.
	fn successors(&self, allocator: Alloc<'a>) -> Successors<'a> {
		let mut offsets = Vec::with_capacity_in(self.nodes.len() + 1, allocator);
		let mut edges = Vec::new_in(allocator);

		offsets.push(0);

		for node in &self.nodes {
			for input in &node.inputs {
				match &input.source {
					Source::Node { node, .. } => edges.push(*node),
					Source::Graph { graph, output } => {
						if let Some((node, _)) = self.trace(*graph, *output) {
							edges.push(node);
						}
					}
					_ => {}
				}
			}

			offsets.push(edges.len() as u32);
		}

		Successors { offsets, edges }
	}

	/// Walks every node depth first, returning them in dependency order or naming a cycle.
	///
	/// A depth-first post-order finishes every dependency before the node that reads it, and a node
	/// reached while it is still on the walk's own path is exactly what closes a cycle.
	fn order_nodes(&self, allocator: Alloc<'a>) -> Result<Vec<NodeId, Alloc<'a>>, Vec<String>> {
		#[derive(Clone, Copy, PartialEq)]
		enum Mark {
			Unseen,
			OnPath,
			Done,
		}

		let successors = self.successors(allocator);

		let mut marks = Vec::with_capacity_in(self.nodes.len(), allocator);
		let mut order = Vec::with_capacity_in(self.nodes.len(), allocator);
		let mut stack: Vec<(usize, usize), Alloc<'a>> = Vec::new_in(allocator);

		marks.resize(self.nodes.len(), Mark::Unseen);

		for start in 0..self.nodes.len() {
			if marks[start] != Mark::Unseen {
				continue;
			}

			marks[start] = Mark::OnPath;
			stack.push((start, 0));

			while let Some((node, cursor)) = stack.pop() {
				let Some(next) = successors.of(node).get(cursor).copied() else {
					marks[node] = Mark::Done;
					order.push(NodeId::new(node));

					continue;
				};

				stack.push((node, cursor + 1));

				match marks[next.index()] {
					Mark::Unseen => {
						marks[next.index()] = Mark::OnPath;
						stack.push((next.index(), 0));
					}
					Mark::OnPath => {
						let entry = stack.iter().position(|(node, _)| *node == next.index()).unwrap_or(0);

						// A cycle ends resolution, so naming its nodes is worth the owned strings.
						let mut cycle: Vec<String> = stack[entry..]
							.iter()
							.map(|(node, _)| self.nodes[*node].name.to_string())
							.collect();

						cycle.push(self.nodes[next.index()].name.to_string());

						return Err(cycle);
					}
					Mark::Done => {}
				}
			}
		}

		Ok(order)
	}
}

/// Holds, for every node, the nodes it reads from, as one flat run per node.
struct Successors<'a> {
	offsets: Vec<u32, Alloc<'a>>,
	edges: Vec<NodeId, Alloc<'a>>,
}

impl Successors<'_> {
	fn of(&self, node: usize) -> &[NodeId] {
		&self.edges[self.offsets[node] as usize..self.offsets[node + 1] as usize]
	}
}

/// Names the port a resolved edge is written back to.
#[derive(Clone, Copy)]
enum Target {
	NodeInput(NodeId, usize),
	NodeToken(NodeId, usize),
	GraphInterface(GraphId, usize),
	GraphToken(GraphId, usize),
	GraphOutput(GraphId, usize),
}

/// Holds one authored reference until every name it could point at exists.
struct Link<'a> {
	/// The scope the reference is looked up in, which for a graph's interface is its parent.
	scope: GraphId,
	namespace: Option<&'a str>,
	target: Target,
	connection: document::Connection<'a>,
}

struct Resolver<'r, 'a> {
	document: &'r Document<'a>,
	allocator: Alloc<'a>,
	declarations: Vec<Declaration<'a>, Alloc<'a>>,
	declaration_by_name: Map<'a, Key<'a>, DeclarationId>,
	declarations_by_category: Map<'a, &'a str, Vec<DeclarationId, Alloc<'a>>>,
	/// The declarations each nodegraph implements, keyed by the name an implementation names it with.
	implementations_by_graph: Map<'a, &'a str, Vec<&'a str, Alloc<'a>>>,
	graphs: Vec<Graph<'a>, Alloc<'a>>,
	graph_by_name: Map<'a, Key<'a>, GraphId>,
	nodes: Vec<Node<'a>, Alloc<'a>>,
	/// The nodes of each scope, by the name the document wrote.
	scopes: Vec<Map<'a, &'a str, NodeId>, Alloc<'a>>,
	links: Vec<Link<'a>, Alloc<'a>>,
}

impl<'r, 'a> Resolver<'r, 'a> {
	fn new(document: &'r Document<'a>, allocator: Alloc<'a>) -> Self {
		Resolver {
			document,
			allocator,
			declarations: Vec::new_in(allocator),
			declaration_by_name: HashMap::with_hasher_in(RandomState::new(), allocator),
			declarations_by_category: HashMap::with_hasher_in(RandomState::new(), allocator),
			implementations_by_graph: HashMap::with_hasher_in(RandomState::new(), allocator),
			graphs: Vec::new_in(allocator),
			graph_by_name: HashMap::with_hasher_in(RandomState::new(), allocator),
			nodes: Vec::new_in(allocator),
			scopes: Vec::new_in(allocator),
			links: Vec::new_in(allocator),
		}
	}

	fn run(mut self) -> Result<Dag<'a>, ResolveError> {
		self.declare_declarations()?;
		self.index_implementations();
		self.declare_graphs()?;
		self.connect()?;

		let mut materials = Vec::new_in(self.allocator);

		for (index, node) in self.nodes.iter().enumerate() {
			if self.document.semantic(&node.data_type) == TypeSemantic::Material {
				materials.push(NodeId::new(index));
			}
		}

		let mut dag = Dag {
			nodes: self.nodes,
			graphs: self.graphs,
			declarations: self.declarations,
			materials,
			order: Vec::new_in(self.allocator),
		};

		dag.order = dag
			.order_nodes(self.allocator)
			.map_err(|nodes| ResolveError::Cycle { nodes })?;

		Ok(dag)
	}

	/// Turns every `<nodedef>` into a declaration, folding each one's inherited ports into it.
	fn declare_declarations(&mut self) -> Result<(), ResolveError> {
		for (index, node_def) in self.document.node_defs.iter().enumerate() {
			let chain = inheritance_chain(&self.document.node_defs, index, self.allocator, |node_def| node_def.inherit)?;

			let mut inputs = Vec::new_in(self.allocator);
			let mut tokens = Vec::new_in(self.allocator);
			let mut outputs = Vec::new_in(self.allocator);

			// The chain runs from the declaration outwards, so the nearest definition of a name wins.
			for link in chain {
				let link = &self.document.node_defs[link];

				merge_by_name(&mut inputs, link.inputs.iter().map(Port::from_input));
				merge_by_name(&mut tokens, link.tokens.iter().map(Port::from_input));
				merge_by_name(&mut outputs, link.outputs.iter().map(Port::from_output));
			}

			let id = DeclarationId::new(self.declarations.len());

			if self
				.declaration_by_name
				.insert((node_def.namespace, node_def.name), id)
				.is_some()
			{
				return Err(ResolveError::DuplicateName {
					scope: "nodedef".to_string(),
					name: qualify(node_def.namespace, node_def.name),
				});
			}

			self.declarations_by_category
				.entry(node_def.node)
				.or_insert_with(|| Vec::new_in(self.allocator))
				.push(id);

			let mut targets = Vec::new_in(self.allocator);

			targets.extend_from_slice(&node_def.targets);

			self.declarations.push(Declaration {
				name: node_def.name,
				namespace: node_def.namespace,
				category: node_def.node,
				version: node_def.version,
				is_default_version: node_def.is_default_version,
				targets,
				node_group: node_def.node_group,
				inputs,
				tokens,
				outputs,
			});
		}

		Ok(())
	}

	/// Indexes the implementations that give a nodegraph its declaration, which is the second way
	/// the specification allows a functional nodegraph to be linked to the node it implements.
	fn index_implementations(&mut self) {
		for implementation in &self.document.implementations {
			if let Some(graph) = implementation.node_graph {
				self.implementations_by_graph
					.entry(graph)
					.or_insert_with(|| Vec::new_in(self.allocator))
					.push(implementation.node_def);
			}
		}
	}

	/// Creates every scope, node and port, recording each authored reference for the second pass.
	fn declare_graphs(&mut self) -> Result<(), ResolveError> {
		let namespace = self.document.namespace;

		let root = self.push_graph("", None, None, None, self.document.colorspace, self.document.file_prefix);

		// Ports written at document scope belong to the root graph, so nodes in that scope reference
		// them exactly as they would inside a nodegraph, and they are connected in that same scope.
		self.declare_interface(root, root, namespace, &self.document.inputs, &self.document.tokens);
		self.declare_outputs(root, namespace, &self.document.outputs);
		self.declare_scope(root, namespace, &self.document.nodes)?;

		for graph in &self.document.node_graphs {
			self.declare_node_graph(graph, root, namespace)?;
		}

		Ok(())
	}

	fn declare_node_graph(
		&mut self,
		source: &'r document::NodeGraph<'a>,
		parent: GraphId,
		namespace: Option<&'a str>,
	) -> Result<GraphId, ResolveError> {
		let namespace = source.namespace.or(namespace);

		let parent_graph = &self.graphs[parent.index()];
		let colorspace = source.colorspace.or(parent_graph.colorspace);
		let file_prefix = source.file_prefix.or(parent_graph.file_prefix);

		// A functional nodegraph takes its interface from the declarations it implements. One graph
		// can implement several versions of a node, so the interface is everything they declare.
		let declarations = self.graph_declarations(source, namespace)?;

		let id = self.push_graph(
			source.name,
			namespace,
			Some(parent),
			declarations.first().copied(),
			colorspace,
			file_prefix,
		);

		if self.graph_by_name.insert((namespace, source.name), id).is_some() {
			return Err(ResolveError::DuplicateName {
				scope: "nodegraph".to_string(),
				name: qualify(namespace, source.name),
			});
		}

		self.graphs[parent.index()].children.push(id);

		if declarations.is_empty() {
			// A compound graph declares its own interface, fed from the scope that encloses it.
			self.declare_interface(id, parent, namespace, &source.inputs, &source.tokens);
		} else {
			let mut interface = Vec::new_in(self.allocator);
			let mut interface_tokens = Vec::new_in(self.allocator);

			for declaration in declarations {
				let declaration = &self.declarations[declaration.index()];

				merge_by_name(&mut interface, declaration.inputs.iter().map(declare_port));
				merge_by_name(&mut interface_tokens, declaration.tokens.iter().map(declare_port));
			}

			self.graphs[id.index()].interface = interface;
			self.graphs[id.index()].interface_tokens = interface_tokens;
		}

		self.declare_outputs(id, namespace, &source.outputs);
		self.declare_scope(id, namespace, &source.nodes)?;

		for child in &source.node_graphs {
			self.declare_node_graph(child, id, namespace)?;
		}

		Ok(id)
	}

	/// Declares the ports a graph presents to its caller, resolving their references in `scope`.
	///
	/// Interface ports are written inside the graph but connected from outside it, so the two scopes
	/// differ for every graph but the root.
	fn declare_interface(
		&mut self,
		graph: GraphId,
		scope: GraphId,
		namespace: Option<&'a str>,
		inputs: &'r [document::Input<'a>],
		tokens: &'r [document::Input<'a>],
	) {
		// An interface input is written inside the graph, so the graph's own color space and file
		// prefix apply to it. Tokens are substitution strings, which neither applies to.
		let colorspace = self.graphs[graph.index()].colorspace;
		let file_prefix = self.graphs[graph.index()].file_prefix;

		self.graphs[graph.index()].interface = self.declare_ports(inputs, colorspace, file_prefix);
		self.graphs[graph.index()].interface_tokens = self.declare_ports(tokens, None, None);

		for (index, input) in inputs.iter().enumerate() {
			self.link(scope, namespace, Target::GraphInterface(graph, index), input.connection);
		}

		for (index, token) in tokens.iter().enumerate() {
			self.link(scope, namespace, Target::GraphToken(graph, index), token.connection);
		}
	}

	fn declare_ports(
		&self,
		inputs: &'r [document::Input<'a>],
		colorspace: Option<&'a str>,
		file_prefix: Option<&'a str>,
	) -> Vec<Input<'a>, Alloc<'a>> {
		let mut ports = Vec::with_capacity_in(inputs.len(), self.allocator);

		ports.extend(
			inputs
				.iter()
				.map(|input| declare_input(input, colorspace, file_prefix, self.allocator)),
		);

		ports
	}

	fn declare_outputs(&mut self, graph: GraphId, namespace: Option<&'a str>, outputs: &'r [document::Output<'a>]) {
		let mut ports = Vec::with_capacity_in(outputs.len(), self.allocator);

		ports.extend(outputs.iter().map(|output| declare_output(output, self.allocator)));

		self.graphs[graph.index()].outputs = ports;

		for (index, output) in outputs.iter().enumerate() {
			self.link(graph, namespace, Target::GraphOutput(graph, index), output.connection);
		}
	}

	/// Creates the nodes of one scope, flattening each node's `inherit` chain as it goes.
	fn declare_scope(
		&mut self,
		graph: GraphId,
		namespace: Option<&'a str>,
		nodes: &'r [document::Node<'a>],
	) -> Result<(), ResolveError> {
		let colorspace = self.graphs[graph.index()].colorspace;
		let file_prefix = self.graphs[graph.index()].file_prefix;

		for (index, node) in nodes.iter().enumerate() {
			let chain = inheritance_chain(nodes, index, self.allocator, |node| node.inherit)?;

			let mut inputs = Vec::new_in(self.allocator);
			let mut tokens = Vec::new_in(self.allocator);
			let mut outputs = Vec::new_in(self.allocator);

			for link in chain {
				let link = &nodes[link];

				merge_by_name(&mut inputs, link.inputs.iter());
				merge_by_name(&mut tokens, link.tokens.iter());
				merge_by_name(&mut outputs, link.outputs.iter());
			}

			let declaration = self.select_declaration(node, &inputs, namespace)?;
			let node_outputs = self.node_outputs(node, &outputs, declaration)?;

			let node_colorspace = node.colorspace.or(colorspace);
			let node_file_prefix = node.file_prefix.or(file_prefix);

			let id = NodeId::new(self.nodes.len());

			if self.scopes[graph.index()].insert(node.name, id).is_some() {
				return Err(ResolveError::DuplicateName {
					scope: self.graphs[graph.index()].name.to_string(),
					name: node.name.to_string(),
				});
			}

			let mut node_inputs = Vec::with_capacity_in(inputs.len(), self.allocator);
			let mut node_tokens = Vec::with_capacity_in(tokens.len(), self.allocator);

			node_inputs.extend(
				inputs
					.iter()
					.map(|input| declare_input(input, node_colorspace, node_file_prefix, self.allocator)),
			);
			node_tokens.extend(tokens.iter().map(|token| declare_input(token, None, None, self.allocator)));

			self.nodes.push(Node {
				category: node.category,
				name: node.name,
				graph,
				data_type: node.data_type,
				declaration,
				inputs: node_inputs,
				outputs: node_outputs,
				tokens: node_tokens,
				colorspace: node_colorspace,
				file_prefix: node_file_prefix,
			});

			self.graphs[graph.index()].nodes.push(id);

			for (index, input) in inputs.iter().enumerate() {
				self.link(graph, namespace, Target::NodeInput(id, index), input.connection);
			}

			for (index, token) in tokens.iter().enumerate() {
				self.link(graph, namespace, Target::NodeToken(id, index), token.connection);
			}
		}

		Ok(())
	}

	/// Records an authored reference to resolve once every name in the document exists.
	fn link(
		&mut self,
		scope: GraphId,
		namespace: Option<&'a str>,
		target: Target,
		connection: Option<document::Connection<'a>>,
	) {
		if let Some(connection) = connection {
			self.links.push(Link {
				scope,
				namespace,
				target,
				connection,
			});
		}
	}

	fn push_graph(
		&mut self,
		name: &'a str,
		namespace: Option<&'a str>,
		parent: Option<GraphId>,
		declaration: Option<DeclarationId>,
		colorspace: Option<&'a str>,
		file_prefix: Option<&'a str>,
	) -> GraphId {
		let id = GraphId::new(self.graphs.len());

		self.graphs.push(Graph {
			name,
			namespace,
			parent,
			declaration,
			interface: Vec::new_in(self.allocator),
			interface_tokens: Vec::new_in(self.allocator),
			outputs: Vec::new_in(self.allocator),
			nodes: Vec::new_in(self.allocator),
			children: Vec::new_in(self.allocator),
			colorspace,
			file_prefix,
		});

		self.scopes.push(HashMap::with_hasher_in(RandomState::new(), self.allocator));

		id
	}

	/// Returns the declarations a nodegraph implements, whether it names one or is named by an
	/// `<implementation>`.
	fn graph_declarations(
		&self,
		source: &'r document::NodeGraph<'a>,
		namespace: Option<&'a str>,
	) -> Result<Vec<DeclarationId, Alloc<'a>>, ResolveError> {
		let mut declarations = Vec::new_in(self.allocator);

		if let Some(reference) = source.node_def {
			let id =
				lookup(&self.declaration_by_name, namespace, reference).ok_or_else(|| ResolveError::UnknownDeclaration {
					referrer: qualify(namespace, source.name),
					name: reference.to_string(),
				})?;

			declarations.push(*id);

			return Ok(declarations);
		}

		// An implementation naming a declaration this document does not carry describes a definition
		// for some other renderer, which says nothing about this graph's interface.
		if let Some(implementations) = self.implementations_by_graph.get(source.name) {
			for node_def in implementations {
				if let Some(id) = lookup(&self.declaration_by_name, namespace, node_def)
					&& !declarations.contains(id)
				{
					declarations.push(*id);
				}
			}
		}

		Ok(declarations)
	}

	/// Works out which outputs a node instance has: the ones it wrote, its declaration's, or the one
	/// implied by its own type.
	fn node_outputs(
		&self,
		node: &document::Node<'a>,
		outputs: &[&'r document::Output<'a>],
		declaration: Option<DeclarationId>,
	) -> Result<Vec<Port<'a>, Alloc<'a>>, ResolveError> {
		let mut ports = Vec::new_in(self.allocator);

		if !outputs.is_empty() {
			ports.extend(outputs.iter().map(|output| Port::from_output(output)));

			return Ok(ports);
		}

		if let Some(declaration) = declaration {
			ports.extend_from_slice(&self.declarations[declaration.index()].outputs);

			return Ok(ports);
		}

		// Without a declaration the only thing left to go on is the instance's own type, which says
		// nothing at all when the node has several outputs.
		if node.data_type == DataType::MultiOutput {
			return Err(ResolveError::UndeclaredOutputs {
				node: node.name.to_string(),
				category: node.category.to_string(),
			});
		}

		ports.push(Port {
			name: DEFAULT_OUTPUT_NAME,
			data_type: node.data_type,
			default: None,
			default_input: None,
			uniform: false,
			default_geom_prop: None,
		});

		Ok(ports)
	}

	/// Picks the declaration a node instance uses, either the one it names or the best signature match.
	fn select_declaration(
		&self,
		node: &document::Node<'a>,
		inputs: &[&'r document::Input<'a>],
		namespace: Option<&'a str>,
	) -> Result<Option<DeclarationId>, ResolveError> {
		if let Some(reference) = node.node_def {
			return lookup(&self.declaration_by_name, namespace, reference)
				.copied()
				.map(Some)
				.ok_or_else(|| ResolveError::UnknownDeclaration {
					referrer: node.name.to_string(),
					name: reference.to_string(),
				});
		}

		let Some(candidates) = self.declarations_by_category.get(node.category) else {
			return Ok(None);
		};

		let mut matches = Vec::new_in(self.allocator);

		matches.extend(candidates.iter().copied().filter(|candidate| {
			let candidate = &self.declarations[candidate.index()];

			node.version.is_none_or(|version| candidate.version == Some(version)) && produces(candidate, &node.data_type)
		}));

		// Several declarations can share a category and output type, so fall back to the input
		// signature the instance actually wrote before preferring the default version.
		if matches.len() > 1
			&& matches
				.iter()
				.any(|candidate| accepts_inputs(&self.declarations[candidate.index()], inputs))
		{
			matches.retain(|candidate| accepts_inputs(&self.declarations[candidate.index()], inputs));
		}

		let selected = matches
			.iter()
			.find(|candidate| self.declarations[candidate.index()].is_default_version)
			.or_else(|| {
				matches
					.iter()
					.find(|candidate| self.declarations[candidate.index()].targets.is_empty())
			})
			.or_else(|| matches.first());

		Ok(selected.copied())
	}

	/// Turns every authored reference into an edge, now that every node and port exists.
	fn connect(&mut self) -> Result<(), ResolveError> {
		let links = std::mem::replace(&mut self.links, Vec::new_in(self.allocator));

		for link in &links {
			let source = self.resolve_connection(link)?;

			self.port_mut(link.target).source = source;
		}

		Ok(())
	}

	fn port(&self, target: Target) -> &Input<'a> {
		match target {
			Target::NodeInput(node, index) => &self.nodes[node.index()].inputs[index],
			Target::NodeToken(node, index) => &self.nodes[node.index()].tokens[index],
			Target::GraphInterface(graph, index) => &self.graphs[graph.index()].interface[index],
			Target::GraphToken(graph, index) => &self.graphs[graph.index()].interface_tokens[index],
			Target::GraphOutput(graph, index) => &self.graphs[graph.index()].outputs[index],
		}
	}

	fn port_mut(&mut self, target: Target) -> &mut Input<'a> {
		match target {
			Target::NodeInput(node, index) => &mut self.nodes[node.index()].inputs[index],
			Target::NodeToken(node, index) => &mut self.nodes[node.index()].tokens[index],
			Target::GraphInterface(graph, index) => &mut self.graphs[graph.index()].interface[index],
			Target::GraphToken(graph, index) => &mut self.graphs[graph.index()].interface_tokens[index],
			Target::GraphOutput(graph, index) => &mut self.graphs[graph.index()].outputs[index],
		}
	}

	/// Names a port the way an error message should, as `owner.port`.
	///
	/// Only the failure path needs this, so a resolved reference never builds the string.
	fn referrer(&self, target: Target) -> String {
		let owner = match target {
			Target::NodeInput(node, _) | Target::NodeToken(node, _) => self.nodes[node.index()].name,
			Target::GraphInterface(graph, _) | Target::GraphToken(graph, _) | Target::GraphOutput(graph, _) => {
				self.graphs[graph.index()].name
			}
		};

		match owner.is_empty() {
			true => self.port(target).name.to_string(),
			false => format!("{owner}.{}", self.port(target).name),
		}
	}

	fn resolve_connection(&self, link: &Link<'a>) -> Result<Source<'a>, ResolveError> {
		let (source, found) = match link.connection {
			// A `nodename` normally names a node in the same scope, but documents in the wild also
			// reach a nodegraph with it.
			document::Connection::Node { node, output } => match self.scopes[link.scope.index()].get(node) {
				Some(target) => {
					let ports = &self.nodes[target.index()].outputs;
					let index = select_port(ports, output, || self.referrer(link.target), node)?;

					(
						Source::Node {
							node: *target,
							output: index,
						},
						ports[index.index()].data_type,
					)
				}
				None => {
					let target =
						*lookup(&self.graph_by_name, link.namespace, node).ok_or_else(|| ResolveError::UnknownNode {
							referrer: self.referrer(link.target),
							name: node.to_string(),
						})?;

					self.graph_output(target, output, link.target, node)?
				}
			},
			document::Connection::NodeGraph { node_graph, output } => {
				let target =
					*lookup(&self.graph_by_name, link.namespace, node_graph).ok_or_else(|| ResolveError::UnknownNodeGraph {
						referrer: self.referrer(link.target),
						name: node_graph.to_string(),
					})?;

				self.graph_output(target, output, link.target, node_graph)?
			}
			document::Connection::Interface { input } => {
				let graph = &self.graphs[link.scope.index()];

				if graph.interface.is_empty() {
					return Err(ResolveError::MissingInterface {
						referrer: self.referrer(link.target),
					});
				}

				let index = find_port(&graph.interface, input).ok_or_else(|| ResolveError::UnknownInterfaceInput {
					referrer: self.referrer(link.target),
					name: input.to_string(),
				})?;

				(
					Source::Interface {
						graph: link.scope,
						input: index,
					},
					graph.interface[index.index()].data_type,
				)
			}
		};

		let expected = self.port(link.target).data_type;

		if !expected.accepts(&found) {
			return Err(ResolveError::TypeMismatch {
				referrer: self.referrer(link.target),
				expected: expected.to_string(),
				found: found.to_string(),
			});
		}

		Ok(source)
	}

	fn graph_output(
		&self,
		graph: GraphId,
		output: Option<&str>,
		referrer: Target,
		target: &str,
	) -> Result<(Source<'a>, DataType<'a>), ResolveError> {
		let ports = &self.graphs[graph.index()].outputs;
		let index = select_port(ports, output, || self.referrer(referrer), target)?;

		Ok((Source::Graph { graph, output: index }, ports[index.index()].data_type))
	}
}

/// The output name MaterialX uses by convention for a node with a single result.
const DEFAULT_OUTPUT_NAME: &str = "out";

/// Returns whether a declaration produces exactly the type an instance says it does.
fn produces(declaration: &Declaration<'_>, data_type: &DataType<'_>) -> bool {
	match data_type {
		DataType::MultiOutput => declaration.outputs.len() > 1,
		_ => declaration.outputs.len() == 1 && &declaration.outputs[0].data_type == data_type,
	}
}

/// Returns whether every input an instance wrote exists on a declaration with the same type.
fn accepts_inputs(declaration: &Declaration<'_>, inputs: &[&document::Input<'_>]) -> bool {
	inputs.iter().all(|input| {
		declaration
			.inputs
			.iter()
			.any(|port| port.name == input.name && port.data_type == input.data_type)
	})
}

/// Returns the index of one port by name.
fn find_port<T: Named>(ports: &[T], name: &str) -> Option<PortIndex> {
	ports.iter().position(|port| port.name() == name).map(PortIndex::new)
}

/// Picks the output a reference selects, whether the upstream element is a node or a nodegraph.
///
/// The referrer's name is only built when the selection fails, so it stays off the resolved path.
fn select_port<T: Named>(
	ports: &[T],
	output: Option<&str>,
	referrer: impl Fn() -> String,
	target: &str,
) -> Result<PortIndex, ResolveError> {
	// MaterialX ignores the output selection when the upstream element only has one output.
	if ports.len() == 1 {
		return Ok(PortIndex(0));
	}

	let Some(name) = output else {
		return Err(ResolveError::UnselectedOutput {
			referrer: referrer(),
			target: target.to_string(),
		});
	};

	find_port(ports, name).ok_or_else(|| ResolveError::UnknownOutput {
		referrer: referrer(),
		target: target.to_string(),
		output: name.to_string(),
	})
}

/// Works out what drives a port that carries no connection.
///
/// A written value wins; otherwise a geometric property, and otherwise the type's own default. Types
/// with no meaningful zero, such as shaders and closures, resolve to [`Source::Unconnected`].
fn implicit_source<'a>(
	data_type: &DataType<'a>,
	value: Option<Value<'a>>,
	geom_prop: Option<&'a str>,
	allocator: Alloc<'a>,
) -> Source<'a> {
	// MaterialX writes an empty value on a shader or material input to mean "nothing plugged in".
	if let Some(value) = value {
		return match value {
			Value::Opaque("") => Source::Unconnected,
			value => Source::Value(value),
		};
	}

	if let Some(geom_prop) = geom_prop {
		return Source::GeomProp(geom_prop);
	}

	match Value::default_for(data_type, allocator) {
		Value::Opaque(_) => Source::Unconnected,
		default => Source::Value(default),
	}
}

/// Declares the port an `<input>` writes, before any connection on it has been resolved.
fn declare_input<'a>(
	input: &document::Input<'a>,
	colorspace: Option<&'a str>,
	file_prefix: Option<&'a str>,
	allocator: Alloc<'a>,
) -> Input<'a> {
	Input {
		source: implicit_source(&input.data_type, input.value.clone(), input.default_geom_prop, allocator),
		channels: input.channels,
		colorspace: input.colorspace.or(colorspace),
		file_prefix: input.file_prefix.or(file_prefix),
		unit: input.unit,
		unit_type: input.unit_type,
		..Input::declared(input.name, input.data_type)
	}
}

/// Declares the port an `<output>` publishes, before its connection has been resolved.
fn declare_output<'a>(output: &document::Output<'a>, allocator: Alloc<'a>) -> Input<'a> {
	Input {
		source: implicit_source(&output.data_type, output.default_value.clone(), None, allocator),
		colorspace: output.colorspace,
		..Input::declared(output.name, output.data_type)
	}
}

/// Turns a declaration's port into the interface entry a functional nodegraph presents.
fn declare_port<'a>(port: &Port<'a>) -> Input<'a> {
	Input {
		source: match &port.default {
			Some(value) => Source::Value(value.clone()),
			None => match port.default_geom_prop {
				Some(geom_prop) => Source::GeomProp(geom_prop),
				None => Source::Unconnected,
			},
		},
		..Input::declared(port.name, port.data_type)
	}
}

/// Returns the elements an `inherit` chain visits, nearest first.
fn inheritance_chain<'a, T: Named>(
	elements: &[T],
	start: usize,
	allocator: Alloc<'a>,
	inherit: impl Fn(&T) -> Option<&str>,
) -> Result<Vec<usize, Alloc<'a>>, ResolveError> {
	let mut chain = Vec::new_in(allocator);
	let mut current = start;

	chain.push(start);

	while let Some(parent) = inherit(&elements[current]) {
		let name = elements[current].name();

		let index =
			elements
				.iter()
				.position(|element| element.name() == parent)
				.ok_or_else(|| ResolveError::UnknownInheritance {
					referrer: name.to_string(),
					name: parent.to_string(),
				})?;

		if chain.contains(&index) {
			return Err(ResolveError::InheritanceCycle { name: name.to_string() });
		}

		chain.push(index);
		current = index;
	}

	Ok(chain)
}

/// Splits a reference into the namespace it names and the name inside it.
fn split_name(reference: &str) -> (Option<&str>, &str) {
	match reference.split_once(':') {
		Some((namespace, name)) => (Some(namespace), name),
		None => (None, reference),
	}
}

/// Joins a namespace and a name the way MaterialX qualifies references across files.
///
/// Only error messages need the joined form, so nothing on the resolved path calls this.
fn qualify(namespace: Option<&str>, name: &str) -> String {
	match namespace {
		Some(namespace) if !namespace.is_empty() => format!("{namespace}:{name}"),
		_ => name.to_string(),
	}
}

/// Looks a reference up, trying the current namespace before the global one.
///
/// A reference that already carries a namespace is used exactly as written, which is what the
/// specification asks for.
fn lookup<'m, 'a, V>(map: &'m Map<'a, Key<'a>, V>, namespace: Option<&'a str>, reference: &'a str) -> Option<&'m V> {
	if let (Some(namespace), name) = split_name(reference) {
		return map.get(&(Some(namespace), name));
	}

	namespace
		.filter(|namespace| !namespace.is_empty())
		.and_then(|namespace| map.get(&(Some(namespace), reference)))
		.or_else(|| map.get(&(None, reference)))
}
