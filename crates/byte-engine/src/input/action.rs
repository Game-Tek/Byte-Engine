//! Application-facing action declarations.
//!
//! Actions decouple gameplay concepts from physical controls. Create an
//! [`Action`] with bindings such as `Keyboard.W` or `Gamepad.LeftStick`, then
//! submit it through the action factory owned by
//! [`crate::application::graphics::GraphicsApplication`]. The standard trigger
//! names are defined by [`crate::input::utils`].

use math::{Quaternion, Vector2, Vector3};
use utils::RGBA;

use super::TriggerHandle;
use super::{input_manager::TriggerReference, Function, TickPolicy, Types, Value};
use crate::core::{Entity, EntityHandle};
use crate::input::ValueMapping;

trait ActionLike {
	fn get_bindings(&self) -> &[ActionBindingDescription];
	fn get_inputs(&self) -> &[TriggerMapping];
}

#[derive(Clone)]
/// The [`Action`] struct describes an application-level input value and the
/// physical trigger bindings that can produce it.
pub struct Action {
	pub(crate) name: &'static str,
	pub(crate) bindings: Vec<ActionBindingDescription>,
	pub(crate) inputs: Vec<TriggerMapping>,
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

impl InputValue for Vector2 {
	fn get_type() -> Types {
		Types::Vector2
	}
}

impl InputValue for Vector3 {
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
	pub fn new(name: &'static str, bindings: &[ActionBindingDescription], r#type: Types) -> Action {
		Action {
			name,
			bindings: bindings.to_vec(),
			inputs: Vec::new(),
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
	pub(crate) mapping: ValueMapping,
}

impl ActionBindingDescription {
	pub fn new(input_source: &'static str) -> Self {
		ActionBindingDescription {
			input_source: TriggerReference::Name(input_source),
			mapping: false.into(),
		}
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
	/// The value that this trigger maps to.
	pub(crate) mapping: Value,
	/// The function that this mapping uses to convert the trigger value to the action value.
	pub(crate) function: Option<Function>,
}

#[derive(Copy, Clone, PartialEq, Eq, Hash)]
/// The [`ActionHandle`] struct identifies an action registered with an
/// [`crate::input::InputManager`].
pub struct ActionHandle(pub(super) u32);
