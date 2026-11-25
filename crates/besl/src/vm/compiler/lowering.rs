//! BESL analysis and lowering into executable VM instructions.

use std::collections::HashMap;

use super::super::*;
use super::support::*;
use crate::NodeMap;

mod calls;
mod expressions;
mod intrinsics;
mod resources;
mod statements;

/// Compiles one lexed program while keeping compiler implementation details behind this seam.
#[allow(clippy::mutable_key_type)]
pub(crate) fn compile(program: &Program, specializations: &SpecializationValues) -> Result<ExecutableProgram, VmError> {
	let main = program.get_main().ok_or(VmError::MissingMainFunction)?;
	let closest_hit_payload = closest_hit_payload(&extract_function_signature(program, &main)?)?;

	let function_nodes = program.reachable_functions(main);
	let mut function_ids = NodeMap::default();
	for (index, function) in function_nodes.iter().enumerate() {
		function_ids.insert(*function, index);
	}

	let mut descriptor_layouts = HashMap::new();
	let mut functions = Vec::with_capacity(function_nodes.len());
	for function in &function_nodes {
		functions.push(Compiler::compile_function(
			program,
			function,
			&function_ids,
			&mut descriptor_layouts,
			specializations,
		)?);
	}

	// The lexer makes every `invoke` call in a program pass the same payload type, so the first call names it.
	let invoke_payload =
		functions
			.iter()
			.flat_map(|function| &function.instructions)
			.find_map(|instruction| match instruction {
				Instruction::RayTracing(RayTracingInstruction::Invoke { payload_type, .. }) => Some(payload_type.clone()),
				_ => None,
			});

	let mut descriptor_slots = HashMap::new();
	collect_descriptor_slots(program, &program.root(), &mut descriptor_slots);

	Ok(ExecutableProgram {
		descriptor_layouts,
		descriptor_slots,
		functions,
		main_function: function_ids[&main],
		closest_hit_payload,
		invoke_payload,
	})
}

/// Records the slot of every placed descriptor the program, or a layer under it, declares, by name.
fn collect_descriptor_slots(program: &Program, node: &NodeId, slots: &mut HashMap<String, ResourceSlot>) {
	let borrowed = &program[node];
	match borrowed.node() {
		Nodes::Scope { children, .. } | Nodes::Layer { members: children, .. } => {
			for child in children {
				collect_descriptor_slots(program, child, slots);
			}
		}
		Nodes::Binding {
			name,
			placement: Some(placement),
			..
		} => {
			slots.insert(program.text(*name).to_string(), ResourceSlot::placed(*placement));
		}
		_ => {}
	}
}

/// Returns the payload of a closest-hit `main`, which takes a payload struct and returns the same type, or `None` for
/// every other `main`, which takes nothing and returns nothing.
fn closest_hit_payload(signature: &FunctionSignature) -> Result<Option<ValueType>, VmError> {
	match (signature.params.as_slice(), &signature.return_type) {
		([], None) => Ok(None),
		([payload], Some(return_type @ ValueType::Struct { .. })) if payload.value_type == *return_type => {
			Ok(Some(return_type.clone()))
		}
		([], Some(return_type)) => Err(VmError::UnsupportedMainSignature {
			message: format!("Main functions must return void, but found `{}`", return_type.name()),
		}),
		_ => Err(VmError::UnsupportedMainSignature {
			message: "Main functions with parameters are not supported, except a closest hit's payload".to_string(),
		}),
	}
}

/// Rejects malformed linked calls before any argument is indexed or lowered.
fn require_argument_count(arguments: &[NodeId], expected: usize) -> Result<(), VmError> {
	let found = arguments.len();
	if found != expected {
		return Err(VmError::CallArgumentMismatch { expected, found });
	}
	Ok(())
}

