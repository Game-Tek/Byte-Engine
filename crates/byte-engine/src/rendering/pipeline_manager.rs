use smallvec::SmallVec;
use utils::{Box, Extent, hash::HashMap, sync::RwLock};

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
	/// Marks the end of one simulation step.
	///
	/// Transform updates waiting on a listener at this point were published by the step that just ran. A manager
	/// that shows motion between steps reads them here and keeps them as the sample its next frames move toward;
	/// whatever it reads later, in [`Self::prepare`], was written outside a step and is shown where it is. A
	/// manager that shows every transform as it arrives has nothing to do here.
	fn step(&mut self) {}

	/// Adopts scene messages and prepares sink-local render commands for one frame.
	///
	/// `alpha` is how far this frame lies between the two most recent simulation steps: `0` at the older one,
	/// `1` at the one [`Self::step`] last marked. It is `1` when simulation runs once per frame.
	///
	/// Drain loader completions before publishing newly resident resources to
	/// waiting scene instances and building draws.
	fn prepare<'a>(
		&'a mut self,
		frame: &mut ghi::implementation::Frame,
		sinks: &[Sink],
		frame_allocator: &'a bumpalo::Bump,
		alpha: f32,
	) -> Option<SmallVec<[RenderPassReturn<'a>; 16]>>;

	/// Creates the persistent pass state needed by one new render sink.
	fn create_sink(&mut self, sink_id: usize, render_pass_builder: &mut RenderPassBuilder);
}
