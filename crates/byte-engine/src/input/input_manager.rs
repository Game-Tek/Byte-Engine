//! The input manager is responsible for managing HID devices and their events and properties.
//!
//! # Concepts
//! ## Device Class
//! A device class represents a type of device. Such as a keyboard, mouse, or gamepad.
//! ## Input Source
//! An input source is a source of input on a device class/type. Such as the UP key on a keyboard or the left trigger on a gamepad.
//! ## Input Destination
//! An input destination is a destination of input on a device. Such as the rumble motors on a gamepad.
//! ## Action
//! An action is an application specific event that is triggered by a combination of input sources.
//! For example move sideways is triggered by the left and right keys being pressed.
//!
//! # TODO
//! - [ ] Clamp input source values to their min and max values.
//! - [ ] Add deadzone support.
//! - [ ] Remove panics.
//! - [ ] Add device class and device grouping.

use std::{collections::HashMap, default, f32::consts::PI};

use log::warn;
use serde::de;
use utils::{insert_return_length, RGBA};

use crate::{core::{entity::EntityBuilder, listener::{EntitySubscriber, Listener}, orchestrator, property::Property, Entity, EntityHandle}, Quaternion, Vector2, Vector3};

use super::{action::{TriggerMapping, InputValue}, device::Device, device_class::{DeviceClass, DeviceClassHandle}, input_trigger::{Trigger, TriggerDescription}, Action, ActionBindingDescription, ActionHandle, DeviceHandle, Function, TriggerHandle, Types, Value};

#[derive(Copy, Clone, Debug)]
/// A trigger reference is a way to reference an input trigger.
/// It can be referenced by it's name or by it's handle.
/// It's provided as a convenience to the developer.
pub enum TriggerReference {
	/// Refer to the input trigger by it's handle.
	Handle(TriggerHandle),
	/// Refer to the input trigger by it's name.
	Name(&'static str)
}

#[derive(Copy, Clone, PartialEq)]
struct Record {
	device_handle: DeviceHandle,
	trigger_handle: TriggerHandle,
	value: Value,
	time: std::time::SystemTime,
}

impl PartialOrd for Record {
	fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
		self.time.partial_cmp(&other.time)
	}
}

struct InputSourceState {
	value: Value,
	time: std::time::SystemTime,
}

enum TypedHandle {
	Bool(EntityHandle<Action<bool>>),
	Float(EntityHandle<Action<f32>>),
	Vector2(EntityHandle<Action<Vector2>>),
	Vector3(EntityHandle<Action<Vector3>>),
}

/// An input event is an application specific event that is triggered by a combination of input sources.
struct InputAction {
	name: String,
	r#type: Types,
	trigger_mappings: Vec<TriggerMapping>,
	/// The stack is a list of input source records that simultaneous, connected, and currently active.
	/// The stack is used to determine the value of the input event.
	/// The stack is ordered by the time the input source was first pressed.
	/// An element is popped when it's corresponding input source is released.
	/// Example:
	/// 	- Input source `A` is pressed.
	/// 		(Value is A)
	/// 	- Input source `B` is pressed.
	/// 		(Value is B)
	/// 	- Input source `B` is released.
	/// 		(Value is A)
	/// 	- Input source `B` is pressed.
	/// 		(Value is B)
	/// 	- Input source `A` is released.
	/// 		(Value is B)
	/// 	- Input source `B` is released.
	/// 		(Value is None)
	stack: Vec<Record>,
	handle: Option<TypedHandle>,
}

pub struct InputSourceEventState {
	device_handle: DeviceHandle,
	value: Value,
}

/// The input event state is the value of an input event.
pub struct InputEventState {
	/// The device that triggered the input event.
	device_handle: DeviceHandle,
	/// The handle to the input event.
	input_event_handle: ActionHandle,
	/// The value of the input event.
	value: Value,
}

/// The input manager is responsible for managing input devices and input events.
pub struct InputManager {
	device_classes: Vec<DeviceClass>,
	triggers: Vec<Trigger>,
	devices: Vec<Device>,
	records: Vec<Record>,
	actions: Vec<InputAction>,
	/// Stores the last value of a trigger relative to the device it belongs to.
	trigger_values: HashMap<(DeviceHandle, TriggerHandle), Record>,
	/// Stores the last value of an action relative to the device it belongs to.
	action_values: HashMap<(DeviceHandle, ActionHandle), Value>,
}

impl InputManager {
	/// Creates a new input manager.
	pub fn new() -> Self {
		InputManager {
			device_classes: Vec::new(),
			triggers: Vec::new(),
			devices: Vec::new(),
			records: Vec::new(),
			actions: Vec::new(),
			trigger_values: HashMap::with_capacity(512),
			action_values: HashMap::with_capacity(64),
		}
	}

	pub fn new_as_system<'a>() -> EntityBuilder<'a, InputManager> {
		EntityBuilder::new(Self::new())
			.listen_to::<Action<bool>>()
			.listen_to::<Action<f32>>()
			.listen_to::<Action<Vector2>>()
			.listen_to::<Action<Vector3>>()
	}

	/// Registers a device class/type.
	///
	/// One example is a keyboard.
	///
	/// # Arguments
	///
	/// * `name` - The name of the device type. **Should be pascalcase.**
	///
	/// # Example
	///
	/// ```
	/// # use byte_engine::input::InputManager;
	/// # let mut input_manager = InputManager::new();
	/// input_manager.register_device_class("Keyboard");
	/// ```
	pub fn register_device_class(&mut self, name: &str) -> DeviceClassHandle {
		let device_class = DeviceClass {
			name: name.to_string(),
		};

		DeviceClassHandle(insert_return_length(&mut self.device_classes, device_class) as u32)
	}

