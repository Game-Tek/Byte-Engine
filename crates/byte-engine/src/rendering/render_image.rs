//! Images the renderer draws a camera into instead of a window.
//!
//! Create a [`RenderImage`] through a factory on the
//! [`crate::application::graphics::GraphicsApplication::messages`] scope, wait for its [`RenderImageReady`], and show
//! the image it names, for example in an editor panel. See [render images](/docs/reference/rendering#render-images).

use utils::Extent;

use crate::core::factory::Handle;
use crate::rendering::SceneId;

/// The `RenderImage` struct asks the renderer for an image it draws a camera's view of a scene into, so an application
/// can show a view inside its own interface, such as a preview of an isolated scene in an editor panel, instead of in
/// a window of its own.
///
/// Create it on the [`crate::application::graphics::GraphicsApplication::messages`] scope. The renderer opens it before
/// the next frame and answers with a [`RenderImageReady`] naming the image. Resize it with [`SetRenderImageExtent`] and
/// stop it with a [`crate::core::message::DeleteMessage`] on the same scope.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RenderImage {
	scene: SceneId,
	extent: Extent,
	camera: Handle,
}

impl RenderImage {
	/// Asks for an image of `extent` pixels that shows `scene` from `camera`.
	pub fn new(scene: SceneId, extent: Extent, camera: Handle) -> Self {
		Self { scene, extent, camera }
	}

	/// The scene the image shows.
	pub fn scene(&self) -> SceneId {
		self.scene
	}

	/// The size of the image, in pixels.
	pub fn extent(&self) -> Extent {
		self.extent
	}

	/// The camera the image shows.
	pub fn camera(&self) -> Handle {
		self.camera
	}
}

/// The `RenderImageReady` struct tells the application which image the renderer draws a [`RenderImage`] into, so its
/// interface can show it.
///
/// The renderer sends it on the [`crate::application::graphics::GraphicsApplication::messages`] channel once, when it
/// opens the render image. The image keeps its handle when [`SetRenderImageExtent`] resizes it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RenderImageReady {
	render_image: Handle,
	image: ghi::ImageHandle,
}

impl RenderImageReady {
	pub(crate) fn new(render_image: Handle, image: ghi::ImageHandle) -> Self {
		Self { render_image, image }
	}

	/// The handle the render image was created with.
	pub fn render_image(&self) -> Handle {
		self.render_image
	}

	/// The image the renderer draws into, in the swapchain's format.
	pub fn image(&self) -> ghi::ImageHandle {
		self.image
	}
}

/// The `SetRenderImageExtent` struct resizes an open render image, such as when the panel that shows it changes size.
///
/// Send it on the [`crate::application::graphics::GraphicsApplication::messages`] channel with the handle the render
/// image was created with. The image takes the new extent from the next frame on and keeps its handle.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SetRenderImageExtent {
	render_image: Handle,
	extent: Extent,
}

impl SetRenderImageExtent {
	/// Makes the render image created with `render_image` `extent` pixels large.
	pub fn new(render_image: Handle, extent: Extent) -> Self {
		Self { render_image, extent }
	}

	/// The handle the render image was created with.
	pub fn render_image(&self) -> Handle {
		self.render_image
	}

	/// The extent the image takes, in pixels.
	pub fn extent(&self) -> Extent {
		self.extent
	}
}
