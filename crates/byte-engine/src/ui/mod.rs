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
//! See [Build a user interface](/docs/use/gui)
//! for component, layout, event, focus, and rendering guidance.

pub mod animation;
pub mod components;
pub mod control_flow;
mod drag;
pub mod element;
pub mod flow;
pub(crate) mod font;
pub mod intersection;
pub mod layout;
mod point;
pub mod primitive;
pub mod reactive;
pub mod render_pass;
pub mod scale;
pub mod style;
pub mod timer;
pub mod transform;
pub mod watch;

pub use animation::{
	Animatable, Animation, Easing, EasingState, Ending, Motion, Smoothing, Spring, SpringState, Values, back_out, ease_in,
	ease_in_out, ease_out, ease_out_cubic, ease_out_quart, emphasized_out,
};
pub use components::{
	container::{Container, Sector},
	curve::{Curve, CurvePath, CurvePoint, CurveSegment},
	image::{Image, ImageSource},
	path::{FillRule, Path},
	text::Text,
};
pub use drag::{DragCapture, DragDrop};
pub use element::Id;
pub use flow::{FlowFunction, FlowInput, FlowOutput, Location, Location3, Offset, Size};
pub use font::{Fonts, load_font};
pub use layout::{
	Anchor, Depth, ElementGeometry, Geometry, Position, Sizing,
	context::{ContainerContext, Context, ElementContext, ElementKey, ElementSlot},
	engine::{
		AnimationFrame, AnimationHandle, ElementKind, Engine, EvaluationContext, FocusChangeFuture, MountedComponentFuture,
		PointerState, Properties, Render, RenderRevision, Runtime, Setup, SizeChangeFuture, UiEvent, UiKeyEvent,
		UiTextEditEvent,
	},
};
pub use point::{UiPoint, UiVector};
pub use primitive::{Events, Key, Primitives, TextEdit};
pub use reactive::{Next, Polled, Reactive, Source};
pub use render_pass::UiRenderPass;
pub use style::{Color, ConcreteLayer, ConcreteStyle, EdgeFeather, Layer, LayerKind, LinearGradient, MixModes, Shadow};
pub use timer::WaitFuture;
pub use transform::Transform;
pub use watch::{Subscriber, Watch};
