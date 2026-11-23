//! Optimizes linked BESL programs before reflection and backend lowering.
//!
//! The pass works on the linked semantic tree, where each variable use refers
//! directly to its declaration. That lets it remove dead local declarations
//! without relying on names or backend-specific syntax.


use crate::{ElseBranch, Expressions, NodeId, NodeMap, NodeSet, Nodes, Operators, Program};

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
pub fn optimize(program: &mut Program, main_function_node: &NodeId) -> OptimizationReport {
	let functions = program.reachable_functions(*main_function_node);
	let mut report = OptimizationReport::default();

	loop {
		let mut changed = false;
		for function in &functions {
			changed |= cull_unreachable_statements(program, function, &mut report);
		}

		let mut effects = EffectAnalysis::new(program);
		let mut removals = NodeSet::for_program(program);
		for function in &functions {
			collect_unused_local_declarations(program, function, &mut effects, &mut removals);
		}

		if removals.is_empty() {
			if !changed {
				break;
			}
			continue;
		}

		for function in &functions {
			report.culled_unused_local_variables += remove_statements(program, function, &removals);
		}
	}

	report
}

/// Removes statements that cannot execute after a terminator in the same block.
fn cull_unreachable_statements(program: &mut Program, function: &NodeId, report: &mut OptimizationReport) -> bool {
	update_blocks(program, function, &mut |program, statements| {
		cull_unreachable_statements_in_block(program, statements, report)
	})
}

fn cull_unreachable_statements_in_block(program: &mut Program, statements: &mut Vec<NodeId>, report: &mut OptimizationReport) -> bool {
	let mut changed = false;

	if let Some(terminator) = statements.iter().position(|statement| is_block_terminator(program, statement)) {
		report.culled_unreachable_statements += statements.len() - terminator - 1;
		changed |= terminator + 1 < statements.len();
		statements.truncate(terminator + 1);
	}

	for statement in statements.iter() {
		changed |= update_blocks(program, statement, &mut |program, statements| {
			cull_unreachable_statements_in_block(program, statements, report)
		});
	}

	changed
}

fn is_block_terminator(program: &Program, statement: &NodeId) -> bool {
	matches!(
		program[statement].node(),
		Nodes::Expression(Expressions::Return { .. } | Expressions::Continue | Expressions::Break | Expressions::Discard)
	)
}

/// Finds declaration assignments whose values have no remaining reader and no observable initializer effect.
fn collect_unused_local_declarations(program: &Program, function: &NodeId, effects: &mut EffectAnalysis<'_>, removals: &mut NodeSet) {
	let function_ref = &program[function];
	let Nodes::Function { statements, .. } = function_ref.node() else {
		return;
	};

	// One walk finds every declaration still read, so each candidate is a set lookup instead of a function walk.
	let mut used = NodeSet::for_program(program);
	let mut visited = NodeSet::for_program(program);
	for statement in statements {
		collect_used_declarations(program, statement, &mut used, &mut visited);
	}

	let mut candidates = Vec::new();
	collect_local_declaration_candidates(program, statements, &mut candidates);
	for candidate in candidates {
		if !used.contains(candidate.declaration) && effects.is_pure(&candidate.initializer) {
			removals.insert(candidate.statement);
		}
	}
}

struct LocalDeclarationCandidate {
	statement: NodeId,
	declaration: NodeId,
	initializer: NodeId,
}

fn collect_local_declaration_candidates(program: &Program, statements: &[NodeId], candidates: &mut Vec<LocalDeclarationCandidate>) {
	for statement in statements {
		if let Some((declaration, initializer)) = local_declaration_assignment(program, statement) {
			candidates.push(LocalDeclarationCandidate {
				statement: *statement,
				declaration,
				initializer,
			});
		}

		match program[statement].node() {
			Nodes::Conditional {
				statements, else_branch, ..
			} => {
				collect_local_declaration_candidates(program, statements, candidates);
				if let Some(else_branch) = else_branch {
					collect_local_declaration_candidates(program, else_branch.statements(), candidates);
				}
			}
			Nodes::Match { arms, default, .. } => {
				for arm in arms {
					collect_local_declaration_candidates(program, &arm.statements, candidates);
				}
				collect_local_declaration_candidates(program, default, candidates);
			}
			Nodes::ForLoop { statements, .. } => {
				collect_local_declaration_candidates(program, statements, candidates);
			}
			_ => {}
		}
	}
}

fn local_declaration_assignment(program: &Program, statement: &NodeId) -> Option<(NodeId, NodeId)> {
	let statement = &program[statement];
	let Nodes::Expression(Expressions::Operator {
		operator: Operators::Assignment,
		left,
		right,
	}) = statement.node()
	else {
		return None;
	};

	matches!(
		program[left].node(),
		Nodes::Expression(Expressions::VariableDeclaration { .. })
	)
	.then(|| (*left, *right))
}

