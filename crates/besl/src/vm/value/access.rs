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
		value => Err(VmError::mismatch("resource handle", value.value_type().name())),
	}
}

pub(crate) fn read_buffer_array_index(registers: &[Option<Value>], register: usize, count: usize) -> Result<usize, VmError> {
	let index = expect_u32(register_ref(registers, register)?)? as usize;
	if index >= count {
		return Err(VmError::BufferArrayIndexOutOfBounds { index, count });
	}

	Ok(index)
}

/// Copies `value` into `slot`, reusing the buffers of a struct the slot already holds.
///
/// Use it wherever the VM copies a value into a register, local, or payload slot, so a frame that runs again copies
/// structs without allocating.
pub(crate) fn store(slot: &mut Option<Value>, value: &Value) {
	match slot {
		Some(held) => held.clone_from(value),
		None => *slot = Some(value.clone()),
	}
}

/// Copies what `select` picks from register `source` into register `target`, reusing the buffers of a struct the
/// target already holds.
///
/// `select` returns the source value itself, or a part of it such as a struct's field.
pub(crate) fn copy_register(
	registers: &mut [Option<Value>],
	target: usize,
	source: usize,
	select: impl FnOnce(&Value) -> Result<&Value, VmError>,
) -> Result<(), VmError> {
	if target >= registers.len() {
		return Err(VmError::UninitializedRegister { register: target });
	}
	if source >= registers.len() {
		return Err(VmError::UninitializedRegister { register: source });
	}
	if target == source {
		// A register can't be read and written at once; lowering never emits this, but a copy keeps it correct.
		let value = select(register_ref(registers, source)?)?.clone();
		registers[target] = Some(value);
		return Ok(());
	}
	let (target_slot, value) = target_and_source(registers, target, source);
	store(target_slot, select(value?)?);
	Ok(())
}

/// Writes register `member`'s value into member `index` of the aggregate in register `target`, reusing the buffers of
/// the field it replaces.
pub(crate) fn insert_register(
	registers: &mut [Option<Value>],
	target: usize,
	index: usize,
	member: usize,
) -> Result<(), VmError> {
	if member >= registers.len() {
		return Err(VmError::UninitializedRegister { register: member });
	}
	if target == member {
		// Lowering never inserts an aggregate into itself, but a copy keeps it correct.
		let member = register_ref(registers, member)?.clone();
		let aggregate = registers[target]
			.as_mut()
			.ok_or(VmError::UninitializedRegister { register: target })?;
		return insert_value(aggregate, index, &member);
	}
	let (target_slot, member) = target_and_source(registers, target, member);
	let aggregate = target_slot
		.as_mut()
		.ok_or(VmError::UninitializedRegister { register: target })?;
	insert_value(aggregate, index, member?)
}

/// Lends register `target` for writing and the value of register `source` for reading. The two must differ and be in
/// range.
fn target_and_source(
	registers: &mut [Option<Value>],
	target: usize,
	source: usize,
) -> (&mut Option<Value>, Result<&Value, VmError>) {
	// Splitting the registers between the two indices lends one for writing and the other for reading.
	let (target_slot, source_slot) = if target < source {
		let (low, high) = registers.split_at_mut(source);
		(&mut low[target], &high[0])
	} else {
		let (low, high) = registers.split_at_mut(target);
		(&mut high[0], &low[source])
	};
	(
		target_slot,
		source_slot
			.as_ref()
			.ok_or(VmError::UninitializedRegister { register: source }),
	)
}
