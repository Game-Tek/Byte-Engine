//! Lowers `if` and `match` values into nodes that every consumer of the linked tree already handles.
//!
//! An `if` whose branches are plain values becomes an [`Expressions::Ternary`], which the VM short-circuits and every
//! backend writes as `?:`. Any other `if` value, and every `match` value, is hoisted: a temporary, and an ordinary
//! `if` or `match` statement that sets it, run before the statement that uses the value, which becomes a read of the
//! temporary. Platform compilers keep such a temporary in a register, so it costs the same as hand-written branches.
//!
//! Hoisting keeps evaluation left to right. Operands that ran before a hoisted value are captured into temporaries
//! first, so its statements can't change what they read, and statements hoisted from the right side of `&&` or `||`
//! go under a guard, so they still run only when that side would.

use super::*;

const YIELDING_DOCUMENTATION: &str =
	"https://byte-engine.0x44491229.dev/docs/reference/besl/language#use-if-and-match-as-values";

/// The `BranchBlock` struct holds one lexed branch of an `if` or `match` value, so the lowering can choose between a
/// `?:` and hoisted statements once every branch is known.
struct BranchBlock {
	/// Statements that run before the value, including those the value hoisted.
	statements: Vec<NodeReference>,
	/// The value the branch yields, or `None` when it always leaves with `return`, `break`, `continue`, or `discard`.
	value: Option<NodeReference>,
}

impl BranchBlock {
	/// Returns the value of a branch that runs no statements, which a `?:` can select directly.
	fn plain_value(&self) -> Option<NodeReference> {
		self.statements.is_empty().then(|| self.value.clone()).flatten()
	}

	/// Returns the branch's statements followed by an assignment of its value, if it has one, to `temporary`.
	fn assign_into(self, temporary: &NodeReference) -> Vec<NodeReference> {
		let mut statements = self.statements;
		statements.extend(self.value.map(|value| assign(temporary, value)));
		statements
	}
}

impl Lexer {
	/// Returns how many statements the statement being lexed has hoisted so far.
	pub(super) fn hoisted_len(&self) -> usize {
		self.hoisted.as_ref().map_or(0, Vec::len)
	}

	/// Returns the return type of the function being lexed, which a `return` value takes.
	pub(super) fn enclosing_return_type(&self) -> Option<NodeReference> {
		self.scopes.iter().rev().find_map(|scope| match scope.borrow().node() {
			Nodes::Function { return_type, .. } => Some(return_type.clone()),
			_ => None,
		})
	}

	/// Lexes `value` where the statement already knows its type, `expected`, such as the right side of a typed `let`,
	/// so an `if` or `match` value takes that type instead of guessing one from literals.
	pub(super) fn lex_value(
		&mut self,
		value: &parser::Node,
		expected: Option<NodeReference>,
	) -> Result<NodeReference, LexError> {
		match &value.node {
			parser::Nodes::Conditional { .. } | parser::Nodes::Match { .. } => {
				self.lex_branch_value(value, expected.as_ref(), false)
			}
			// The group keeps its node, so backends still parenthesize the value.
			parser::Nodes::Expression(parser::Expressions::Expression(elements)) if elements.len() == 1 => {
				let element = self.lex_value(&elements[0], expected)?;
				Ok(Node::expression(Expressions::Expression { elements: vec![element] }).into())
			}
			_ => self.lex(value),
		}
	}

	/// Lowers an `if` or `match` used as a value. A plain-value `if` becomes a [`Expressions::Ternary`]; anything else
	/// is hoisted in front of the statement, and the value becomes a read of its temporary. `needs_local` forces the
	/// temporary, for an accessor base, which must have a declared type.
	pub(super) fn lex_branch_value(
		&mut self,
		branch: &parser::Node,
		expected: Option<&NodeReference>,
		needs_local: bool,
	) -> Result<NodeReference, LexError> {
		match &branch.node {
			parser::Nodes::Conditional {
				condition,
				statements,
				else_branch,
			} => self.lex_if_value(condition, statements, else_branch.as_ref(), expected, needs_local),
			parser::Nodes::Match { scrutinee, arms } => self.lex_match_value(scrutinee, arms, expected),
			_ => unreachable!("Only `if` and `match` nodes are lowered as branch values"),
		}
	}