/// Records the identity of every declaration that `node` reads, through member accesses and raw-code inputs.
///
/// A declaration's own `VariableDeclaration` is not a read. `visited` skips subtrees shared between an intrinsic
/// call's arguments and its inlined body.
fn collect_used_declarations(program: &Program, node: &NodeId, used: &mut NodeSet, visited: &mut NodeSet) {
	if !visited.insert(*node) {
		return;
	}

	let node = &program[node];
	match node.node() {
		Nodes::Expression(Expressions::Member { source, .. }) => {
			used.insert(*source);
		}
		// Constants are declarations read through members, not statements that run in the function.
		Nodes::Const { .. } => {}
		other => {
			for child in other.children() {
				collect_used_declarations(program, child, used, visited);
			}
		}
	}
}

fn remove_statements(program: &mut Program, function: &NodeId, removals: &NodeSet) -> usize {
	let mut removed = 0;
	update_blocks(program, function, &mut |program, statements| {
		remove_statements_in_block(program, statements, removals, &mut removed)
	});
	removed
}

/// Removes the statements in `removals` from one block and its nested blocks. Returns whether anything was removed.
fn remove_statements_in_block(program: &mut Program, statements: &mut Vec<NodeId>, removals: &NodeSet, removed: &mut usize) -> bool {
	let length = statements.len();
	statements.retain(|statement| !removals.contains(*statement));
	*removed += length - statements.len();
	let mut changed = length != statements.len();

	for statement in statements.iter() {
		changed |= update_blocks(program, statement, &mut |program, statements| {
			remove_statements_in_block(program, statements, removals, removed)
		});
	}

	changed
}

/// Applies `update` to each statement block that `node` owns, including the blocks of `else if` links and `match`
/// arms. Returns whether any block changed. Nodes without statement blocks are left untouched.
///
/// Each block is taken out of its node while `update` runs, so `update` can read and change the rest of the program,
/// and is put back afterwards. Taking a `Vec` out leaves an empty one behind, which allocates nothing.
fn update_blocks(program: &mut Program, node: &NodeId, update: &mut dyn FnMut(&mut Program, &mut Vec<NodeId>) -> bool) -> bool {
	let mut changed = false;
	let mut index = 0;
	while let Some(block) = block_mut(program[node].node_mut(), index) {
		let mut statements = std::mem::take(block);
		changed |= update(program, &mut statements);
		*block_mut(program[node].node_mut(), index).expect("The block was found above") = statements;
		index += 1;
	}
	if let Nodes::Conditional {
		else_branch: Some(ElseBranch::If(conditional)),
		..
	} = program[node].node()
	{
		let conditional = *conditional;
		changed |= update_blocks(program, &conditional, update);
	}
	changed
}

/// Returns the `index`-th statement block that `node` owns, in the order [`update_blocks`] visits them, or `None` past
/// the last one.
fn block_mut(node: &mut Nodes, index: usize) -> Option<&mut Vec<NodeId>> {
	match node {
		Nodes::Function { statements, .. } | Nodes::ForLoop { statements, .. } => (index == 0).then_some(statements),
		Nodes::Conditional {
			statements, else_branch, ..
		} => match (index, else_branch) {
			(0, _) => Some(statements),
			(1, Some(ElseBranch::Block(statements))) => Some(statements),
			_ => None,
		},
		Nodes::Match { arms, default, .. } => match index.cmp(&arms.len()) {
			std::cmp::Ordering::Less => Some(&mut arms[index].statements),
			std::cmp::Ordering::Equal => Some(default),
			std::cmp::Ordering::Greater => None,
		},
		_ => None,
	}
}

/// Tracks whether expressions can be removed without changing externally visible shader behavior.
struct EffectAnalysis<'p> {
	program: &'p Program,
	function_purity: NodeMap<bool>,
	active_functions: NodeSet,
}

impl<'p> EffectAnalysis<'p> {
	fn new(program: &'p Program) -> Self {
		Self {
			program,
			function_purity: NodeMap::default(),
			active_functions: NodeSet::default(),
		}
	}

