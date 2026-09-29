//! Optimizes linked BESL programs before reflection and backend lowering.
//!
//! The pass works on the linked semantic tree, where each variable use refers
//! directly to its declaration. That lets it remove dead local declarations
//! without relying on names or backend-specific syntax.

use std::collections::{HashMap, HashSet};

use crate::{ElseBranch, Expressions, NodeReference, Nodes, Operators};

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
	let functions = main_function_node.reachable_functions();
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

/// Removes statements that cannot execute after a terminator in the same block.
fn cull_unreachable_statements(function: &NodeReference, report: &mut OptimizationReport) -> bool {
	update_blocks(function, &mut |statements| {
		cull_unreachable_statements_in_block(statements, report)
	})
}

fn cull_unreachable_statements_in_block(statements: &mut Vec<NodeReference>, report: &mut OptimizationReport) -> bool {
	let mut changed = false;

	if let Some(terminator) = statements.iter().position(is_block_terminator) {
		report.culled_unreachable_statements += statements.len() - terminator - 1;
		changed |= terminator + 1 < statements.len();
		statements.truncate(terminator + 1);
	}

	for statement in statements.iter() {
		changed |= update_blocks(statement, &mut |statements| {
			cull_unreachable_statements_in_block(statements, report)
		});
	}

	changed
}

fn is_block_terminator(statement: &NodeReference) -> bool {
	matches!(
		statement.borrow().node(),
		Nodes::Expression(Expressions::Return { .. } | Expressions::Continue | Expressions::Break | Expressions::Discard)
	)
}

