use utils::hash::HashSet;

/// Returns every node the `main` function reaches, each after the nodes it depends on, and `main` last.
///
/// Backends emit declarations in this order, so every type, binding, and function is declared before its first use.
/// The walk visits each node's dependencies in a fixed order, so the order does not change between runs.
///
/// # Panics
///
/// Panics if `main_function_node` is not the `main` function, or if the nodes depend on each other in a cycle.
pub fn dependency_order(main_function_node: &besl::NodeReference) -> Vec<besl::NodeReference> {
	let mut order = Vec::new();
	let mut expanded = HashSet::default();
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
		visit_node(child, &mut order, &mut expanded, &mut active);
	}
	order.push(main_function_node.clone());

	/// Appends `node` after its dependencies unless an earlier visit already appended it.
	// This match is the exhaustive shader-node dependency-edge contract.
	fn visit_node(
		node: &besl::NodeReference,
		order: &mut Vec<besl::NodeReference>,
		expanded: &mut HashSet<besl::NodeReference>,
		active: &mut Vec<besl::NodeReference>,
	) {
		if expanded.contains(node) {
			return;
		}

		assert!(
			!active.contains(node),
			"Cyclic shader dependency detected while building the shader graph. The most likely cause is a self-referential or mutually recursive BESL node graph."
		);

		active.push(node.clone());

		let node_borrow = node.borrow();
		// Dependencies are visited in the order the arms list them.
		let mut visit = |child: &besl::NodeReference| visit_node(child, order, expanded, active);

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
		order.push(node.clone());
	}

	order
}
