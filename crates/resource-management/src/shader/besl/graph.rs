use std::collections::{HashMap, HashSet};

/// The `Graph` struct exists to track dependencies between shader nodes.
#[derive(Clone, Debug)]
pub struct Graph {
	pub set: HashMap<besl::NodeReference, Vec<besl::NodeReference>>,
}

impl Default for Graph {
	fn default() -> Self {
		Self::new()
	}
}

impl Graph {
	pub fn new() -> Self {
		Graph {
			set: HashMap::with_capacity(1024),
		}
	}

	pub fn add(&mut self, from: besl::NodeReference, to: besl::NodeReference) {
		self.set.entry(from).or_default().push(to);
	}
}

/// Performs a topological sort on the graph to determine the order in which nodes should be emitted.
///
/// Every node of a graph from [`build_graph`] is reachable from its main function, so one depth-first walk from
/// that root visits them all. `set` hashes nodes by address, so walking its keys instead would make emission order
/// change between runs.
pub fn topological_sort(graph: &Graph, root: &besl::NodeReference) -> Vec<besl::NodeReference> {
	fn visit(
		node: &besl::NodeReference,
		graph: &Graph,
		visited: &mut HashSet<besl::NodeReference>,
		stack: &mut Vec<besl::NodeReference>,
	) {
		if !visited.insert(node.clone()) {
			return;
		}

		for neighbour in graph.set.get(node).into_iter().flatten() {
			visit(neighbour, graph, visited, stack);
		}

		stack.push(node.clone());
	}

	let mut visited = HashSet::new();
	let mut stack = Vec::new();
	visit(root, graph, &mut visited, &mut stack);
	stack
}

/// Builds a dependency graph from the main function node.
pub fn build_graph(main_function_node: besl::NodeReference) -> Graph {
	let mut graph = Graph::new();
	let mut expanded = HashSet::new();
	let mut active = Vec::new();

	let node = main_function_node.borrow();
	let besl::Nodes::Function {
		params,
		return_type,
		statements,
		name,
		..
	} = node.node()
	else {
		panic!("Root node must be a function node.")
	};
	assert_eq!(name, "main");
	for child in params.iter().chain(statements).chain([return_type]) {
		build_graph_impl(
			main_function_node.clone(),
			child.clone(),
			&mut graph,
			&mut expanded,
			&mut active,
		);
	}

	// This match is the exhaustive shader-node dependency-edge contract.
	fn build_graph_impl(
		parent: besl::NodeReference,
		node: besl::NodeReference,
		graph: &mut Graph,
		expanded: &mut HashSet<besl::NodeReference>,
		active: &mut Vec<besl::NodeReference>,
	) {
		graph.add(parent, node.clone());

		if expanded.contains(&node) {
			return;
		}

		assert!(
			!active.contains(&node),
			"Cyclic shader dependency detected while building the shader graph. The most likely cause is a self-referential or mutually recursive BESL node graph."
		);

		active.push(node.clone());

		let node_borrow = node.borrow();
		// Each child gets an edge from `node`, in the order the arms list them.
		let mut visit = |child: &besl::NodeReference| build_graph_impl(node.clone(), child.clone(), graph, expanded, active);

		match node_borrow.node() {
			besl::Nodes::Scope { children, .. }
			| besl::Nodes::Struct { fields: children, .. }
			| besl::Nodes::PushConstant { members: children }
			| besl::Nodes::Intrinsic { elements: children, .. } => children.iter().for_each(&mut visit),
			besl::Nodes::Function {
				statements,
				params,
				return_type,
				..
			} => params.iter().chain(statements).chain([return_type]).for_each(&mut visit),
			branch @ (besl::Nodes::Conditional { .. } | besl::Nodes::Match { .. }) => {
				branch.branch_children().for_each(&mut visit)
			}
			besl::Nodes::ForLoop {
				initializer,
				condition,
				update,
				statements,
			} => [initializer, condition, update]
				.into_iter()
				.chain(statements)
				.for_each(&mut visit),
			besl::Nodes::Specialization { r#type, .. }
			| besl::Nodes::Member { r#type, .. }
			| besl::Nodes::Parameter { r#type, .. }
			| besl::Nodes::Input { format: r#type, .. }
			| besl::Nodes::Output { format: r#type, .. }
			| besl::Nodes::TaskPayload { format: r#type, .. }
			| besl::Nodes::Workgroup { format: r#type, .. } => visit(r#type),
			besl::Nodes::Raw { input, output, .. } => input.iter().chain(output).for_each(&mut visit),
			besl::Nodes::Expression(expression) => match expression {
				besl::Expressions::Operator { left, right, .. } | besl::Expressions::Accessor { left, right } => {
					visit(left);
					visit(right);
				}
				besl::Expressions::FunctionCall {
					parameters, function, ..
				} => {
					visit(&function.get());
					parameters.iter().for_each(&mut visit);
				}
				besl::Expressions::IntrinsicCall { arguments, elements, .. } => {
					arguments.iter().chain(elements).for_each(&mut visit)
				}
				besl::Expressions::Expression { elements } => elements.iter().for_each(&mut visit),
				besl::Expressions::Macro { body, .. } => visit(body),
				besl::Expressions::Member { source, .. } => visit(source),
				besl::Expressions::VariableDeclaration { r#type, .. } => visit(r#type),
				besl::Expressions::Return { value } => value.iter().for_each(&mut visit),
				besl::Expressions::Literal { .. }
				| besl::Expressions::Continue
				| besl::Expressions::Break
				| besl::Expressions::Discard => {}
			},
			besl::Nodes::Binding { r#type, .. } => match r#type {
				besl::BindingTypes::Buffer { members } => members.iter().for_each(&mut visit),
				besl::BindingTypes::BufferArray { element, .. } => visit(element),
				besl::BindingTypes::Image { .. } | besl::BindingTypes::CombinedImageSampler { .. } => {}
			},
			besl::Nodes::Const { r#type, value, .. } => {
				visit(r#type);
				visit(value);
			}
		}

		active.pop();
		expanded.insert(node.clone());
	}

	graph
}