	/// Registers an input source on a device class.
	///
	/// One example is the UP key on a keyboard.
	///
	/// The input source is associated with a device class/type.
	/// The input source has a default value assigned from the `value_type` param.
	///
	/// # Arguments
	///
	/// * `device_handle` - The handle of the device.
	/// * `name` - The name of the input source.
	/// * `value_type` - The type of the value of the input source.
	///
	/// # Example
	///
	/// ```rust
	/// # use byte_engine::input::{InputManager, input_manager::{InputTypes, InputSourceDescription}};
	/// # let mut input_manager = InputManager::new();
	/// # let keyboard_device_class_handle = input_manager.register_device_class("Keyboard");
	/// input_manager.register_input_source(&keyboard_device_class_handle, "Up", InputTypes::Bool(InputSourceDescription::new(false, false, false, true)));
	/// ```
	pub fn register_trigger<T>(&mut self, device_handle: &DeviceClassHandle, name: &str, value_type: TriggerDescription<T>) -> TriggerHandle where T: InputValue + Into<Value> {
		let default = value_type.default;

		let default: Value = default.into();
		let default_value_type: Types = default.into();

		assert_eq!(default_value_type, T::get_type(), "Default value type does not match input source type");

		let input_source = Trigger {
			device_class_handle: *device_handle,
			name: name.to_string(),
			r#type: T::get_type(),
			default,
		};

		TriggerHandle(insert_return_length(&mut self.triggers, input_source) as u32)
	}

	/// Registers an input destination on a device.
	///
	/// One example is the rumble motors on a gamepad.
	///
	/// The input destination is associated with a device.
	///
	/// # Arguments
	/// * `device_handle` - The handle of the device.
	/// * `name` - The name of the input destination.
	/// * `value_type` - The type of the value of the input destination.
	///
	/// # Example
	/// ```rust
	/// # use byte_engine::input::{InputManager, input_manager::{InputTypes, InputSourceDescription}};
	/// # let mut input_manager = InputManager::new();
	/// # let gamepad_device_class_handle = input_manager.register_device_class("Gamepad");
	/// input_manager.register_input_destination(&gamepad_device_class_handle, "Rumble", InputTypes::Float(InputSourceDescription::new(0f32, 0f32, 0f32, 1f32)));
	/// ```
	pub fn register_input_destination<T: InputValue>(&mut self, _device_class_handle: &DeviceClassHandle, _name: &str, _value_type: TriggerDescription<T>) -> TriggerHandle {
		TriggerHandle(0)
	}

	/// Creates an instance of a device class.
	/// This represents a particular device of a device class. Such as a single controller or a keyboard.
	///
	/// # Arguments
	///
	/// * `device_class_handle` - The handle of the device class.
	///
	/// # Example
	///
	/// ```rust
	/// # use byte_engine::input::InputManager;
	/// # let mut input_manager = InputManager::new();
	/// # let keyboard_device_class_handle = input_manager.register_device_class("Keyboard");
	/// let keyboard_device = input_manager.create_device(&keyboard_device_class_handle);
	/// ```
	pub fn create_device(&mut self, device_class_handle: &DeviceClassHandle) -> DeviceHandle {
		let other_device = self.devices.iter().filter(|d| d.device_class_handle.0 == device_class_handle.0).min_by_key(|d| d.index);

		let index = match other_device {
			Some(device) => device.index + 1,
			None => 0,
		};

		let device = Device {
			device_class_handle: device_class_handle.clone(),
			index,
		};

		DeviceHandle(insert_return_length(&mut self.devices, device) as u32)
	}

	/// Registers an input event.
	///
	/// One example is a "move forward" being pressed.
	///
	/// - Action:
	/// 	- Action: Returns the action value.
	/// 	- Character: Returns a character when the action is pressed.
	/// 	- Linear: Returns a float value when the action is pressed.
	/// 	- 2D: Returns a 2D point when the action is pressed.
	/// 	- 3D: Returns a 3D point when the action is pressed.
	/// 	- Quaternion: Returns a quaternion when the action is pressed.
	/// 	- RGBA: Returns a RGBA color when the action is pressed.
	/// - Character:
	/// 	- Action: Returns an action value when the character is pressed.
	/// 	- Character: Returns the character value.
	/// 	- Linear: Returns a float value when the character is pressed.
	/// 	- 2D: Returns a 2D point when the character is pressed.
	/// 	- 3D: Returns a 3D point when the character is pressed.
	/// 	- Quaternion: Returns a quaternion when the character is pressed.
	/// 	- RGBA: Returns a RGBA color when the character is pressed.
	/// - Linear:
	/// 	- Action: Returns an action value when the float value is reached.
	/// 	- Character: Returns a character when the float value is reached.
	/// 	- Linear: Returns the float value.
	/// 	- 2D: Interpolates between two 2D points based on the range of the float value.
	/// 	- 3D: Interpolates between two 3D points based on the range of the float value.
	/// 	- Quaternion: Interpolates between two quaternions based on the range of the float value.
	/// 	- RGBA: Interpolates between two RGBA colors based on the range of the float value.
	/// - 2D:
	/// 	- Action: Returns an action value when the 2D point is reached.
	/// 	- Character: Returns a character when the 2D point is reached.
	/// 	- Linear: Returns a float value when the 2D point is reached.
	/// 	- 2D: Returns the 2D point.
	/// 	- 3D: Returns a 3D point when the 2D point is reached.
	/// 	- Quaternion: Returns a quaternion when the 2D point is reached.
	/// 	- RGBA: Returns a RGBA color when the 2D point is reached.
	/// - 3D:
	/// 	- Action: Returns an action value when the 3D point is reached.
	/// 	- Character: Returns a character when the 3D point is reached.
	/// 	- Linear: Returns a float value when the 3D point is reached.
	/// 	- 2D: Returns a 2D point when the 3D point is reached.
	/// 	- 3D: Returns the 3D point.
	/// 	- Quaternion: Returns a quaternion when the 3D point is reached.
	/// 	- RGBA: Returns a RGBA color when the 3D point is reached.
	/// - Quaternion:
	/// 	- Action: Returns an action value when the quaternion is reached.
	/// 	- Character: Returns a character when the quaternion is reached.
	/// 	- Linear: Returns a float value when the quaternion is reached.
	/// 	- 2D: Returns a 2D point when the quaternion is reached.
	/// 	- 3D: Returns a 3D point when the quaternion is reached.
	/// 	- Quaternion: Returns the quaternion.
	/// 	- RGBA: Returns a RGBA color when the quaternion is reached.
	/// - RGBA:
	/// 	- Action: Returns an action value when the RGBA color is reached.
	/// 	- Character: Returns a character when the RGBA color is reached.
	/// 	- Linear: Returns a float value when the RGBA color is reached.
	/// 	- 2D: Returns a 2D point when the RGBA color is reached.
	/// 	- 3D: Returns a 3D point when the RGBA color is reached.
	/// 	- Quaternion: Returns a quaternion when the RGBA color is reached.
	/// 	- RGBA: Returns the RGBA color.