	fn lex_if_value(
		&mut self,
		condition: &parser::Node,
		statements: &[parser::Node],
		else_branch: Option<&parser::ElseBranch>,
		expected: Option<&NodeReference>,
		needs_local: bool,
	) -> Result<NodeReference, LexError> {
		let condition = self.lex(condition)?;
		let then = self.lex_branch_block(statements, expected)?;
		let otherwise = match else_branch {
			Some(parser::ElseBranch::Block(statements)) => self.lex_branch_block(statements, expected)?,
			// An `else if` is an `else` block whose value is the next `if`, so its condition runs only when this one fails.
			Some(parser::ElseBranch::If(link)) => self.lex_branch_block(std::slice::from_ref(link), expected)?,
			None => {
				return Err(LexError::invalid(format!(
					"An `if` used as a value needs an `else` branch. The most likely cause is a missing `else`, which would leave the value undefined when the condition is false. See {YIELDING_DOCUMENTATION}."
				)));
			}
		};

		if !needs_local && let (Some(if_true), Some(if_false)) = (then.plain_value(), otherwise.plain_value()) {
			return Ok(Node::expression(Expressions::Ternary {
				condition,
				if_true,
				if_false,
			})
			.into());
		}

		let temporary = self.temporary(value_type([&then, &otherwise], expected)?);
		let statement = Node::conditional(
			condition,
			then.assign_into(&temporary),
			Some(ElseBranch::Block(otherwise.assign_into(&temporary))),
		);
		self.hoist(temporary, statement.into())
	}

	/// Lowers a `match` value to a hoisted `match` statement, which backends write as a `switch`, like the statement
	/// form.
	fn lex_match_value(
		&mut self,
		scrutinee: &parser::Node,
		arms: &[parser::MatchArm],
		expected: Option<&NodeReference>,
	) -> Result<NodeReference, LexError> {
		let (scrutinee, r#type, domain, arms) = self.lex_match_parts(scrutinee, arms, |lexer, statements| {
			lexer.lex_branch_block(statements, expected)
		})?;
		let temporary = self.temporary(value_type(arms.iter().map(|(_, block)| block), expected)?);
		let arms = arms
			.into_iter()
			.map(|(values, block)| (values, block.assign_into(&temporary)))
			.collect();
		let (arms, default) = matching::normalize_arms(domain, arms)?;
		self.hoist(temporary, Node::r#match(scrutinee, r#type, arms, default).into())
	}

	/// Lexes one branch of an `if` or `match` value. Its locals stay private to it, as in [`Lexer::lex_block`].
	fn lex_branch_block(&mut self, items: &[parser::Node], expected: Option<&NodeReference>) -> Result<BranchBlock, LexError> {
		let Some((last, leading)) = items.split_last() else {
			return Err(no_value_error());
		};

		let length = self.scopes.len();
		let mut statements = Vec::with_capacity(items.len());
		let value = leading
			.iter()
			.try_for_each(|item| {
				let item = self.lex_statement(item, &mut statements)?;
				self.scopes.push(item);
				Ok(())
			})
			.and_then(|()| self.lex_branch_end(last, expected, &mut statements));
		self.scopes.truncate(length);

		Ok(BranchBlock {
			value: value?,
			statements,
		})
	}

