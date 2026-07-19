use std::{
	alloc::{Allocator, Global},
	cell::RefCell,
	collections::{HashMap, HashSet},
	hash::RandomState,
	vec::Vec as AllocVec,
};

/// The `Graph` struct exists to track dependencies between shader nodes.
#[derive(Clone, Debug)]
pub struct Graph<A: Allocator + Clone = Global> {
	pub set: HashMap<besl::NodeReference, AllocVec<besl::NodeReference, A>, RandomState, A>,
	allocator: A,
}

impl Default for Graph<Global> {
	fn default() -> Self {
		Self::new()
	}
}

impl Graph<Global> {
	pub fn new() -> Self {
		Self::new_in(Global)
	}
}

impl<A: Allocator + Clone> Graph<A> {
	pub fn new_in(allocator: A) -> Self {
		Graph {
			set: HashMap::with_capacity_and_hasher_in(1024, RandomState::new(), allocator.clone()),
			allocator,
		}
	}

	pub fn add(&mut self, from: besl::NodeReference, to: besl::NodeReference) {
		self.set
			.entry(from)
			.or_insert_with(|| AllocVec::new_in(self.allocator.clone()))
			.push(to);
	}
}

/// Performs a topological sort on the graph to determine the order in which nodes should be emitted.
pub fn topological_sort(graph: &Graph) -> Vec<besl::NodeReference> {
	topological_sort_in(graph, Global)
}

/// Performs a topological sort using the provided allocator for temporary traversal state.
pub fn topological_sort_in<A: Allocator + Clone>(graph: &Graph<A>, allocator: A) -> AllocVec<besl::NodeReference, A> {
	let mut visited = HashSet::with_hasher_in(RandomState::new(), allocator.clone());
	let mut stack = AllocVec::new_in(allocator.clone());

	for node in graph.set.keys() {
		if !visited.contains(node) {
			topological_sort_impl(node.clone(), graph, &mut visited, &mut stack, allocator.clone());
		}
	}

	fn topological_sort_impl<A: Allocator + Clone>(
		node: besl::NodeReference,
		graph: &Graph<A>,
		visited: &mut HashSet<besl::NodeReference, RandomState, A>,
		stack: &mut AllocVec<besl::NodeReference, A>,
		allocator: A,
	) {
		visited.insert(node.clone());

		if let Some(neighbours) = graph.set.get(&node) {
			for neighbour in neighbours {
				if !visited.contains(neighbour) {
					topological_sort_impl(neighbour.clone(), graph, visited, stack, allocator.clone());
				}
			}
		}

		stack.push(node);
	}

	stack
}

/// Builds a dependency graph from the main function node.
pub fn build_graph(main_function_node: besl::NodeReference) -> Graph {
	build_graph_in(main_function_node, Global)
}