	/// Records an input trigger value for a device into a queue.
	/// The new value for the input trigger is not reflected until the next call to `update()`.
	///
	/// One example is the UP key on a keyboard being pressed.
	pub fn record_trigger_value_for_device(&mut self, device_handle: DeviceHandle, trigger_reference: TriggerReference, value: Value) {
		let trigger = if let Some(trigger) = self.get_trigger_from_trigger_reference(&trigger_reference) {
			trigger
		} else {
			warn!("Tried to record an input source action that doesn't exist");
			return;
		};

		if trigger.r#type != value.into() {
			warn!("Tried to record an extraneous type into input source: {}", trigger.name);
			return; // Value type does not match input source declared type, so don't record.
		}

		let trigger_handle = if let Some(input_source_handle) = self.to_trigger_handle(&trigger_reference) {
			input_source_handle
		} else {
			warn!("Tried to record an input source action that doesn't exist");
			return;
		};

		let device = &mut self.devices[device_handle.0 as usize];

		let time = std::time::SystemTime::now();

		let record = Record {
			device_handle,
			trigger_handle,
			value,
			time,
		};

		self.records.push(record);
	}

	pub fn update(&mut self) {
		if self.records.is_empty() { return; }

		let mut records = self.records.clone();
		self.records.clear();

		records.sort_by(|a, b| a.time.cmp(&b.time));

		let mut last_records: HashMap<(DeviceHandle, TriggerHandle), Record> = HashMap::with_capacity(records.len());

		for record in records {
			last_records.insert((record.device_handle, record.trigger_handle), record);
		}

		// Deduped and most recent records.
		let records = last_records.into_values().collect::<Vec<_>>();

		for record in &records {
			let value = record.value;
			let device_handle = record.device_handle;
			let trigger_handle = record.trigger_handle;

			self.trigger_values.insert((device_handle, trigger_handle), record.clone());
		}

		for record in &records {
			if let Value::Bool(pressed) = record.value {
				let actions = self.actions.iter_mut().filter(|a| a.trigger_mappings.iter().any(|tm| tm.trigger_handle == record.trigger_handle));
	
				if pressed {
					for action in actions {
						action.stack.push(record.clone());
					}
				} else {
					for action in actions {
						action.stack.retain(|r| (record.device_handle, record.trigger_handle) != (r.device_handle, r.trigger_handle));
					}
				}
			}
		}

		for (i, action) in self.actions.iter().enumerate() {
			let action_records = records.iter().filter(|r| action.trigger_mappings.iter().any(|tm| tm.trigger_handle == r.trigger_handle));

			let most_recent_record = action_records.max_by_key(|r| r.time);
			
			let record = if let Some(record) = most_recent_record { record } else { continue; };

			let value = if let Some(value) = Self::resolve_action_value_from_record(action, record) { value } else { continue; };

			self.action_values.insert((record.device_handle, ActionHandle(i as u32)), value);

			if let Some(handle) = &action.handle {
				match value {
					Value::Bool(v) => {
						match handle {
							TypedHandle::Bool(handle) => { handle.map(|a| { let mut a = a.write(); a.value_mut().set(|_| { v }); }) }
							_ => {}
						}
					}
					Value::Float(v) => {
						match handle {
							TypedHandle::Float(handle) => { handle.map(|a| { let mut a = a.write(); a.value_mut().set(|_| { v }); }) }
							_ => {}
						}
					}
					Value::Vector2(v) => {
						match handle {
							TypedHandle::Vector2(handle) => { handle.map(|a| { let mut a = a.write(); a.value_mut().set(|_| { v }); }) }
							_ => {}
						}
					}
					Value::Vector3(v) => {
						match handle {
							TypedHandle::Vector3(handle) => { handle.map(|a| { let mut a = a.write(); a.value_mut().set(|_| { v }); }) }
							_ => {}
						}
					}
					_ => {
						log::error!("Not implemented!");
					}
				}
			}
		}
	}

	pub fn create_action(&mut self, name: &str, r#type: Types, action_binding_descriptions: &[ActionBindingDescription]) -> ActionHandle {
		let input_event = InputAction {
			name: name.to_string(),
			r#type,
			trigger_mappings: action_binding_descriptions.iter().map(|input_event| {
				Some(TriggerMapping {
					trigger_handle: self.to_trigger_handle(&input_event.input_source)?,
					mapping: input_event.mapping,
					function: input_event.function,
				})
			}).filter_map(|input_event| input_event).collect::<Vec<_>>(),
			stack: Vec::new(),
			handle: None,
		};

		let handle = ActionHandle(self.actions.len() as u32);
		self.actions.push(input_event);

		handle
	}

	/// Get the latest processed value for an trigger for a device.
	pub fn get_trigger_value_for_device(&self, device_handle: DeviceHandle, trigger_reference: TriggerReference) -> Result<Value, ()> {
		let trigger_handle = self.to_trigger_handle(&trigger_reference).ok_or(())?;

		let trigger = self.get_trigger_from_trigger_reference(&trigger_reference).ok_or(())?;

		Ok(self.trigger_values.get(&(device_handle, trigger_handle)).map(|record| record.value).unwrap_or(trigger.default))
	}

	/// Gets the latest processed value of an action for a device.
	pub fn get_action_state(&self, action_handle: ActionHandle, device_handle: DeviceHandle) -> InputEventState {
		self.action_values.get(&(device_handle, action_handle)).map(|record| {
			InputEventState {
				device_handle,
				input_event_handle: action_handle,
				value: record.clone(),
			}
		}).unwrap_or_else(|| {
			let action = self.actions.get(action_handle.0 as usize).unwrap();
			let default_value = match action.r#type {
				Types::Bool => Value::Bool(false),
				Types::Float => Value::Float(0f32),
				Types::Vector2 => Value::Vector2(Vector2 { x: 0f32, y: 0f32 }),
				Types::Vector3 => Value::Vector3(Vector3 { x: 0f32, y: 0f32, z: 0f32 }),
				_ => panic!("Not implemented!"),
			};

			InputEventState {
				device_handle,
				input_event_handle: action_handle,
				value: default_value,
			}
		})
	}

	fn resolve_action_value_from_record(action: &InputAction, record: &Record) -> Option<Value> {
		let mapping = action.trigger_mappings.iter().find(|ied| ied.trigger_handle == record.trigger_handle)?;

		match action.r#type {
			Types::Bool => {
				let bool: Option<bool> = match record.value {
					Value::Bool(record_value) => {
						record_value.into()
					}
					Value::Float(record_value) => {
						(record_value != 0f32).into()
					}
					_ => {
						log::error!("resolve_action_value_from_record not implemented for type!");
						return None;
					},
				};

				bool.map(|b| Value::Bool(b))
			}
			Types::Float => {
				let float: Option<f32> = match record.value {
					Value::Bool(record_value) => {
						if let Some(last) = action.stack.last() {
							if let Value::Bool(pressed) = last.value { // Stack entry value does not really matter because if it is in the stack it _is_ pressed.
								let trigger_mapping = action.trigger_mappings.iter().find(|description| description.trigger_handle == last.trigger_handle)?;

								match trigger_mapping.mapping {
									Value::Bool(value) => if value { 1f32 } else { 0f32 },
									Value::Unicode(_) => 0f32,
									Value::Float(value) => value,
									Value::Int(value) => value as f32,
									Value::Rgba(value) => value.r,
									Value::Vector2(value) => value.x,
									Value::Vector3(value) => value.x,
									Value::Quaternion(value) => value[0],
								}
							} else {
								panic!("Last value is not a boolean!");
							}
						} else {
							match mapping.mapping {
								Value::Bool(value) => if value { 1f32 } else { 0f32 },
								Value::Unicode(_) => 0f32,
								Value::Float(mapping_value) => mapping_value * record_value as u32 as f32,
								Value::Int(value) => value as f32,
								Value::Rgba(value) => value.r,
								Value::Vector2(value) => value.x,
								Value::Vector3(value) => value.x,
								Value::Quaternion(value) => value[0],
							}
						}.into()
					}
					Value::Float(record_value) => {
						record_value.into()
					}
					_ => {
						log::error!("Not implemented!");
						return None;
					},
				};

				float.map(|f| Value::Float(f))
			}
			Types::Vector2 => {
				let vector2: Option<Vector2> = match record.value {
					Value::Bool(record_value) => {
						match mapping.mapping {
							Value::Vector2(mapping_value) => (record_value as u32 as f32 * mapping_value).into(),
							_ => None,
						}
					},
					Value::Unicode(_) => {
						log::error!("Not implemented!");
						return None;
					},
					Value::Float(_) => {
						log::error!("Not implemented!");
						return None;
					},
					Value::Int(_) => {
						log::error!("Not implemented!");
						return None;
					},
					Value::Rgba(_) => {
						log::error!("Not implemented!");
						return None;
					},
					Value::Vector2(value) => value.into(),
					Value::Vector3(value) => Vector2 { x: value.x, y: value.y }.into(),
					Value::Quaternion(_) => {
						log::error!("Not implemented!");
						return None;
					},
				};

				vector2.map(|v| Value::Vector2(v))
			}
			Types::Vector3 => {
				let vector3: Option<Vector3> = match record.value {
					Value::Bool(record_value) => {
						match mapping.mapping {
							Value::Vector3(mapping_value) => (record_value as u32 as f32 * mapping_value).into(),
							_ => None,
						}
					},
					Value::Unicode(_) => {
						log::error!("Not implemented!");
						return None;
					},
					Value::Float(_) => {
						log::error!("Not implemented!");
						return None;
					},
					Value::Int(_) => {
						log::error!("Not implemented!");
						return None;
					},
					Value::Rgba(_) => {
						log::error!("Not implemented!");
						return None;
					},
					Value::Vector2(record_value) => {
						if let Some(function) = mapping.function {
							if let Function::Sphere = function {
								let r = record_value;

								let x_pi = r.x * PI;
								let y_pi = r.y * PI * 0.5f32;

								let x = x_pi.sin() * y_pi.cos();
								let y = y_pi.sin();
								let z = x_pi.cos() * y_pi.cos();

								let transformation = if let Value::Vector3(transformation) = mapping.mapping { transformation } else { log::error!("Not implemented!"); return None; };

								(Vector3 { x, y, z } * transformation).into()
							} else {
								log::error!("Not implemented!");
								return None;
							}
						} else {
							Vector3 { x: record_value.x, y: record_value.y, z: 0f32 }.into()
						}
					},
					Value::Vector3(value) => value.into(),
					Value::Quaternion(_) => {
						log::error!("Not implemented!");
						return None;
					},
				};

				vector3.map(|v| Value::Vector3(v))
			}
			_ => {
				log::error!("Not implemented!");
				return None;
			},
		}
	}

	fn get_trigger_from_trigger_reference(&self, trigger_reference: &TriggerReference) -> Option<&Trigger> {
		self.to_trigger_handle(trigger_reference).and_then(|trigger_handle| self.triggers.get(trigger_handle.0 as usize))
	}

	fn get_device(&self, device_handle: &DeviceHandle) -> &Device {
		&self.devices[device_handle.0 as usize]
	}

	fn to_trigger_handle(&self, trigger_reference: &TriggerReference) -> Option<TriggerHandle> {
		match trigger_reference {
			TriggerReference::Handle(handle) => Some(*handle),
			TriggerReference::Name(name) => {
				let tokens = (*name).split('.');

				let input_device_class = self.device_classes.iter().enumerate().find(|(_, device_class)| device_class.name == tokens.clone().next().unwrap());

				if let Some((idc_index, _)) = input_device_class {
					let input_device_class_handle = DeviceClassHandle(idc_index as u32);

					let trigger = self.triggers.iter().enumerate().find(|(_, input_source)| input_source.name == tokens.clone().last().unwrap() && input_source.device_class_handle == input_device_class_handle);

					if let Some(trigger) = trigger {
						Some(TriggerHandle(trigger.0 as u32))
					} else {
						None
					}
				} else {
					None
				}
			}
		}
	}
}

