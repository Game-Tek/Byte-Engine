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
                build_graph_impl(main_function_node.clone(), p.clone(), &mut graph);
            }

            for statement in statements {
                build_graph_impl(main_function_node.clone(), statement.clone(), &mut graph);
            }

            build_graph_impl(main_function_node.clone(), return_type.clone(), &mut graph);
        }
        _ => panic!("Root node must be a function node."),
    }

    fn build_graph_impl(
        parent: besl::NodeReference,
        node: besl::NodeReference,
        graph: &mut Graph,
    ) -> () {
        graph.add(parent, node.clone());

        let node_borrow = RefCell::borrow(&node);
        let node_ref = node_borrow.node();

        match node_ref {
            besl::Nodes::Null => {}
            besl::Nodes::Scope { children, .. } => {
                for child in children {
                    build_graph_impl(node.clone(), child.clone(), graph);
                }
            }
            besl::Nodes::Function {
                statements,
                params,
                return_type,
                ..
            } => {
                for parameter in params {
                    build_graph_impl(node.clone(), parameter.clone(), graph);
                }

                for statement in statements {
                    build_graph_impl(node.clone(), statement.clone(), graph);
                }

                build_graph_impl(node.clone(), return_type.clone(), graph);
            }
            besl::Nodes::Struct { fields, .. } => {
                for field in fields {
                    build_graph_impl(node.clone(), field.clone(), graph);
                }
            }
            besl::Nodes::PushConstant { members } => {
                for member in members {
                    build_graph_impl(node.clone(), member.clone(), graph);
                }
            }
            besl::Nodes::Specialization { r#type, .. } => {
                build_graph_impl(node.clone(), r#type.clone(), graph);
            }
            besl::Nodes::Member { r#type, .. } => {
                build_graph_impl(node.clone(), r#type.clone(), graph);
            }
            besl::Nodes::Raw { input, output, .. } => {
                for reference in input {
                    build_graph_impl(node.clone(), reference.clone(), graph);
                }

                for reference in output {
                    build_graph_impl(node.clone(), reference.clone(), graph);
                }
            }
            besl::Nodes::Parameter { r#type, .. } => {
                build_graph_impl(node.clone(), r#type.clone(), graph);
            }
            besl::Nodes::Expression(expression) => {
                match expression {
                    besl::Expressions::Operator {
                        operator,
                        left,
                        right,
                    } => {
                        if operator == &besl::Operators::Assignment {
                            build_graph_impl(node.clone(), left.clone(), graph);
                            build_graph_impl(node.clone(), right.clone(), graph);
                        }
                    }
                    besl::Expressions::FunctionCall {
                        parameters,
                        function,
                        ..
                    } => {
                        build_graph_impl(node.clone(), function.clone(), graph);

                        for parameter in parameters {
                            build_graph_impl(node.clone(), parameter.clone(), graph);
                        }
                    }
                    besl::Expressions::IntrinsicCall {
                        elements: parameters,
                        ..
                    } => {
                        for e in parameters {
                            build_graph_impl(node.clone(), e.clone(), graph);
                        }
                    }
                    besl::Expressions::Expression { elements } => {
                        for element in elements {
                            build_graph_impl(node.clone(), element.clone(), graph);
                        }
                    }
                    besl::Expressions::Macro { body, .. } => {
                        build_graph_impl(node.clone(), body.clone(), graph);
                    }
                    besl::Expressions::Member { source, .. } => match source.borrow().node() {
                        besl::Nodes::Expression { .. } | besl::Nodes::Member { .. } => {}
                        _ => {
                            build_graph_impl(node.clone(), source.clone(), graph);
                        }
                    },
                    besl::Expressions::VariableDeclaration { r#type, .. } => {
                        build_graph_impl(node.clone(), r#type.clone(), graph);
                    }
                    besl::Expressions::Literal { .. } => {
                        // build_graph_inner(node.clone(), value.clone(), graph);
                    }
                    besl::Expressions::Return => {}
                    besl::Expressions::Accessor { left, right } => {
                        build_graph_impl(node.clone(), left.clone(), graph);
                        build_graph_impl(node.clone(), right.clone(), graph);
                    }
                }
            }
            besl::Nodes::Binding { r#type, .. } => match r#type {
                besl::BindingTypes::Buffer { members } => {
                    for member in members {
                        build_graph_impl(node.clone(), member.clone(), graph);
                    }
                }
                besl::BindingTypes::Image { .. } => {}
                besl::BindingTypes::CombinedImageSampler { .. } => {}
            },
            besl::Nodes::Input { format, .. } | besl::Nodes::Output { format, .. } => {
                build_graph_impl(node.clone(), format.clone(), graph);
            }
            besl::Nodes::Intrinsic { elements, .. } => {
                for element in elements {
                    build_graph_impl(node.clone(), element.clone(), graph);
                }
            }
            besl::Nodes::Literal { value, .. } => {
                build_graph_impl(node.clone(), value.clone(), graph);
            }
        }
    }

    graph
}
