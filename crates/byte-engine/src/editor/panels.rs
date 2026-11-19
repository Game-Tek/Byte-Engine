//! Panels an application adds to the entity panel for its own component types, such as an NPC's debug commands or a
//! readout of its state.
//!
//! An application registers one with [`Editor::panel`](crate::editor::Editor::panel). While an entity with the
//! panel's component is selected, the shell shows a block of its own at the bottom of the entity panel and runs the
//! panel's function in it with a [`Panel`]; the selection changing removes the block and ends the function. The panel
//! reaches the game through the inspector, as the rest of the editor does: it reads the properties the application
//! watches and posts the messages it registers.

use std::future::Future;
use std::pin::Pin;
use std::rc::Rc;

use serde::Serialize;
use serde::de::DeserializeOwned;

use super::shell::{CONTENT_WIDTH, Shell, TEXT, TOOL_HEIGHT, raised, solid};
use crate::core::factory::Handle;
use crate::ui::flow;
use crate::ui::layout::context::{ContainerContext, Context, ElementContext};
use crate::ui::layout::engine::EvaluationContext;
use crate::ui::primitive::Events;
use crate::ui::{Polled, Reactive};
use crate::utils::r#async::{FusedFuture, FutureExt as _, select};

/// Starts a panel's function in the block's slot, for the selected entity.
pub(crate) type Mount = Rc<dyn Fn(&mut EvaluationContext<Shell>, Handle) -> Pin<Box<dyn Future<Output = ()> + '_>>>;

/// The `Registration` struct is one panel an application registered: the component type that shows it, its block's
/// title, and how to start it.
pub(crate) struct Registration {
	/// The component's Rust type name, as the inspector lists it among an entity's types.
	pub(crate) component: &'static str,
	pub(crate) title: &'static str,
	pub(crate) mount: Mount,
}

/// The `Panel` struct is what a panel's function builds its controls with: the selected entity, its properties, the
/// messages it can post, and the panel's own column of elements.
///
/// Add controls with [`Self::button`] and [`Self::label`], or declare any elements on [`Self::ui`]. Then loop: wait for
/// a click or for a change of a [`Self::property`], and send commands with [`Self::post`].
pub struct Panel<'a> {
	ctx: &'a mut EvaluationContext<Shell>,
	entity: Handle,
	/// How many controls the panel added, which keys the next one.
	controls: usize,
}

impl<'a> Panel<'a> {
	pub(crate) fn new(ctx: &'a mut EvaluationContext<Shell>, entity: Handle) -> Self {
		Self {
			ctx,
			entity,
			controls: 0,
		}
	}

	/// The selected entity the panel shows.
	pub fn entity(&self) -> Handle {
		self.entity
	}

	/// The width of the panel's column, in pixels, for elements declared on [`Self::ui`].
	pub fn width(&self) -> f32 {
		CONTENT_WIDTH
	}

	/// The panel's column, where its controls stack from the top, for declaring elements of your own, such as a bar
	/// that shows health.
	pub fn ui(&mut self) -> &mut EvaluationContext<Shell> {
		self.ctx
	}

	/// Follows the entity's property `name`, as the application watches it with
	/// [`DefaultInspector::watch_property`](crate::inspector::DefaultInspector::watch_property): `None` while it has none
	/// or it doesn't read as a `T`.
	///
	/// Await [`Reactive::next`] in a `select!` beside [`Button::clicked`]: the first wait resolves at once with the
	/// current value, and later ones each time it changed. The editor reads the scene a few times a second, so a value
	/// can be that old. Waiting on it asks for no frames, so an idle panel leaves the editor idle.
	pub fn property<T: DeserializeOwned + PartialEq + 'static>(
		&self,
		name: &str,
	) -> Reactive<Polled<impl FnMut(&Shell) -> Option<T> + use<T>>, Shell> {
		let (entity, name) = (self.entity, name.to_owned());
		self.ctx.proactive(move |shell: &Shell| {
			let entity = shell.scene.iter().find(|scene| scene.handle == entity)?;
			let (_, value) = entity.properties.iter().find(|(property, _)| *property == name)?;
			T::deserialize(value).ok()
		})
	}

	/// Posts the message `message_type` with `payload` to the entity, through the inspector, which must have it
	/// registered with [`DefaultInspector::register_message`](crate::inspector::DefaultInspector::register_message).
	/// The editor posts it at the start of the next frame, and logs a warning when the inspector refuses it.
	pub async fn post(&mut self, message_type: &'static str, payload: &impl Serialize) {
		let payload = match serde_json::to_string(payload) {
			Ok(payload) => payload,
			Err(error) => {
				log::warn!(
					"A panel's message wasn't posted. The most likely cause is that its payload doesn't serialize to JSON: {error}. Make the payload's `Serialize` implementation produce JSON-compatible data."
				);
				return;
			}
		};
		let entity = self.entity;
		self.ctx
			.with(move |shell: &mut Shell| shell.posts.push((entity, message_type, payload)))
			.await;
	}

	/// Adds a button that spans the panel, labeled `label`. Wait for its clicks with [`Button::clicked`].
	pub async fn button(&mut self, label: &str) -> Button {
		let key = self.next_key();
		let mut element = self
			.ctx
			.element(("button", key))
			.container(|c| {
				c.width(CONTENT_WIDTH.into())
					.height(TOOL_HEIGHT.into())
					.corner_radius(7.0)
					.flow(flow::center)
					.hit_testable(true)
					.style(raised(TOOL_HEIGHT, false))
			})
			.await;
		element
			.element("label")
			.text(label, |t| t.font_size(13.0).style([solid(TEXT)]))
			.await;
		Button { element }
	}

	/// Adds a line of text that spans the panel. Change it with [`Label::set`].
	pub async fn label(&mut self, text: &str) -> Label {
		let key = self.next_key();
		let mut line = self
			.ctx
			.element(("label", key))
			.container(|c| c.width(CONTENT_WIDTH.into()).height(18.into()).flow(flow::centered_row))
			.await;
		let text = line
			.element("text")
			.text(text, |t| t.font_size(12.0).style([solid(TEXT)]))
			.await;
		Label {
			text,
			shown: String::new(),
		}
	}

	fn next_key(&mut self) -> usize {
		self.controls += 1;
		self.controls
	}
}

/// The `Button` struct is a button a [`Panel`] added, which a panel's loop waits on for clicks.
pub struct Button {
	element: EvaluationContext<Shell>,
}

impl Button {
	/// Waits for the next click, lighting the button while the pointer is over it. It can wait in a `select!` beside
	/// [`Panel::render`].
	pub fn clicked(&mut self) -> impl FusedFuture<Output = ()> + '_ {
		async move {
			loop {
				select! {
					_ = self.element.on(Events::Actuated) => return,
					_ = self.element.on(Events::PointerEntered) => self.light(true).await,
					_ = self.element.on(Events::PointerExited) => self.light(false).await,
				}
			}
		}
		.fuse()
	}

	async fn light(&mut self, hovered: bool) {
		self.element.update_container(|c| c.style(raised(TOOL_HEIGHT, hovered))).await;
	}
}

/// The `Label` struct is a line of text a [`Panel`] added, which a panel's loop changes as what it shows changes.
pub struct Label {
	text: EvaluationContext<Shell>,
	shown: String,
}

impl Label {
	/// Shows `text`, changing nothing when it already does.
	pub async fn set(&mut self, text: &str) {
		if self.shown != text {
			self.shown.clear();
			self.shown.push_str(text);
			self.text.update_text(|t| t.content(text)).await;
		}
	}
}