impl Into<TypedHandle> for EntityHandle<Action<bool>> {
	fn into(self) -> TypedHandle {
		TypedHandle::Bool(self)
	}
}

impl Into<TypedHandle> for EntityHandle<Action<f32>> {
	fn into(self) -> TypedHandle {
		TypedHandle::Float(self)
	}
}

impl Into<TypedHandle> for EntityHandle<Action<Vector2>> {
	fn into(self) -> TypedHandle {
		TypedHandle::Vector2(self)
	}
}

impl Into<TypedHandle> for EntityHandle<Action<Vector3>> {
	fn into(self) -> TypedHandle {
		TypedHandle::Vector3(self)
	}
}

impl <T: InputValue> EntitySubscriber<Action<T>> for InputManager where EntityHandle<Action<T>>: Into<TypedHandle> {
	fn on_create<'a>(&'a mut self, handle: EntityHandle<Action<T>>, action: &Action<T>) -> () {
		let (name, r#type, input_events,) = (action.name, T::get_type(), &action.bindings);

		let input_event = InputAction {
			name: name.to_string(),
			r#type,
			trigger_mappings: input_events.iter().map(|input_event| {
				Some(TriggerMapping {
					trigger_handle: self.to_trigger_handle(&input_event.input_source)?,
					mapping: input_event.mapping,
					function: input_event.function,
				})
			}).filter_map(|input_event| input_event).collect::<Vec<_>>(),
			stack: Vec::new(),
			handle: Some(handle.clone().into()),
		};

		self.actions.push(input_event);
	}
}

