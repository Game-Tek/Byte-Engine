//! Compiled program metadata and bounded invocation configuration.

use std::collections::HashMap;

use super::*;

/// The `SpecializationValues` struct supplies host-selected values for BESL specialization declarations.
#[derive(Clone, Debug, Default)]
pub struct SpecializationValues {
	values: HashMap<String, Value>,
}

impl SpecializationValues {
	/// Creates an empty specialization map for programs that use only defaults or no specializations.
	pub fn new() -> Self {
		Self::default()
	}

	/// Supplies one named specialization value before compiling an executable program.
	pub fn set(&mut self, name: impl Into<String>, value: Value) -> Option<Value> {
		self.values.insert(name.into(), value)
	}

	/// Returns a previously supplied specialization value by declaration name.
	pub fn get(&self, name: &str) -> Option<&Value> {
		self.values.get(name)
	}
}

/// The `ExecutionConfig` struct bounds a VM invocation and supplies its shader-visible thread coordinates.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ExecutionConfig {
	instruction_limit: usize,
	call_depth_limit: usize,
	max_mesh_vertex_count: u32,
	max_mesh_primitive_count: u32,
	max_task_mesh_output_count: u32,
	thread_id: [u32; 2],
	thread_idx: u32,
	thread_position: u32,
	threadgroup_position: u32,
	subgroup_size: u32,
}

impl Default for ExecutionConfig {
	fn default() -> Self {
		Self {
			instruction_limit: 1_000_000,
			call_depth_limit: 64,
			max_mesh_vertex_count: 256,
			max_mesh_primitive_count: 256,
			max_task_mesh_output_count: 256,
			thread_id: [0, 0],
			thread_idx: 0,
			thread_position: 0,
			threadgroup_position: 0,
			subgroup_size: 32,
		}
	}
}

impl ExecutionConfig {
	/// Creates an invocation config with an explicit instruction budget and default coordinates.
	pub fn new(instruction_limit: usize) -> Self {
		Self {
			instruction_limit,
			..Self::default()
		}
	}

	/// Returns the maximum number of instructions shared by the invocation's call tree.
	pub const fn instruction_limit(&self) -> usize {
		self.instruction_limit
	}

	/// Returns the maximum nested BESL function-call depth.
	pub const fn call_depth_limit(&self) -> usize {
		self.call_depth_limit
	}

	/// Returns the maximum vertex count a mesh invocation may request.
	pub const fn max_mesh_vertex_count(&self) -> u32 {
		self.max_mesh_vertex_count
	}

	/// Returns the maximum primitive count a mesh invocation may request.
	pub const fn max_mesh_primitive_count(&self) -> u32 {
		self.max_mesh_primitive_count
	}

	/// Returns the maximum mesh workgroup count a task invocation may request.
	pub const fn max_task_mesh_output_count(&self) -> u32 {
		self.max_task_mesh_output_count
	}

	/// Returns the two-dimensional compute invocation coordinate.
	pub const fn thread_id(&self) -> [u32; 2] {
		self.thread_id
	}

	/// Returns the mesh or workgroup-local invocation index.
	pub const fn thread_idx(&self) -> u32 {
		self.thread_idx
	}

	/// Returns the task invocation's scalar position in the dispatched grid.
	pub const fn thread_position(&self) -> u32 {
		self.thread_position
	}

	/// Returns the mesh workgroup position visible to the shader.
	pub const fn threadgroup_position(&self) -> u32 {
		self.threadgroup_position
	}

	/// Returns the number of active lanes that participate in this invocation's subgroup collectives.
	pub const fn subgroup_size(&self) -> u32 {
		self.subgroup_size
	}

	/// Selects an explicit nested function-call limit for this invocation.
	pub fn with_call_depth_limit(mut self, limit: usize) -> Self {
		self.call_depth_limit = limit;
		self
	}

	/// Selects the maximum vertex count accepted from mesh output-count intrinsics.
	pub fn with_max_mesh_vertex_count(mut self, limit: u32) -> Self {
		self.max_mesh_vertex_count = limit;
		self
	}

	/// Selects the maximum primitive count accepted from mesh output-count intrinsics.
	pub fn with_max_mesh_primitive_count(mut self, limit: u32) -> Self {
		self.max_mesh_primitive_count = limit;
		self
	}

	/// Selects the maximum mesh workgroup count accepted from task output-count intrinsics.
	pub fn with_max_task_mesh_output_count(mut self, limit: u32) -> Self {
		self.max_task_mesh_output_count = limit;
		self
	}

	/// Selects the two-dimensional compute invocation coordinate.
	pub fn with_thread_id(mut self, thread_id: [u32; 2]) -> Self {
		self.thread_id = thread_id;
		self
	}

	/// Selects the mesh or workgroup-local invocation index.
	pub fn with_thread_idx(mut self, thread_idx: u32) -> Self {
		self.thread_idx = thread_idx;
		self
	}

	/// Selects the task invocation's scalar position in the dispatched grid.
	pub fn with_thread_position(mut self, position: u32) -> Self {
		self.thread_position = position;
		self
	}

	/// Selects the mesh workgroup position visible to the shader.
	pub fn with_threadgroup_position(mut self, position: u32) -> Self {
		self.threadgroup_position = position;
		self
	}

	/// Selects the subgroup width used by VM collective execution.
	pub fn with_subgroup_size(mut self, subgroup_size: u32) -> Self {
		self.subgroup_size = subgroup_size;
		self
	}
}

