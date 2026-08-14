use super::super::*;

impl Context {
	pub fn create_acceleration_structure_instance_buffer(
		&mut self,
		name: Option<&str>,
		max_instance_count: u32,
	) -> graphics_hardware_interface::BaseBufferHandle {
		let size = max_instance_count as usize * std::mem::size_of::<mtl::MTLAccelerationStructureInstanceDescriptor>();
		let buffer = self.create_buffer_resource(
			name,
			size,
			crate::Uses::AccelerationStructure,
			crate::DeviceAccesses::DeviceOnly,
		);
		let mut creator = self.buffers.creator();

		creator.add(buffer);

		creator.into()
	}

	pub fn create_top_level_acceleration_structure(
		&mut self,
		_name: Option<&str>,
		_max_instance_count: u32,
	) -> graphics_hardware_interface::TopLevelAccelerationStructureHandle {
		self.acceleration_structures.push(AccelerationStructure {
			structure: None,
			buffer: None,
		});
		// TODO: Build MTLAccelerationStructure and backing buffer.
		graphics_hardware_interface::TopLevelAccelerationStructureHandle((self.acceleration_structures.len() - 1) as u64)
	}

	pub fn create_bottom_level_acceleration_structure(
		&mut self,
		_description: &graphics_hardware_interface::BottomLevelAccelerationStructure,
	) -> graphics_hardware_interface::BottomLevelAccelerationStructureHandle {
		self.acceleration_structures.push(AccelerationStructure {
			structure: None,
			buffer: None,
		});
		// TODO: Build MTLAccelerationStructure for mesh or AABB.
		graphics_hardware_interface::BottomLevelAccelerationStructureHandle((self.acceleration_structures.len() - 1) as u64)
	}

	pub fn write_instance(
		&mut self,
		_instances_buffer_handle: graphics_hardware_interface::BaseBufferHandle,
		_instance_index: usize,
		_transform: [[f32; 4]; 3],
		_custom_index: u16,
		_mask: u8,
		_sbt_record_offset: usize,
		_acceleration_structure: graphics_hardware_interface::BottomLevelAccelerationStructureHandle,
	) {
		// TODO: Populate MTLAccelerationStructureInstanceDescriptor buffer.
	}

	pub fn write_sbt_entry(
		&mut self,
		_sbt_buffer_handle: graphics_hardware_interface::BaseBufferHandle,
		_sbt_record_offset: usize,
		_pipeline_handle: graphics_hardware_interface::PipelineHandle,
		_shader_handle: graphics_hardware_interface::ShaderHandle,
	) {
		// TODO: Metal ray tracing shader binding table mapping.
	}
}
