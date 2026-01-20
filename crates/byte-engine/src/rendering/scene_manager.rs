use crate::rendering::{Viewport, render_pass::{RenderPassBuilder, RenderPassFunction}};
use utils::{hash::{HashMap, HashMapExt}, sync::RwLock, Box, Extent};

/// A `SceneManager` is responsible for managing scenes in the rendering engine.
pub trait SceneManager {
	/// Called when a frame is being prepared for rendering.
	fn prepare(&mut self, frame: &mut ghi::Frame, viewports: &[Viewport]) -> Option<Vec<Box<dyn RenderPassFunction>>>;

	fn create_view(&mut self, id: usize, render_pass_builder: &mut RenderPassBuilder);
}
