//! Application-facing action declarations.
//!
//! Actions decouple gameplay concepts from physical controls. Create an
//! [`Action`] with bindings such as `Keyboard.W` or `Gamepad.LeftStick`, then
//! submit it through
//! [`GraphicsApplication::world`](crate::application::graphics::GraphicsApplication::world).
//! The standard trigger names are defined by [`crate::input::utils`].

#[derive(Clone)]
/// The [`Action`] struct describes an application-level input value and the
/// physical trigger bindings that can produce it.
pub struct Action {
	pub(crate) bindings: SmallVec<[ActionBindingDescription; 8]>,
	pub(crate) r#type: Types,
	pub(crate) tick_policy: TickPolicy,
}

/// The [`InputValue`] trait marks typed values supported by the input runtime.
///
/// It is primarily used with [`crate::input::trigger::TriggerDescription`]
/// and should only be implemented when a matching [`Value`] representation
/// exists.
pub trait InputValue: Default + Clone + Copy + 'static {
	/// Returns the representation used by this input's [`Value`] conversion.
	fn get_type() -> Types
	where
		Self: Into<Value>,
	{
		Self::default().into().into()
	}
}

impl InputValue for bool {}
impl InputValue for i32 {}
impl InputValue for char {}
impl InputValue for f32 {}
impl InputValue for Axis2 {}
impl InputValue for Axis3 {}
impl InputValue for Quaternion {}
impl InputValue for RGBA {}

impl Action {
	/// Creates an action from its physical trigger bindings and output type.
	pub fn new(bindings: &[ActionBindingDescription], r#type: Types) -> Action {
		Action {
			bindings: bindings.into(),
			r#type,
			tick_policy: TickPolicy::default(),
		}
	}

	/// Sets the tick policy for this action, controlling how frequently it emits events.
	pub fn tick_policy(mut self, tick_policy: TickPolicy) -> Self {
		self.tick_policy = tick_policy;
		self
	}
}

/// The [`ActionBindingDescription`] struct connects a named or handled trigger to
/// one contribution to an [`Action`].
#[derive(Copy, Clone, Debug)]
pub struct ActionBindingDescription {
	pub(crate) input_source: TriggerReference,
	pub(crate) trigger: Option<TriggerReference>,
	pub(crate) trigger_mode: TriggerMode,
	pub(crate) mapping: ValueMapping,
}

impl ActionBindingDescription {
	pub fn new(input_source: &'static str) -> Self {
		ActionBindingDescription {
			input_source: TriggerReference::Name(input_source),
			trigger: None,
			trigger_mode: TriggerMode::default(),
			mapping: false.into(),
		}
	}

	/// Samples this source on release by default, using the same seat and
	/// device. Missing source values produce no event. Use [`Self::trigger_on`]
	/// to choose presses instead. Triggered bindings bypass the action's tick
	/// policy: each matching record emits one snapshot.
	/// Next, pass this binding to [`Action::new`].
	pub fn triggered_by(mut self, trigger: &'static str) -> Self {
		self.trigger = Some(TriggerReference::Name(trigger));
		self
	}

	/// Follows this source from a button press through movement and release.
	///
	/// The button must be a retained boolean control on the source's device class.
	/// A source value must exist at press time. Drag events bypass tick policy and
	/// carry [`super::ActionPhase::Started`], [`super::ActionPhase::Updated`], or
	/// [`super::ActionPhase::Ended`] with the current source value.
	/// Next, pass this binding to [`Action::new`] and handle [`super::ActionEvent::phase`].
	pub fn dragged_by(self, button: &'static str) -> Self {
		self.triggered_by(button).trigger_on(TriggerMode::Drag)
	}

	/// Chooses how the button in [`Self::triggered_by`] drives this binding.
	/// The default is [`TriggerMode::Release`]. Next, pass this binding to [`Action::new`].
	pub fn trigger_on(mut self, mode: TriggerMode) -> Self {
		self.trigger_mode = mode;
		self
	}

	pub fn mapped(mut self, mapping: ValueMapping) -> Self {
		self.mapping = mapping;
		self
	}
}

/// Chooses whether a button requests a snapshot or a continuous drag.
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq)]
pub enum TriggerMode {
	/// Captures the value when the key or button is released.
	#[default]
	Release,
	/// Captures the value when the key or button is pressed.
	Press,
	/// Follows the source from press through movement and release.
	Drag,
}

#[derive(Copy, Clone, PartialEq, Eq, Hash, Debug)]
/// The [`ActionHandle`] struct identifies an action created on an
/// [`InputSink`](crate::input::InputSink).
pub struct ActionHandle(pub(super) u32);

use math::Quaternion;
use smallvec::SmallVec;
use utils::RGBA;

use super::{Axis2, Axis3, TickPolicy, TriggerReference, Types, Value};
use crate::input::ValueMapping;