/// Builds a dependency graph using the provided allocator for graph and traversal storage.
pub fn build_graph_in<A: Allocator + Clone>(main_function_node: besl::NodeReference, allocator: A) -> Graph<A> {
	let mut graph = Graph::new_in(allocator.clone());
	let mut expanded = HashSet::with_hasher_in(RandomState::new(), allocator.clone());
	let mut active = AllocVec::new_in(allocator.clone());

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
				build_graph_impl(
					main_function_node.clone(),
					p.clone(),
					&mut graph,
					&mut expanded,
					&mut active,
					allocator.clone(),
				);
			}

			for statement in statements {
				build_graph_impl(
					main_function_node.clone(),
					statement.clone(),
					&mut graph,
					&mut expanded,
					&mut active,
					allocator.clone(),
				);
			}

			build_graph_impl(
				main_function_node.clone(),
				return_type.clone(),
				&mut graph,
				&mut expanded,
				&mut active,
				allocator.clone(),
			);
		}
		_ => panic!("Root node must be a function node."),
	}

	fn build_graph_impl<A: Allocator + Clone>(
		parent: besl::NodeReference,
		node: besl::NodeReference,
		graph: &mut Graph<A>,
		expanded: &mut HashSet<besl::NodeReference, RandomState, A>,
		active: &mut AllocVec<besl::NodeReference, A>,
		allocator: A,
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
					build_graph_impl(node.clone(), child.clone(), graph, expanded, active, allocator.clone());
				}
			}
			besl::Nodes::Function {
				statements,
				params,
				return_type,
				..
			} => {
				for parameter in params {
					build_graph_impl(node.clone(), parameter.clone(), graph, expanded, active, allocator.clone());
				}

				for statement in statements {
					build_graph_impl(node.clone(), statement.clone(), graph, expanded, active, allocator.clone());
				}

				build_graph_impl(node.clone(), return_type.clone(), graph, expanded, active, allocator.clone());
			}
			besl::Nodes::Conditional { condition, statements } => {
				build_graph_impl(node.clone(), condition.clone(), graph, expanded, active, allocator.clone());

				for statement in statements {
					build_graph_impl(node.clone(), statement.clone(), graph, expanded, active, allocator.clone());
				}
			}
			besl::Nodes::ForLoop {
				initializer,
				condition,
				update,
				statements,
			} => {
				build_graph_impl(node.clone(), initializer.clone(), graph, expanded, active, allocator.clone());
				build_graph_impl(node.clone(), condition.clone(), graph, expanded, active, allocator.clone());
				build_graph_impl(node.clone(), update.clone(), graph, expanded, active, allocator.clone());

				for statement in statements {
					build_graph_impl(node.clone(), statement.clone(), graph, expanded, active, allocator.clone());
				}
			}
			besl::Nodes::Struct { fields, .. } => {
				for field in fields {
					build_graph_impl(node.clone(), field.clone(), graph, expanded, active, allocator.clone());
				}
			}
			besl::Nodes::PushConstant { members } => {
				for member in members {
					build_graph_impl(node.clone(), member.clone(), graph, expanded, active, allocator.clone());
				}
			}
			besl::Nodes::Specialization { r#type, .. } => {
				build_graph_impl(node.clone(), r#type.clone(), graph, expanded, active, allocator.clone());
			}
			besl::Nodes::Member { r#type, .. } => {
				build_graph_impl(node.clone(), r#type.clone(), graph, expanded, active, allocator.clone());
			}
			besl::Nodes::Raw { input, output, .. } => {
				for reference in input {
					build_graph_impl(node.clone(), reference.clone(), graph, expanded, active, allocator.clone());
				}

				for reference in output {
					build_graph_impl(node.clone(), reference.clone(), graph, expanded, active, allocator.clone());
				}
			}
			besl::Nodes::Parameter { r#type, .. } => {
				build_graph_impl(node.clone(), r#type.clone(), graph, expanded, active, allocator.clone());
			}
			besl::Nodes::Expression(expression) => {
				match expression {
					besl::Expressions::Operator { left, right, .. } => {
						build_graph_impl(node.clone(), left.clone(), graph, expanded, active, allocator.clone());
						build_graph_impl(node.clone(), right.clone(), graph, expanded, active, allocator.clone());
					}
					besl::Expressions::FunctionCall {
						parameters, function, ..
					} => {
						build_graph_impl(node.clone(), function.clone(), graph, expanded, active, allocator.clone());

						for parameter in parameters {
							build_graph_impl(node.clone(), parameter.clone(), graph, expanded, active, allocator.clone());
						}
					}
					besl::Expressions::IntrinsicCall {
						elements: parameters, ..
					} => {
						for e in parameters {
							build_graph_impl(node.clone(), e.clone(), graph, expanded, active, allocator.clone());
						}
					}
					besl::Expressions::Expression { elements } => {
						for element in elements {
							build_graph_impl(node.clone(), element.clone(), graph, expanded, active, allocator.clone());
						}
					}
					besl::Expressions::Macro { body, .. } => {
						build_graph_impl(node.clone(), body.clone(), graph, expanded, active, allocator.clone());
					}
					besl::Expressions::Member { source, .. } => match source.borrow().node() {
						besl::Nodes::Expression { .. } | besl::Nodes::Member { .. } => {}
						_ => {
							build_graph_impl(node.clone(), source.clone(), graph, expanded, active, allocator.clone());
						}
					},
					besl::Expressions::VariableDeclaration { r#type, .. } => {
						build_graph_impl(node.clone(), r#type.clone(), graph, expanded, active, allocator.clone());
					}
					besl::Expressions::Literal { .. } => {
						// build_graph_inner(node.clone(), value.clone(), graph);
					}
					besl::Expressions::Return { value } => {
						if let Some(value) = value {
							build_graph_impl(node.clone(), value.clone(), graph, expanded, active, allocator.clone());
						}
					}
					besl::Expressions::Continue => {}
					besl::Expressions::Accessor { left, right } => {
						build_graph_impl(node.clone(), left.clone(), graph, expanded, active, allocator.clone());
						build_graph_impl(node.clone(), right.clone(), graph, expanded, active, allocator.clone());
					}
				}
			}
			besl::Nodes::Binding { r#type, .. } => match r#type {
				besl::BindingTypes::Buffer { members } => {
					for member in members {
						build_graph_impl(node.clone(), member.clone(), graph, expanded, active, allocator.clone());
					}
				}
				besl::BindingTypes::Image { .. } => {}
				besl::BindingTypes::CombinedImageSampler { .. } => {}
			},
			besl::Nodes::Input { format, .. }
			| besl::Nodes::Output { format, .. }
			| besl::Nodes::TaskPayload { format, .. }
			| besl::Nodes::Workgroup { format, .. } => {
				build_graph_impl(node.clone(), format.clone(), graph, expanded, active, allocator.clone());
			}
			besl::Nodes::Intrinsic { elements, .. } => {
				for element in elements {
					build_graph_impl(node.clone(), element.clone(), graph, expanded, active, allocator.clone());
				}
			}
			besl::Nodes::Literal { value, .. } => {
				build_graph_impl(node.clone(), value.clone(), graph, expanded, active, allocator.clone());
			}
			besl::Nodes::Const { r#type, value, .. } => {
				build_graph_impl(node.clone(), r#type.clone(), graph, expanded, active, allocator.clone());
				build_graph_impl(node.clone(), value.clone(), graph, expanded, active, allocator.clone());
			}
		}

		active.pop();
		expanded.insert(node.clone());
	}

	graph
}