/// The `ExecutionState` struct shares invocation limits and coordinates across nested VM calls.
pub(super) struct ExecutionState<'a> {
	pub(super) config: &'a ExecutionConfig,
	remaining_instructions: usize,
	call_depth: usize,
	pub(super) discarded: bool,
}

impl<'a> ExecutionState<'a> {
	pub(super) fn new(config: &'a ExecutionConfig) -> Self {
		Self {
			config,
			remaining_instructions: config.instruction_limit(),
			call_depth: 0,
			discarded: false,
		}
	}

	pub(super) fn consume_instruction(&mut self) -> Result<(), VmError> {
		if self.remaining_instructions == 0 {
			return Err(VmError::InstructionLimitExceeded {
				limit: self.config.instruction_limit(),
			});
		}
		self.remaining_instructions -= 1;
		Ok(())
	}

	pub(super) fn enter_call(&mut self) -> Result<(), VmError> {
		if self.call_depth >= self.config.call_depth_limit() {
			return Err(VmError::CallDepthLimitExceeded {
				limit: self.config.call_depth_limit(),
			});
		}
		self.call_depth += 1;
		Ok(())
	}

	pub(super) fn leave_call(&mut self) {
		self.call_depth -= 1;
	}
}

/// The `ExecutableProgram` struct provides a reusable host-side execution form for one lexed BESL program.
pub struct ExecutableProgram {
	pub(super) descriptor_layouts: HashMap<ResourceSlot, DescriptorLayout>,
	pub(super) functions: Vec<ExecutableFunction>,
	pub(super) main_function: usize,
}

/// The `ExecutableFunction` struct isolates one compiled BESL call target for bounded VM execution.
pub(super) struct ExecutableFunction {
	pub(super) instructions: Vec<Instruction>,
	pub(super) local_types: Vec<ValueType>,
	pub(super) register_count: usize,
	pub(super) parameter_count: usize,
	pub(super) return_type: Option<ValueType>,
}

/// The `ExecutionFrame` struct retains reusable register and local storage for one active VM function.
#[derive(Default)]
pub(super) struct ExecutionFrame {
	pub(super) function_index: usize,
	pub(super) registers: Vec<Option<Value>>,
	pub(super) locals: Vec<Option<Value>>,
	pub(super) constructor_values: Vec<Value>,
	pub(super) instruction_index: usize,
}

impl ExecutionFrame {
	/// Resets retained storage for a parameterless function without reallocating after its first use.
	pub(super) fn reset(&mut self, function_index: usize, function: &ExecutableFunction) -> Result<(), VmError> {
		if function.parameter_count != 0 {
			return Err(VmError::CallArgumentMismatch {
				expected: function.parameter_count,
				found: 0,
			});
		}
		self.reset_storage(function_index, function);
		Ok(())
	}

	/// Initializes parameter locals directly from the caller's registers without an intermediate argument vector.
	pub(super) fn reset_from_registers(
		&mut self,
		function_index: usize,
		function: &ExecutableFunction,
		arguments: &[usize],
		caller_registers: &[Option<Value>],
	) -> Result<(), VmError> {
		if arguments.len() != function.parameter_count {
			return Err(VmError::CallArgumentMismatch {
				expected: function.parameter_count,
				found: arguments.len(),
			});
		}
		self.reset_storage(function_index, function);
		for (local, register) in arguments.iter().copied().enumerate() {
			self.locals[local] = Some(read_register(caller_registers, register)?);
		}
		Ok(())
	}

	fn reset_storage(&mut self, function_index: usize, function: &ExecutableFunction) {
		// Clearing preserves vector capacity so repeated invocations reuse their previous frame allocations.
		self.function_index = function_index;
		self.instruction_index = 0;
		self.registers.clear();
		self.registers.resize(function.register_count, None);
		self.locals.clear();
		self.locals.resize(function.local_types.len(), None);
	}
}

impl ExecutableProgram {
	/// Compiles a lexed BESL program into a runnable VM program.
	#[allow(clippy::mutable_key_type)]
	pub fn compile(program: NodeReference) -> Result<Self, VmError> {
		Self::compile_with_specializations(program, &SpecializationValues::new())
	}

	/// Compiles a lexed BESL program using host-provided specialization values.
	#[allow(clippy::mutable_key_type)]
	pub fn compile_with_specializations(
		program: NodeReference,
		specializations: &SpecializationValues,
	) -> Result<Self, VmError> {
		compiler::compile(program, specializations)
	}

	pub fn descriptor_layout(&self, slot: ResourceSlot) -> Option<&DescriptorLayout> {
		self.descriptor_layouts.get(&slot)
	}

	pub fn buffer_layout(&self, slot: ResourceSlot) -> Option<&BufferLayout> {
		match self.descriptor_layouts.get(&slot) {
			Some(DescriptorLayout::Buffer(layout)) => Some(layout),
			Some(DescriptorLayout::Texture) => None,
			Some(DescriptorLayout::Image) => None,
			Some(DescriptorLayout::PushConstant(_)) => None,
			None => None,
		}
	}

	pub fn push_constant_layout(&self) -> Option<&BufferLayout> {
		self.descriptor_layouts.values().find_map(|layout| match layout {
			DescriptorLayout::PushConstant(layout) => Some(layout),
			_ => None,
		})
	}

	pub fn input_layout(&self, location: u8) -> Option<&BufferLayout> {
		self.buffer_layout(input_slot(location))
	}

	pub fn output_layout(&self, location: u8) -> Option<&BufferLayout> {
		self.buffer_layout(output_slot(location))
	}

	pub fn builtin_position_layout(&self) -> Option<&BufferLayout> {
		self.buffer_layout(builtin_position_slot())
	}
}
