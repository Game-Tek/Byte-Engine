//! Optimizes linked BESL programs before reflection and backend lowering.
//!
//! The pass works on the linked semantic tree, where each variable use refers
//! directly to its declaration. That lets it remove dead local declarations
//! without relying on names or backend-specific syntax.

use std::collections::{HashMap, HashSet};

use crate::{Expressions, NodeReference, Nodes, Operators};

/// The `OptimizationReport` struct describes the portable BESL code removed by [`optimize`].
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct OptimizationReport {
	/// Number of dead local declaration statements removed from reachable functions.
	pub culled_unused_local_variables: usize,
	/// Number of statements removed because an earlier statement always exits the same block.
	pub culled_unreachable_statements: usize,
}

/// Removes semantically dead local declarations from functions reachable from `main_function_node`.
///
/// The pass mutates the linked program in place and is idempotent. It removes a
/// declaration only when no remaining code reads it and its initializer has no
/// observable effect. Calls, atomics, image writes, barriers, raw backend code,
/// and assignments outside a local value remain intact. Statements after a
/// `return` or `continue` in the same block are unreachable and are removed
/// regardless of their effects.
///
/// Next, pass the optimized node to shader reflection or a backend generator.
pub fn optimize(main_function_node: &NodeReference) -> OptimizationReport {
	let functions = reachable_functions(main_function_node);
	let mut report = OptimizationReport::default();

	loop {
		let mut changed = false;
		for function in &functions {
			changed |= cull_unreachable_statements(function, &mut report);
		}

		let mut effects = EffectAnalysis::default();
		let mut removals = HashSet::new();
		for function in &functions {
			collect_unused_local_declarations(function, &mut effects, &mut removals);
		}

		if removals.is_empty() {
			if !changed {
				break;
			}
			continue;
		}

		for function in &functions {
			report.culled_unused_local_variables += remove_statements(function, &removals);
		}
	}

	report
}

/// Finds every user function that a reachable function can call.
fn reachable_functions(main_function_node: &NodeReference) -> Vec<NodeReference> {
	let mut functions = Vec::new();
	let mut visited = HashSet::new();
	collect_reachable_function(main_function_node, &mut functions, &mut visited);
	functions
}

fn collect_reachable_function(function: &NodeReference, functions: &mut Vec<NodeReference>, visited: &mut HashSet<usize>) {
	if !visited.insert(function.identity()) {
		return;
	}

	let Nodes::Function { statements, .. } = cloned_node(function) else {
		return;
	};
	functions.push(function.clone());
	for statement in statements {
		collect_called_functions(&statement, functions, visited);
	}
}

fn collect_called_functions(node: &NodeReference, functions: &mut Vec<NodeReference>, visited: &mut HashSet<usize>) {
	match cloned_node(node) {
		Nodes::Conditional { condition, statements } => {
			collect_called_functions(&condition, functions, visited);
			for statement in statements {
				collect_called_functions(&statement, functions, visited);
			}
		}
		Nodes::ForLoop {
			initializer,
			condition,
			update,
			statements,
		} => {
			collect_called_functions(&initializer, functions, visited);
			collect_called_functions(&condition, functions, visited);
			collect_called_functions(&update, functions, visited);
			for statement in statements {
				collect_called_functions(&statement, functions, visited);
			}
		}
		Nodes::Raw { input, output, .. } => {
			for input in input {
				collect_called_functions(&input, functions, visited);
			}
			for output in output {
				collect_called_functions(&output, functions, visited);
			}
		}
		Nodes::Expression(expression) => collect_called_functions_in_expression(&expression, functions, visited),
		_ => {}
	}
}