/// Finds declaration assignments whose values have no remaining reader and no observable initializer effect.
fn collect_unused_local_declarations(function: &NodeReference, effects: &mut EffectAnalysis, removals: &mut HashSet<usize>) {
	let function_ref = function.borrow();
	let Nodes::Function { statements, .. } = function_ref.node() else {
		return;
	};

	// One walk finds every declaration still read, so each candidate is a set lookup instead of a function walk.
	let mut used = HashSet::new();
	let mut visited = HashSet::new();
	for statement in statements {
		collect_used_declarations(statement, &mut used, &mut visited);
	}

	let mut candidates = Vec::new();
	collect_local_declaration_candidates(statements, &mut candidates);
	for candidate in candidates {
		if !used.contains(&candidate.declaration.identity()) && effects.is_pure(&candidate.initializer) {
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

		match statement.borrow().node() {
			Nodes::Conditional {
				statements, else_branch, ..
			} => {
				collect_local_declaration_candidates(statements, candidates);
				if let Some(else_branch) = else_branch {
					collect_local_declaration_candidates(else_branch.statements(), candidates);
				}
			}
			Nodes::Match { arms, default, .. } => {
				for arm in arms {
					collect_local_declaration_candidates(&arm.statements, candidates);
				}
				collect_local_declaration_candidates(default, candidates);
			}
			Nodes::ForLoop { statements, .. } => {
				collect_local_declaration_candidates(statements, candidates);
			}
			_ => {}
		}
	}
}

fn local_declaration_assignment(statement: &NodeReference) -> Option<(NodeReference, NodeReference)> {
	let statement = statement.borrow();
	let Nodes::Expression(Expressions::Operator {
		operator: Operators::Assignment,
		left,
		right,
	}) = statement.node()
	else {
		return None;
	};

	matches!(
		left.borrow().node(),
		Nodes::Expression(Expressions::VariableDeclaration { .. })
	)
	.then(|| (left.clone(), right.clone()))
}

/// Records the identity of every declaration that `node` reads, through member accesses and raw-code inputs.
///
/// A declaration's own `VariableDeclaration` is not a read. `visited` skips subtrees shared between an intrinsic
/// call's arguments and its inlined body.
fn collect_used_declarations(node: &NodeReference, used: &mut HashSet<usize>, visited: &mut HashSet<usize>) {
	if !visited.insert(node.identity()) {
		return;
	}

	let node = node.borrow();
	match node.node() {
		Nodes::Expression(Expressions::Member { source, .. }) => {
			used.insert(source.identity());
		}
		Nodes::Raw { input, .. } => used.extend(input.iter().map(NodeReference::identity)),
		// Constants are declarations read through members, not statements that run in the function.
		Nodes::Const { .. } => {}
		other => {
			for child in other.children() {
				collect_used_declarations(child, used, visited);
			}
		}
	}
}

fn remove_statements(function: &NodeReference, removals: &HashSet<usize>) -> usize {
	let mut removed = 0;
	update_blocks(function, &mut |statements| {
		remove_statements_in_block(statements, removals, &mut removed)
	});
	removed
}

/// Removes the statements in `removals` from one block and its nested blocks. Returns whether anything was removed.
fn remove_statements_in_block(statements: &mut Vec<NodeReference>, removals: &HashSet<usize>, removed: &mut usize) -> bool {
	let length = statements.len();
	statements.retain(|statement| !removals.contains(&statement.identity()));
	*removed += length - statements.len();
	let mut changed = length != statements.len();

	for statement in statements.iter() {
		changed |= update_blocks(statement, &mut |statements| {
			remove_statements_in_block(statements, removals, removed)
		});
	}

	changed
}

/// Applies `update` in place to each statement block that `node` owns, including the blocks of `else if` links
/// and `match` arms.
/// Returns whether any block changed. Nodes without statement blocks are left untouched.
fn update_blocks(node: &NodeReference, update: &mut dyn FnMut(&mut Vec<NodeReference>) -> bool) -> bool {
	// `update` only borrows the block's statements, which are separate nodes, so holding this borrow is safe.
	match node.borrow_mut().node_mut() {
		Nodes::Function { statements, .. } | Nodes::ForLoop { statements, .. } => update(statements),
		Nodes::Conditional {
			statements, else_branch, ..
		} => {
			update(statements)
				| match else_branch {
					Some(ElseBranch::Block(statements)) => update(statements),
					Some(ElseBranch::If(conditional)) => update_blocks(conditional, update),
					None => false,
				}
		}
		Nodes::Match { arms, default, .. } => arms
			.iter_mut()
			.map(|arm| &mut arm.statements)
			.chain([default])
			.fold(false, |changed, statements| update(statements) | changed),
		_ => false,
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
		let node_ref = node.borrow();
		match node_ref.node() {
			Nodes::Function { .. } => self.is_pure_function(node),
			// Raw code is opaque, a loop can change shader termination even when its body only contains arithmetic, and
			// control transfers change which statements run.
			Nodes::Raw { .. }
			| Nodes::ForLoop { .. }
			| Nodes::Intrinsic { .. }
			| Nodes::Expression(Expressions::Continue | Expressions::Break | Expressions::Discard) => false,
			Nodes::Expression(Expressions::FunctionCall { function, parameters }) => {
				parameters.iter().all(|parameter| self.is_pure(parameter)) && self.callable_is_pure(&function.get())
			}
			Nodes::Expression(Expressions::IntrinsicCall { intrinsic, .. }) => {
				node_ref.node().children().all(|child| self.is_pure(child)) && self.intrinsic_is_pure(intrinsic)
			}
			Nodes::Expression(Expressions::Operator {
				operator: Operators::Assignment,
				left,
				..
			}) if !assignment_target_is_local(left) => false,
			// Declarations and value reads have no effect of their own; every other node is as pure as its parts.
			other => other.children().all(|child| self.is_pure(child)),
		}
	}

	fn callable_is_pure(&mut self, callable: &NodeReference) -> bool {
		match callable.borrow().node() {
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

		let pure = match function.borrow().node() {
			Nodes::Function { statements, .. } => statements.iter().all(|statement| self.is_pure(statement)),
			_ => false,
		};
		self.active_functions.remove(&function_id);
		self.function_purity.insert(function_id, pure);
		pure
	}

	fn intrinsic_is_pure(&mut self, intrinsic: &NodeReference) -> bool {
		let intrinsic = intrinsic.borrow();
		let Nodes::Intrinsic { name, elements, .. } = intrinsic.node() else {
			return false;
		};

		let mut body = elements
			.iter()
			.filter(|element| !matches!(element.borrow().node(), Nodes::Parameter { .. }))
			.peekable();
		if body.peek().is_some() {
			return body.all(|element| self.is_pure(element));
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
				| "find_lsb" | "pow"
				| "reflect" | "abs"
				| "sqrt" | "is_nan"
				| "is_infinite"
				| "is_finite"
				| "is_normal"
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
	match node.borrow().node() {
		Nodes::Expression(Expressions::VariableDeclaration { .. }) => true,
		Nodes::Expression(Expressions::Member { source, .. }) => {
			matches!(
				source.borrow().node(),
				Nodes::Expression(Expressions::VariableDeclaration { .. })
			)
		}
		Nodes::Expression(Expressions::Accessor { left, .. }) => assignment_target_is_local(left),
		_ => false,
	}
}

#[cfg(test)]
mod tests {
	use super::{OptimizationReport, optimize};
	use crate::{
		BindingTypes, Expressions, Node, Nodes, compile_to_besl,
		vm::{Buffer, DescriptorBindings, ExecutableProgram, ResourceSlot, Value},
	};

	/// Returns the linked program with its `main` function. Keep the program alive: it owns the functions `main` calls.
	fn main(source: &str) -> (crate::NodeReference, crate::NodeReference) {
		let program = compile_to_besl(source, None).expect("Expected BESL source to link");
		let main = program.get_main().expect("Expected main function");
		(program, main)
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
		let (_program, main) = main(
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
		let (_program, main) = main(
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
		let (_program, main) = main(
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
		let (_program, main) = main(
			r#"
			Counters: struct {
				value: atomicu32,
			}
			counters: descriptor<{ type: Counters, binding: 0, access: read_write }>;
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
		let (_program, main) = main(
			r#"
			Counters: struct {
				value: atomicu32,
			}
			counters: descriptor<{ type: Counters, binding: 0, access: read_write }>;
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
		let (_program, main) = main(
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
		let (_program, main) = main(
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
