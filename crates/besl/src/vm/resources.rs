//! Invocation-scoped descriptor bindings and shared shader-stage capture state.

use std::collections::HashMap;

use super::*;

enum DescriptorBinding<'a> {
	Buffer(&'a mut Buffer),
	Texture { texture: &'a mut Texture, sampler: Sampler },
	Image(&'a mut Texture),
}

impl DescriptorBinding<'_> {
	const fn kind(&self) -> &'static str {
		match self {
			Self::Buffer(_) => "buffer",
			Self::Texture { .. } => "texture",
			Self::Image(_) => "image",
		}
	}

	fn type_mismatch(&self, slot: ResourceSlot, expected: &'static str) -> VmError {
		VmError::DescriptorTypeMismatch {
			slot,
			expected,
			found: self.kind(),
		}
	}
}

/// The `MeshOutputs` struct captures mesh-stage rasterization outputs for VM assertions.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct MeshOutputs {
	vertex_count: u32,
	primitive_count: u32,
	pub(super) vertex_positions: Vec<[f32; 4]>,
	pub(super) triangles: Vec<[u32; 3]>,
	pub(super) render_target_array_indices: Vec<u32>,
}

impl MeshOutputs {
	/// Creates an empty capture that can be bound before a mesh shader invocation.
	pub fn new() -> Self {
		Self::default()
	}

	/// Returns the vertex count declared by the most recent mesh invocation.
	pub const fn vertex_count(&self) -> u32 {
		self.vertex_count
	}

	/// Returns the primitive count declared by the most recent mesh invocation.
	pub const fn primitive_count(&self) -> u32 {
		self.primitive_count
	}

	/// Returns one captured mesh vertex position when the shader wrote that declared slot.
	pub fn vertex_position(&self, index: usize) -> Option<[f32; 4]> {
		self.vertex_positions.get(index).copied()
	}

	/// Returns one captured mesh triangle when the shader wrote that declared slot.
	pub fn triangle(&self, index: usize) -> Option<[u32; 3]> {
		self.triangles.get(index).copied()
	}

	/// Returns the render-target array layer selected for one declared primitive.
	pub fn render_target_array_index(&self, index: usize) -> Option<u32> {
		self.render_target_array_indices.get(index).copied()
	}

	/// Prepares mesh output ranges after validating shader-controlled counts.
	pub(super) fn set_counts(
		&mut self,
		vertex_count: u32,
		primitive_count: u32,
		max_vertex_count: u32,
		max_primitive_count: u32,
		clear: bool,
	) -> Result<(), VmError> {
		if vertex_count > max_vertex_count {
			return Err(VmError::MeshOutputCountLimitExceeded {
				kind: "vertex",
				requested: vertex_count,
				limit: max_vertex_count,
			});
		}
		if primitive_count > max_primitive_count {
			return Err(VmError::MeshOutputCountLimitExceeded {
				kind: "primitive",
				requested: primitive_count,
				limit: max_primitive_count,
			});
		}

		if clear {
			self.begin_invocation();
		}
		self.vertex_count = vertex_count;
		self.primitive_count = primitive_count;
		self.vertex_positions.resize(vertex_count as usize, [0.0; 4]);
		self.triangles.resize(primitive_count as usize, [0; 3]);
		self.render_target_array_indices.resize(primitive_count as usize, 0);
		Ok(())
	}

	pub(super) fn begin_invocation(&mut self) {
		// The first lane clears the shared capture once; later workgroup lanes retain earlier lane writes.
		self.vertex_positions.fill([0.0; 4]);
		self.triangles.fill([0; 3]);
		self.render_target_array_indices.fill(0);
	}
}

/// The `TaskOutputs` struct captures task-stage mesh dispatch counts and payload values for VM assertions.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct TaskOutputs {
	mesh_output_count: Option<u32>,
	payloads: HashMap<String, Vec<Option<Value>>>,
}

impl TaskOutputs {
	/// Creates an empty capture that can be bound before a task shader invocation.
	pub fn new() -> Self {
		Self::default()
	}

	/// Returns the mesh workgroup count declared by the task invocation, if it declared one.
	pub const fn mesh_output_count(&self) -> Option<u32> {
		self.mesh_output_count
	}

	/// Returns one task-payload value when the shader wrote the requested declared element.
	pub fn payload_value(&self, name: &str, index: usize) -> Option<&Value> {
		self.payloads.get(name)?.get(index)?.as_ref()
	}

