use super::*;

/// Borrows one operand register. Instructions compute their result from borrowed operands before writing the
/// destination, so reads never clone register values.
pub(crate) fn register_ref(registers: &[Option<Value>], register: usize) -> Result<&Value, VmError> {
	registers
		.get(register)
		.and_then(Option::as_ref)
		.ok_or(VmError::UninitializedRegister { register })
}

pub(crate) fn resolve_resource_slot(slot: ResourceSlot, registers: &[Option<Value>]) -> Result<ResourceSlot, VmError> {
	if !slot.is_dynamic_resource() {
		return Ok(slot);
	}
	match register_ref(registers, slot.slot() as usize)? {
		Value::Resource { slot, .. } => Ok(*slot),
		value => Err(VmError::TypeMismatch {
			expected: "resource handle".to_string(),
			found: value.value_type().name().to_string(),
		}),
	}
}

pub(crate) fn read_buffer_array_index(registers: &[Option<Value>], register: usize, count: usize) -> Result<usize, VmError> {
	let index = expect_u32(register_ref(registers, register)?)? as usize;
	if index >= count {
		return Err(VmError::BufferArrayIndexOutOfBounds { index, count });
	}

	Ok(index)
}