impl Entity for InputManager {}

#[cfg(test)]
mod tests {
	use crate::{core::{spawn, spawn_as_child}, input::input_trigger::TriggerDescription};
	use std::{cell::RefCell, ops::DerefMut, rc::Rc, sync::Arc};

	use maths_rs::prelude::Base;

	use crate::{gameplay::space::Space, input::ActionBindingDescription};

	use super::*;

	fn declare_keyboard_input_device_class(input_manager: &mut InputManager) -> DeviceClassHandle {
		let device_class_handle = input_manager.register_device_class("Keyboard");

		let _up_input_source = input_manager.register_trigger(&device_class_handle, "Up", TriggerDescription::<bool>::default());
		let _down_input_source = input_manager.register_trigger(&device_class_handle, "Down", TriggerDescription::<bool>::default());
		let _left_input_source = input_manager.register_trigger(&device_class_handle, "Left", TriggerDescription::<bool>::default());
		let _right_input_source = input_manager.register_trigger(&device_class_handle, "Right", TriggerDescription::<bool>::default());

		let key_source_description = TriggerDescription::new('\0', '\0', '\0', 'Z');

		let _a_input_source = input_manager.register_trigger(&device_class_handle, "Character", TriggerDescription::<bool>::default());

		device_class_handle
	}

	fn declare_gamepad_input_device_class(input_manager: &mut InputManager) -> DeviceClassHandle {
		let device_class_handle = input_manager.register_device_class("Gamepad");

		let key_source_description = TriggerDescription::new(0.0f32, 0.0f32, 0.0f32, 1.0f32);

		let _up_input_source = input_manager.register_trigger(&device_class_handle, "LeftTrigger", key_source_description);
		let _down_input_source = input_manager.register_trigger(&device_class_handle, "RighTrigger", key_source_description);

		let key_source_description = TriggerDescription::new(Vector2::zero(), Vector2::zero(), Vector2 { x: -1.0, y: -1.0, }, Vector2 { x: 1.0, y: 1.0, });

		let _a_input_source = input_manager.register_trigger(&device_class_handle, "LeftStick", key_source_description);
		let _b_input_source = input_manager.register_trigger(&device_class_handle, "RightStick", key_source_description);

		let light_source_description = TriggerDescription::new(RGBA { r: 0.0f32, g: 0.0f32, b: 0.0f32, a: 0.0f32 }, RGBA { r: 0.0f32, g: 0.0f32, b: 0.0f32, a: 0.0f32 }, RGBA { r: 0.0f32, g: 0.0f32, b: 0.0f32, a: 0.0f32 }, RGBA { r: 1.0f32, g: 1.0f32, b: 1.0f32, a: 1.0f32 });

		let _light_destination = input_manager.register_input_destination(&device_class_handle, "Light", light_source_description);

		device_class_handle
	}

	fn declare_vr_headset_input_device_class(input_manager: &mut InputManager) -> DeviceClassHandle {
		let device_class_handle = input_manager.register_device_class("Headset");

		let source_description = TriggerDescription::new(Vector3::new(0f32, 1.80f32, 0f32), Vector3::new(0f32, 0f32, 0f32), Vector3::min_value(), Vector3::max_value());

		let _position_input_source = input_manager.register_trigger(&device_class_handle, "Position", source_description);

		let _rotation_input_source = input_manager.register_trigger(&device_class_handle, "Orientation", TriggerDescription::<Quaternion>::default());

		device_class_handle
	}

