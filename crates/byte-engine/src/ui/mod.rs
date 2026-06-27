//! Retained UI components, layout evaluation, styling, and rendering.
//!
//! Implement [`Component`] to describe a UI tree and evaluate it with
//! [`layout::engine::Engine`]. Components create [`Container`], [`Text`], and
//! other [`Primitive`] values through the layout context. Send the resulting
//! [`layout::engine::Render`] data to [`render_pass::UiRenderPass`] when
//! integrating UI into a graphics application.

use crate::core::Entity;

pub mod animation;
pub mod components;
pub mod control_flow;
pub mod element;
pub mod flow;
pub(crate) mod font;
pub mod intersection;
pub mod layout;
pub mod primitive;
pub mod render_pass;
pub mod style;
pub mod timer;
pub mod transform;

pub use animation::{
	animate, back_out, ease_in, ease_in_out, ease_out, ease_out_cubic, ease_out_quart, emphasized_out, spring, AnimationDriver,
	BackOut, Easing, Spring,
};
pub use components::container::Container;
pub use components::text::Text;
pub use element::Element;
pub use layout::Depth;
pub use primitive::Primitive;
pub use timer::{seconds, wait, WaitFuture};
pub use transform::Transform;