/// Resolves one parameter from the overload selected by BESL type checking.
fn resolve_intrinsic_parameter_type(program: &Program, intrinsic: &NodeId, index: usize) -> Result<ValueType, VmError> {
	let intrinsic_ref = &program[intrinsic];
	let Nodes::Intrinsic { elements, .. } = intrinsic_ref.node() else {
		return Err(VmError::UnsupportedExpression {
			message: format!("Expected an intrinsic, but found {}", describe_node(intrinsic_ref.node())),
		});
	};
	let parameter_type = elements
		.iter()
		.filter_map(|element| match program[element].node() {
			Nodes::Parameter { r#type, .. } => Some(*r#type),
			_ => None,
		})
		.nth(index)
		.ok_or(VmError::CallArgumentMismatch {
			expected: index + 1,
			found: index,
		})?;
	resolve_value_type(program, &parameter_type)
}

/// The `Compiler` struct lowers one BESL function into bounded register-machine instructions.
struct Compiler<'a> {
	/// The linked program whose nodes the compiler reads.
	program: &'a Program<'a>,
	function_ids: &'a NodeMap<usize>,
	specializations: &'a SpecializationValues,
	instructions: Vec<Instruction>,
	local_types: Vec<ValueType>,
	locals_by_reference: NodeMap<usize>,
	register_count: usize,
	return_type: Option<ValueType>,
	descriptor_layouts: &'a mut HashMap<ResourceSlot, DescriptorLayout>,
	/// The jump placeholders of each open loop, innermost last.
	loops: Vec<LoopPatches>,
}

/// The `LoopPatches` struct collects the jumps that `continue` and `break` emit inside one open loop. The loop points
/// them at its update and its end once both are compiled.
#[derive(Default)]
struct LoopPatches {
	continues: Vec<usize>,
	breaks: Vec<usize>,
}

impl<'a> Compiler<'a> {
	/// Appends one instruction, taking either a grouped sub-enum or a top-level [`Instruction`].
	fn emit(&mut self, instruction: impl Into<Instruction>) {
		self.instructions.push(instruction.into());
	}

	/// Allocates a register, emits the instruction `build` makes for it, and returns the register.
	fn emit_to<I: Into<Instruction>>(&mut self, build: impl FnOnce(usize) -> I) -> usize {
		let register = self.allocate_register();
		self.emit(build(register));
		register
	}

	/// Compiles the `u32` element index of an indexed workgroup or buffer access, or nothing for a whole value.
	fn compile_index(&mut self, index: Option<&NodeId>) -> Result<Option<usize>, VmError> {
		index.map(|index| self.compile_value_expression(index, &ValueType::U32)).transpose()
	}

	#[allow(clippy::mutable_key_type)]
	fn compile_function(
		program: &'a Program<'a>,
		function: &NodeId,
		function_ids: &'a NodeMap<usize>,
		descriptor_layouts: &'a mut HashMap<ResourceSlot, DescriptorLayout>,
		specializations: &'a SpecializationValues,
	) -> Result<ExecutableFunction, VmError> {
		let signature = extract_function_signature(program, function)?;
		let mut compiler = Self {
			program,
			function_ids,
			specializations,
			instructions: Vec::new(),
			local_types: Vec::new(),
			locals_by_reference: NodeMap::default(),
			register_count: 0,
			return_type: signature.return_type.clone(),
			descriptor_layouts,
			loops: Vec::new(),
		};

		for (index, param) in signature.params.iter().enumerate() {
			compiler.local_types.push(param.value_type.clone());
			compiler.locals_by_reference.insert(param.node, index);
		}

		for statement in &signature.statements {
			compiler.compile_statement(statement)?;
		}

		if compiler.return_type.is_none()
			&& !matches!(
				compiler.instructions.last(),
				Some(Instruction::Control(ControlInstruction::Return { .. }))
			) {
			compiler.emit(ControlInstruction::Return { register: None });
		}

		Ok(ExecutableFunction {
			instructions: compiler.instructions,
			local_types: compiler.local_types,
			register_count: compiler.register_count,
			return_type: compiler.return_type,
		})
	}
}