fn collect_called_functions_in_expression(
	expression: &Expressions,
	functions: &mut Vec<NodeReference>,
	visited: &mut HashSet<usize>,
) {
	match expression {
		Expressions::Return { value } => {
			if let Some(value) = value {
				collect_called_functions(value, functions, visited);
			}
		}
		Expressions::Expression { elements } => {
			for element in elements {
				collect_called_functions(element, functions, visited);
			}
		}
		Expressions::FunctionCall { function, parameters } => {
			collect_reachable_function(function, functions, visited);
			for parameter in parameters {
				collect_called_functions(parameter, functions, visited);
			}
		}
		Expressions::IntrinsicCall { arguments, elements, .. } => {
			for argument in arguments {
				collect_called_functions(argument, functions, visited);
			}
			for element in elements {
				collect_called_functions(element, functions, visited);
			}
		}
		Expressions::Operator { left, right, .. } | Expressions::Accessor { left, right } => {
			collect_called_functions(left, functions, visited);
			collect_called_functions(right, functions, visited);
		}
		Expressions::Macro { body, .. } => collect_called_functions(body, functions, visited),
		Expressions::Continue
		| Expressions::Literal { .. }
		| Expressions::Member { .. }
		| Expressions::VariableDeclaration { .. } => {}
	}
}

/// Removes statements that cannot execute after a terminator in the same block.
fn cull_unreachable_statements(function: &NodeReference, report: &mut OptimizationReport) -> bool {
	let Nodes::Function { statements, .. } = cloned_node(function) else {
		return false;
	};
	cull_unreachable_statements_in_block(function, statements, report)
}

fn cull_unreachable_statements_in_block(
	block: &NodeReference,
	statements: Vec<NodeReference>,
	report: &mut OptimizationReport,
) -> bool {
	let mut kept = Vec::with_capacity(statements.len());
	let mut terminated = false;
	let mut changed = false;

	for statement in statements {
		if terminated {
			report.culled_unreachable_statements += 1;
			changed = true;
			continue;
		}

		changed |= cull_unreachable_statements_in_nested_block(&statement, report);
		terminated = is_block_terminator(&statement);
		kept.push(statement);
	}

	if changed {
		replace_block_statements(block, kept);
	}

	changed
}

fn cull_unreachable_statements_in_nested_block(statement: &NodeReference, report: &mut OptimizationReport) -> bool {
	match cloned_node(statement) {
		Nodes::Conditional { statements, .. } | Nodes::ForLoop { statements, .. } => {
			cull_unreachable_statements_in_block(statement, statements, report)
		}
		_ => false,
	}
}

fn is_block_terminator(statement: &NodeReference) -> bool {
	matches!(
		cloned_node(statement),
		Nodes::Expression(Expressions::Return { .. } | Expressions::Continue)
	)
}

/// Finds declaration assignments whose values have no remaining reader and no observable initializer effect.
fn collect_unused_local_declarations(function: &NodeReference, effects: &mut EffectAnalysis, removals: &mut HashSet<usize>) {
	let Nodes::Function { statements, .. } = cloned_node(function) else {
		return;
	};

	let mut candidates = Vec::new();
	collect_local_declaration_candidates(&statements, &mut candidates);
	for candidate in candidates {
		if !declaration_is_used(function, &candidate.declaration) && effects.is_pure(&candidate.initializer) {
			removals.insert(candidate.statement.identity());
		}
	}
}

struct LocalDeclarationCandidate {
	statement: NodeReference,
	declaration: NodeReference,
	initializer: NodeReference,
}

fn collect_local_declaration_candidates(statements: &[NodeReference], candidates: &mut Vec<LocalDeclarationCandidate>) {
	for statement in statements {
		if let Some((declaration, initializer)) = local_declaration_assignment(statement) {
			candidates.push(LocalDeclarationCandidate {
				statement: statement.clone(),
				declaration,
				initializer,
			});
		}

		match cloned_node(statement) {
			Nodes::Conditional { statements, .. } | Nodes::ForLoop { statements, .. } => {
				collect_local_declaration_candidates(&statements, candidates);
			}
			_ => {}
		}
	}
}

fn local_declaration_assignment(statement: &NodeReference) -> Option<(NodeReference, NodeReference)> {
	let Nodes::Expression(Expressions::Operator {
		operator: Operators::Assignment,
		left,
		right,
	}) = cloned_node(statement)
	else {
		return None;
	};

	if matches!(cloned_node(&left), Nodes::Expression(Expressions::VariableDeclaration { .. })) {
		Some((left, right))
	} else {
		None
	}
}

