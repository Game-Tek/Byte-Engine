//! On-screen debug text drawn with the retained UI.

use std::{cell::RefCell, fmt, rc::Rc};

use utils::{Extent, RGBA};

use crate::{
	core::{
		factory::Handle,
		listener::{DefaultListener, Listener as _},
	},
	gameplay::transform::{Transform, TransformationUpdate},
	rendering::{
		Sink,
		render_pass::{RenderPass, RenderPassBuilder, RenderPassReturn},
	},
	ui::{ConcreteLayer, Context as _, ElementContext as _, Engine, Size, UiRenderPass, render_pass::AdoptedRender},
};

/// The size of the overlay's text, in pixels.
const FONT_SIZE: f32 = 14.0;

/// The `DebugOverlaySource` struct keeps the newest transform of the entity the debug overlay watches.
///
/// [`crate::application::graphics::setup_debug_overlay_render_pass`] creates one and shares it with every sink's
/// [`DebugOverlayRenderPass`], so each window shows the same value while the route is drained once.
pub(crate) struct DebugOverlaySource {
	handle: Handle,
	listener: DefaultListener<TransformationUpdate>,
	transform: Option<Transform>,
	/// Counts the watched transforms received, so each sink can tell whether it has drawn the newest.
	revision: u64,
}

impl DebugOverlaySource {
	/// Watches the transforms `listener` carries for `handle`.
	pub(crate) fn new(handle: Handle, listener: DefaultListener<TransformationUpdate>) -> Self {
		Self {
			handle,
			listener,
			transform: None,
			revision: 0,
		}
	}

	/// Reads every pending transform and returns the newest one for the watched handle, with its revision.
	fn current(&mut self) -> (u64, Option<&Transform>) {
		while let Some(message) = self.listener.read() {
			if message.handle() == self.handle {
				self.transform = Some(message.into_data());
				self.revision += 1;
			}
		}
		(self.revision, self.transform.as_ref())
	}
}

/// The `DebugOverlayRenderPass` struct puts a watched entity's transform on top of one sink's image.
///
/// Register it through [`crate::application::graphics::setup_debug_overlay_render_pass`]. It owns a small UI
/// [`Engine`] and draws it with a [`UiRenderPass`], so the text composites over everything registered before it.
pub(crate) struct DebugOverlayRenderPass {
	source: Rc<RefCell<DebugOverlaySource>>,
	/// The source revision this sink last laid out.
	seen: u64,
	engine: Engine<Option<Transform>>,
	adopted: AdoptedRender,
	/// The sink size the UI was last laid out at.
	extent: Extent,
	render_pass: UiRenderPass,
}

impl DebugOverlayRenderPass {
	/// Creates one sink's overlay, reading the shared `source`.
	pub(crate) fn new(render_pass_builder: &mut RenderPassBuilder<'_>, source: Rc<RefCell<DebugOverlaySource>>) -> Self {
		let mut engine = Engine::with_context(None);
		engine.mount(async |ctx| {
			let white = ConcreteLayer::default().color(RGBA::white().into());
			let mut label = ctx
				.element("transform")
				.text(TransformText(None), |t| t.font_size(FONT_SIZE).style([white]))
				.await;
			loop {
				let transform = ctx.with(|transform: &mut Option<Transform>| transform.clone()).await;
				// Unchanged text leaves the UI's revision alone, so steady frames redraw nothing.
				label.update_text(|t| t.content(TransformText(transform.as_ref()))).await;
				ctx.render().await;
			}
		});

		Self {
			source,
			seen: 0,
			engine,
			adopted: AdoptedRender::default(),
			extent: Extent::rectangle(0, 0),
			render_pass: UiRenderPass::new(render_pass_builder, None),
		}
	}

