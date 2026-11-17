//! Scenes the renderer keeps apart, so a sink can show objects no other sink sees.
//!
//! See [scenes](/docs/reference/rendering#scenes).

/// The `SceneId` struct names one scene to the renderer, so scene pipelines, lights, fog, and render images know
/// which objects they belong to.
///
/// [`Self::WORLD`] is the scene every window shows. Create another with
/// [`crate::application::graphics::GraphicsApplication::create_scene`], then install its scene pipelines and create a
/// [`crate::rendering::RenderImage`] that shows it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct SceneId(u32);

impl SceneId {
	/// The world, the scene the application starts with.
	pub const WORLD: SceneId = SceneId(0);

	/// The scene at `index` in the renderer's scenes.
	pub(crate) fn new(index: usize) -> Self {
		Self(u32::try_from(index).expect("Scene count overflowed. The most likely cause is creating scenes in a loop."))
	}

	/// The index of the scene in the renderer's scenes.
	pub(crate) fn index(self) -> usize {
		self.0 as usize
	}
}