fn declaration_is_used(function: &NodeReference, declaration: &NodeReference) -> bool {
	let Nodes::Function { statements, .. } = cloned_node(function) else {
		return false;
	};
	let mut visited = HashSet::new();
	statements
		.iter()
		.any(|statement| node_uses_declaration(statement, declaration, &mut visited))
}

fn node_uses_declaration(node: &NodeReference, declaration: &NodeReference, visited: &mut HashSet<usize>) -> bool {
	if !visited.insert(node.identity()) {
		return false;
	}

	match cloned_node(node) {
		Nodes::Conditional { condition, statements } => {
			node_uses_declaration(&condition, declaration, visited)
				|| statements
					.iter()
					.any(|statement| node_uses_declaration(statement, declaration, visited))
		}
		Nodes::ForLoop {
			initializer,
			condition,
			update,
			statements,
		} => {
			node_uses_declaration(&initializer, declaration, visited)
				|| node_uses_declaration(&condition, declaration, visited)
				|| node_uses_declaration(&update, declaration, visited)
				|| statements
					.iter()
					.any(|statement| node_uses_declaration(statement, declaration, visited))
		}
		Nodes::Raw { input, .. } => input.iter().any(|input| input == declaration),
		Nodes::Expression(expression) => uses_declaration_in_expression(&expression, declaration, visited),
		_ => false,
	}
}

fn uses_declaration_in_expression(expression: &Expressions, declaration: &NodeReference, visited: &mut HashSet<usize>) -> bool {
	match expression {
		Expressions::Member { source, .. } => source == declaration,
		Expressions::Return { value } => value
			.as_ref()
			.is_some_and(|value| node_uses_declaration(value, declaration, visited)),
		Expressions::Expression { elements } => elements
			.iter()
			.any(|element| node_uses_declaration(element, declaration, visited)),
		Expressions::FunctionCall { parameters, .. } => parameters
			.iter()
			.any(|parameter| node_uses_declaration(parameter, declaration, visited)),
		Expressions::IntrinsicCall { arguments, elements, .. } => {
			arguments
				.iter()
				.any(|argument| node_uses_declaration(argument, declaration, visited))
				|| elements
					.iter()
					.any(|element| node_uses_declaration(element, declaration, visited))
		}
		Expressions::Operator { left, right, .. } | Expressions::Accessor { left, right } => {
			node_uses_declaration(left, declaration, visited) || node_uses_declaration(right, declaration, visited)
		}
		Expressions::Macro { body, .. } => node_uses_declaration(body, declaration, visited),
		Expressions::Continue | Expressions::Literal { .. } | Expressions::VariableDeclaration { .. } => false,
	}
}

fn remove_statements(function: &NodeReference, removals: &HashSet<usize>) -> usize {
	let Nodes::Function { statements, .. } = cloned_node(function) else {
		return 0;
	};
	remove_statements_in_block(function, statements, removals)
}

fn remove_statements_in_block(block: &NodeReference, statements: Vec<NodeReference>, removals: &HashSet<usize>) -> usize {
	let mut removed = 0;
	let mut kept = Vec::with_capacity(statements.len());
	for statement in statements {
		if removals.contains(&statement.identity()) {
			removed += 1;
			continue;
		}

		removed += remove_statements_in_nested_block(&statement, removals);
		kept.push(statement);
	}

	if removed > 0 {
		replace_block_statements(block, kept);
	}

	removed
}

fn remove_statements_in_nested_block(statement: &NodeReference, removals: &HashSet<usize>) -> usize {
	match cloned_node(statement) {
		Nodes::Conditional { statements, .. } | Nodes::ForLoop { statements, .. } => {
			remove_statements_in_block(statement, statements, removals)
		}
		_ => 0,
	}
}

fn replace_block_statements(block: &NodeReference, statements: Vec<NodeReference>) {
	match block.borrow_mut().node_mut() {
		Nodes::Function {
			statements: existing, ..
		}
		| Nodes::Conditional {
			statements: existing, ..
		}
		| Nodes::ForLoop {
			statements: existing, ..
		} => *existing = statements,
		_ => unreachable!("Only function and control-flow nodes own statement blocks"),
	}
}

