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
	fn prepare_transfers(&mut self, _transfer: &mut ghi::implementation::CommandBufferRecording) -> bool {
		false
	}

	/// Called when a frame is being prepared for rendering.
	fn prepare(&mut self, frame: &mut ghi::implementation::Frame, sinks: &[Sink]) -> Option<Vec<Box<dyn RenderPassFunction>>>;

	fn create_sink(&mut self, sink_id: usize, render_pass_builder: &mut RenderPassBuilder);
}
