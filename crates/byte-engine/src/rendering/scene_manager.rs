use utils::{
	hash::{HashMap, HashMapExt},
	sync::RwLock,
	Box, Extent,
};

use crate::rendering::{
	render_pass::{RenderPassBuilder, RenderPassFunction},
	Sink,
};

/// The `SceneManager` trait bridges scene state with render work for active sinks.
pub trait SceneManager {
	/// Prepares the transfer buffer for the given frame.
	///
	/// Returns the remaining transfer buffer capacity and whether transfer work was recorded.
	///
	/// # Arguments
	///
	/// * `transfer` - The command buffer recording to prepare the transfer buffer for.
	/// * `key` - The frame key identifying the frame to prepare the transfer buffer for.
	/// * `slice` - The buffer slice to prepare the transfer buffer in.
	fn prepare_transfers<'a>(
		&mut self,
		transfer: &mut ghi::implementation::CommandBufferRecording,
		key: ghi::FrameKey,
		slice: utils::BufferAllocator<'a>,
	) -> (utils::BufferAllocator<'a>, bool) {
		(slice, false)
	}

	/// Called when a frame is being prepared for rendering.
	fn prepare(&mut self, frame: &mut ghi::implementation::Frame, sinks: &[Sink]) -> Option<Vec<Box<dyn RenderPassFunction>>>;

	fn create_sink(&mut self, sink_id: usize, render_pass_builder: &mut RenderPassBuilder);
}