/// Tracks whether expressions can be removed without changing externally visible shader behavior.
#[derive(Default)]
struct EffectAnalysis {
	function_purity: HashMap<usize, bool>,
	active_functions: HashSet<usize>,
}

impl EffectAnalysis {
	fn is_pure(&mut self, node: &NodeReference) -> bool {
		match cloned_node(node) {
			Nodes::Scope { .. }
			| Nodes::Null
			| Nodes::Member { .. }
			| Nodes::Binding { .. }
			| Nodes::PushConstant { .. }
			| Nodes::Input { .. }
			| Nodes::Output { .. }
			| Nodes::TaskPayload { .. }
			| Nodes::Workgroup { .. }
			| Nodes::Parameter { .. }
			| Nodes::Specialization { .. }
			| Nodes::Literal { .. } => true,
			Nodes::Const { value, .. } => self.is_pure(&value),
			Nodes::Raw { .. } => false,
			Nodes::Struct { .. } => true,
			Nodes::Function { .. } => self.is_pure_function(node),
			Nodes::Conditional { condition, statements } => {
				self.is_pure(&condition) && statements.iter().all(|statement| self.is_pure(statement))
			}
			// A loop can change shader termination even when its body only contains arithmetic.
			Nodes::ForLoop { .. } => false,
			Nodes::Intrinsic { .. } => false,
			Nodes::Expression(expression) => self.is_pure_expression(&expression),
		}
	}

	fn is_pure_expression(&mut self, expression: &Expressions) -> bool {
		match expression {
			Expressions::Continue => false,
			Expressions::Return { value } => value.as_ref().is_none_or(|value| self.is_pure(value)),
			Expressions::Member { .. } | Expressions::Literal { .. } | Expressions::VariableDeclaration { .. } => true,
			Expressions::Expression { elements } => elements.iter().all(|element| self.is_pure(element)),
			Expressions::FunctionCall { function, parameters } => {
				parameters.iter().all(|parameter| self.is_pure(parameter)) && self.callable_is_pure(function)
			}
			Expressions::IntrinsicCall {
				intrinsic,
				arguments,
				elements,
			} => {
				arguments.iter().all(|argument| self.is_pure(argument))
					&& elements.iter().all(|element| self.is_pure(element))
					&& self.intrinsic_is_pure(intrinsic)
			}
			Expressions::Operator { operator, left, right } => {
				if *operator == Operators::Assignment && !assignment_target_is_local(left) {
					return false;
				}
				self.is_pure(left) && self.is_pure(right)
			}
			Expressions::Accessor { left, right } => self.is_pure(left) && self.is_pure(right),
			Expressions::Macro { body, .. } => self.is_pure(body),
		}
	}

	fn callable_is_pure(&mut self, callable: &NodeReference) -> bool {
		match cloned_node(callable) {
			Nodes::Function { .. } => self.is_pure_function(callable),
			Nodes::Struct { .. } => true,
			_ => false,
		}
	}

	fn is_pure_function(&mut self, function: &NodeReference) -> bool {
		let function_id = function.identity();
		if let Some(pure) = self.function_purity.get(&function_id) {
			return *pure;
		}
		// Recursive calls could diverge, so preserve them unless a future pass proves otherwise.
		if !self.active_functions.insert(function_id) {
			return false;
		}

		let pure = match cloned_node(function) {
			Nodes::Function { statements, .. } => statements.iter().all(|statement| self.is_pure(statement)),
			_ => false,
		};
		self.active_functions.remove(&function_id);
		self.function_purity.insert(function_id, pure);
		pure
	}

