use std::{
	cell::RefCell,
	collections::{HashMap, HashSet},
};

/// Graph structure for tracking dependencies between shader nodes.
#[derive(Clone, Debug)]
pub struct Graph {
	pub set: HashMap<besl::NodeReference, Vec<besl::NodeReference>>,
}

impl Graph {
	pub fn new() -> Self {
		Graph {
			set: HashMap::with_capacity(1024),
		}
	}

	pub fn add(&mut self, from: besl::NodeReference, to: besl::NodeReference) {
		self.set.entry(from).or_insert(Vec::new()).push(to);
	}
}

/// Performs a topological sort on the graph to determine the order in which nodes should be emitted.
pub fn topological_sort(graph: &Graph) -> Vec<besl::NodeReference> {
	let mut visited = HashSet::new();
	let mut stack = Vec::new();

	for (node, _) in graph.set.iter() {
		if !visited.contains(node) {
			topological_sort_impl(node.clone(), graph, &mut visited, &mut stack);
		}
	}

	fn topological_sort_impl(
		node: besl::NodeReference,
		graph: &Graph,
		visited: &mut HashSet<besl::NodeReference>,
		stack: &mut Vec<besl::NodeReference>,
	) {
		visited.insert(node.clone());

		if let Some(neighbours) = graph.set.get(&node) {
			for neighbour in neighbours {
				if !visited.contains(neighbour) {
					topological_sort_impl(neighbour.clone(), graph, visited, stack);
				}
			}
		}

		stack.push(node);
	}

	stack
}

