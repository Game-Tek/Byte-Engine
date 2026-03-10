use crate::{
	BaseBufferHandle, BottomLevelAccelerationStructureHandle, BufferDescriptor, BufferStridedRange, DataTypes, Encodings,
	TopLevelAccelerationStructureHandle,
};

pub enum BottomLevelAccelerationStructureBuildDescriptions {
	Mesh {
		vertex_buffer: BufferStridedRange,
		vertex_count: u32,
		vertex_position_encoding: Encodings,
		index_buffer: BufferStridedRange,
		triangle_count: u32,
		index_format: DataTypes,
	},
	AABB {
		aabb_buffer: BaseBufferHandle,
		transform_buffer: BaseBufferHandle,
		transform_count: u32,
	},
}

pub enum TopLevelAccelerationStructureBuildDescriptions {
	Instance {
		instances_buffer: BaseBufferHandle,
		instance_count: u32,
	},
}

pub struct BottomLevelAccelerationStructureBuild {
	pub acceleration_structure: BottomLevelAccelerationStructureHandle,
	pub scratch_buffer: BufferDescriptor,
	pub description: BottomLevelAccelerationStructureBuildDescriptions,
}

pub struct TopLevelAccelerationStructureBuild {
	pub acceleration_structure: TopLevelAccelerationStructureHandle,
	pub scratch_buffer: BufferDescriptor,
	pub description: TopLevelAccelerationStructureBuildDescriptions,
}

pub struct BindingTables {
	pub raygen: BufferStridedRange,
	pub hit: BufferStridedRange,
	pub miss: BufferStridedRange,
	pub callable: Option<BufferStridedRange>,
}