	fn is_pure(&mut self, node: &NodeId) -> bool {
		let node_ref = &self.program[node];
		match node_ref.node() {
			Nodes::Function { .. } => self.is_pure_function(node),
			// A loop can change shader termination even when its body only contains arithmetic, and control transfers
			// change which statements run.
			Nodes::ForLoop { .. }
			| Nodes::Intrinsic { .. }
			| Nodes::Expression(Expressions::Continue | Expressions::Break | Expressions::Discard) => false,
			Nodes::Expression(Expressions::FunctionCall { function, parameters }) => {
				parameters.iter().all(|parameter| self.is_pure(parameter)) && self.callable_is_pure(function)
			}
			Nodes::Expression(Expressions::IntrinsicCall { intrinsic, .. }) => {
				node_ref.node().children().all(|child| self.is_pure(child)) && self.intrinsic_is_pure(intrinsic)
			}
			Nodes::Expression(Expressions::Operator {
				operator: Operators::Assignment,
				left,
				..
			}) if !assignment_target_is_local(self.program, left) => false,
			// Declarations and value reads have no effect of their own; every other node is as pure as its parts.
			other => other.children().all(|child| self.is_pure(child)),
		}
	}

	fn callable_is_pure(&mut self, callable: &NodeId) -> bool {
		match self.program[callable].node() {
			Nodes::Function { .. } => self.is_pure_function(callable),
			Nodes::Struct { .. } => true,
			_ => false,
		}
	}

	fn is_pure_function(&mut self, function: &NodeId) -> bool {
		let function_id = *function;
		if let Some(pure) = self.function_purity.get(&function_id) {
			return *pure;
		}
		// Recursive calls could diverge, so preserve them unless a future pass proves otherwise.
		if !self.active_functions.insert(function_id) {
			return false;
		}

		let pure = match self.program[function].node() {
			Nodes::Function { statements, .. } => statements.iter().all(|statement| self.is_pure(statement)),
			_ => false,
		};
		self.active_functions.remove(function_id);
		self.function_purity.insert(function_id, pure);
		pure
	}

	fn intrinsic_is_pure(&mut self, intrinsic: &NodeId) -> bool {
		let intrinsic = &self.program[intrinsic];
		let Nodes::Intrinsic { name, elements, .. } = intrinsic.node() else {
			return false;
		};

		let mut body = elements
			.iter()
			.filter(|element| !matches!(self.program[*element].node(), Nodes::Parameter { .. }))
			.peekable();
		if body.peek().is_some() {
			return body.all(|element| self.is_pure(element));
		}

		matches!(
			self.program.text(*name),
			"sample"
				| "texture_lod"
				| "downsample_min"
				| "downsample_max"
				| "gather" | "fetch"
				| "fetch_u32"
				| "dot" | "cross"
				| "length" | "normalize"
				| "max" | "min"
				| "clamp" | "log2"
				| "find_lsb" | "pow"
				| "reflect" | "abs"
				| "sign" | "exp2"
				| "distance"
				| "sqrt" | "is_nan"
				| "is_infinite"
				| "is_finite"
				| "is_normal"
				| "exp" | "sin"
				| "cos" | "sincos"
				| "tan" | "asin"
				| "atan2" | "floor" | "ceil"
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
				| "thread_layer"
				| "output_extent"
				| "image_load"
				| "image_load_u32"
				| "atomic_load"
				| "texture_size"
				| "image_size"
				// Ray queries only read the scene. `invoke` runs a closest hit, which may write resources, so it stays
				// impure.
				| "intersect"
				| "trace_occlusion"
				| "launch_index"
				| "launch_size"
				| "hit_distance"
				| "hit_custom_index"
				| "hit_geometry_index"
				| "hit_primitive_index"
				| "hit_barycentrics"
				| "ray_origin"
				| "ray_direction"
		)
	}
}

fn assignment_target_is_local(program: &Program, node: &NodeId) -> bool {
	match program[node].node() {
		Nodes::Expression(Expressions::VariableDeclaration { .. }) => true,
		Nodes::Expression(Expressions::Member { source, .. }) => {
			matches!(
				program[source].node(),
				Nodes::Expression(Expressions::VariableDeclaration { .. })
			)
		}
		Nodes::Expression(Expressions::Accessor { left, .. }) => assignment_target_is_local(program, left),
		_ => false,
	}
}

#[cfg(test)]
mod tests {
	use super::{OptimizationReport, optimize};
	use crate::{
		BindingTypes, Expressions, Node, NodeId, Nodes, Program, compile_to_besl,
		vm::{Buffer, DescriptorBindings, ExecutableProgram, ResourceSlot, Value},
	};

	/// Returns the linked program with its `main` function.
	fn main(source: &str) -> (Program, NodeId) {
		let program = compile_to_besl(source, None).expect("Expected BESL source to link");
		let main = program.get_main().expect("Expected main function");
		(program, main)
	}

	fn statements(program: &Program, function: NodeId) -> Vec<NodeId> {
		let Nodes::Function { statements, .. } = program[function].node() else {
			panic!("Expected function");
		};
		statements.clone()
	}