/// Builds a dependency graph from the main function node.
pub fn build_graph(main_function_node: besl::NodeReference) -> Graph {
	let mut graph = Graph::new();
	let mut expanded = HashSet::new();
	let mut active = Vec::new();

	let node_borrow = RefCell::borrow(&main_function_node);
	let node_ref = node_borrow.node();

	match node_ref {
		besl::Nodes::Function {
			params,
			return_type,
			statements,
			name,
			..
		} => {
			assert_eq!(name, "main");

			for p in params {
				build_graph_impl(main_function_node.clone(), p.clone(), &mut graph, &mut expanded, &mut active);
			}

			for statement in statements {
				build_graph_impl(
					main_function_node.clone(),
					statement.clone(),
					&mut graph,
					&mut expanded,
					&mut active,
				);
			}

			build_graph_impl(
				main_function_node.clone(),
				return_type.clone(),
				&mut graph,
				&mut expanded,
				&mut active,
			);
		}
		_ => panic!("Root node must be a function node."),
	}

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

		if active.contains(&node) {
			panic!(
				"Cyclic shader dependency detected while building the shader graph. The most likely cause is a self-referential or mutually recursive BESL node graph."
			);
		}

		active.push(node.clone());

		let node_borrow = RefCell::borrow(&node);
		let node_ref = node_borrow.node();

		match node_ref {
			besl::Nodes::Null => {}
			besl::Nodes::Scope { children, .. } => {
				for child in children {
					build_graph_impl(node.clone(), child.clone(), graph, expanded, active);
				}
			}
			besl::Nodes::Function {
				statements,
				params,
				return_type,
				..
			} => {
				for parameter in params {
					build_graph_impl(node.clone(), parameter.clone(), graph, expanded, active);
				}

				for statement in statements {
					build_graph_impl(node.clone(), statement.clone(), graph, expanded, active);
				}

				build_graph_impl(node.clone(), return_type.clone(), graph, expanded, active);
			}
			besl::Nodes::Conditional { condition, statements } => {
				build_graph_impl(node.clone(), condition.clone(), graph, expanded, active);

				for statement in statements {
					build_graph_impl(node.clone(), statement.clone(), graph, expanded, active);
				}
			}
			besl::Nodes::Struct { fields, .. } => {
				for field in fields {
					build_graph_impl(node.clone(), field.clone(), graph, expanded, active);
				}
			}
			besl::Nodes::PushConstant { members } => {
				for member in members {
					build_graph_impl(node.clone(), member.clone(), graph, expanded, active);
				}
			}
			besl::Nodes::Specialization { r#type, .. } => {
				build_graph_impl(node.clone(), r#type.clone(), graph, expanded, active);
			}
			besl::Nodes::Member { r#type, .. } => {
				build_graph_impl(node.clone(), r#type.clone(), graph, expanded, active);
			}
			besl::Nodes::Raw { input, output, .. } => {
				for reference in input {
					build_graph_impl(node.clone(), reference.clone(), graph, expanded, active);
				}

				for reference in output {
					build_graph_impl(node.clone(), reference.clone(), graph, expanded, active);
				}
			}
			besl::Nodes::Parameter { r#type, .. } => {
				build_graph_impl(node.clone(), r#type.clone(), graph, expanded, active);
			}
			besl::Nodes::Expression(expression) => {
				match expression {
					besl::Expressions::Operator { operator, left, right } => {
						if operator == &besl::Operators::Assignment {
							build_graph_impl(node.clone(), left.clone(), graph, expanded, active);
							build_graph_impl(node.clone(), right.clone(), graph, expanded, active);
						}
					}
					besl::Expressions::FunctionCall {
						parameters, function, ..
					} => {
						build_graph_impl(node.clone(), function.clone(), graph, expanded, active);

						for parameter in parameters {
							build_graph_impl(node.clone(), parameter.clone(), graph, expanded, active);
						}
					}
					besl::Expressions::IntrinsicCall {
						elements: parameters, ..
					} => {
						for e in parameters {
							build_graph_impl(node.clone(), e.clone(), graph, expanded, active);
						}
					}
					besl::Expressions::Expression { elements } => {
						for element in elements {
							build_graph_impl(node.clone(), element.clone(), graph, expanded, active);
						}
					}
					besl::Expressions::Macro { body, .. } => {
						build_graph_impl(node.clone(), body.clone(), graph, expanded, active);
					}
					besl::Expressions::Member { source, .. } => match source.borrow().node() {
						besl::Nodes::Expression { .. } | besl::Nodes::Member { .. } => {}
						_ => {
							build_graph_impl(node.clone(), source.clone(), graph, expanded, active);
						}
					},
					besl::Expressions::VariableDeclaration { r#type, .. } => {
						build_graph_impl(node.clone(), r#type.clone(), graph, expanded, active);
					}
					besl::Expressions::Literal { .. } => {
						// build_graph_inner(node.clone(), value.clone(), graph);
					}
					besl::Expressions::Return { value } => {
						if let Some(value) = value {
							build_graph_impl(node.clone(), value.clone(), graph, expanded, active);
						}
					}
					besl::Expressions::Accessor { left, right } => {
						build_graph_impl(node.clone(), left.clone(), graph, expanded, active);
						build_graph_impl(node.clone(), right.clone(), graph, expanded, active);
					}
				}
			}
			besl::Nodes::Binding { r#type, .. } => match r#type {
				besl::BindingTypes::Buffer { members } => {
					for member in members {
						build_graph_impl(node.clone(), member.clone(), graph, expanded, active);
					}
				}
				besl::BindingTypes::Image { .. } => {}
				besl::BindingTypes::CombinedImageSampler { .. } => {}
			},
			besl::Nodes::Input { format, .. } | besl::Nodes::Output { format, .. } => {
				build_graph_impl(node.clone(), format.clone(), graph, expanded, active);
			}
			besl::Nodes::Intrinsic { elements, .. } => {
				for element in elements {
					build_graph_impl(node.clone(), element.clone(), graph, expanded, active);
				}
			}
			besl::Nodes::Literal { value, .. } => {
				build_graph_impl(node.clone(), value.clone(), graph, expanded, active);
			}
			besl::Nodes::Const { r#type, value, .. } => {
				build_graph_impl(node.clone(), r#type.clone(), graph, expanded, active);
				build_graph_impl(node.clone(), value.clone(), graph, expanded, active);
			}
		}

		active.pop();
		expanded.insert(node.clone());
	}

	graph
}
