use smallvec::SmallVec;
use utils::{
	Box, Extent,
	hash::{HashMap, HashMapExt},
	sync::RwLock,
};

use crate::rendering::{
	Sink,
	render_pass::{RenderPassBuilder, RenderPassReturn},
};

/// The [`PipelineManager`] trait bridges scene state with render work for active
/// sinks.
///
/// Implement this for a scene rendering strategy that needs persistent
/// per-sink resources. Post-processing that only consumes rendered images should
/// implement [`crate::rendering::RenderPass`] instead.
pub trait PipelineManager {
	/// Releases completed GPU work, adopts prepared uploads, and returns whether this frame must record them.
	fn begin_frame(&mut self, completed_frame: Option<ghi::FrameKey>) -> bool;

	/// Records GPU uploads before this frame's render commands.
	fn record_frame_uploads(&mut self, frame: ghi::FrameKey, recording: &mut ghi::implementation::CommandBufferRecording<'_>);

	/// Called when a frame is being prepared for rendering.
	fn prepare<'a>(
		&'a mut self,
		frame: &mut ghi::implementation::Frame,
		sinks: &[Sink],
		frame_allocator: &'a bumpalo::Bump,
	) -> Option<SmallVec<[RenderPassReturn<'a>; 16]>>;

	fn create_sink(&mut self, sink_id: usize, render_pass_builder: &mut RenderPassBuilder);
}
