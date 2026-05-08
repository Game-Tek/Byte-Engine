use utils::{
	hash::{HashMap, HashMapExt},
	sync::RwLock,
	Box, Extent,
};

use crate::rendering::{
	render_pass::{RenderPassBuilder, RenderPassFunction},
	Sink,
};

/// The `TransferPrepareResult` struct carries transfer preparation state across scene managers.
pub struct TransferPrepareResult<'a> {
	pub slice: utils::BufferAllocator<'a>,
	pub recorded_work: bool,
}

/// The `PipelineManager` trait bridges scene state with render work for active sinks.
pub trait PipelineManager {
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
		completed_frame: Option<ghi::FrameKey>,
		staging_data_buffer: ghi::BaseBufferHandle,
		slice: utils::BufferAllocator<'a>,
	) -> TransferPrepareResult<'a> {
		let _ = (transfer, key, completed_frame, staging_data_buffer);
		TransferPrepareResult {
			slice,
			recorded_work: false,
		}
	}

	/// Called when a frame is being prepared for rendering.
	fn prepare(&mut self, frame: &mut ghi::implementation::Frame, sinks: &[Sink]) -> Option<Vec<Box<dyn RenderPassFunction>>>;

	fn create_sink(&mut self, sink_id: usize, render_pass_builder: &mut RenderPassBuilder);
}