	#[test]
	fn culls_an_unused_pure_local_and_its_function() {
		let (mut program, main) = main(
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
			optimize(&mut program, &main),
			OptimizationReport {
				culled_unused_local_variables: 1,
				culled_unreachable_statements: 0,
			}
		);
		assert!(matches!(
			statements(&program, main).as_slice(),
			[statement] if matches!(program[*statement].node(), Nodes::Expression(Expressions::Return { .. }))
		));
	}

	#[test]
	fn retains_a_local_that_contributes_to_the_return_value() {
		let (mut program, main) = main(
			r#"
			main: fn() -> f32 {
				let x: f32 = 42.0;
				return x;
			}
		"#,
		);

		assert_eq!(optimize(&mut program, &main), OptimizationReport::default());
		assert_eq!(statements(&program, main).len(), 2);
	}

	#[test]
	fn culls_dead_local_chains_to_a_fixed_point() {
		let (mut program, main) = main(
			r#"
			main: fn() -> void {
				let first: f32 = 1.0;
				let second: f32 = first;
				return;
			}
		"#,
		);

		let report = optimize(&mut program, &main);

		assert_eq!(report.culled_unused_local_variables, 2);
		assert_eq!(statements(&program, main).len(), 1);
	}

	#[test]
	fn preserves_unused_locals_with_atomic_side_effects() {
		let (mut program, main) = main(
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

		assert_eq!(optimize(&mut program, &main), OptimizationReport::default());
		assert_eq!(statements(&program, main).len(), 2);
	}

	#[test]
	fn preserves_dead_atomic_initializer_behavior_in_the_vm() {
		let mut program = Program::new();
		let u32_type = program.get_child(program.root(), "u32").expect("Expected u32 type");
		let atomicu32_name = program.intern("atomicu32");
		let atomic_u32_type = program.add_to_root(Node::r#struct(atomicu32_name, Vec::new()));
		let count_name = program.intern("count");
		let count = program.add(Node::member(count_name, atomic_u32_type));
		let counter_name = program.intern("counter");
		program.add_to_root(Node::binding(
			counter_name,
			BindingTypes::Buffer { members: vec![count] },
			0,
			true,
			true,
		));
		let value_name = program.intern("value");
		let value = program.add(Node::member(value_name, u32_type));
		let result_name = program.intern("result");
		program.add_to_root(Node::binding(
			result_name,
			BindingTypes::Buffer { members: vec![value] },
			1,
			false,
			true,
		));
		let atomic_add_name = program.intern("atomic_add");
		let atomic_add = program.add_to_root(Node::intrinsic(atomic_add_name, Vec::new(), u32_type));
		let value_parameter = program.add(Node::new(Nodes::Parameter {
			name: value_name,
			r#type: atomic_u32_type,
		}));
		let increment_name = program.intern("increment");
		let increment_parameter = program.add(Node::new(Nodes::Parameter {
			name: increment_name,
			r#type: u32_type,
		}));
		program[atomic_add].add_children(vec![value_parameter, increment_parameter]);

		let mut program = compile_to_besl(
			r#"
			main: fn() -> void {
				let ignored: u32 = atomic_add(counter.count, 1);
				result.value = 7;
			}
		"#,
			Some(program),
		)
		.expect("Expected atomic side-effect fixture to link");
		let main = program.get_main().expect("Expected atomic side-effect main function");

		assert_eq!(optimize(&mut program, &main), OptimizationReport::default());

		let executable = ExecutableProgram::compile(&program).expect("Expected optimized fixture to compile for the VM");
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
		let (mut program, main) = main(
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

		let report = optimize(&mut program, &main);

		assert_eq!(report.culled_unreachable_statements, 1);
		assert_eq!(statements(&program, main).len(), 1);
	}

	#[test]
	fn culls_unreachable_locals_inside_nested_blocks() {
		let (mut program, main) = main(
			r#"
			main: fn() -> void {
				if (true) {
					return;
					let x: f32 = 1.0;
				}
			}
		"#,
		);

		let report = optimize(&mut program, &main);

		assert_eq!(report.culled_unreachable_statements, 1);
		let main_statements = statements(&program, main);
		let [conditional] = main_statements.as_slice() else {
			panic!("Expected one conditional statement");
		};
		let conditional = &program[*conditional];
		let Nodes::Conditional { statements, .. } = conditional.node() else {
			panic!("Expected conditional statement");
		};

		assert_eq!(statements.len(), 1);
		assert!(matches!(
			program[statements[0]].node(),
			Nodes::Expression(Expressions::Return { .. })
		));
	}

	#[test]
	fn is_idempotent_after_the_first_optimization() {
		let (mut program, main) = main(
			r#"
			main: fn() -> void {
				let x: f32 = 1.0;
				return;
			}
		"#,
		);

		assert_eq!(optimize(&mut program, &main).culled_unused_local_variables, 1);
		assert_eq!(optimize(&mut program, &main), OptimizationReport::default());
	}
}