	fn intrinsic_is_pure(&mut self, intrinsic: &NodeReference) -> bool {
		let Nodes::Intrinsic { name, elements, .. } = cloned_node(intrinsic) else {
			return false;
		};

		let has_body = elements
			.iter()
			.any(|element| !matches!(cloned_node(element), Nodes::Parameter { .. }));
		if has_body {
			return elements
				.iter()
				.filter(|element| !matches!(cloned_node(element), Nodes::Parameter { .. }))
				.all(|element| self.is_pure(element));
		}

		matches!(
			name.as_str(),
			"sample"
				| "texture_lod"
				| "downsample_min"
				| "downsample_max"
				| "fetch" | "fetch_u32"
				| "dot" | "cross"
				| "length" | "normalize"
				| "max" | "min"
				| "clamp" | "log2"
				| "pow" | "reflect"
				| "abs" | "sqrt"
				| "exp" | "sin"
				| "cos" | "sincos"
				| "tan" | "asin"
				| "atan2" | "floor"
				| "round" | "round_to_i32"
				| "fma" | "fract"
				| "fwidth" | "radians"
				| "inversesqrt"
				| "f32" | "u32"
				| "smoothstep"
				| "step" | "mix"
				| "thread_idx"
				| "threadgroup_position"
				| "thread_position"
				| "thread_id"
				| "image_load"
				| "image_load_u32"
				| "atomic_load"
				| "texture_size"
				| "image_size"
		)
	}
}

fn assignment_target_is_local(node: &NodeReference) -> bool {
	match cloned_node(node) {
		Nodes::Expression(Expressions::VariableDeclaration { .. }) => true,
		Nodes::Expression(Expressions::Member { source, .. }) => {
			matches!(
				cloned_node(&source),
				Nodes::Expression(Expressions::VariableDeclaration { .. })
			)
		}
		Nodes::Expression(Expressions::Accessor { left, .. }) => assignment_target_is_local(&left),
		_ => false,
	}
}

fn cloned_node(node: &NodeReference) -> Nodes {
	node.borrow().node().clone()
}

#[cfg(test)]
mod tests {
	use super::{optimize, OptimizationReport};
	use crate::{
		compile_to_besl,
		vm::{Buffer, DescriptorBindings, ExecutableProgram, ResourceSlot, Value},
		BindingTypes, Expressions, Node, Nodes,
	};

	fn main(source: &str) -> crate::NodeReference {
		let program = compile_to_besl(source, None).expect("Expected BESL source to link");
		program.get_main().expect("Expected main function")
	}

	fn statements(function: &crate::NodeReference) -> Vec<crate::NodeReference> {
		let function = function.borrow();
		let Nodes::Function { statements, .. } = function.node() else {
			panic!("Expected function");
		};
		statements.clone()
	}

	#[test]
	fn culls_an_unused_pure_local_and_its_function() {
		let main = main(
			r#"
			Foo: struct {
				value: f32,
			}
			expensive: fn() -> Foo {
				return Foo(42.0);
			}
			main: fn() -> void {
				let x: Foo = expensive();
				return;
			}
		"#,
		);

		assert_eq!(
			optimize(&main),
			OptimizationReport {
				culled_unused_local_variables: 1,
				culled_unreachable_statements: 0,
			}
		);
		assert!(matches!(
			statements(&main).as_slice(),
			[statement] if matches!(statement.borrow().node(), Nodes::Expression(Expressions::Return { .. }))
		));
	}

	#[test]
	fn retains_a_local_that_contributes_to_the_return_value() {
		let main = main(
			r#"
			main: fn() -> f32 {
				let x: f32 = 42.0;
				return x;
			}
		"#,
		);

		assert_eq!(optimize(&main), OptimizationReport::default());
		assert_eq!(statements(&main).len(), 2);
	}

	#[test]
	fn culls_dead_local_chains_to_a_fixed_point() {
		let main = main(
			r#"
			main: fn() -> void {
				let first: f32 = 1.0;
				let second: f32 = first;
				return;
			}
		"#,
		);

		let report = optimize(&main);
		assert_eq!(report.culled_unused_local_variables, 2);
		assert_eq!(statements(&main).len(), 1);
	}

	#[test]
	fn preserves_unused_locals_with_atomic_side_effects() {
		let main = main(
			r#"
			Counters: struct {
				value: atomicu32,
			}
			counters: descriptor<Counters, 0, read_write>;
			increment: fn() -> u32 {
				return atomic_add(counters.value, 1);
			}
			main: fn() -> void {
				let previous: u32 = increment();
				return;
			}
		"#,
		);

		assert_eq!(optimize(&main), OptimizationReport::default());
		assert_eq!(statements(&main).len(), 2);
	}