	pub(super) fn set_mesh_output_count(&mut self, count: u32) {
		self.mesh_output_count = Some(count);
		let count = count as usize;
		for payload in self.payloads.values_mut() {
			if count < payload.len() {
				// Values outside the published dispatch range must not survive capture reuse.
				payload[count..].fill(None);
			}
		}
	}

	/// Clears shader-authored values while retaining the capture's allocated payload storage.
	pub(super) fn begin_workgroup(&mut self) {
		self.mesh_output_count = None;
		for payload in self.payloads.values_mut() {
			payload.fill(None);
		}
	}

	/// Writes one declared task-payload element while preserving earlier lane writes in the same capture.
	pub(super) fn write_payload(&mut self, name: &str, index: usize, count: usize, value: Value) -> Result<(), VmError> {
		if index >= count {
			return Err(VmError::TaskPayloadOutputIndexOutOfBounds {
				name: name.to_string(),
				index,
				count,
			});
		}

		let payload = if let Some(payload) = self.payloads.get_mut(name) {
			payload
		} else {
			self.payloads.insert(name.to_string(), vec![None; count]);
			self.payloads.get_mut(name).expect(
				"Missing inserted task payload. The most likely cause is that the payload map changed between insertion and lookup.",
			)
		};
		if payload.len() != count {
			// A capture is scoped to one declared task interface; clear stale values if a caller reuses it with another layout.
			payload.clear();
			payload.resize(count, None);
		}
		payload[index] = Some(value);
		Ok(())
	}
}

/// The `WorkgroupState` struct provides task and compute invocations with explicitly shared workgroup storage.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct WorkgroupState {
	values: HashMap<String, Vec<Option<Value>>>,
}

impl WorkgroupState {
	/// Creates empty workgroup storage for one VM workgroup fixture.
	pub fn new() -> Self {
		Self::default()
	}

	/// Clears values from the previous workgroup while retaining its names and allocated map storage.
	pub(super) fn begin_workgroup(&mut self) {
		for values in self.values.values_mut() {
			values.fill(None);
		}
	}

	/// Loads one value initialized by an earlier instruction in the bound workgroup state.
	pub(super) fn load(&self, name: &str, index: usize, count: usize, value_type: &ValueType) -> Result<Value, VmError> {
		if index >= count {
			return Err(VmError::WorkgroupIndexOutOfBounds {
				name: name.to_string(),
				index,
				count,
			});
		}
		let value = self
			.values
			.get(name)
			.filter(|values| values.len() == count)
			.and_then(|values| values[index].as_ref())
			.ok_or_else(|| VmError::UninitializedWorkgroupValue { name: name.to_string() })?;
		if !value.matches_type(value_type) {
			return Err(VmError::TypeMismatch {
				expected: value_type.name().to_string(),
				found: value.value_type().name().to_string(),
			});
		}
		Ok(value.clone())
	}

	/// Replaces one workgroup value after validating the declaration's portable type.
	pub(super) fn store(
		&mut self,
		name: &str,
		index: usize,
		count: usize,
		value_type: &ValueType,
		value: Value,
	) -> Result<(), VmError> {
		if index >= count {
			return Err(VmError::WorkgroupIndexOutOfBounds {
				name: name.to_string(),
				index,
				count,
			});
		}
		if !value.matches_type(value_type) {
			return Err(VmError::TypeMismatch {
				expected: value_type.name().to_string(),
				found: value.value_type().name().to_string(),
			});
		}
		let values = self.values.entry(name.to_string()).or_insert_with(|| vec![None; count]);
		if values.len() != count {
			values.clear();
			values.resize(count, None);
		}
		values[index] = Some(value);
		Ok(())
	}

	/// Applies wrapping atomic-u32 addition to one shared scalar or array element.
	pub(super) fn atomic_add_u32(&mut self, name: &str, index: usize, count: usize, value: u32) -> Result<u32, VmError> {
		if index >= count {
			return Err(VmError::WorkgroupIndexOutOfBounds {
				name: name.to_string(),
				index,
				count,
			});
		}
		let stored = self
			.values
			.get_mut(name)
			.filter(|values| values.len() == count)
			.and_then(|values| values[index].as_mut())
			.ok_or_else(|| VmError::UninitializedWorkgroupValue { name: name.to_string() })?;
		let Value::U32(previous) = stored else {
			return Err(VmError::TypeMismatch {
				expected: ValueType::U32.name().to_string(),
				found: stored.value_type().name().to_string(),
			});
		};
		let previous = *previous;
		*stored = Value::U32(previous.wrapping_add(value));
		Ok(previous)
	}

