use std::{cell::RefCell, collections::HashSet};

/// The `BindingUsage` struct describes a used binding in a BESL program.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BindingUsage {
	pub name: String,
	pub kind: BindingKind,
	pub count: u32,
	pub slot: u32,
	pub read: bool,
	pub write: bool,
}

/// Identifies the descriptor category declared by a BESL binding.
#[derive(
	Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize, rkyv::Archive, rkyv::Serialize, rkyv::Deserialize,
)]
pub enum BindingKind {
	/// A structured storage buffer. Read-only access does not change the descriptor category.
	StorageBuffer,
	CombinedImageSampler {
		view: TextureView,
	},
	StorageImage,
}

/// Identifies the texture shape required by a BESL sampled-image binding.
#[derive(
	Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize, rkyv::Archive, rkyv::Serialize, rkyv::Deserialize,
)]
pub enum TextureView {
	Texture2D,
	Texture2DArray,
	Texture3D,
}

/// The `BindingRecord` trait exists to keep binding discovery independent from evaluated and compiled metadata representations.
pub(crate) trait BindingRecord: Sized {
	fn from_usage(name: &str, kind: BindingKind, count: u32, slot: u32, read: bool, write: bool) -> Self;
	fn usage(&self) -> (u32, BindingKind, u32, bool, bool);
}

impl BindingRecord for BindingUsage {
	fn from_usage(name: &str, kind: BindingKind, count: u32, slot: u32, read: bool, write: bool) -> Self {
		Self {
			name: name.to_string(),
			kind,
			count,
			slot,
			read,
			write,
		}
	}

	fn usage(&self) -> (u32, BindingKind, u32, bool, bool) {
		(self.slot, self.kind, self.count, self.read, self.write)
	}
}

/// The `BindingCollectionState` struct keeps reflection traversal aligned with graph identity deduplication.
struct BindingCollectionState {
	visited: HashSet<besl::NodeReference>,
	error: Option<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OpacityEvaluation {
	Opaque,
	NonOpaque,
	Unknown,
}

/// The `ProgramEvaluation` struct holds information derived from evaluating a BESL program.
#[derive(Clone, Debug)]
pub struct ProgramEvaluation {
	bindings: Vec<BindingUsage>,
	opacity: OpacityEvaluation,
}

impl ProgramEvaluation {
	pub fn from_program(program: &besl::NodeReference) -> Result<Self, String> {
		let main = program.get_main().ok_or_else(|| {
			"Main function not found. The program description likely does not define a `main` function.".to_string()
		})?;

		Self::from_main(&main)
	}

	pub fn from_main(main_function_node: &besl::NodeReference) -> Result<Self, String> {
		{
			let node_borrow = RefCell::borrow(main_function_node);
			let node_ref = node_borrow.node();

			match node_ref {
				besl::Nodes::Function { name, .. } => {
					if name != "main" {
						return Err(
							"Main node is not `main`. The program description likely passed a non-main function node."
								.to_string(),
						);
					}
				}
				_ => {
					return Err(
						"Invalid main node. The program description likely contains a `main` symbol that is not a function."
							.to_string(),
					);
				}
			}
		}

		let bindings = collect_bindings(main_function_node)?;

		let opacity = evaluate_opacity(main_function_node);

		Ok(Self { bindings, opacity })
	}

	pub fn bindings(&self) -> &[BindingUsage] {
		&self.bindings
	}

	pub fn into_bindings(self) -> Vec<BindingUsage> {
		self.bindings
	}