	fn declare_funky_input_device_class(input_manager: &mut InputManager) -> DeviceClassHandle {
		let device_class_handle = input_manager.register_device_class("Funky");

		let _funky_input_source = input_manager.register_trigger(&device_class_handle, "Int", TriggerDescription::new(0, 0, 0, 3));

		input_manager.register_trigger(&device_class_handle, "Rgba", TriggerDescription::new(RGBA { r: 0.0f32, g: 0.0f32, b: 0.0f32, a: 0.0f32 }, RGBA { r: 0.0f32, g: 0.0f32, b: 0.0f32, a: 0.0f32 }, RGBA { r: 0.0f32, g: 0.0f32, b: 0.0f32, a: 0.0f32 }, RGBA { r: 1.0f32, g: 1.0f32, b: 1.0f32, a: 1.0f32 }));

		device_class_handle
	}

	#[test]
	fn create_device_class() {
		let mut input_manager = InputManager::new();

		let _device_class_handle = input_manager.register_device_class("Keyboard");
	}

	#[test]
	fn create_input_sources() {
		let mut input_manager = InputManager::new();

		let gamepad_class_handle = input_manager.register_device_class("Gamepad");

		declare_keyboard_input_device_class(&mut input_manager);

		let stick_source_description = TriggerDescription::new(Vector2::zero(), Vector2::zero(), Vector2 { x: -1.0, y: -1.0, }, Vector2 { x: 1.0, y: 1.0, });

		let _gamepad_left_stick_input_source = input_manager.register_trigger(&gamepad_class_handle, "LeftStick", stick_source_description);
		let _gamepad_right_stick_input_source = input_manager.register_trigger(&gamepad_class_handle, "RightStick", stick_source_description);

		let trigger_source_description = TriggerDescription::<f32>::default();

		let _trigger_input_source = input_manager.register_trigger(&gamepad_class_handle, "LeftTrigger", trigger_source_description);
	}

	#[test]
	fn test_boolean_source_input_overlap_action() {
		let mut input_manager = InputManager::new();

		let x = declare_keyboard_input_device_class(&mut input_manager);

		let action = input_manager.create_action("MoveLongitudinally", Types::Float, &[ActionBindingDescription::new("Keyboard.Up").mapped(1f32.into(), Function::Boolean), ActionBindingDescription::new("Keyboard.Down").mapped((-1f32).into(), Function::Boolean)]);

		let device = input_manager.create_device(&x);

		assert_eq!(input_manager.get_action_state(action, device).value, Value::Float(0f32));

		input_manager.record_trigger_value_for_device(device, TriggerReference::Name("Keyboard.Up"), true.into());

		input_manager.update();

		assert_eq!(input_manager.get_action_state(action, device).value, Value::Float(1f32));

		input_manager.record_trigger_value_for_device(device, TriggerReference::Name("Keyboard.Up"), false.into());

		input_manager.update();

		assert_eq!(input_manager.get_action_state(action, device).value, Value::Float(0f32));

		input_manager.record_trigger_value_for_device(device, TriggerReference::Name("Keyboard.Up"), true.into());

		input_manager.update();

		assert_eq!(input_manager.get_action_state(action, device).value, Value::Float(1f32));

		input_manager.record_trigger_value_for_device(device, TriggerReference::Name("Keyboard.Down"), true.into());

		input_manager.update();

		assert_eq!(input_manager.get_action_state(action, device).value, Value::Float(-1f32));

		input_manager.record_trigger_value_for_device(device, TriggerReference::Name("Keyboard.Down"), false.into());

		input_manager.update();

		assert_eq!(input_manager.get_action_state(action, device).value, Value::Float(1f32));

		input_manager.record_trigger_value_for_device(device, TriggerReference::Name("Keyboard.Up"), false.into());

		input_manager.update();

		assert_eq!(input_manager.get_action_state(action, device).value, Value::Float(0f32));

		input_manager.record_trigger_value_for_device(device, TriggerReference::Name("Keyboard.Up"), true.into());

		input_manager.update();

		assert_eq!(input_manager.get_action_state(action, device).value, Value::Float(1f32));

		input_manager.record_trigger_value_for_device(device, TriggerReference::Name("Keyboard.Down"), true.into());

		input_manager.update();

		assert_eq!(input_manager.get_action_state(action, device).value, Value::Float(-1f32));

		input_manager.record_trigger_value_for_device(device, TriggerReference::Name("Keyboard.Up"), false.into());

		input_manager.update();

		assert_eq!(input_manager.get_action_state(action, device).value, Value::Float(-1f32));

		input_manager.record_trigger_value_for_device(device, TriggerReference::Name("Keyboard.Down"), false.into());

		input_manager.update();

		assert_eq!(input_manager.get_action_state(action, device).value, Value::Float(0f32));
	}