	/// Replaces one shared u32 only when it still matches the expected value.
	pub(super) fn atomic_compare_exchange_u32(
		&mut self,
		name: &str,
		index: usize,
		count: usize,
		expected: u32,
		desired: u32,
	) -> Result<u32, VmError> {
		if index >= count {
			return Err(VmError::WorkgroupIndexOutOfBounds {
				name: name.to_string(),
				index,
				count,
			});
		}
		let stored = self
			.values
			.get_mut(name)
			.filter(|values| values.len() == count)
			.and_then(|values| values[index].as_mut())
			.ok_or_else(|| VmError::UninitializedWorkgroupValue { name: name.to_string() })?;
		let Value::U32(previous) = stored else {
			return Err(VmError::TypeMismatch {
				expected: ValueType::U32.name().to_string(),
				found: stored.value_type().name().to_string(),
			});
		};
		let previous = *previous;
		if previous == expected {
			*stored = Value::U32(desired);
		}
		Ok(previous)
	}
}

/// The `DescriptorBindings` struct provides invocation-scoped host resources and reusable execution storage to a compiled BESL program.
///
/// Bind the resources required by [`ExecutableProgram::run_main`] or
/// [`ExecutableProgram::run_workgroup`]. Reuse one binding set for sequential
/// invocations to retain the VM's register and local storage.
pub struct DescriptorBindings<'a> {
	// VM fixtures bind only a few resources, then query them for every instruction. Sorted dense storage avoids hashes and bounds lookups for larger binding sets.
	bindings: Vec<(ResourceSlot, DescriptorBinding<'a>)>,
	push_constant: Option<&'a mut Buffer>,
	mesh_outputs: Option<&'a mut MeshOutputs>,
	task_outputs: Option<&'a mut TaskOutputs>,
	workgroup_state: Option<&'a mut WorkgroupState>,
	task_payloads: HashMap<String, Vec<Value>>,
	// Frames belong to one mutable binding set, so sequential invocations can reuse their register and local storage safely.
	execution_frames: Vec<ExecutionFrame>,
}

impl<'a> Default for DescriptorBindings<'a> {
	fn default() -> Self {
		Self::new()
	}
}

impl<'a> DescriptorBindings<'a> {
	pub fn new() -> Self {
		Self {
			bindings: Vec::new(),
			push_constant: None,
			mesh_outputs: None,
			task_outputs: None,
			workgroup_state: None,
			task_payloads: HashMap::new(),
			execution_frames: Vec::new(),
		}
	}

	pub fn bind_buffer(&mut self, slot: ResourceSlot, buffer: &'a mut Buffer) {
		self.bind_descriptor(slot, DescriptorBinding::Buffer(buffer));
	}

	pub fn bind_texture(&mut self, slot: ResourceSlot, texture: &'a mut Texture) {
		self.bind_texture_with_sampler(slot, texture, Sampler::default());
	}

	/// Binds one combined texture and sampler for deterministic sampling behavior.
	pub fn bind_texture_with_sampler(&mut self, slot: ResourceSlot, texture: &'a mut Texture, sampler: Sampler) {
		self.bind_descriptor(slot, DescriptorBinding::Texture { texture, sampler });
	}

	pub fn bind_image(&mut self, slot: ResourceSlot, image: &'a mut Texture) {
		self.bind_descriptor(slot, DescriptorBinding::Image(image));
	}

	pub fn bind_push_constant(&mut self, push_constant: &'a mut Buffer) {
		self.push_constant = Some(push_constant);
	}

	/// Binds the capture used by mesh output-count, position, and triangle intrinsics.
	pub fn bind_mesh_outputs(&mut self, mesh_outputs: &'a mut MeshOutputs) {
		self.mesh_outputs = Some(mesh_outputs);
	}

	/// Binds the capture used by task payload writes and the task mesh-output-count intrinsic.
	pub fn bind_task_outputs(&mut self, task_outputs: &'a mut TaskOutputs) {
		self.task_outputs = Some(task_outputs);
	}

	/// Binds shared storage for task fixtures executed through the workgroup scheduler.
	pub fn bind_workgroup_state(&mut self, workgroup_state: &'a mut WorkgroupState) {
		self.workgroup_state = Some(workgroup_state);
	}

	/// Binds the authored values produced for one named task-payload array before a mesh-stage invocation.
	///
	/// Values are copied into invocation-owned storage so callers may use arrays and other temporary iterators.
	pub fn bind_task_payload(&mut self, name: impl Into<String>, values: impl IntoIterator<Item = Value>) {
		self.task_payloads.insert(name.into(), values.into_iter().collect());
	}

