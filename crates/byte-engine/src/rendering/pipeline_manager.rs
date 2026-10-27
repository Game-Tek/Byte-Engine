use smallvec::SmallVec;

use crate::{
	rendering::{
		ShownTransforms, Sink,
		render_pass::{RenderPassBuilder, RenderPassReturn},
	},
	time::MediaTime,
};

/// The `PipelineManager` trait connects scene state and resource loading to render work for active sinks.
///
/// Implement this for a scene rendering strategy that needs persistent
/// per-sink resources. Post-processing that only consumes rendered images should
/// implement [`crate::rendering::RenderPass`] instead. Managers using
/// [`crate::rendering::loading`] request resources when scene messages arrive
/// and adopt fully resident loader events before building draws.
pub trait PipelineManager {
	/// Returns the stable name the renderer reports this manager's GPU time under, as `scene.<name>`.
	///
	/// The default is the implementing type's name without its module path.
	fn name(&self) -> &'static str {
		std::any::type_name::<Self>().rsplit("::").next().unwrap_or_default()
	}

	/// Adopts scene messages and requests resources before new windows are created.
	///
	/// The application calls this after publishing the tick's changes and without borrowing the graphics
	/// context. Start loading here so it can overlap native window and sink setup. Adopt completed resources
	/// later in [`Self::prepare`], where the current frame can use everything that finished in the meantime.
	///
	/// `transforms` places anything the manager starts drawing here: [`ShownTransforms::get`] already shows every
	/// transform published so far.
	fn update(&mut self, _transforms: &ShownTransforms) {}

	/// Adopts loader completions and prepares sink-local render commands for one frame.
	///
	/// `transforms` holds what this frame shows. Move what the manager already draws by
	/// [`ShownTransforms::changed`], and place what it starts drawing with [`ShownTransforms::get`], so it moves with
	/// the cameras and the other managers between simulation steps.
	///
	/// `time` is the frame clock's elapsed time. Drive playback that must follow the display instead of the
	/// simulation rate from it, such as flipbook frames.
	///
	/// Drain loader completions before publishing newly resident resources to
	/// waiting scene instances and building draws.
	///
	/// Return one command per sink that renders this frame, paired with that sink's [`Sink::index`]. A sink without
	/// an entry records nothing, so the renderer clears the targets this manager would have written first.
	fn prepare<'a>(
		&'a mut self,
		frame: &mut ghi::implementation::Frame,
		sinks: &[Sink],
		frame_allocator: &'a bumpalo::Bump,
		transforms: &ShownTransforms,
		time: MediaTime,
	) -> SmallVec<[(usize, RenderPassReturn<'a>); 16]>;

	/// Creates the persistent pass state needed by one new render sink.
	fn create_sink(&mut self, sink_id: usize, render_pass_builder: &mut RenderPassBuilder);
}
