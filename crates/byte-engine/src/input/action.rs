use crate::core::{property::Property, Entity, EntityHandle};

use crate::{Vector2, Vector3};

use super::{input_manager::{InputSourceAction, InputSourceHandle}, Function, Types, Value};

trait ActionLike: Entity {
	fn get_bindings(&self) -> &[ActionBindingDescription];
	fn get_inputs(&self) -> &[InputSourceMapping];
}

pub struct Action<T: InputValue> {
	pub(crate) name: &'static str,
	pub(crate) bindings: Vec<ActionBindingDescription>,
	pub(crate) inputs: Vec<InputSourceMapping>,
	pub(crate) value: Property<T>,
}

impl <T: InputValue> Entity for Action<T> {}

impl <T: InputValue> ActionLike for Action<T> {
	fn get_bindings(&self) -> &[ActionBindingDescription] { &self.bindings }
	fn get_inputs(&self) -> &[InputSourceMapping] { &self.inputs }
}

pub trait InputValue: Default + Clone + Copy + 'static {
	fn get_type() -> Types;
}

impl InputValue for bool {
	fn get_type() -> Types { Types::Bool }
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

impl <T: InputValue + Clone + 'static> Action<T> {
	pub fn new(name: &'static str, bindings: &[ActionBindingDescription]) -> Action<T> {
		Action {
			name,
			bindings: bindings.to_vec(),
			value: Property::default(),
			inputs: Vec::new(),
		}
	}

	pub fn value(&self) -> &Property<T> { &self.value }
	pub fn value_mut(&mut self) -> &mut Property<T> { &mut self.value }
}

/// An action binding description is a description of how an input source is mapped to a value for an action.
#[derive(Copy, Clone, Debug)]
pub struct ActionBindingDescription {
	pub(crate) input_source: InputSourceAction,
	pub(crate) mapping: Value,
	pub(crate) function: Option<Function>
}

impl ActionBindingDescription {
	pub fn new(input_source: &'static str) -> Self {
		ActionBindingDescription {
			input_source: InputSourceAction::Name(input_source),
			mapping: Value::Bool(false),
			function: None,
		}
	}

	pub fn mapped(mut self, mapping: Value, function: Function) -> Self {
		self.mapping = mapping;
		self.function = Some(function);
		self
	}
}

pub struct InputSourceMapping {
	pub(crate) input_source_handle: InputSourceHandle,
	pub(crate) mapping: Value,
	pub(crate) function: Option<Function>,
}