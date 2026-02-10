use math::{Quaternion, Vector2, Vector3};
use utils::RGBA;

use crate::core::{Entity, EntityHandle};

use crate::input::ValueMapping;

use super::TriggerHandle;
use super::{input_manager::TriggerReference, Function, Types, Value};

trait ActionLike {
	fn get_bindings(&self) -> &[ActionBindingDescription];
	fn get_inputs(&self) -> &[TriggerMapping];
}

#[derive(Clone)]
pub struct Action {
	pub(crate) name: &'static str,
	pub(crate) bindings: Vec<ActionBindingDescription>,
	pub(crate) inputs: Vec<TriggerMapping>,
	pub(crate) r#type: Types,
}

impl ActionLike for Action {
	fn get_bindings(&self) -> &[ActionBindingDescription] { &self.bindings }
	fn get_inputs(&self) -> &[TriggerMapping] { &self.inputs }
}

/// This trait allows us to extract the `Types` from a generic type as a convenience to clients and also allows us to constrain generic types to only those that are valid for input values.
pub trait InputValue: Default + Clone + Copy + 'static {
	fn get_type() -> Types;
}

impl InputValue for bool {
	fn get_type() -> Types { Types::Boolean }
}

impl InputValue for i32 {
	fn get_type() -> Types { Types::Int }
}

impl InputValue for char {
	fn get_type() -> Types { Types::Unicode }
}

impl InputValue for f32 {
	fn get_type() -> Types { Types::Float }
}

impl InputValue for Vector2 {
	fn get_type() -> Types { Types::Vector2 }
}

impl InputValue for Vector3 {
	fn get_type() -> Types { Types::Vector3 }
}

impl InputValue for Quaternion {
	fn get_type() -> Types { Types::Quaternion }
}

impl InputValue for RGBA {
	fn get_type() -> Types { Types::Rgba }
}

impl Action {
	pub fn new(name: &'static str, bindings: &[ActionBindingDescription], r#type: Types) -> Action {
		Action {
			name,
			bindings: bindings.to_vec(),
			inputs: Vec::new(),
			r#type,
		}
	}
}

/// An action binding description is a description of how an input source is mapped to a value for an action.
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

/// A trigger mapping is a mapping of an input trigger to a value for an action.
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
/// Handle to an input event.
pub struct ActionHandle(pub(super) u32);
