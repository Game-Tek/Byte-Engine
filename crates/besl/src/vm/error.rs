//! Error types and diagnostics for VM compilation and execution.

use super::ResourceSlot;

#[derive(Debug, PartialEq, Eq)]
pub enum VmError {
	MissingMainFunction,
	UnsupportedRawCode,
	UnsupportedMainSignature {
		message: String,
	},
	UnsupportedType {
		type_name: String,
	},
	UnsupportedStatement {
		message: String,
	},
	UnsupportedAssignmentTarget {
		message: String,
	},
	UnsupportedExpression {
		message: String,
	},
	UnsupportedBufferLayout {
		message: String,
	},
	UnsupportedDescriptor {
		slot: ResourceSlot,
		message: String,
	},
	DescriptorAccessDenied {
		slot: ResourceSlot,
		access: &'static str,
	},
	DescriptorTypeMismatch {
		slot: ResourceSlot,
		expected: &'static str,
		found: &'static str,
	},
	UnknownBufferMember {
		member: String,
	},
	UnboundDescriptor {
		slot: ResourceSlot,
	},
	MissingPushConstant,
	MissingMeshOutputs,
	MissingTaskOutputs,
	MissingWorkgroupState,
	UninitializedWorkgroupValue {
		name: String,
	},
	MissingTaskPayload {
		name: String,
	},
	TaskPayloadIndexOutOfBounds {
		name: String,
		index: usize,
		count: usize,
	},
	TaskPayloadOutputIndexOutOfBounds {
		name: String,
		index: usize,
		count: usize,
	},
	MeshOutputIndexOutOfBounds {
		kind: &'static str,
		index: usize,
		count: usize,
	},
	MeshOutputCountLimitExceeded {
		kind: &'static str,
		requested: u32,
		limit: u32,
	},
	TaskMeshOutputCountLimitExceeded {
		requested: u32,
		limit: u32,
	},
	DivergentWorkgroupBarrier {
		lane: usize,
		expected_instruction: usize,
		found_instruction: Option<usize>,
	},
	MissingSpecialization {
		name: String,
	},
	InstructionLimitExceeded {
		limit: usize,
	},
	CallDepthLimitExceeded {
		limit: usize,
	},
	CallArgumentMismatch {
		expected: usize,
		found: usize,
	},
	BufferAccessOutOfBounds {
		offset: usize,
		size: usize,
		buffer_size: usize,
	},
	BufferArrayIndexOutOfBounds {
		index: usize,
		count: usize,
	},
	TextureAccessOutOfBounds {
		x: u32,
		y: u32,
		z: u32,
		width: u32,
		height: u32,
		depth: u32,
	},
	InvalidTextureDimensions {
		width: u32,
		height: u32,
		depth: u32,
	},
	TextureTexelCountOverflow {
		width: u32,
		height: u32,
		depth: u32,
	},
	TextureFormatMismatch {
		expected: &'static str,
		found: &'static str,
	},
	InvalidLiteral {
		value: String,
		value_type: String,
	},
	ArithmeticError {
		message: String,
	},
	TypeMismatch {
		expected: String,
		found: String,
	},
	UninitializedRegister {
		register: usize,
	},
	UninitializedLocal {
		local: usize,
	},
}

