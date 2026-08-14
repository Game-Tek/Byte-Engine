use super::*;

pub(crate) fn read_register(registers: &[Option<Value>], register: usize) -> Result<Value, VmError> {
	registers
		.get(register)
		.and_then(Option::clone)
		.ok_or(VmError::UninitializedRegister { register })
}

pub(crate) fn resolve_resource_slot(slot: ResourceSlot, registers: &[Option<Value>]) -> Result<ResourceSlot, VmError> {
	if !slot.is_dynamic_resource() {
		return Ok(slot);
	}
	match read_register(registers, slot.slot() as usize)? {
		Value::Resource { slot, .. } => Ok(slot),
		value => Err(VmError::TypeMismatch {
			expected: "resource handle".to_string(),
			found: value.value_type().name().to_string(),
		}),
	}
}

pub(crate) fn read_buffer_array_index(registers: &[Option<Value>], register: usize, count: usize) -> Result<usize, VmError> {
	let index = read_register(registers, register)?;
	let Value::U32(index) = index else {
		return Err(VmError::TypeMismatch {
			expected: ValueType::U32.name().to_string(),
			found: index.value_type().name().to_string(),
		});
	};
	let index = index as usize;
	if index >= count {
		return Err(VmError::BufferArrayIndexOutOfBounds { index, count });
	}

	Ok(index)
}