	/// Lays the UI out again when the watched transform or the sink size changed, and hands the result to the UI pass.
	fn update(&mut self, sink: &Sink, frame_allocator: &bumpalo::Bump) {
		let mut source = self.source.borrow_mut();
		let (revision, transform) = source.current();
		if revision == self.seen && self.extent == sink.extent() {
			return;
		}
		*self.engine.ctx_mut() = transform.cloned();
		self.seen = revision;
		self.extent = sink.extent();
		self.engine
			.evaluate(Size::new(self.extent.width(), self.extent.height()), frame_allocator);
		if self.adopted.adopt(self.engine.render()) {
			self.render_pass.update(&self.adopted);
		}
	}
}

impl RenderPass for DebugOverlayRenderPass {
	fn name(&self) -> &'static str {
		"debug-overlay"
	}

	fn prepare<'a>(
		&mut self,
		frame: &mut ghi::implementation::Frame,
		sink: &Sink,
		frame_allocator: &'a bumpalo::Bump,
	) -> Option<RenderPassReturn<'a>> {
		self.update(sink, frame_allocator);
		self.render_pass.prepare(frame, sink, frame_allocator)
	}

	fn bypass<'a>(
		&mut self,
		frame: &mut ghi::implementation::Frame,
		sink: &Sink,
		frame_allocator: &'a bumpalo::Bump,
	) -> Option<RenderPassReturn<'a>> {
		self.render_pass.bypass(frame, sink, frame_allocator)
	}

	fn needs_frame(&mut self) -> bool {
		self.source.borrow_mut().current().0 != self.seen || self.render_pass.needs_frame()
	}
}

/// The `TransformText` struct formats a transform as the overlay's lines of text without allocating.
struct TransformText<'a>(Option<&'a Transform>);

impl fmt::Display for TransformText<'_> {
	fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
		let Some(transform) = self.0 else {
			return f.write_str("no transform");
		};
		let position = transform.get_position();
		let orientation = transform.get_orientation().into_maths();
		let scale = transform.scale();
		write!(
			f,
			"position {:.2} {:.2} {:.2}\norientation {:.2} {:.2} {:.2} {:.2}\nscale {:.2} {:.2} {:.2}",
			position.x(),
			position.y(),
			position.z(),
			orientation.x,
			orientation.y,
			orientation.z,
			orientation.w,
			scale.x(),
			scale.y(),
			scale.z(),
		)
	}
}

#[cfg(test)]
mod tests {
	use math::{Orientation, Point, Scale};

	use super::*;
	use crate::core::{Creator as _, channel::Channel as _, message_bus::MessageScope};

	#[test]
	fn transform_text_lists_position_orientation_and_scale() {
		let transform = Transform::new(
			Point::new(1.0, -2.5, 3.137),
			Scale::new(2.0, 2.0, 2.0),
			Orientation::identity(),
		);

		assert_eq!(
			TransformText(Some(&transform)).to_string(),
			"position 1.00 -2.50 3.14\norientation 0.00 0.00 0.00 1.00\nscale 2.00 2.00 2.00"
		);
		assert_eq!(TransformText(None).to_string(), "no transform");
	}

	#[test]
	fn source_keeps_the_newest_transform_of_the_watched_handle() {
		let world = MessageScope::standalone("world");
		let watched = world.create(Transform::identity()).handle();
		let other = world.create(Transform::identity()).handle();
		let channel = world.channel::<TransformationUpdate>();
		let mut source = DebugOverlaySource::new(watched, channel.listener());

		channel.send(TransformationUpdate::new(
			watched,
			Transform::from_position(Point::new(1.0, 0.0, 0.0)),
		));
		channel.send(TransformationUpdate::new(
			watched,
			Transform::from_position(Point::new(2.0, 0.0, 0.0)),
		));
		channel.send(TransformationUpdate::new(
			other,
			Transform::from_position(Point::new(9.0, 0.0, 0.0)),
		));

		let newest = Transform::from_position(Point::new(2.0, 0.0, 0.0));
		let (revision, transform) = source.current();
		assert_eq!(transform, Some(&newest));

		// Another entity moving leaves the watched transform and its revision alone.
		channel.send(TransformationUpdate::new(other, Transform::identity()));
		assert_eq!(source.current(), (revision, Some(&newest)));
	}
}