impl std::fmt::Display for VmError {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		match self {
			VmError::MissingMainFunction => {
				write!(
					f,
					"Missing main function. The most likely cause is that the lexed BESL program does not define `main`."
				)
			}
			VmError::UnsupportedRawCode => write!(
				f,
				"Raw code blocks are not supported. The most likely cause is that a reachable BESL function contains non-empty platform shader code with no portable VM semantics."
			),
			VmError::UnsupportedMainSignature { message } => write!(
				f,
				"Unsupported main signature: {}. The most likely cause is that the VM only accepts `main: fn () -> void` right now.",
				message
			),
			VmError::UnsupportedType { type_name } => write!(
				f,
				"Unsupported type `{}`. The most likely cause is that the BESL type has no portable VM value or resource representation.",
				type_name
			),
			VmError::UnsupportedStatement { message } => write!(
				f,
				"Unsupported statement. {}. The most likely cause is that this statement form has not been lowered into VM control-flow or side-effect instructions.",
				message
			),
			VmError::UnsupportedAssignmentTarget { message } => write!(
				f,
				"Unsupported assignment target. {}. The most likely cause is that the target is not a writable local, buffer member, output, or image operation.",
				message
			),
			VmError::UnsupportedExpression { message } => write!(
				f,
				"Unsupported expression. {}. The most likely cause is that this BESL syntax or operand-type combination has no VM lowering yet.",
				message
			),
			VmError::UnsupportedBufferLayout { message } => write!(
				f,
				"Unsupported buffer layout. {}. The most likely cause is that a member cannot be represented in the VM's packed CPU buffer layout.",
				message
			),
			VmError::UnsupportedDescriptor { slot, message } => write!(
				f,
				"Unsupported resource at slot {}. {}. The most likely cause is a resource-kind mismatch, incompatible reused slot, or unsupported resource access.",
				slot.slot(),
				message
			),
			VmError::DescriptorAccessDenied { slot, access } => write!(
				f,
				"Resource access denied at slot {}. The most likely cause is that the BESL resource was not declared with `{}` access.",
				slot.slot(),
				access
			),
			VmError::DescriptorTypeMismatch { slot, expected, found } => write!(
				f,
				"Resource type mismatch at slot {}: expected `{}` but found `{}`. The most likely cause is that the host bound a different resource kind than the compiled BESL program requires.",
				slot.slot(),
				expected,
				found
			),
			VmError::UnknownBufferMember { member } => write!(
				f,
				"Unknown buffer member `{}`. The most likely cause is that the BESL accessor does not match the bound buffer layout.",
				member
			),
			VmError::UnboundDescriptor { slot } => write!(
				f,
				"Unbound resource at slot {}. The most likely cause is that no resource was bound into the slot before execution.",
				slot.slot()
			),
			VmError::MissingPushConstant => write!(
				f,
				"Missing push constant binding. The most likely cause is that the BESL program reads `push_constant` but the host did not bind any push constant data before execution."
			),
			VmError::MissingMeshOutputs => write!(
				f,
				"Missing mesh output capture. The most likely cause is that the BESL mesh shader ran without binding `MeshOutputs`."
			),
			VmError::MissingTaskOutputs => write!(
				f,
				"Missing task output capture. The most likely cause is that the BESL task shader ran without binding `TaskOutputs`."
			),
			VmError::MissingWorkgroupState => write!(
				f,
				"Missing workgroup state. The most likely cause is that a BESL task shader accessed workgroup storage without binding `WorkgroupState`."
			),
			VmError::UninitializedWorkgroupValue { name } => write!(
				f,
				"Uninitialized workgroup value `{name}`. The most likely cause is that the BESL shader loaded workgroup storage before one invocation initialized it."
			),
			VmError::MissingTaskPayload { name } => write!(
				f,
				"Missing task payload `{name}`. The most likely cause is that the BESL mesh shader read a task-payload array that the host did not bind before execution."
			),
			VmError::TaskPayloadIndexOutOfBounds { name, index, count } => write!(
				f,
				"Task payload `{name}` index {index} exceeds {count} bound elements. The most likely cause is that the host supplied fewer task-payload values than the mesh shader reads."
			),
			VmError::TaskPayloadOutputIndexOutOfBounds { name, index, count } => write!(
				f,
				"Task payload output `{name}` index {index} exceeds {count} declared elements. The most likely cause is that the task shader wrote beyond its payload declaration."
			),
			VmError::MeshOutputIndexOutOfBounds { kind, index, count } => write!(
				f,
				"Mesh {kind} output index {index} exceeds {count} declared outputs. The most likely cause is that the shader wrote beyond the counts supplied to `set_mesh_output_counts`."
			),
			VmError::MeshOutputCountLimitExceeded {
				kind,
				requested,
				limit,
			} => write!(
				f,
				"Mesh {kind} output count {requested} exceeds the configured limit of {limit}. The most likely cause is that the shader requested more mesh output storage than the host allows."
			),
			VmError::TaskMeshOutputCountLimitExceeded { requested, limit } => write!(
				f,
				"Task mesh output count {requested} exceeds the configured limit of {limit}. The most likely cause is that the shader requested more mesh workgroups than the host allows."
			),
			VmError::DivergentWorkgroupBarrier {
				lane,
				expected_instruction,
				found_instruction,
			} => match found_instruction {
				Some(found_instruction) => write!(
					f,
					"Divergent workgroup barrier in lane {lane}: expected instruction {expected_instruction} but found {found_instruction}. The most likely cause is that task invocations reached different barriers in the same synchronization phase."
				),
				None => write!(
					f,
					"Divergent workgroup barrier in lane {lane}: expected instruction {expected_instruction} but the lane completed. The most likely cause is that task control flow skipped a barrier reached by peer invocations."
				),
			},
			VmError::MissingSpecialization { name } => write!(
				f,
				"Missing specialization `{}`. The most likely cause is that the host did not provide a value for a specialization used by the BESL program.",
				name
			),
			VmError::InstructionLimitExceeded { limit } => write!(
				f,
				"VM instruction limit {} exceeded. The most likely cause is that the BESL program contains an unbounded loop or needs a larger explicit execution budget.",
				limit
			),
			VmError::CallDepthLimitExceeded { limit } => write!(
				f,
				"VM call-depth limit {} exceeded. The most likely cause is that the BESL program recurses without reaching a base case.",
				limit
			),
			VmError::CallArgumentMismatch { expected, found } => write!(
				f,
				"Function call argument mismatch: expected {} arguments but found {}. The most likely cause is that the BESL function call does not match the declared parameter list.",
				expected, found
			),
			VmError::BufferAccessOutOfBounds {
				offset,
				size,
				buffer_size,
			} => write!(
				f,
				"Buffer access out of bounds at byte {} for {} bytes in a {} byte buffer. The most likely cause is that the bound buffer does not match the compiled BESL buffer layout.",
				offset, size, buffer_size
			),
			VmError::BufferArrayIndexOutOfBounds { index, count } => write!(
				f,
				"Buffer array index {} is out of bounds for {} elements. The most likely cause is that the BESL program indexed a buffer array member outside its declared length.",
				index, count
			),
			VmError::TextureAccessOutOfBounds {
				x,
				y,
				z,
				width,
				height,
				depth,
			} => write!(
				f,
				"Texture access out of bounds at ({}, {}, {}) in a {}x{}x{} texture. The most likely cause is that the BESL program fetched a texel outside the bound texture dimensions.",
				x, y, z, width, height, depth
			),
			VmError::InvalidTextureDimensions { width, height, depth } => write!(
				f,
				"Invalid texture dimensions {}x{}x{}. The most likely cause is that the host created a texture with a zero dimension.",
				width, height, depth
			),
			VmError::TextureTexelCountOverflow { width, height, depth } => write!(
				f,
				"Texture dimensions {}x{}x{} are too large. The most likely cause is that their texel count exceeds addressable CPU memory.",
				width, height, depth
			),
			VmError::TextureFormatMismatch { expected, found } => write!(
				f,
				"Texture format mismatch: expected `{}` but found `{}`. The most likely cause is that the same CPU texture was used for incompatible float and integer shader operations.",
				expected, found
			),
			VmError::InvalidLiteral { value, value_type } => write!(
				f,
				"Invalid literal `{}` for `{}`. The most likely cause is that the literal cannot be parsed as the target BESL scalar type.",
				value, value_type
			),
			VmError::ArithmeticError { message } => write!(
				f,
				"Invalid arithmetic operation. {}. The most likely cause is that the BESL program evaluated an unsupported numeric operation such as division or modulo by zero.",
				message
			),
			VmError::TypeMismatch { expected, found } => write!(
				f,
				"Type mismatch: expected `{}` but found `{}`. The most likely cause is that the BESL assignment mixes incompatible scalar types.",
				expected, found
			),
			VmError::UninitializedRegister { register } => write!(
				f,
				"Uninitialized register {}. The most likely cause is that the VM tried to use a register before any instruction wrote a value into it.",
				register
			),
			VmError::UninitializedLocal { local } => write!(
				f,
				"Uninitialized local {}. The most likely cause is that the BESL program read a local variable before assigning a value to it.",
				local
			),
		}
	}
}

impl std::error::Error for VmError {}
