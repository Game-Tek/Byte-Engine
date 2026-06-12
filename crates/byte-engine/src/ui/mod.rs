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
pub mod element;
pub mod flow;
pub(crate) mod font;
pub mod intersection;
pub mod layout;
pub mod primitive;
pub mod render_pass;
pub mod style;

pub use components::container::Container;
pub use components::text::Text;
pub use element::Element;
pub use layout::engine::Component;
pub use primitive::Primitive;