	pub fn opacity(&self) -> OpacityEvaluation {
		self.opacity
	}
}

/// Collects sorted binding metadata while sharing repeated references and rejecting distinct slot aliases.
pub(crate) fn collect_bindings<T: BindingRecord>(node: &besl::NodeReference) -> Result<Vec<T>, String> {
	let mut bindings: Vec<T> = Vec::with_capacity(16);
	let mut state = BindingCollectionState {
		visited: HashSet::new(),
		error: None,
	};
	build_bindings(&mut bindings, node, &mut state);
	if let Some(error) = state.error {
		return Err(error);
	}

	bindings.sort_by_key(|binding| binding.usage().0);
	for (index, binding) in bindings.iter().enumerate() {
		let (slot, _, count, ..) = binding.usage();
		let end_slot = slot.checked_add(count).ok_or_else(|| {
			format!(
				"Resource slot range overflow at slot {slot}. The most likely cause is that the declared resource range has no representable exclusive end."
			)
		})?;
		if let Some(next) = bindings.get(index + 1) {
			let (next_slot, ..) = next.usage();
			if next_slot < end_slot {
				return Err(format!(
					"Resource slot ranges overlap at slots {slot} and {next_slot}. The most likely cause is that a resource array reserves a slot used by another declaration."
				));
			}
		}
	}

	Ok(bindings)
}

fn build_bindings<T: BindingRecord>(bindings: &mut Vec<T>, node: &besl::NodeReference, state: &mut BindingCollectionState) {
	if state.error.is_some() || !state.visited.insert(node.clone()) {
		return;
	}
	let node_borrow = RefCell::borrow(node);
	let node_ref = node_borrow.node();

	match node_ref {
		besl::Nodes::Function { statements, .. } => {
			for statement in statements {
				build_bindings(bindings, statement, state);
			}
		}
		besl::Nodes::Conditional { condition, statements } => {
			build_bindings(bindings, condition, state);
			for statement in statements {
				build_bindings(bindings, statement, state);
			}
		}
		besl::Nodes::ForLoop {
			initializer,
			condition,
			update,
			statements,
		} => {
			build_bindings(bindings, initializer, state);
			build_bindings(bindings, condition, state);
			build_bindings(bindings, update, state);
			for statement in statements {
				build_bindings(bindings, statement, state);
			}
		}
		besl::Nodes::Expression(expression) => match expression {
			besl::Expressions::FunctionCall {
				function: callable,
				parameters: arguments,
			} => {
				build_bindings(bindings, callable, state);
				for argument in arguments {
					build_bindings(bindings, argument, state);
				}
			}
			besl::Expressions::IntrinsicCall { elements, .. } => {
				// Intrinsic lowering emits the instantiated elements, not the definition template.
				for element in elements {
					build_bindings(bindings, element, state);
				}
			}
			besl::Expressions::Accessor { left, right } | besl::Expressions::Operator { left, right, .. } => {
				build_bindings(bindings, left, state);
				build_bindings(bindings, right, state);
			}
			besl::Expressions::Expression { elements } => {
				for element in elements {
					build_bindings(bindings, element, state);
				}
			}
			besl::Expressions::Macro { body, .. } => {
				build_bindings(bindings, body, state);
			}
			besl::Expressions::Member { source, .. } => {
				build_bindings(bindings, source, state);
			}
			besl::Expressions::VariableDeclaration { r#type, .. } => {
				build_bindings(bindings, r#type, state);
			}
			besl::Expressions::Return { .. } | besl::Expressions::Literal { .. } | besl::Expressions::Continue => {}
		},
		besl::Nodes::Binding {
			name,
			slot,
			read,
			write,
			r#type,
			count,
		} => {
			let kind = match r#type {
				besl::BindingTypes::Buffer { .. } => BindingKind::StorageBuffer,
				besl::BindingTypes::CombinedImageSampler { format } => BindingKind::CombinedImageSampler {
					view: match format.as_str() {
						"Texture3D" => TextureView::Texture3D,
						"ArrayTexture2D" => TextureView::Texture2DArray,
						_ => TextureView::Texture2D,
					},
				},
				besl::BindingTypes::Image { .. } => BindingKind::StorageImage,
			};
			let count = count.map_or(1, |count| count.get());
			if bindings.iter().any(|record| record.usage().0 == *slot) {
				state.error = Some(format!(
					"Duplicate resource declaration at slot {slot}. The most likely cause is that distinct binding nodes reuse one flat slot instead of sharing the same binding reference."
				));
			} else {
				bindings.push(T::from_usage(name, kind, count, *slot, *read, *write));
			}
		}
		besl::Nodes::Raw { input, output, .. } => {
			for reference in input.iter().chain(output.iter()) {
				build_bindings(bindings, reference, state);
			}
		}
		besl::Nodes::Intrinsic { elements, r#return, .. } => {
			for element in elements {
				build_bindings(bindings, element, state);
			}
			build_bindings(bindings, r#return, state);
		}
		besl::Nodes::Literal { value: nested, .. }
		| besl::Nodes::Member { r#type: nested, .. }
		| besl::Nodes::Parameter { r#type: nested, .. }
		| besl::Nodes::Specialization { r#type: nested, .. } => {
			build_bindings(bindings, nested, state);
		}
		besl::Nodes::Input { format, .. }
		| besl::Nodes::Output { format, .. }
		| besl::Nodes::TaskPayload { format, .. }
		| besl::Nodes::Workgroup { format, .. } => {
			build_bindings(bindings, format, state);
		}
		besl::Nodes::Struct { fields: nested, .. }
		| besl::Nodes::PushConstant { members: nested }
		| besl::Nodes::Scope { children: nested, .. } => {
			for child in nested {
				build_bindings(bindings, child, state);
			}
		}
		besl::Nodes::Null => {}
		besl::Nodes::Const { r#type, value, .. } => {
			build_bindings(bindings, r#type, state);
			build_bindings(bindings, value, state);
		}
	}
}

fn evaluate_opacity(main_function_node: &besl::NodeReference) -> OpacityEvaluation {
	let mut main_contains_raw_code = false;
	let mut local_output_symbols = HashSet::new();

	{
		let node_borrow = RefCell::borrow(main_function_node);
		let node_ref = node_borrow.node();

		if let besl::Nodes::Function { statements, params, .. } = node_ref {
			for param in params {
				let param_borrow = RefCell::borrow(param);
				if let besl::Nodes::Parameter {
					name: parameter_name, ..
				} = param_borrow.node()
				{
					if parameter_name == "output" {
						local_output_symbols.insert(param.clone());
					}
				}
			}

			for statement in statements {
				let statement_borrow = RefCell::borrow(statement);
				match statement_borrow.node() {
					besl::Nodes::Raw { .. } => {
						main_contains_raw_code = true;
					}
					_ => collect_local_output_symbols(statement, &mut local_output_symbols),
				}
			}
		}
	}

	if main_contains_raw_code {
		return OpacityEvaluation::Unknown;
	}

	if writes_non_opaque_vec4f_to_non_local_output(main_function_node, &local_output_symbols) {
		return OpacityEvaluation::NonOpaque;
	}

	if references_non_local_output(main_function_node, &local_output_symbols) {
		OpacityEvaluation::Opaque
	} else {
		OpacityEvaluation::Unknown
	}
}

fn collect_local_output_symbols(node: &besl::NodeReference, local_output_symbols: &mut HashSet<besl::NodeReference>) {
	let node_borrow = RefCell::borrow(node);
	let node_ref = node_borrow.node();

	match node_ref {
		besl::Nodes::Function { statements, params, .. } => {
			for param in params {
				collect_local_output_symbols(param, local_output_symbols);
			}
			for statement in statements {
				collect_local_output_symbols(statement, local_output_symbols);
			}
		}
		besl::Nodes::Conditional { condition, statements } => {
			collect_local_output_symbols(condition, local_output_symbols);
			for statement in statements {
				collect_local_output_symbols(statement, local_output_symbols);
			}
		}
		besl::Nodes::ForLoop {
			initializer,
			condition,
			update,
			statements,
		} => {
			collect_local_output_symbols(initializer, local_output_symbols);
			collect_local_output_symbols(condition, local_output_symbols);
			collect_local_output_symbols(update, local_output_symbols);
			for statement in statements {
				collect_local_output_symbols(statement, local_output_symbols);
			}
		}
		besl::Nodes::Expression(expression) => match expression {
			besl::Expressions::VariableDeclaration { name, .. } => {
				if name == "output" {
					local_output_symbols.insert(node.clone());
				}
			}
			besl::Expressions::FunctionCall {
				function: callable,
				parameters: arguments,
			}
			| besl::Expressions::IntrinsicCall {
				intrinsic: callable,
				elements: arguments,
				..
			} => {
				collect_local_output_symbols(callable, local_output_symbols);
				for argument in arguments {
					collect_local_output_symbols(argument, local_output_symbols);
				}
			}
			besl::Expressions::Accessor { left, right } | besl::Expressions::Operator { left, right, .. } => {
				collect_local_output_symbols(left, local_output_symbols);
				collect_local_output_symbols(right, local_output_symbols);
			}
			besl::Expressions::Expression { elements } => {
				for element in elements {
					collect_local_output_symbols(element, local_output_symbols);
				}
			}
			besl::Expressions::Member { source, .. } => {
				collect_local_output_symbols(source, local_output_symbols);
			}
			besl::Expressions::Macro { body, .. } => {
				collect_local_output_symbols(body, local_output_symbols);
			}
			besl::Expressions::Return { .. } | besl::Expressions::Literal { .. } | besl::Expressions::Continue => {}
		},
		besl::Nodes::Raw { input, output, .. } => {
			for value in input.iter().chain(output.iter()) {
				collect_local_output_symbols(value, local_output_symbols);
			}
		}
		besl::Nodes::Intrinsic { elements, r#return, .. } => {
			for element in elements {
				collect_local_output_symbols(element, local_output_symbols);
			}
			collect_local_output_symbols(r#return, local_output_symbols);
		}
		besl::Nodes::Literal { value: nested, .. }
		| besl::Nodes::Member { r#type: nested, .. }
		| besl::Nodes::Input { format: nested, .. }
		| besl::Nodes::Output { format: nested, .. }
		| besl::Nodes::TaskPayload { format: nested, .. }
		| besl::Nodes::Workgroup { format: nested, .. }
		| besl::Nodes::Specialization { r#type: nested, .. } => {
			collect_local_output_symbols(nested, local_output_symbols);
		}
		besl::Nodes::Parameter {
			name: parameter_name,
			r#type: parameter_type,
		} => {
			if parameter_name == "output" {
				local_output_symbols.insert(node.clone());
			}
			collect_local_output_symbols(parameter_type, local_output_symbols);
		}
		besl::Nodes::Struct { fields: nested, .. }
		| besl::Nodes::PushConstant { members: nested }
		| besl::Nodes::Scope { children: nested, .. } => {
			for child in nested {
				collect_local_output_symbols(child, local_output_symbols);
			}
		}
		besl::Nodes::Binding { .. } | besl::Nodes::Null => {}
		besl::Nodes::Const { r#type, value, .. } => {
			collect_local_output_symbols(r#type, local_output_symbols);
			collect_local_output_symbols(value, local_output_symbols);
		}
	}
}

fn references_non_local_output(node: &besl::NodeReference, local_output_symbols: &HashSet<besl::NodeReference>) -> bool {
	let node_borrow = RefCell::borrow(node);
	let node_ref = node_borrow.node();

	match node_ref {
		besl::Nodes::Function { statements, .. } => statements
			.iter()
			.any(|statement| references_non_local_output(statement, local_output_symbols)),
		besl::Nodes::Conditional { condition, statements } => {
			references_non_local_output(condition, local_output_symbols)
				|| statements
					.iter()
					.any(|statement| references_non_local_output(statement, local_output_symbols))
		}
		besl::Nodes::ForLoop {
			initializer,
			condition,
			update,
			statements,
		} => {
			references_non_local_output(initializer, local_output_symbols)
				|| references_non_local_output(condition, local_output_symbols)
				|| references_non_local_output(update, local_output_symbols)
				|| statements
					.iter()
					.any(|statement| references_non_local_output(statement, local_output_symbols))
		}
		besl::Nodes::Expression(expression) => match expression {
			besl::Expressions::Member { name, source } => {
				if name == "output" && !local_output_symbols.contains(source) {
					return true;
				}

				references_non_local_output(source, local_output_symbols)
			}
			besl::Expressions::Expression { elements } => elements
				.iter()
				.any(|element| references_non_local_output(element, local_output_symbols)),
			besl::Expressions::FunctionCall {
				function: callable,
				parameters: arguments,
			}
			| besl::Expressions::IntrinsicCall {
				intrinsic: callable,
				elements: arguments,
				..
			} => {
				references_non_local_output(callable, local_output_symbols)
					|| arguments
						.iter()
						.any(|argument| references_non_local_output(argument, local_output_symbols))
			}
			besl::Expressions::Accessor { left, right } | besl::Expressions::Operator { left, right, .. } => {
				references_non_local_output(left, local_output_symbols)
					|| references_non_local_output(right, local_output_symbols)
			}
			besl::Expressions::VariableDeclaration { r#type: nested, .. } | besl::Expressions::Macro { body: nested, .. } => {
				references_non_local_output(nested, local_output_symbols)
			}
			besl::Expressions::Return { .. } | besl::Expressions::Literal { .. } | besl::Expressions::Continue => false,
		},
		besl::Nodes::Raw { input, output, .. } => input
			.iter()
			.chain(output.iter())
			.any(|reference| references_non_local_output(reference, local_output_symbols)),
		besl::Nodes::Intrinsic { elements, r#return, .. } => {
			elements
				.iter()
				.any(|element| references_non_local_output(element, local_output_symbols))
				|| references_non_local_output(r#return, local_output_symbols)
		}
		besl::Nodes::Literal { value: nested, .. }
		| besl::Nodes::Member { r#type: nested, .. }
		| besl::Nodes::Input { format: nested, .. }
		| besl::Nodes::Output { format: nested, .. }
		| besl::Nodes::TaskPayload { format: nested, .. }
		| besl::Nodes::Workgroup { format: nested, .. }
		| besl::Nodes::Parameter { r#type: nested, .. }
		| besl::Nodes::Specialization { r#type: nested, .. } => references_non_local_output(nested, local_output_symbols),
		besl::Nodes::Struct { fields: nested, .. }
		| besl::Nodes::PushConstant { members: nested }
		| besl::Nodes::Scope { children: nested, .. } => nested
			.iter()
			.any(|child| references_non_local_output(child, local_output_symbols)),
		besl::Nodes::Binding { .. } | besl::Nodes::Null => false,
		besl::Nodes::Const { r#type, value, .. } => {
			references_non_local_output(r#type, local_output_symbols)
				|| references_non_local_output(value, local_output_symbols)
		}
	}
}

fn writes_non_opaque_vec4f_to_non_local_output(
	node: &besl::NodeReference,
	local_output_symbols: &HashSet<besl::NodeReference>,
) -> bool {
	let node_borrow = RefCell::borrow(node);
	let node_ref = node_borrow.node();

	match node_ref {
		besl::Nodes::Function { statements, .. } => statements
			.iter()
			.any(|statement| writes_non_opaque_vec4f_to_non_local_output(statement, local_output_symbols)),
		besl::Nodes::Conditional { condition, statements } => {
			writes_non_opaque_vec4f_to_non_local_output(condition, local_output_symbols)
				|| statements
					.iter()
					.any(|statement| writes_non_opaque_vec4f_to_non_local_output(statement, local_output_symbols))
		}
		besl::Nodes::ForLoop {
			initializer,
			condition,
			update,
			statements,
		} => {
			writes_non_opaque_vec4f_to_non_local_output(initializer, local_output_symbols)
				|| writes_non_opaque_vec4f_to_non_local_output(condition, local_output_symbols)
				|| writes_non_opaque_vec4f_to_non_local_output(update, local_output_symbols)
				|| statements
					.iter()
					.any(|statement| writes_non_opaque_vec4f_to_non_local_output(statement, local_output_symbols))
		}
		besl::Nodes::Expression(expression) => match expression {
			besl::Expressions::Operator { operator, left, right } => {
				if operator == &besl::Operators::Assignment
					&& is_non_local_output_target(left, local_output_symbols)
					&& is_non_opaque_vec4f_constructor(right)
				{
					return true;
				}

				writes_non_opaque_vec4f_to_non_local_output(left, local_output_symbols)
					|| writes_non_opaque_vec4f_to_non_local_output(right, local_output_symbols)
			}
			besl::Expressions::Expression { elements } => elements
				.iter()
				.any(|element| writes_non_opaque_vec4f_to_non_local_output(element, local_output_symbols)),
			besl::Expressions::FunctionCall {
				function: callable,
				parameters: arguments,
			}
			| besl::Expressions::IntrinsicCall {
				intrinsic: callable,
				elements: arguments,
				..
			} => {
				writes_non_opaque_vec4f_to_non_local_output(callable, local_output_symbols)
					|| arguments
						.iter()
						.any(|argument| writes_non_opaque_vec4f_to_non_local_output(argument, local_output_symbols))
			}
			besl::Expressions::Accessor { left, right } => {
				writes_non_opaque_vec4f_to_non_local_output(left, local_output_symbols)
					|| writes_non_opaque_vec4f_to_non_local_output(right, local_output_symbols)
			}
			besl::Expressions::Member { source, .. } => {
				writes_non_opaque_vec4f_to_non_local_output(source, local_output_symbols)
			}
			besl::Expressions::VariableDeclaration { r#type: nested, .. } | besl::Expressions::Macro { body: nested, .. } => {
				writes_non_opaque_vec4f_to_non_local_output(nested, local_output_symbols)
			}
			besl::Expressions::Return { .. } | besl::Expressions::Literal { .. } | besl::Expressions::Continue => false,
		},
		besl::Nodes::Raw { input, output, .. } => input
			.iter()
			.chain(output.iter())
			.any(|reference| writes_non_opaque_vec4f_to_non_local_output(reference, local_output_symbols)),
		besl::Nodes::Intrinsic { elements, r#return, .. } => {
			elements
				.iter()
				.any(|element| writes_non_opaque_vec4f_to_non_local_output(element, local_output_symbols))
				|| writes_non_opaque_vec4f_to_non_local_output(r#return, local_output_symbols)
		}
		besl::Nodes::Literal { value: nested, .. }
		| besl::Nodes::Member { r#type: nested, .. }
		| besl::Nodes::Input { format: nested, .. }
		| besl::Nodes::Output { format: nested, .. }
		| besl::Nodes::TaskPayload { format: nested, .. }
		| besl::Nodes::Workgroup { format: nested, .. }
		| besl::Nodes::Parameter { r#type: nested, .. }
		| besl::Nodes::Specialization { r#type: nested, .. } => {
			writes_non_opaque_vec4f_to_non_local_output(nested, local_output_symbols)
		}
		besl::Nodes::Struct { fields: nested, .. }
		| besl::Nodes::PushConstant { members: nested }
		| besl::Nodes::Scope { children: nested, .. } => nested
			.iter()
			.any(|child| writes_non_opaque_vec4f_to_non_local_output(child, local_output_symbols)),
		besl::Nodes::Binding { .. } | besl::Nodes::Null => false,
		besl::Nodes::Const { r#type, value, .. } => {
			writes_non_opaque_vec4f_to_non_local_output(r#type, local_output_symbols)
				|| writes_non_opaque_vec4f_to_non_local_output(value, local_output_symbols)
		}
	}
}

fn is_non_local_output_target(node: &besl::NodeReference, local_output_symbols: &HashSet<besl::NodeReference>) -> bool {
	let node_borrow = RefCell::borrow(node);
	let node_ref = node_borrow.node();

	match node_ref {
		besl::Nodes::Expression(besl::Expressions::Member {
			name: member_name,
			source: member_source,
		}) => member_name == "output" && !local_output_symbols.contains(member_source),
		besl::Nodes::Expression(besl::Expressions::Accessor { left, .. }) => {
			is_non_local_output_target(left, local_output_symbols)
		}
		_ => false,
	}
}

fn is_non_opaque_vec4f_constructor(node: &besl::NodeReference) -> bool {
	let node_borrow = RefCell::borrow(node);
	let node_ref = node_borrow.node();

	match node_ref {
		besl::Nodes::Expression(besl::Expressions::FunctionCall { function, parameters }) => {
			let function_borrow = RefCell::borrow(function);
			if function_borrow.get_name() != Some("vec4f") {
				return false;
			}

			let w_parameter = match parameters.len() {
				4 => Some(&parameters[3]),
				2 if is_vec3f_constructor(&parameters[0]) => Some(&parameters[1]),
				_ => None,
			};

			let Some(w_parameter) = w_parameter else {
				return false;
			};

			match parse_literal_number(w_parameter) {
				Some(w) => w != 1.0,
				None => false,
			}
		}
		_ => false,
	}
}

fn is_vec3f_constructor(node: &besl::NodeReference) -> bool {
	let node_borrow = RefCell::borrow(node);
	let node_ref = node_borrow.node();

	match node_ref {
		besl::Nodes::Expression(besl::Expressions::FunctionCall { function, parameters }) => {
			let function_borrow = RefCell::borrow(function);
			function_borrow.get_name() == Some("vec3f") && parameters.len() == 3
		}
		_ => false,
	}
}

fn parse_literal_number(node: &besl::NodeReference) -> Option<f64> {
	let node_borrow = RefCell::borrow(node);
	let node_ref = node_borrow.node();

	match node_ref {
		besl::Nodes::Expression(besl::Expressions::Literal { value }) => value.parse().ok(),
		_ => None,
	}
}

#[cfg(test)]
mod tests {
	use super::*;
	use crate::shader::generator;

	#[test]
	fn binding_metadata_is_sorted_and_classified() {
		let main = generator::tests::bindings();

		let evaluation = ProgramEvaluation::from_main(&main).expect("Failed to evaluate program");
		let bindings = evaluation
			.bindings()
			.iter()
			.map(|binding| {
				(
					binding.name.as_str(),
					binding.kind,
					binding.count,
					binding.slot,
					binding.read,
					binding.write,
				)
			})
			.collect::<Vec<_>>();

		assert_eq!(
			bindings,
			vec![
				("buff", BindingKind::StorageBuffer, 1, 0, true, true),
				("image", BindingKind::StorageImage, 1, 1, false, true),
				(
					"texture",
					BindingKind::CombinedImageSampler {
						view: TextureView::Texture2D,
					},
					1,
					2,
					true,
					false,
				),
			]
		);
	}

	#[test]
	fn sampled_texture_shapes_and_descriptor_counts_are_preserved() {
		let root = besl::Node::root();
		let void = root.get_child("void").expect("Expected the built-in void type");
		let main: besl::NodeReference = besl::Node::function(
			"main",
			Vec::new(),
			void,
			vec![besl::Node::binding_array(
				"volumes",
				besl::BindingTypes::CombinedImageSampler {
					format: "Texture3D".to_string(),
				},
				0,
				true,
				false,
				3,
			)
			.into()],
		)
		.into();

		let bindings = ProgramEvaluation::from_main(&main)
			.expect("Expected sampled binding metadata to evaluate")
			.into_bindings();
		assert_eq!(bindings[0].count, 3);
		assert_eq!(
			bindings[0].kind,
			BindingKind::CombinedImageSampler {
				view: TextureView::Texture3D
			}
		);
	}

	#[test]
	fn bindings_from_program() {
		let script = r#"
		main: fn () -> void {
			buff;
			image;
			texture;
		}
		"#;

		let mut root_node = besl::Node::root();

		let float_type = root_node.get_child("f32").unwrap();

		root_node.add_children(vec![
			besl::Node::binding(
				"buff",
				besl::BindingTypes::Buffer {
					members: vec![besl::Node::member("member", float_type).into()],
				},
				0,
				true,
				true,
			)
			.into(),
			besl::Node::binding(
				"image",
				besl::BindingTypes::Image {
					format: "r8".to_string(),
				},
				1,
				false,
				true,
			)
			.into(),
			besl::Node::binding(
				"texture",
				besl::BindingTypes::CombinedImageSampler { format: "".to_string() },
				2,
				true,
				false,
			)
			.into(),
		]);

		let program_node = besl::compile_to_besl(&script, Some(root_node)).unwrap();
		let evaluation = ProgramEvaluation::from_program(&program_node).expect("Failed to evaluate program");
		let bindings = evaluation.bindings();

		assert_eq!(bindings.len(), 3);
	}

	#[test]
	fn opacity_is_opaque_when_non_local_output_is_referenced() {
		let script = r#"
		main: fn () -> void {
			output;
		}
		"#;

		let mut root_node = besl::Node::root();
		let vec3f_type = root_node.get_child("vec3f").unwrap();
		root_node.add_child(besl::Node::output("output", vec3f_type, 0).into());

		let program_node = besl::compile_to_besl(script, Some(root_node)).unwrap();
		let evaluation = ProgramEvaluation::from_program(&program_node).expect("Failed to evaluate program");

		assert_eq!(evaluation.opacity(), OpacityEvaluation::Opaque);
	}

	#[test]
	fn opacity_is_unknown_when_output_is_shadowed_locally() {
		let script = r#"
		main: fn () -> void {
			let output: vec3f = vec3f(1.0, 0.0, 0.0);
			output;
		}
		"#;

		let mut root_node = besl::Node::root();
		let vec3f_type = root_node.get_child("vec3f").unwrap();
		root_node.add_child(besl::Node::output("output", vec3f_type, 0).into());

		let program_node = besl::compile_to_besl(script, Some(root_node)).unwrap();
		let evaluation = ProgramEvaluation::from_program(&program_node).expect("Failed to evaluate program");

		assert_eq!(evaluation.opacity(), OpacityEvaluation::Unknown);
	}

	#[test]
	fn opacity_is_unknown_when_main_contains_raw_code() {
		let mut root_node = besl::Node::root();
		let return_type = root_node.get_child("void").unwrap();
		let main = besl::Node::function(
			"main",
			Vec::new(),
			return_type,
			vec![besl::Node::glsl("output = vec3f(1.0, 0.0, 0.0);".to_string(), Vec::new(), Vec::new()).into()],
		);
		root_node.add_child(main.into());

		let program_node: besl::NodeReference = root_node.into();
		let evaluation = ProgramEvaluation::from_program(&program_node).expect("Failed to evaluate program");

		assert_eq!(evaluation.opacity(), OpacityEvaluation::Unknown);
	}

	#[test]
	fn opacity_is_non_opaque_when_output_vec4f_w_is_not_one() {
		let script = r#"
		main: fn () -> void {
			output = vec4f(1.0, 0.0, 0.0, 0.5);
		}
		"#;

		let mut root_node = besl::Node::root();
		let vec4f_type = root_node.get_child("vec4f").unwrap();
		root_node.add_child(besl::Node::output("output", vec4f_type, 0).into());

		let program_node = besl::compile_to_besl(script, Some(root_node)).unwrap();
		let evaluation = ProgramEvaluation::from_program(&program_node).expect("Failed to evaluate program");

		assert_eq!(evaluation.opacity(), OpacityEvaluation::NonOpaque);
	}

	#[test]
	fn opacity_is_opaque_when_output_vec4f_w_is_one() {
		let script = r#"
		main: fn () -> void {
			output = vec4f(1.0, 0.0, 0.0, 1.0);
		}
		"#;

		let mut root_node = besl::Node::root();
		let vec4f_type = root_node.get_child("vec4f").unwrap();
		root_node.add_child(besl::Node::output("output", vec4f_type, 0).into());

		let program_node = besl::compile_to_besl(script, Some(root_node)).unwrap();
		let evaluation = ProgramEvaluation::from_program(&program_node).expect("Failed to evaluate program");

		assert_eq!(evaluation.opacity(), OpacityEvaluation::Opaque);
	}

	#[test]
	fn opacity_vec4f_with_vec3f_first_param_uses_w_for_opacity() {
		fn evaluate(w: &str) -> OpacityEvaluation {
			let mut root_node = besl::Node::root();
			let void_type = root_node.get_child("void").unwrap();
			let vec3f_type = root_node.get_child("vec3f").unwrap();
			let vec4f_type = root_node.get_child("vec4f").unwrap();

			let output_node: besl::NodeReference = besl::Node::output("output", vec4f_type.clone(), 0).into();

			let vec3f_call = besl::Node::expression(besl::Expressions::FunctionCall {
				function: vec3f_type,
				parameters: vec![
					besl::Node::expression(besl::Expressions::Literal {
						value: "1.0".to_string(),
					})
					.into(),
					besl::Node::expression(besl::Expressions::Literal {
						value: "0.0".to_string(),
					})
					.into(),
					besl::Node::expression(besl::Expressions::Literal {
						value: "0.0".to_string(),
					})
					.into(),
				],
			})
			.into();

			let vec4f_call = besl::Node::expression(besl::Expressions::FunctionCall {
				function: vec4f_type,
				parameters: vec![
					vec3f_call,
					besl::Node::expression(besl::Expressions::Literal { value: w.to_string() }).into(),
				],
			})
			.into();

			let output_member = besl::Node::expression(besl::Expressions::Member {
				name: "output".to_string(),
				source: output_node.clone(),
			})
			.into();

			let assignment = besl::Node::expression(besl::Expressions::Operator {
				operator: besl::Operators::Assignment,
				left: output_member,
				right: vec4f_call,
			})
			.into();

			let main = besl::Node::function("main", Vec::new(), void_type, vec![assignment]).into();

			root_node.add_children(vec![output_node, main]);

			let program_node: besl::NodeReference = root_node.into();
			let evaluation = ProgramEvaluation::from_program(&program_node).expect("Failed to evaluate program");
			evaluation.opacity()
		}

		assert_eq!(evaluate("1.0"), OpacityEvaluation::Opaque);
		assert_eq!(evaluate("0.5"), OpacityEvaluation::NonOpaque);
	}
}
