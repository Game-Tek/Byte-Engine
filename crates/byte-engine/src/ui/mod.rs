//! Retained UI components, layout evaluation, styling, and rendering.
//!
//! Implement an async component function to describe a UI tree and evaluate it with
//! [`layout::engine::Engine`]. Components create [`Container`], [`Text`], and
//! other [`Primitive`] values through the layout context. Send the resulting
//! [`layout::engine::Render`] data to [`render_pass::UiRenderPass`] when
//! integrating UI into a graphics application.

use crate::core::Entity;

#[doc(hidden)]
pub mod animation;
#[doc(hidden)]
pub mod components;
#[doc(hidden)]
pub mod control_flow;
pub mod element;
#[doc(hidden)]
pub mod flow;
pub(crate) mod font;
#[doc(hidden)]
pub mod intersection;
#[doc(hidden)]
pub mod layout;
#[doc(hidden)]
pub mod primitive;
#[doc(hidden)]
pub mod render_pass;
#[doc(hidden)]
pub mod style;
#[doc(hidden)]
pub mod timer;
#[doc(hidden)]
pub mod transform;
#[doc(hidden)]
pub mod visual;

pub use animation::{
	animate, back_out, ease_in, ease_in_out, ease_out, ease_out_cubic, ease_out_quart, emphasized_out, spring, Animation,
	AnimationDriver, BackOut, Curves, Easing, Interpolate, Spring, Track,
};
pub use components::{
	container::Container,
	curve::{Curve, CurvePath, CurvePoint, CurveSegment},
	image::Image,
	text::Text,
	text_field::TextField,
};
pub use element::{ConcreteElement, Element, ElementHandle, Id};
pub use flow::{FlowFunction, FlowInput, FlowOutput, Location, Location3, Offset, Size};
pub use layout::{
	context::{ContainerContext, Context, ElementContext, ElementSlot, MountedUiFuture, UiFuture},
	engine::{Engine, EvaluationContext, PointerState, Render, Runtime, UiEvent, UiKeyEvent, UiTextEditEvent},
	Depth, Geometry, Position, Sizing,
};
pub use primitive::{BasePrimitive, CustomShape, Events, Key, Primitive, Primitives, Shapes, TextEdit};
pub use render_pass::UiRenderPass;
pub use style::{Color, ConcreteLayer, ConcreteStyle, EdgeFeather, Layer, LayerKind, MixModes};
pub use timer::{seconds, wait, WaitFuture};
pub use transform::Transform;
pub use visual::Visual;