	fn record_and_assert_input_source_action_sequence<A, Z>(input_manager: &mut InputManager, device: DeviceHandle, trigger_reference: TriggerReference, a: A, b: A, z: Z) where A: Into<Value>, Z: Into<Value> {
		let a: Value = a.into();
		let b: Value = b.into();
		let z: Value = z.into();

		assert_eq!(input_manager.get_trigger_value_for_device(device, trigger_reference).unwrap(), a); // Assert default value

		input_manager.record_trigger_value_for_device(device, trigger_reference, b); // Record alternate value.

		input_manager.update();

		assert_eq!(input_manager.get_trigger_value_for_device(device, trigger_reference).unwrap(), b); // Assert alternate value after recording.

		input_manager.record_trigger_value_for_device(device, trigger_reference, a); // Record default value.

		input_manager.update();

		assert_eq!(input_manager.get_trigger_value_for_device(device, trigger_reference).unwrap(), a); // Assert default value after recording.

		input_manager.record_trigger_value_for_device(device, trigger_reference, a); // Record default value again.

		input_manager.update();

		assert_eq!(input_manager.get_trigger_value_for_device(device, trigger_reference).unwrap(), a); // Assert default value after recording.

		input_manager.record_trigger_value_for_device(device, trigger_reference, a); // Record default value.
		input_manager.record_trigger_value_for_device(device, trigger_reference, b); // Record alternate value after recording default value.

		input_manager.update();

		assert_eq!(input_manager.get_trigger_value_for_device(device, trigger_reference).unwrap(), b); // Assert value is last value recorded.

		input_manager.record_trigger_value_for_device(device, trigger_reference, z); // Record a different type.

		input_manager.update();

		assert_eq!(input_manager.get_trigger_value_for_device(device, trigger_reference).unwrap(), b); // Assert last value is kept after recording a different type.
	}

	#[test]
	fn record_bool_input_source_actions() {
		let mut input_manager = InputManager::new();

		let device_class_handle = declare_keyboard_input_device_class(&mut input_manager);

		let device = input_manager.create_device(&device_class_handle);

		let handle = TriggerReference::Name("Keyboard.Up");

		record_and_assert_input_source_action_sequence(&mut input_manager, device, handle, false, true, 961f32);
	}

	#[test]
	fn record_unicode_input_source_actions() {
		let mut input_manager = InputManager::new();

		let device_class_handle = declare_keyboard_input_device_class(&mut input_manager);

		let device = input_manager.create_device(&device_class_handle);

		let handle = TriggerReference::Name("Keyboard.Character");

		record_and_assert_input_source_action_sequence(&mut input_manager, device, handle, '\0', 'a', true);
	}

	#[test]
	fn record_int_input_source_actions() {
		let mut input_manager = InputManager::new();

		let device_class_handle = declare_funky_input_device_class(&mut input_manager);

		let device = input_manager.create_device(&device_class_handle);

		let handle = TriggerReference::Name("Funky.Int");

		record_and_assert_input_source_action_sequence(&mut input_manager, device, handle, 0, 1, true);
	}

	#[test]
	fn record_float_input_source_actions() {
		let mut input_manager = InputManager::new();

		let device_class_handle = declare_gamepad_input_device_class(&mut input_manager);

		let device = input_manager.create_device(&device_class_handle);

		let handle = TriggerReference::Name("Gamepad.LeftTrigger");

		record_and_assert_input_source_action_sequence(&mut input_manager, device, handle, 0.0f32, 1f32, true);
	}

	#[test]
	fn record_vector2_input_source_action() {
		let mut input_manager = InputManager::new();

		let device_class_handle = declare_gamepad_input_device_class(&mut input_manager);

		let device = input_manager.create_device(&device_class_handle);

		let handle = TriggerReference::Name("Gamepad.LeftStick");

		record_and_assert_input_source_action_sequence(&mut input_manager, device, handle, Vector2 { x: 0f32, y: 0f32, }, Vector2 { x: 1f32, y: 1f32, }, true);
	}

	#[test]
	fn record_vector3_input_source_actions() {
		let mut input_manager = InputManager::new();

		let device_class_handle = declare_vr_headset_input_device_class(&mut input_manager);

		let device = input_manager.create_device(&device_class_handle);

		let handle = TriggerReference::Name("Headset.Position");

		record_and_assert_input_source_action_sequence(&mut input_manager, device, handle, Vector3 { x: 0f32, y: 1.8f32, z: 0f32 }, Vector3 { x: 1f32, y: 1f32, z: 1f32 }, true);
	}

	#[test]
	fn record_quaternion_input_source_actions() {
		let mut input_manager = InputManager::new();

		let device_class_handle = declare_vr_headset_input_device_class(&mut input_manager);

		let device = input_manager.create_device(&device_class_handle);

		let handle = TriggerReference::Name("Headset.Orientation");

		record_and_assert_input_source_action_sequence(&mut input_manager, device, handle, Quaternion::from_euler_angles(0f32, 0f32, 0f32), Quaternion::from_euler_angles(1f32, 1f32, 1f32), true);
	}

	#[test]
	fn record_rgba_input_source_actions() {
		let mut input_manager = InputManager::new();

		let device_class_handle = declare_funky_input_device_class(&mut input_manager);

		let device = input_manager.create_device(&device_class_handle);

		let handle = TriggerReference::Name("Funky.Rgba");

		record_and_assert_input_source_action_sequence(&mut input_manager, device, handle, RGBA { r: 0f32, g: 0f32, b: 0f32, a: 0f32 }, RGBA { r: 1f32, g: 1f32, b: 1f32, a: 1f32 }, true);
	}

	fn record_and_assert_boolean_input_source_action_interpolation<T>(input_manager: &mut InputManager, device: DeviceHandle, handle: TriggerReference, action_name: &str, input_source_name: &'static str, a: T, b: T) where T: InputValue + Into<Value> + Copy {
		let action = input_manager.create_action(action_name, T::get_type(), &[ActionBindingDescription::new(input_source_name).mapped(b.into(), Function::Linear)]);

		assert_eq!(input_manager.get_action_state(action, device).value, a.into());
		
		input_manager.record_trigger_value_for_device(device, handle, true.into());
		
		input_manager.update();

		assert_eq!(input_manager.get_action_state(action, device).value, b.into());

		input_manager.record_trigger_value_for_device(device, handle, false.into());

		input_manager.update();

		assert_eq!(input_manager.get_action_state(action, device).value, a.into());
	}

	#[test]
	fn test_boolean_float_interpolation() {
		let mut input_manager = InputManager::new();

		let device_class_handle = declare_keyboard_input_device_class(&mut input_manager);

		let device = input_manager.create_device(&device_class_handle);

		let handle = TriggerReference::Name("Keyboard.Up");

		record_and_assert_boolean_input_source_action_interpolation(&mut input_manager, device, handle, "MoveForward", "Keyboard.Up", 0f32, 1f32);
	}

