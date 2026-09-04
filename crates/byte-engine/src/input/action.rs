//! Application-facing action declarations.
//!
//! Actions decouple gameplay concepts from physical controls. Create an
//! [`Action`] with bindings such as `Keyboard.W` or `Gamepad.LeftStick`, then
//! submit it through
//! [`GraphicsApplication::world`](crate::application::graphics::GraphicsApplication::world).
//! The standard trigger names are defined by [`crate::input::utils`].

trait ActionLike {
	fn get_bindings(&self) -> &[ActionBindingDescription];
	fn get_inputs(&self) -> &[TriggerMapping];
}

#[derive(Clone)]
/// The [`Action`] struct describes an application-level input value and the
/// physical trigger bindings that can produce it.
pub struct Action {
	pub(crate) bindings: SmallVec<[ActionBindingDescription; 8]>,
	pub(crate) inputs: SmallVec<[TriggerMapping; 8]>,
	pub(crate) r#type: Types,
	pub(crate) tick_policy: TickPolicy,
}

impl ActionLike for Action {
	fn get_bindings(&self) -> &[ActionBindingDescription] {
		&self.bindings
	}
	fn get_inputs(&self) -> &[TriggerMapping] {
		&self.inputs
	}
}

/// The [`InputValue`] trait marks typed values supported by the input runtime.
///
/// It is primarily used with [`crate::input::input_trigger::TriggerDescription`]
/// and should only be implemented when a matching [`Value`] representation
/// exists.
pub trait InputValue: Default + Clone + Copy + 'static {
	fn get_type() -> Types;
}

impl InputValue for bool {
	fn get_type() -> Types {
		Types::Boolean
	}
}

impl InputValue for i32 {
	fn get_type() -> Types {
		Types::Int
	}
}

impl InputValue for char {
	fn get_type() -> Types {
		Types::Unicode
	}
}

impl InputValue for f32 {
	fn get_type() -> Types {
		Types::Float
	}
}

impl InputValue for Axis2 {
	fn get_type() -> Types {
		Types::Vector2
	}
}

impl InputValue for Axis3 {
	fn get_type() -> Types {
		Types::Vector3
	}
}

impl InputValue for Quaternion {
	fn get_type() -> Types {
		Types::Quaternion
	}
}

impl InputValue for RGBA {
	fn get_type() -> Types {
		Types::Rgba
	}
}

impl Action {
	/// Creates an action from its physical trigger bindings and output type.
	pub fn new(bindings: &[ActionBindingDescription], r#type: Types) -> Action {
		Action {
			bindings: bindings.into(),
			inputs: SmallVec::new(),
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
	pub(crate) mapping: ValueMapping,
}

impl ActionBindingDescription {
	pub fn new(input_source: &'static str) -> Self {
		ActionBindingDescription {
			input_source: TriggerReference::Name(input_source),
			trigger: None,
			mapping: false.into(),
		}
	}

	/// Samples this source whenever `trigger` records `true`, using the same seat
	/// and device. Missing source values produce no event; releases are ignored.
	/// Actions containing triggered bindings bypass their tick policy; each
	/// positive record emits one snapshot. Next, pass this binding to [`Action::new`].
	pub fn triggered_by(mut self, trigger: &'static str) -> Self {
		self.trigger = Some(TriggerReference::Name(trigger));
		self
	}

	pub fn mapped(mut self, mapping: ValueMapping) -> Self {
		self.mapping = mapping;
		self
	}
}

/// The [`TriggerMapping`] struct is the resolved form of an action binding used by
/// the evaluator after trigger registration.
#[derive(Copy, Clone, Debug)]
pub struct TriggerMapping {
	/// The handle to the trigger that this mapping is for.
	pub(crate) trigger_handle: TriggerHandle,
	/// The optional boolean source that requests a snapshot.
	pub(crate) trigger: Option<TriggerHandle>,
	/// The value that this trigger maps to.
	pub(crate) mapping: Value,
	/// The function that this mapping uses to convert the trigger value to the action value.
	pub(crate) function: Option<Function>,
}

#[derive(Copy, Clone, PartialEq, Eq, Hash, Debug)]
/// The [`ActionHandle`] struct identifies an action registered with an
/// [`crate::input::InputManager`].
pub struct ActionHandle(pub(super) u32);

use math::Quaternion;
use smallvec::SmallVec;
use utils::RGBA;

use super::TriggerHandle;
use super::{Axis2, Axis3, Function, TickPolicy, Types, Value, input_manager::TriggerReference};
use crate::core::{Entity, EntityHandle};
use crate::input::ValueMapping;
