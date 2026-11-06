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
	/// The stable name a [`Bindings`](super::Bindings) document replaces this action's bindings under.
	pub(crate) name: Cow<'static, str>,
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
	/// Creates an action from its name, its default trigger bindings, and its output type.
	///
	/// `name` identifies the action in a [`Bindings`](super::Bindings) document, so keep it
	/// stable across releases: players' saved bindings refer to it. The bindings are the
	/// defaults a sink restores with [`InputSink::reset`](super::InputSink::reset).
	pub fn new(name: impl Into<Cow<'static, str>>, bindings: &[ActionBindingDescription], r#type: Types) -> Action {
		Action {
			name: name.into(),
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
	/// Binds a trigger, by its `Class.Trigger` name or its handle, to an action.
	///
	/// Pass a [`TriggerHandle`](super::TriggerHandle) to bind a trigger found at run time,
	/// such as the one a player pressed in a rebinding screen.
	pub fn new(input_source: impl Into<TriggerReference>) -> Self {
		ActionBindingDescription {
			input_source: input_source.into(),
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
	pub fn triggered_by(mut self, trigger: impl Into<TriggerReference>) -> Self {
		self.trigger = Some(trigger.into());
		self
	}

	/// Follows this source from a button press through movement and release.
	///
	/// The button must be a retained boolean control on the source's device class.
	/// A source value must exist at press time. Drag events bypass tick policy and
	/// carry [`super::ActionPhase::Started`], [`super::ActionPhase::Updated`], or
	/// [`super::ActionPhase::Ended`] with the current source value.
	/// Next, pass this binding to [`Action::new`] and handle [`super::ActionEvent::phase`].
	pub fn dragged_by(self, button: impl Into<TriggerReference>) -> Self {
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
#[repr(transparent)]
pub struct ActionHandle(pub(super) std::num::NonZeroU32);
impl ActionHandle {
	/// Encodes a bounded registration index as a nonzero identity.
	pub(super) fn from_index(index: u32) -> Self {
		assert!(
			index < super::HANDLE_TAG.get(),
			"Input handle capacity exceeded. A local index exhausted its 31-bit space. Reuse existing input declarations."
		);
		Self(super::HANDLE_TAG | index)
	}
	pub(super) const fn index(self) -> u32 {
		self.0.get() ^ super::HANDLE_TAG.get()
	}
}
const _: () = assert!(size_of::<Option<ActionHandle>>() == size_of::<u32>());

use std::borrow::Cow;

use math::Quaternion;
use smallvec::SmallVec;
use utils::RGBA;

use super::{Axis2, Axis3, TickPolicy, TriggerReference, Types, Value};
use crate::input::ValueMapping;