	#[test]
	fn test_boolean_vector2_interpolation() {
		let mut input_manager = InputManager::new();

		let device_class_handle = declare_keyboard_input_device_class(&mut input_manager);

		let device = input_manager.create_device(&device_class_handle);

		let handle = TriggerReference::Name("Keyboard.Up");

		record_and_assert_boolean_input_source_action_interpolation(&mut input_manager, device, handle, "MoveForward", "Keyboard.Up", Vector2::zero(), Vector2::new(0f32, 1f32));
	}

	#[test]
	fn test_boolean_vector3_interpolation() {
		let mut input_manager = InputManager::new();

		let device_class_handle = declare_keyboard_input_device_class(&mut input_manager);

		let device = input_manager.create_device(&device_class_handle);

		let handle = TriggerReference::Name("Keyboard.Up");

		record_and_assert_boolean_input_source_action_interpolation(&mut input_manager, device, handle, "MoveForward", "Keyboard.Up", Vector3::zero(), Vector3::new(0f32, 0f32, 1f32));
	}

	#[test]
	fn test_system_fps_game() {
		//! Test that the system integration is working and works correctly for a FPS game type setup.

		let space = spawn(Space::new());

		let input_manager: EntityHandle<InputManager> = spawn_as_child(space.clone(), InputManager::new_as_system());

		let mouse_device_handle;
		let keyboard_device_handle;
		let gamepad_device_handle;

		{
			let mut input_manager = input_manager.write();

			let mouse_device_class_handle = input_manager.register_device_class("Mouse");

			input_manager.register_trigger(&mouse_device_class_handle, "Position", TriggerDescription::new(Vector2::zero(), Vector2::zero(), Vector2::new(-1f32, -1f32), Vector2::new(1f32, 1f32)));
			input_manager.register_trigger(&mouse_device_class_handle, "LeftButton", TriggerDescription::new(false, false, false, true));
			input_manager.register_trigger(&mouse_device_class_handle, "RightButton", TriggerDescription::new(false, false, false, true));
			input_manager.register_trigger(&mouse_device_class_handle, "Scroll", TriggerDescription::new(0f32, 0f32, -1f32, 1f32));

			let keyboard_device_class_handle = input_manager.register_device_class("Keyboard");

			input_manager.register_trigger(&keyboard_device_class_handle, "W", TriggerDescription::new(false, false, false, true));
			input_manager.register_trigger(&keyboard_device_class_handle, "S", TriggerDescription::new(false, false, false, true));
			input_manager.register_trigger(&keyboard_device_class_handle, "A", TriggerDescription::new(false, false, false, true));
			input_manager.register_trigger(&keyboard_device_class_handle, "D", TriggerDescription::new(false, false, false, true));
			input_manager.register_trigger(&keyboard_device_class_handle, "Space", TriggerDescription::new(false, false, false, true));

			let gamepad_device_class_handle = input_manager.register_device_class("Gamepad");

			input_manager.register_trigger(&gamepad_device_class_handle, "LeftStick", TriggerDescription::new(Vector2::zero(), Vector2::zero(), Vector2 { x: -1.0, y: -1.0, }, Vector2 { x: 1.0, y: 1.0, }));
			input_manager.register_trigger(&gamepad_device_class_handle, "RightStick", TriggerDescription::new(Vector2::zero(), Vector2::zero(), Vector2 { x: -1.0, y: -1.0, }, Vector2 { x: 1.0, y: 1.0, }));

			mouse_device_handle = input_manager.create_device(&mouse_device_class_handle);
			keyboard_device_handle = input_manager.create_device(&keyboard_device_class_handle);
			gamepad_device_handle = input_manager.create_device(&gamepad_device_class_handle);
		}

		// Create the move action
		let move_action_handle = spawn_as_child(space.clone(), Action::<Vector3>::new("Move", &[
			ActionBindingDescription::new("Keyboard.W").mapped(Value::Vector3(Vector3::new(0f32, 0f32, 1f32)), Function::Linear),
			ActionBindingDescription::new("Keyboard.S").mapped(Value::Vector3(Vector3::new(0f32, 0f32, -1f32)), Function::Linear),
			ActionBindingDescription::new("Keyboard.A").mapped(Value::Vector3(Vector3::new(-1f32, 0f32, 0f32)), Function::Linear),
			ActionBindingDescription::new("Keyboard.D").mapped(Value::Vector3(Vector3::new(1f32, 0f32, 0f32)), Function::Linear),

			ActionBindingDescription::new("Gamepad.LeftStick").mapped(Value::Vector3(Vector3::new(1f32, 0f32, 1f32)), Function::Linear),
		],));

		let input_queue = Rc::new(RefCell::new(Vec::new()));

		{
			let input_queue = input_queue.clone();

			move_action_handle.write().value_mut().add(move |v| {
				input_queue.borrow_mut().push(*v);
			});
		}

		// Create the jump action
		let jump_action_handle = spawn_as_child(space.clone(), Action::<bool>::new("Jump", &[
			ActionBindingDescription::new("Keyboard.Space").mapped(Value::Bool(true), Function::Linear),
			ActionBindingDescription::new("Gamepad.A").mapped(Value::Bool(true), Function::Linear),
		],));

		{
			let input_queue = input_queue.clone();

			jump_action_handle.write().value_mut().add(move |v| {
				input_queue.borrow_mut().push(Vector3::new(0f32, 1f32, 0f32));
			});
		}

		{
			let mut input_manager = input_manager.write();

			input_manager.record_trigger_value_for_device(keyboard_device_handle, TriggerReference::Name("Keyboard.A"), Value::Bool(true));

			input_manager.update();
		}

		assert_eq!(input_queue.borrow().len(), 1);
		assert_eq!(input_queue.borrow()[0], Vector3::new(-1f32, 0f32, 0f32));
	}
}