	#[test]
	fn preserves_dead_atomic_initializer_behavior_in_the_vm() {
		let mut root = Node::root();
		let u32_type = root.get_child("u32").expect("Expected u32 type");
		let atomic_u32_type = root.add_child(Node::r#struct("atomicu32", Vec::new()).into());
		root.add_children(vec![
			Node::binding(
				"counter",
				BindingTypes::Buffer {
					members: vec![Node::member("count", atomic_u32_type.clone()).into()],
				},
				0,
				true,
				true,
			)
			.into(),
			Node::binding(
				"result",
				BindingTypes::Buffer {
					members: vec![Node::member("value", u32_type.clone()).into()],
				},
				1,
				false,
				true,
			)
			.into(),
		]);
		let atomic_add = root.add_child(Node::intrinsic("atomic_add", Vec::new(), u32_type.clone()).into());
		atomic_add.borrow_mut().add_children(vec![
			Node::new(Nodes::Parameter {
				name: "value".to_string(),
				r#type: atomic_u32_type,
			})
			.into(),
			Node::new(Nodes::Parameter {
				name: "increment".to_string(),
				r#type: u32_type,
			})
			.into(),
		]);

		let program = compile_to_besl(
			r#"
			main: fn() -> void {
				let ignored: u32 = atomic_add(counter.count, 1);
				result.value = 7;
			}
		"#,
			Some(root),
		)
		.expect("Expected atomic side-effect fixture to link");
		let main = program.get_main().expect("Expected atomic side-effect main function");
		assert_eq!(optimize(&main), OptimizationReport::default());

		let executable = ExecutableProgram::compile(program).expect("Expected optimized fixture to compile for the VM");
		let counter_slot = ResourceSlot::new(0);
		let result_slot = ResourceSlot::new(1);
		let mut counter = Buffer::new(
			executable
				.buffer_layout(counter_slot)
				.expect("Expected counter layout")
				.clone(),
		);
		let mut result = Buffer::new(executable.buffer_layout(result_slot).expect("Expected result layout").clone());
		counter
			.write("count", Value::U32(0))
			.expect("Expected counter initialization");

		let mut descriptors = DescriptorBindings::new();
		descriptors.bind_buffer(counter_slot, &mut counter);
		descriptors.bind_buffer(result_slot, &mut result);
		executable.run_main(&mut descriptors).expect("Expected VM execution");

		assert_eq!(counter.read("count").expect("Expected counter value"), Value::U32(1));
		assert_eq!(result.read("value").expect("Expected result value"), Value::U32(7));
	}

	#[test]
	fn culls_unreachable_statements_even_when_they_have_side_effects() {
		let main = main(
			r#"
			Counters: struct {
				value: atomicu32,
			}
			counters: descriptor<Counters, 0, read_write>;
			main: fn() -> void {
				return;
				let previous: u32 = atomic_add(counters.value, 1);
			}
		"#,
		);

		let report = optimize(&main);
		assert_eq!(report.culled_unreachable_statements, 1);
		assert_eq!(statements(&main).len(), 1);
	}

	#[test]
	fn culls_unreachable_locals_inside_nested_blocks() {
		let main = main(
			r#"
			main: fn() -> void {
				if (true) {
					return;
					let x: f32 = 1.0;
				}
			}
		"#,
		);

		let report = optimize(&main);
		assert_eq!(report.culled_unreachable_statements, 1);
		let main_statements = statements(&main);
		let [conditional] = main_statements.as_slice() else {
			panic!("Expected one conditional statement");
		};
		let conditional = conditional.borrow();
		let Nodes::Conditional { statements, .. } = conditional.node() else {
			panic!("Expected conditional statement");
		};
		assert_eq!(statements.len(), 1);
		assert!(matches!(
			statements[0].borrow().node(),
			Nodes::Expression(Expressions::Return { .. })
		));
	}

	#[test]
	fn is_idempotent_after_the_first_optimization() {
		let main = main(
			r#"
			main: fn() -> void {
				let x: f32 = 1.0;
				return;
			}
		"#,
		);

		assert_eq!(optimize(&main).culled_unused_local_variables, 1);
		assert_eq!(optimize(&main), OptimizationReport::default());
	}
}
