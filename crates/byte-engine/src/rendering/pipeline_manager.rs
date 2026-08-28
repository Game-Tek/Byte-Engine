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
/// implement [`crate::rendering::RenderPass`] instead. A manager that uses
/// [`crate::rendering::resource_loading`] adopts completed work in
/// [`Self::begin_frame`] and records the resulting transfers in
/// [`Self::record_frame_uploads`].
pub trait PipelineManager {
	/// Releases completed GPU work, adopts prepared resources, and reports whether this frame needs upload commands.
	///
	/// Retire the matching
	/// [`crate::rendering::resource_loading::FrameUploadQueue`] batch before
	/// draining new preparation completions. Return `true` while the queue has
	/// pending work so the renderer opens its resource-upload command buffer.
	fn begin_frame(&mut self, completed_frame: Option<ghi::FrameKey>) -> bool;

	/// Records renderer-specific GPU uploads before this frame's render commands.
	///
	/// Pass `frame` unchanged to
	/// [`crate::rendering::resource_loading::FrameUploadQueue::record_frame`].
	/// This pairing is why the queue can retain source resources until the exact
	/// completion reported to [`Self::begin_frame`].
	fn record_frame_uploads(&mut self, frame: ghi::FrameKey, recording: &mut ghi::implementation::CommandBufferRecording<'_>);

	/// Adopts scene messages and prepares sink-local render commands for one frame.
	///
	/// Resource loading should already have advanced through
	/// [`Self::begin_frame`]. Publish newly resident resources to waiting scene
	/// instances here or during the frame-boundary call, before building draws.
	fn prepare<'a>(
		&'a mut self,
		frame: &mut ghi::implementation::Frame,
		sinks: &[Sink],
		frame_allocator: &'a bumpalo::Bump,
	) -> Option<SmallVec<[RenderPassReturn<'a>; 16]>>;

	/// Creates the persistent pass state needed by one new render sink.
	fn create_sink(&mut self, sink_id: usize, render_pass_builder: &mut RenderPassBuilder);
}