	fn bind_descriptor(&mut self, slot: ResourceSlot, descriptor: DescriptorBinding<'a>) {
		match self.bindings.binary_search_by_key(&slot, |(bound_slot, _)| *bound_slot) {
			Ok(index) => self.bindings[index].1 = descriptor,
			Err(index) => self.bindings.insert(index, (slot, descriptor)),
		}
	}

	fn descriptor_mut(&mut self, slot: ResourceSlot) -> Result<&mut DescriptorBinding<'a>, VmError> {
		let index = self
			.bindings
			.binary_search_by_key(&slot, |(bound_slot, _)| *bound_slot)
			.map_err(|_| VmError::UnboundDescriptor { slot })?;
		Ok(&mut self.bindings[index].1)
	}

	pub(super) fn buffer_mut(&mut self, slot: ResourceSlot) -> Result<&mut Buffer, VmError> {
		let descriptor = self.descriptor_mut(slot)?;

		match descriptor {
			DescriptorBinding::Buffer(buffer) => Ok(&mut **buffer),
			descriptor => Err(descriptor.type_mismatch(slot, "buffer")),
		}
	}

	pub(super) fn texture_mut(&mut self, slot: ResourceSlot) -> Result<&mut Texture, VmError> {
		let descriptor = self.descriptor_mut(slot)?;

		match descriptor {
			DescriptorBinding::Texture { texture, .. } => Ok(&mut **texture),
			descriptor => Err(descriptor.type_mismatch(slot, "texture")),
		}
	}

	pub(super) fn texture_and_sampler_mut(&mut self, slot: ResourceSlot) -> Result<(&mut Texture, Sampler), VmError> {
		let descriptor = self.descriptor_mut(slot)?;

		match descriptor {
			DescriptorBinding::Texture { texture, sampler } => Ok((&mut **texture, *sampler)),
			descriptor => Err(descriptor.type_mismatch(slot, "texture")),
		}
	}

	pub(super) fn image_mut(&mut self, slot: ResourceSlot) -> Result<&mut Texture, VmError> {
		let descriptor = self.descriptor_mut(slot)?;

		match descriptor {
			DescriptorBinding::Image(image) => Ok(&mut **image),
			descriptor => Err(descriptor.type_mismatch(slot, "image")),
		}
	}

	pub(super) fn push_constant_mut(&mut self) -> Result<&mut Buffer, VmError> {
		self.push_constant.as_deref_mut().ok_or(VmError::MissingPushConstant)
	}

	pub(super) fn mesh_outputs_mut(&mut self) -> Result<&mut MeshOutputs, VmError> {
		self.mesh_outputs.as_deref_mut().ok_or(VmError::MissingMeshOutputs)
	}

	pub(super) fn task_outputs_mut(&mut self) -> Result<&mut TaskOutputs, VmError> {
		self.task_outputs.as_deref_mut().ok_or(VmError::MissingTaskOutputs)
	}

	pub(super) fn workgroup_state_mut(&mut self) -> Result<&mut WorkgroupState, VmError> {
		self.workgroup_state.as_deref_mut().ok_or(VmError::MissingWorkgroupState)
	}

	/// Starts a fresh task or compute workgroup without reallocating reusable capture storage.
	pub(super) fn begin_workgroup(&mut self) {
		if let Some(task_outputs) = self.task_outputs.as_deref_mut() {
			task_outputs.begin_workgroup();
		}
		if let Some(workgroup_state) = self.workgroup_state.as_deref_mut() {
			workgroup_state.begin_workgroup();
		}
	}

	/// Borrows one cached frame or creates an empty one for a newly reached call depth.
	pub(super) fn take_execution_frame(&mut self) -> ExecutionFrame {
		self.execution_frames.pop().unwrap_or_default()
	}

	/// Retains a completed frame so the next sequential invocation can reuse its allocations.
	pub(super) fn release_execution_frame(&mut self, frame: ExecutionFrame) {
		self.execution_frames.push(frame);
	}

	pub(super) fn task_payload_value(&self, name: &str, index: usize) -> Result<Value, VmError> {
		let values = self
			.task_payloads
			.get(name)
			.ok_or_else(|| VmError::MissingTaskPayload { name: name.to_string() })?;
		values
			.get(index)
			.cloned()
			.ok_or_else(|| VmError::TaskPayloadIndexOutOfBounds {
				name: name.to_string(),
				index,
				count: values.len(),
			})
	}
}
