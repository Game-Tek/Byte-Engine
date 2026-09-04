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
/// implement [`crate::rendering::RenderPass`] instead. Managers using
/// [`crate::rendering::loading`] request resources when scene messages arrive
/// and adopt fully resident loader events before building draws.
pub trait PipelineManager {
	/// Adopts scene messages and prepares sink-local render commands for one frame.
	///
	/// Drain loader completions before publishing newly resident resources to
	/// waiting scene instances and building draws.
	fn prepare<'a>(
		&'a mut self,
		frame: &mut ghi::implementation::Frame,
		sinks: &[Sink],
		frame_allocator: &'a bumpalo::Bump,
	) -> Option<SmallVec<[RenderPassReturn<'a>; 16]>>;

	/// Creates the persistent pass state needed by one new render sink.
	fn create_sink(&mut self, sink_id: usize, render_pass_builder: &mut RenderPassBuilder);
}
