use ash::vk;

use crate::{CommandBufferRecording, Frame};

pub struct Queue {
	pub(crate) vk_queue: vk::Queue,
	pub(crate) queue_family_index: u32,
	pub(crate) _queue_index: u32,
}

impl crate::queue::Queue for Queue {
	type CBR<'a> = CommandBufferRecording<'a>;
	type Frame<'a> = Frame<'a>;

	fn execute<'a>(
		frame: Self::Frame<'a>,
		cmd: Self::CBR<'a>,
		present_keys: &[crate::PresentKey],
		synchronizer: crate::SynchronizerHandle,
	) {
	}
}
