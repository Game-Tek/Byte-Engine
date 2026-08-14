//! BESL analysis and lowering into executable VM instructions.

use std::collections::HashMap;

use super::super::*;
use super::support::*;

mod calls;
mod expressions;
mod intrinsics;
mod resources;
mod statements;

/// Compiles one lexed program while keeping compiler implementation details behind this seam.
#[allow(clippy::mutable_key_type)]
pub(crate) fn compile(program: NodeReference, specializations: &SpecializationValues) -> Result<ExecutableProgram, VmError> {
	let main = resolve_main_function(&program)?;
	let main_signature = extract_function_signature(&main)?;
	if !main_signature.params.is_empty() {
		return Err(VmError::UnsupportedMainSignature {
			message: "Main functions with parameters are not supported".to_string(),
		});
	}
	if main_signature.return_type.is_some() {
		return Err(VmError::UnsupportedMainSignature {
			message: format!(
				"Main functions must return void, but found `{}`",
				main_signature.return_type.as_ref().map(ValueType::name).unwrap_or("void")
			),
		});
	}

	let function_nodes = collect_functions(&main);
	for function in &function_nodes {
		reject_raw_code_nodes(function)?;
	}
	// NodeReference hashing is pointer-identity based, so function lookup remains stable for RefCell-backed nodes.
	let mut function_ids = HashMap::new();
	for (index, function) in function_nodes.iter().enumerate() {
		function_ids.insert(function.clone(), index);
	}

	let mut descriptor_layouts = HashMap::new();
	let mut functions = Vec::with_capacity(function_nodes.len());
	for function in &function_nodes {
		functions.push(Compiler::compile_function(
			function,
			&function_ids,
			&mut descriptor_layouts,
			specializations,
		)?);
	}

	Ok(ExecutableProgram {
		descriptor_layouts,
		functions,
		main_function: function_ids[&main],
	})
}

/// Rejects malformed linked calls before any argument is indexed or lowered.
fn require_argument_count(arguments: &[NodeReference], expected: usize) -> Result<(), VmError> {
	let found = arguments.len();
	if found != expected {
		return Err(VmError::CallArgumentMismatch { expected, found });
	}
	Ok(())
}

/// Resolves one parameter from the overload selected by BESL type checking.
fn resolve_intrinsic_parameter_type(intrinsic: &NodeReference, index: usize) -> Result<ValueType, VmError> {
	let intrinsic_ref = intrinsic.borrow();
	let Nodes::Intrinsic { elements, .. } = intrinsic_ref.node() else {
		return Err(VmError::UnsupportedExpression {
			message: format!("Expected an intrinsic, but found {}", describe_node(intrinsic_ref.node())),
		});
	};
	let parameter_type = elements
		.iter()
		.filter_map(|element| match element.borrow().node() {
			Nodes::Parameter { r#type, .. } => Some(r#type.clone()),
			_ => None,
		})
		.nth(index)
		.ok_or(VmError::CallArgumentMismatch {
			expected: index + 1,
			found: index,
		})?;
	drop(intrinsic_ref);
	resolve_value_type(&parameter_type)
}

/// The `Compiler` struct lowers one BESL function into bounded register-machine instructions.
struct Compiler<'a> {
	function_ids: &'a HashMap<NodeReference, usize>,
	specializations: &'a SpecializationValues,
	instructions: Vec<Instruction>,
	local_types: Vec<ValueType>,
	locals_by_reference: HashMap<NodeReference, usize>,
	register_count: usize,
	return_type: Option<ValueType>,
	parameter_count: usize,
	loop_continue_targets: Vec<usize>,
	loop_continue_patches: Vec<Vec<usize>>,
}

impl<'a> Compiler<'a> {
	#[allow(clippy::mutable_key_type)]
	fn compile_function(
		function: &NodeReference,
		function_ids: &'a HashMap<NodeReference, usize>,
		descriptor_layouts: &mut HashMap<ResourceSlot, DescriptorLayout>,
		specializations: &'a SpecializationValues,
	) -> Result<ExecutableFunction, VmError> {
		let signature = extract_function_signature(function)?;
		let mut compiler = Self {
			function_ids,
			specializations,
			instructions: Vec::new(),
			local_types: Vec::new(),
			locals_by_reference: HashMap::new(),
			register_count: 0,
			return_type: signature.return_type.clone(),
			parameter_count: signature.params.len(),
			loop_continue_targets: Vec::new(),
			loop_continue_patches: Vec::new(),
		};

		for (index, param) in signature.params.iter().enumerate() {
			compiler.local_types.push(param.value_type.clone());
			compiler.locals_by_reference.insert(param.node.clone(), index);
		}

		for statement in &signature.statements {
			compiler.compile_statement(statement, descriptor_layouts)?;
		}

		if compiler.return_type.is_none() && !matches!(compiler.instructions.last(), Some(Instruction::Return { .. })) {
			compiler.instructions.push(Instruction::Return { register: None });
		}

		Ok(ExecutableFunction {
			instructions: compiler.instructions,
			local_types: compiler.local_types,
			register_count: compiler.register_count,
			parameter_count: compiler.parameter_count,
			return_type: compiler.return_type,
		})
	}
}
