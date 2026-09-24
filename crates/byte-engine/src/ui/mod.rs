//! Retained UI components, layout evaluation, styling, and rendering.
//!
//! Implement an async component function to describe a UI tree and evaluate it with
//! [`layout::engine::Engine`]. Components declare [`Container`], [`Text`], and
//! other elements through the layout context and set their properties with
//! [`Properties`] setters; the engine creates and owns the elements. Convert the resulting
//! [`layout::engine::Render`] with [`render_pass::AdoptedRender`] and give it to
//! [`render_pass::UiRenderPass`] when integrating UI into a graphics application.
//! For pointer gestures, capture a hit-tested source with
//! [`layout::engine::Engine::press`] and validate the released [`DragDrop`]
//! against an application-owned drop target.
//!
//! See the [GUI guide](/docs/develop/gui)
//! for component, layout, event, focus, and rendering guidance.

use crate::core::Entity;

#[doc(hidden)]
pub mod animation;
#[doc(hidden)]
pub mod components;
#[doc(hidden)]
pub mod control_flow;
mod drag;
pub mod element;
#[doc(hidden)]
pub mod flow;
pub(crate) mod font;
#[doc(hidden)]
pub mod intersection;
#[doc(hidden)]
pub mod layout;
mod point;
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
pub mod watch;

pub use animation::{
	Animation, AnimationDriver, BackOut, Curves, Easing, Interpolate, Smoothing, Spring, Track, animate, back_out, ease_in,
	ease_in_out, ease_out, ease_out_cubic, ease_out_quart, emphasized_out, smooth, spring,
};
pub use components::{
	container::{Container, Sector},
	curve::{Curve, CurvePath, CurvePoint, CurveSegment},
	image::Image,
	path::{FillRule, Path},
	text::Text,
	text_field::TextField,
};
pub use drag::{DragCapture, DragDrop};
pub use element::{ConcreteElement, Element, ElementHandle, Id};
pub use flow::{FlowFunction, FlowInput, FlowOutput, Location, Location3, Offset, Size};
pub use layout::{
	Depth, Geometry, Position, Sizing,
	context::{ContainerContext, Context, ElementContext, ElementKey, ElementSlot},
	engine::{
		ElementKind, Engine, EvaluationContext, MountedComponentFuture, PointerState, Properties, Render, RenderRevision,
		Runtime, Setup, UiEvent, UiKeyEvent, UiTextEditEvent,
	},
};
pub use point::{UiPoint, UiVector};
pub use primitive::{BasePrimitive, CustomShape, Events, Key, Primitive, Primitives, Shapes, TextEdit};
pub use render_pass::UiRenderPass;
pub use style::{Color, ConcreteLayer, ConcreteStyle, EdgeFeather, Layer, LayerKind, LinearGradient, MixModes, Shadow};
pub use timer::WaitFuture;
pub use transform::Transform;
pub use visual::Visual;
pub use watch::{Subscriber, Watch};
