use super::super::*;

impl Device {
	/// Returns how many copies the resource a descriptor names has, which bounds the frame offsets a write may use.
	pub(crate) fn descriptor_resource_copies(&self, descriptor: WriteData) -> usize {
		match descriptor {
			WriteData::Buffer { handle } => self.buffer(handle).copies.count(self.frames) as usize,
			WriteData::Image { handle, .. } => self.image_copy_count(handle),
			WriteData::CombinedImageSampler { image_handle, .. } => self.image_copy_count(image_handle),
			_ => 1,
		}
	}
}