	/// Lexes the last item of a value branch into its value, appending the statements it hoists to `statements`, so they
	/// run only when the branch does. A branch whose every path leaves has no value; see [`always_exits`].
	fn lex_branch_end(
		&mut self,
		last: &parser::Node,
		expected: Option<&NodeReference>,
		statements: &mut Vec<NodeReference>,
	) -> Result<Option<NodeReference>, LexError> {
		if always_exits(last) {
			self.lex_statement(last, statements)?;
			return Ok(None);
		}

		let enclosing = self.hoisted.replace(Vec::new());
		let value = match &last.node {
			parser::Nodes::Expression(parser::Expressions::Yield { value }) => self.lex_value(value, expected.cloned()),
			parser::Nodes::Conditional { .. } | parser::Nodes::Match { .. } => self.lex_branch_value(last, expected, false),
			_ => Err(no_value_error()),
		};
		let hoisted = std::mem::replace(&mut self.hoisted, enclosing).unwrap_or_default();
		let value = value?;
		statements.extend(hoisted);
		Ok(Some(value))
	}

	/// Lexes a statement block's last expression, written without `;`. It runs only for its effect, so it must produce
	/// nothing that would be silently lost: it must be an assignment, or a call to a function that returns `void`.
	pub(super) fn lex_final_statement(&mut self, value: &parser::Node) -> Result<NodeReference, LexError> {
		let value = self.lex(value)?;
		let returns_void = |callable: &NodeReference| {
			infer_callable_return_type(callable).is_some_and(|r#type| r#type.borrow().get_name() == Some("void"))
		};
		let produces_nothing = match value.borrow().node() {
			Nodes::Expression(Expressions::Operator {
				operator: Operators::Assignment,
				..
			}) => true,
			Nodes::Expression(Expressions::FunctionCall { function, .. }) => returns_void(&function.get()),
			Nodes::Expression(Expressions::IntrinsicCall { intrinsic, .. }) => returns_void(intrinsic),
			_ => false,
		};
		if !produces_nothing {
			return Err(LexError::invalid(format!(
				"A block ends in a value that nothing uses. The most likely cause is a missing `;`, or a value meant to be returned: BESL functions return with `return`, not with their last expression. See {YIELDING_DOCUMENTATION}."
			)));
		}
		Ok(value)
	}

	/// Declares a hoisted temporary of `r#type`. Its name follows the inlined intrinsic locals', which backends never
	/// escape, and nothing looks it up by name: reads link to it directly.
	fn temporary(&mut self, r#type: NodeReference) -> NodeReference {
		let name = format!("_besl_value_{}", self.generated_id());
		Node::expression(Expressions::VariableDeclaration { name, r#type }).into()
	}

	/// Hoists `temporary` and the `statement` that sets it in front of the current statement and returns a read of it.
	fn hoist(&mut self, temporary: NodeReference, statement: NodeReference) -> Result<NodeReference, LexError> {
		let read = read(&temporary);
		let hoisted = self.hoisted.as_mut().ok_or_else(|| {
			LexError::invalid(format!(
				"This `if` or `match` value has to run statements before its statement, which isn't possible here. The most likely cause is a `match`, or a branch with statements, in a `for` loop's condition or update, or in a `const`; compute the value with a `let` first, at the start of the loop body for a loop. See {YIELDING_DOCUMENTATION}."
			))
		})?;
		hoisted.extend([temporary, statement]);
		Ok(read)
	}

	/// Copies `operand`, which ran before the statements hoisted at `*at`, into a temporary set at `*at`, so those
	/// statements can't change the value it read. An operand that no statement can change is returned as it is.
	pub(super) fn capture(&mut self, operand: NodeReference, at: &mut usize) -> Result<NodeReference, LexError> {
		if is_unchangeable(&operand) {
			return Ok(operand);
		}

		let r#type = infer_expression_type(&operand).ok_or_else(|| {
			LexError::invalid(format!(
				"Can't keep the value of an operand that runs before an `if` or `match` value with statements. The most likely cause is an operand without a known type; assign it to a typed `let` before this statement. See {YIELDING_DOCUMENTATION}."
			))
		})?;
		let temporary = self.temporary(r#type);
		let read = read(&temporary);
		let statements = [temporary.clone(), assign(&temporary, operand)];
		let hoisted = self
			.hoisted
			.as_mut()
			.expect("Statements were hoisted after the operand, so hoisting is allowed here");
		hoisted.splice(*at..*at, statements);
		*at += 2;
		Ok(read)
	}

	/// Captures the index expressions inside `place`, such as `i` in `values[i].x`, which ran before the statements
	/// hoisted at `*at`. The place itself keeps its identity, so an assignment or atomic still reaches the same storage.
	pub(super) fn capture_place(&mut self, place: &NodeReference, at: &mut usize) -> Result<(), LexError> {
		let (inner, index) = match place.borrow().node() {
			Nodes::Expression(Expressions::Accessor { left, right }) => (left.clone(), is_index(right).then(|| right.clone())),
			Nodes::Expression(Expressions::Expression { elements }) if elements.len() == 1 => (elements[0].clone(), None),
			_ => return Ok(()),
		};
		// Inner indices run first, so they are captured first. An index is the one-element group `[i]`.
		self.capture_place(&inner, at)?;
		if let Some(index) = index
			&& let Nodes::Expression(Expressions::Expression { elements }) = index.borrow_mut().node_mut()
		{
			elements[0] = self.capture(elements[0].clone(), at)?;
		}
		Ok(())
	}

	/// Captures what `base[index]` read before `index` hoisted statements at `*at`: the inner indices of a place such
	/// as `values[i]`, whose storage is only read once the index is known, or the whole value of anything else, such
	/// as a call's result.
	pub(super) fn capture_base(&mut self, base: NodeReference, at: &mut usize) -> Result<NodeReference, LexError> {
		if is_place(&base) {
			self.capture_place(&base, at)?;
			Ok(base)
		} else {
			self.capture(base, at)
		}
	}

	/// Lexes call arguments left to right. When one hoists statements, the arguments before it are captured first, so
	/// they keep the values they had when they ran. An atomic's target stays a place, so the atomic still reaches it.
	pub(super) fn lex_arguments(
		&mut self,
		name: &parser::TypeName,
		parameters: &[parser::Node],
	) -> Result<Vec<NodeReference>, LexError> {
		let atomic = matches!(name, parser::TypeName::Named(name) if atomic_access_requirement(name).is_some());
		// An `if` or `match` argument takes its parameter's type, as a plain `if` does when backends type its literals.
		let expected = if parameters.iter().any(|parameter| branch_operand(parameter).is_some()) {
			self.call_parameter_types(name, parameters.len())
		} else {
			Vec::new()
		};
		let mut arguments: Vec<NodeReference> = Vec::with_capacity(parameters.len());
		let mut captured = 0;
		for (index, parameter) in parameters.iter().enumerate() {
			let mut at = self.hoisted_len();
			let argument = self.lex_value(parameter, expected.get(index).cloned().flatten())?;
			if self.hoisted_len() > at {
				for index in captured..arguments.len() {
					if index == 0 && atomic {
						self.capture_place(&arguments[0], &mut at)?;
					} else {
						arguments[index] = self.capture(arguments[index].clone(), &mut at)?;
					}
				}
				captured = arguments.len();
			}
			arguments.push(argument);
		}
		Ok(arguments)
	}

	/// Returns the parameter types of the function or constructor that a call to `name` with `count` arguments reaches.
	/// An intrinsic picks its overload from the argument types, so it gives none.
	fn call_parameter_types(&self, name: &parser::TypeName, count: usize) -> Vec<Option<NodeReference>> {
		// An argument of unknown type matches any overload of the right count, so a function or constructor found with
		// these is the one the call reaches once its arguments are lexed: functions and constructors match by count.
		let unknown: Vec<NodeReference> = (0..count)
			.map(|_| Node::expression(Expressions::Expression { elements: Vec::new() }).into())
			.collect();
		let Ok(target) = resolve_call_target(&self.scopes, name, &unknown) else {
			return Vec::new();
		};
		let target = target.borrow();
		match target.node() {
			Nodes::Function { params, .. } | Nodes::Struct { fields: params, .. } if params.len() == count => params
				.iter()
				// An array field declares its element type, not its own, so inference types its argument instead.
				.map(|param| match param.borrow().node() {
					Nodes::Member { count: Some(_), .. } => None,
					_ => infer_member_type(param),
				})
				.collect(),
			// An array constructor takes one element per argument.
			Nodes::Struct {
				template: Some(element), ..
			} => vec![Some(element.clone()); count],
			_ => Vec::new(),
		}
	}

	/// Lowers `left && right` or `left || right` after lexing `right` hoisted `guarded` statements. They run only
	/// when `left` doesn't already decide the result, as the operator short-circuits, and the result is a read of a
	/// `bool` temporary.
	pub(super) fn short_circuit(
		&mut self,
		operator: Operators,
		left: NodeReference,
		mut guarded: Vec<NodeReference>,
		right: NodeReference,
	) -> Result<NodeReference, LexError> {
		let flag = self.temporary(resolve_type(&self.scopes, "bool")?);
		let mut condition = read(&flag);
		if operator == Operators::LogicalOr {
			condition = Node::expression(Expressions::Unary {
				operator: UnaryOperators::LogicalNot,
				operand: condition,
			})
			.into();
		}
		guarded.push(assign(&flag, right));
		let statements = [
			flag.clone(),
			assign(&flag, left),
			Node::conditional(condition, guarded, None).into(),
		];
		self.hoisted
			.as_mut()
			.expect("The right operand hoisted statements, so hoisting is allowed here")
			.extend(statements);
		Ok(read(&flag))
	}
}

/// Reports whether every path through `node`, the last item of a branch, leaves with `return`, `break`, `continue`, or
/// `discard`, directly or through an `if` or `match` whose branches all do. Such a branch needs no value, as in Rust.
fn always_exits(node: &parser::Node) -> bool {
	let block_exits = |statements: &[parser::Node]| statements.last().is_some_and(always_exits);
	match &node.node {
		parser::Nodes::Expression(
			parser::Expressions::Return { .. }
			| parser::Expressions::Break
			| parser::Expressions::Continue
			| parser::Expressions::Discard,
		) => true,
		parser::Nodes::Conditional {
			statements,
			else_branch: Some(else_branch),
			..
		} => {
			block_exits(statements)
				&& match else_branch {
					parser::ElseBranch::Block(statements) => block_exits(statements),
					parser::ElseBranch::If(link) => always_exits(link),
				}
		}
		parser::Nodes::Match { arms, .. } => arms.iter().all(|arm| block_exits(&arm.statements)),
		_ => false,
	}
}

/// Returns the `if` or `match` that `node` is, looking through parentheses.
pub(super) fn branch_operand<'n, 'a>(node: &'n parser::Node<'a>) -> Option<&'n parser::Node<'a>> {
	match &node.node {
		parser::Nodes::Conditional { .. } | parser::Nodes::Match { .. } => Some(node),
		parser::Nodes::Expression(parser::Expressions::Expression(elements)) if elements.len() == 1 => {
			branch_operand(&elements[0])
		}
		_ => None,
	}
}

/// Returns the error for a branch of an `if` or `match` value that doesn't end in a value.
fn no_value_error() -> LexError {
	LexError::invalid(format!(
		"A branch of an `if` or `match` value has no value. The most likely cause is a `;` after the branch's last expression; remove it, or end the branch with `return`, `break`, `continue`, or `discard`. See {YIELDING_DOCUMENTATION}."
	))
}

/// Picks the type of a hoisted value: `expected` when the statement knows it, or else the type of the first branch
/// value that isn't a literal, whose type BESL only guesses from its spelling, and then of any branch value.
fn value_type<'b>(
	blocks: impl IntoIterator<Item = &'b BranchBlock>,
	expected: Option<&NodeReference>,
) -> Result<NodeReference, LexError> {
	let values: Vec<&NodeReference> = blocks.into_iter().filter_map(|block| block.value.as_ref()).collect();
	if values.is_empty() {
		return Err(LexError::invalid(format!(
			"This `if` or `match` value never produces a value. The most likely cause is that every branch ends with `return`, `break`, `continue`, or `discard`; write it as a statement instead. See {YIELDING_DOCUMENTATION}."
		)));
	}
	if let Some(expected) = expected {
		return Ok(expected.clone());
	}

	values
		.iter()
		.filter(|value| !is_literal(value))
		.chain(values.iter())
		.find_map(|value| infer_expression_type(value))
		.ok_or_else(|| {
			LexError::invalid(format!(
				"Can't tell the type of this `if` or `match` value. The most likely cause is branches whose values have no known type; assign it to a typed `let` first. See {YIELDING_DOCUMENTATION}."
			))
		})
}

/// Builds a read of `temporary`.
fn read(temporary: &NodeReference) -> NodeReference {
	let name = temporary
		.borrow()
		.get_name()
		.expect("A temporary is a named declaration")
		.to_string();
	Node::expression(Expressions::Member {
		name,
		source: temporary.clone(),
	})
	.into()
}

/// Builds `temporary = value`.
fn assign(temporary: &NodeReference, value: NodeReference) -> NodeReference {
	Node::expression(Expressions::Operator {
		operator: Operators::Assignment,
		left: read(temporary),
		right: value,
	})
	.into()
}

/// Reports whether `node` is a literal, possibly negated or parenthesized, whose type BESL guesses from its spelling.
fn is_literal(node: &NodeReference) -> bool {
	match node.borrow().node() {
		Nodes::Expression(Expressions::Literal { .. }) => true,
		Nodes::Expression(Expressions::Unary { operand, .. }) => is_literal(operand),
		Nodes::Expression(Expressions::Expression { elements }) if elements.len() == 1 => is_literal(&elements[0]),
		_ => false,
	}
}

/// Reports whether `node` reads nothing that a hoisted statement could change, so it can stay where it is: a literal,
/// a constant, a specialization constant, an input, a resource, or an operation or constructor built from those.
fn is_unchangeable(node: &NodeReference) -> bool {
	match node.borrow().node() {
		Nodes::Expression(Expressions::Literal { .. }) => true,
		Nodes::Expression(Expressions::Member { source, .. }) => matches!(
			source.borrow().node(),
			Nodes::Const { .. }
				| Nodes::Specialization { .. }
				| Nodes::Input { .. }
				| Nodes::Binding { .. }
				| Nodes::PushConstant { .. }
		),
		Nodes::Expression(Expressions::Unary { operand, .. }) => is_unchangeable(operand),
		Nodes::Expression(Expressions::Operator { operator, left, right }) => {
			*operator != Operators::Assignment && is_unchangeable(left) && is_unchangeable(right)
		}
		Nodes::Expression(Expressions::Expression { elements }) => elements.iter().all(is_unchangeable),
		Nodes::Expression(Expressions::Ternary {
			condition,
			if_true,
			if_false,
		}) => is_unchangeable(condition) && is_unchangeable(if_true) && is_unchangeable(if_false),
		Nodes::Expression(Expressions::FunctionCall { function, parameters }) => {
			matches!(function.get().borrow().node(), Nodes::Struct { .. }) && parameters.iter().all(is_unchangeable)
		}
		_ => false,
	}
}

/// Reports whether `node` names storage, such as `values[i].x`, rather than computing a value.
fn is_place(node: &NodeReference) -> bool {
	match node.borrow().node() {
		Nodes::Expression(Expressions::Accessor { left, .. }) => is_place(left),
		Nodes::Expression(Expressions::Expression { elements }) if elements.len() == 1 => is_place(&elements[0]),
		Nodes::Expression(Expressions::Member { .. }) => true,
		Nodes::Expression(_) => false,
		// An accessor can start at a declaration itself, such as a workgroup array.
		_ => true,
	}
}
