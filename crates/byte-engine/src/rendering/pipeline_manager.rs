use smallvec::SmallVec;
use utils::{
	hash::{HashMap, HashMapExt},
	sync::RwLock,
	Box, Extent,
};

use crate::rendering::{
	render_pass::{RenderPassBuilder, RenderPassReturn},
	Sink,
};

/// The [`PipelineManager`] trait bridges scene state with render work for active
/// sinks.
///
/// Implement this for a scene rendering strategy that needs persistent
/// per-sink resources. Post-processing that only consumes rendered images should
/// implement [`crate::rendering::RenderPass`] instead.
pub trait PipelineManager {
	/// Called when a frame is being prepared for rendering.
	fn prepare<'a>(
		&'a mut self,
		frame: &mut ghi::implementation::Frame,
		sinks: &[Sink],
		frame_allocator: &'a bumpalo::Bump,
	) -> Option<SmallVec<[RenderPassReturn<'a>; 16]>>;

	fn create_sink(&mut self, sink_id: usize, render_pass_builder: &mut RenderPassBuilder);
}
