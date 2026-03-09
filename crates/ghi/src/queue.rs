pub trait Queue {
	type Frame<'a>: crate::frame::Frame<'a>;
	type CBR<'a>: crate::command_buffer::CommandBufferRecording;

	fn execute<'a>(
		frame: Self::Frame<'a>,
		cmd: Self::CBR<'a>,
		present_keys: &[crate::PresentKey],
		synchronizer: crate::SynchronizerHandle,
	);
}
