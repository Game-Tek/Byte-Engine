//! Backend-independent GHI queue types.

use crate::{DataTypes, WorkloadTypes};

/// The `ImageSubresourceLayout` struct defines how one image subresource maps to memory.
pub struct ImageSubresourceLayout {
	/// The byte offset of the first texel in its memory region.
	pub offset: usize,
	/// The size of the texture in bytes.
	pub size: usize,
	/// The row pitch of the texture.
	pub row_pitch: usize,
	/// The array pitch of the texture.
	pub array_pitch: usize,
	/// The depth pitch of the texture.
	pub depth_pitch: usize,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
/// Enumerates the states of a swapchain's validity for presentation.
pub enum SwapchainStates {
	/// The swapchain is valid for presentation.
	Ok,
	/// The swapchain is suboptimal for presentation.
	Suboptimal,
	/// The swapchain can't be used for presentation.
	Invalid,
}

pub enum AccelerationStructureTypes {
	TopLevel {
		instance_count: u32,
	},
	BottomLevel {
		vertex_count: u32,
		triangle_count: u32,
		vertex_position_format: DataTypes,
		index_format: DataTypes,
	},
}

pub struct QueueSelection {
	pub(crate) r#type: WorkloadTypes,
}

impl QueueSelection {
	pub fn new(r#type: WorkloadTypes) -> Self {
		Self { r#type }
	}
}
