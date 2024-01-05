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

use std::{f32::consts::PI, collections::HashMap};

use log::warn;

use crate::{RGBA, Vector2, Vector3, insert_return_length, Quaternion, orchestrator::{EntityHandle, System, self, Entity, EntitySubscriber, EntityHash, Event, EventImplementation, AsyncEventImplementation}};

/// A device class represents a type of device. Such as a keyboard, mouse, or gamepad.
/// It can have associated input sources, such as the UP key on a keyboard or the left trigger on a gamepad.
struct DeviceClass {
	/// The name of the device class.
	name: String,
}

#[derive(Copy, Clone)]
pub struct InputSourceDescription<T> {
	/// The value the input source will have when it's first registered and no events have been recorded for it.
	default: T,
	/// The value the input source will have when it's released.
	rest: T,
	/// The minimum value the input source can have.
	min: T,
	/// The maximum value the input source can have.
	max: T,
}

impl <T> InputSourceDescription<T> {
	pub fn new(default: T, rest: T, min: T, max: T) -> Self {
		InputSourceDescription {
			default,
			rest,
			min,
			max,
		}
	}
}

/// An input source is a source of input on a device class/type. Such as the UP key on a keyboard or the left trigger on a gamepad.
struct InputSource {
	/// The device class the input source is associated with.
	device_class_handle: DeviceClassHandle,
	/// The name of the input source.
	name: String,
	/// The type of the input source.
	type_: InputTypes,
}

#[derive(Copy, Clone, Debug)]
/// An input source action is a way to reference an input source.
/// It can be referenced by it's name or by it's handle.
/// It's provided as a convenience to the developer.
pub enum InputSourceAction {
	/// Refer to the input source by it's handle.
	Handle(InputSourceHandle),
	/// Refer to the input source by it's name.
	Name(&'static str)
}

#[derive(Copy, Clone)]
pub enum InputTypes {
	Bool(InputSourceDescription<bool>),
	Unicode(InputSourceDescription<char>),
	Float(InputSourceDescription<f32>),
	Int(InputSourceDescription<i32>),
	Rgba(InputSourceDescription<RGBA>),
	Vector2(InputSourceDescription<Vector2>),
	Vector3(InputSourceDescription<Vector3>),
	Quaternion(InputSourceDescription<Quaternion>),
}

#[derive(Debug, Copy, Clone, PartialEq)]
/// A simple "typeless" container for several underlying types.
/// Can be used to store any of these types, but will be usually used to traffic record and input event values.
pub enum Value {
	/// A boolean value.
	Bool(bool),
	/// A unicode character.
	Unicode(char),
	/// A floating point value.
	Float(f32),
	/// An integer value.
	Int(i32),
	/// An RGBA color value.
	Rgba(RGBA),
	/// A 2D point value.
	Vector2(Vector2),
	/// A 3D point value.
	Vector3(Vector3),
	/// A quaternion.
	Quaternion(Quaternion),
}

#[derive(Copy, Clone, Debug)]
/// Enumerates the different functions that can be applied to an input event.
pub enum Function {
	Boolean,
	Threshold,
	Linear,
	/// Maps a 2D point to a 3D point on a sphere.
	Sphere,
}

/// An action binding description is a description of how an input source is mapped to a value for an action.
#[derive(Copy, Clone, Debug)]
pub struct ActionBindingDescription {
	pub input_source: InputSourceAction,
	pub mapping: Value,
	pub function: Option<Function>
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

#[derive(Copy, Clone, PartialEq)]
pub struct Record {
	input_source_handle: InputSourceHandle,
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

struct InputSourceMapping {
	input_source_handle: InputSourceHandle,
	mapping: Value,
	function: Option<Function>,
}

/// Enumerates the different types of types of values the input manager can handle.
#[derive(Copy, Clone)]
pub enum Types {
	/// A boolean value.
	Bool,
	/// A unicode character.
	Unicode,
	/// A floating point value.
	Float,
	/// An integer value.
	Int,
	/// A 2D point value.
	Vector2,
	/// A 3D point value.
	Vector3,
	/// A quaternion.
	Quaternion,
	/// An RGBA color value.
	Rgba,
}

enum TypedHandle {
	Bool(EntityHandle<Action<bool>>),
	Vector2(EntityHandle<Action<Vector2>>),
	Vector3(EntityHandle<Action<Vector3>>),
}

/// An input event is an application specific event that is triggered by a combination of input sources.
struct InputAction {
	name: String,
	type_: Types,
	input_event_descriptions: Vec<InputSourceMapping>,
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
	handle: TypedHandle,
}

/// A device represents a particular instance of a device class. Such as the current keyboard, or a specific gamepad.
/// This is useful for when you want to have multiple devices of the same type. Such as multiple gamepads(player 0, player 1, etc).
struct Device {
	device_class_handle: DeviceClassHandle,
	index: u32,
	input_source_states: std::collections::HashMap<InputSourceHandle, InputSourceState>,
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

#[derive(Copy, Clone, PartialEq, Eq)]
/// Handle to an input device class.
pub struct DeviceClassHandle(u32);

#[derive(Copy, Clone, PartialEq, Eq, Hash, Debug)]
/// Handle to an input source.
pub struct InputSourceHandle(u32);

#[derive(Clone, PartialEq, Eq, Debug)]
/// Handle to an device.
pub struct DeviceHandle(u32);

#[derive(Copy, Clone, PartialEq, Eq, Hash)]
/// Handle to an input event.
pub struct ActionHandle(u32);

/// The input manager is responsible for managing input devices and input events.
pub struct InputManager {
	device_classes: Vec<DeviceClass>,
	input_sources: Vec<InputSource>,
	devices: Vec<Device>,
	records: Vec<Record>,
	actions: Vec<InputAction>,
}

impl InputManager {
	/// Creates a new input manager.
	pub fn new() -> Self {
		InputManager {
			device_classes: Vec::new(),
			input_sources: Vec::new(),
			devices: Vec::new(),
			records: Vec::new(),
			actions: Vec::new(),
		}
	}

	pub fn new_as_system() -> orchestrator::EntityReturn<'static, InputManager> {
		orchestrator::EntityReturn::new(Self::new())
			.add_listener::<Action<bool>>()
			.add_listener::<Action<Vector2>>()
			.add_listener::<Action<Vector3>>()
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
	/// # use byte_engine::input_manager::InputManager;
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
	/// # use byte_engine::input_manager::{InputManager, InputTypes, InputSourceDescription};
	/// # let mut input_manager = InputManager::new();
	/// # let keyboard_device_class_handle = input_manager.register_device_class("Keyboard");
	/// input_manager.register_input_source(&keyboard_device_class_handle, "Up", InputTypes::Bool(InputSourceDescription::new(false, false, false, true)));
	/// ```
	pub fn register_input_source(&mut self, device_handle: &DeviceClassHandle, name: &str, value_type: InputTypes) -> InputSourceHandle {
		let input_source = InputSource {
			device_class_handle: *device_handle,
			name: name.to_string(),
			type_: value_type,
		};

		InputSourceHandle(insert_return_length(&mut self.input_sources, input_source) as u32)
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
	/// # use byte_engine::input_manager::{InputManager, InputTypes, InputSourceDescription};
	/// # let mut input_manager = InputManager::new();
	/// # let gamepad_device_class_handle = input_manager.register_device_class("Gamepad");
	/// input_manager.register_input_destination(&gamepad_device_class_handle, "Rumble", InputTypes::Float(InputSourceDescription::new(0f32, 0f32, 0f32, 1f32)));
	/// ```
	pub fn register_input_destination(&mut self, _device_class_handle: &DeviceClassHandle, _name: &str, _value_type: InputTypes) -> InputSourceHandle {
		InputSourceHandle(0)
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
	/// # use byte_engine::input_manager::InputManager;
	/// # let mut input_manager = InputManager::new();
	/// # let keyboard_device_class_handle = input_manager.register_device_class("Keyboard");
	/// let keyboard_device = input_manager.create_device(&keyboard_device_class_handle);
	/// ```
	pub fn create_device(&mut self, device_class_handle: &DeviceClassHandle) -> DeviceHandle {
		let _device_class = &self.device_classes[device_class_handle.0 as usize];

		let other_device = self.devices.iter().filter(|d| d.device_class_handle.0 == device_class_handle.0).min_by_key(|d| d.index);

		let index = match other_device {
			Some(device) => device.index + 1,
			None => 0,
		};

		let input_source_handles = self.input_sources.iter().enumerate().filter(|i| i.1.device_class_handle == *device_class_handle).map(|i| InputSourceHandle(i.0 as u32)).collect::<Vec<_>>();

		let mut input_source_states = std::collections::HashMap::with_capacity(input_source_handles.len());

		for input_source_handle in input_source_handles {
			let input_source = &self.input_sources[input_source_handle.0 as usize];

			let input_source_state = InputSourceState { 
				value: match input_source.type_ {
					InputTypes::Bool(description) => Value::Bool(description.default),
					InputTypes::Unicode(description) => Value::Unicode(description.default),
					InputTypes::Float(description) => Value::Float(description.default),
					InputTypes::Int(description) => Value::Int(description.default),
					InputTypes::Rgba(description) => Value::Rgba(description.default),
					InputTypes::Vector2(description) => Value::Vector2(description.default),
					InputTypes::Vector3(description) => Value::Vector3(description.default),
					InputTypes::Quaternion(description) => Value::Quaternion(description.default),
				},
				time: std::time::SystemTime::now(),
			};

			input_source_states.insert(input_source_handle, input_source_state);
		}

		let device = Device {
			device_class_handle: device_class_handle.clone(),
			index,
			input_source_states,
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

	/// Records an input source action.
	/// 
	/// One example is the UP key on a keyboard being pressed.
	pub fn record_input_source_action(&mut self, device_handle: &DeviceHandle, input_source_action: InputSourceAction, value: Value) {
		let input_source = if let Some(input_source) = self.get_input_source_from_input_source_action(&input_source_action) {
			input_source
		} else {
			warn!("Tried to record an input source action that doesn't exist");
			return;
		};

		let matches = match input_source.type_ {
			InputTypes::Bool(_) => std::mem::discriminant(&value) == std::mem::discriminant(&Value::Bool(false)),
			InputTypes::Unicode(_) => std::mem::discriminant(&value) == std::mem::discriminant(&Value::Unicode('\0')),
			InputTypes::Float(_) => std::mem::discriminant(&value) == std::mem::discriminant(&Value::Float(0f32)),
			InputTypes::Int(_) => std::mem::discriminant(&value) == std::mem::discriminant(&Value::Int(0)),
			InputTypes::Rgba(_) => std::mem::discriminant(&value) == std::mem::discriminant(&Value::Rgba(RGBA { r: 0f32, g: 0f32, b: 0f32, a: 0f32 })),
			InputTypes::Vector2(_) => std::mem::discriminant(&value) == std::mem::discriminant(&Value::Vector2(Vector2 { x: 0f32, y: 0f32 })),
			InputTypes::Vector3(_) => std::mem::discriminant(&value) == std::mem::discriminant(&Value::Vector3(Vector3 { x: 0f32, y: 0f32, z: 0f32 })),
			InputTypes::Quaternion(_) => std::mem::discriminant(&value) == std::mem::discriminant(&Value::Quaternion(Quaternion::identity())),
		};

		if !matches {
			warn!("Tried to record an extraneous type into input source: {}", input_source.name);
			return;
		} // Value type does not match input source declared type, so don't record.

		let input_source_handle = if let Some(input_source_handle) = self.to_input_source_handle(&input_source_action) {
			input_source_handle
		} else {
			warn!("Tried to record an input source action that doesn't exist");
			return;
		};

		let device = &mut self.devices[device_handle.0 as usize];

		let input_source_state = device.input_source_states.get_mut(&input_source_handle);

		if input_source_state.is_none() {
			// TODO: log warning and handle gracefully.
			panic!("Input source state not found!");
		}

		let input_source_state = input_source_state.unwrap();

		let time = std::time::SystemTime::now();

		input_source_state.value = value;
		input_source_state.time = time;

		let record = Record {
			input_source_handle,
			value,
			time,
		};

		{
			let mut i = 0;
			while i < self.records.len() {
				if self.records[i].input_source_handle == input_source_handle {
					self.records.remove(i);
				} else {
					i += 1;
				}
			}
		}

		self.records.push(record);

		if let Value::Bool(boo) = value {
			let input_events = self.actions.iter_mut().filter(|ia| ia.input_event_descriptions.iter().any(|ied| ied.input_source_handle == input_source_handle));
	
			if boo {
				for input_event in input_events {
					input_event.stack.push(record);
				}
			} else {
				for input_event in input_events {
					input_event.stack.retain(|r| r.input_source_handle != input_source_handle);
				}
			}
		}
	}

	pub fn update(&mut self) {
		let records = &self.records;

		if records.is_empty() { return; }

		for (i, action) in self.actions.iter().enumerate() {
			let action_records = records.iter().filter(|r| action.input_event_descriptions.iter().any(|ied| ied.input_source_handle == r.input_source_handle));

			let most_recent_record = action_records.max_by_key(|r| r.time);

			if let Some(record) = most_recent_record {
				let value = self.resolve_action_value_from_record(action, record).unwrap_or(Value::Bool(false));

				match value {
					Value::Bool(v) => {
						match &action.handle {
							TypedHandle::Bool(handle) => { handle.map(|a| { let a = a.read_sync(); a.events.iter().for_each(|f| f.fire(&v)) }); }
							_ => {}
						}
					}
					Value::Vector2(v) => {
						match &action.handle {
							TypedHandle::Vector2(handle) => { handle.map(|a| { let a = a.read_sync(); a.events.iter().for_each(|f| f.fire(&v)) }); }
							_ => {}
						}
					}
					Value::Vector3(v) => {
						match &action.handle {
							TypedHandle::Vector3(handle) => { handle.map(|a| { let a = a.read_sync(); a.events.iter().for_each(|f| f.fire(&v)) }); }
							_ => {}
						}
					}
					_ => {
						log::error!("Not implemented!");
					}
				}
			}
		}

		self.records.clear();
	}

	/// Gets the input source action from the input source action.
	pub fn get_input_source_record(&self, device_handle: &DeviceHandle, input_source_action: InputSourceAction) -> Record {
		let _input_source = self.get_input_source_from_input_source_action(&input_source_action);

		let device = self.get_device(device_handle);
		let state = &device.input_source_states[&self.to_input_source_handle(&input_source_action).unwrap()];

		Record {
			input_source_handle: InputSourceHandle(0),
			value: state.value,
			time: state.time
		}
	}

	pub fn get_input_source_value(&self, device_handle: &DeviceHandle, input_source_action: InputSourceAction) -> Value {
		let _input_source = self.get_input_source_from_input_source_action(&input_source_action);

		let device = self.get_device(device_handle);
		let state = &device.input_source_states[&self.to_input_source_handle(&input_source_action).unwrap()];

		state.value
	}

	pub fn get_input_source_values(&self, input_source_action: InputSourceAction) -> Vec<InputSourceEventState> {
		let _input_source = self.get_input_source_from_input_source_action(&input_source_action);

		self.devices.iter().enumerate().map(|(i, _device)| {
			let device_handle = DeviceHandle(i as u32);

			InputSourceEventState {
				value: self.get_input_source_value(&device_handle, input_source_action),
				device_handle,
			}
		}).collect::<Vec<_>>()
	}

	/// Gets the value of an input event.
	pub fn get_action_state(&self, input_event_handle: ActionHandle, device_handle: &DeviceHandle) -> InputEventState {
		let action = &self.actions[input_event_handle.0 as usize];
		let (r#type, input_event_descriptions) = (action.type_, &action.input_event_descriptions);

		if let Some(record) = self.records.iter().filter(|r| input_event_descriptions.iter().any(|ied| ied.input_source_handle == r.input_source_handle)).max_by_key(|r| r.time) {
			let value = self.resolve_action_value_from_record(action, record).unwrap_or(Value::Bool(false));
	
			InputEventState {
				device_handle: device_handle.clone(),
				input_event_handle,
				value,
			}
		} else {
			let value = match self.input_sources[input_event_descriptions[0].input_source_handle.0 as usize].type_ {
				InputTypes::Bool(_v) => {
					match r#type {
						Types::Bool => {
							Value::Bool(false)
						}
						Types::Float => {
							Value::Float(0.0f32)
						}
						_ => panic!()
					}
				}
				InputTypes::Float(v) => Value::Float(v.default),
				InputTypes::Vector2(v) => Value::Vector2(v.default),
				_ => panic!()
			};

			InputEventState {
				device_handle: device_handle.clone(),
				input_event_handle,
				value,
			}
		}
	}

	fn resolve_action_value_from_record(&self, action: &InputAction, record: &Record) -> Option<Value> {
		let mapping = action.input_event_descriptions.iter().find(|ied| ied.input_source_handle == record.input_source_handle).unwrap();

		let value = match action.type_ {
			Types::Bool => {
				match record.value {
					Value::Bool(record_value) => {
						Value::Bool(record_value)
					}
					_ => {
						log::error!("Not implemented!");
						return None;
					},
				}
			}
			Types::Float => {
				let float = match record.value {
					Value::Bool(record_value) => {
						if let Some(last) = action.stack.last() {
							if let Value::Bool(_value) = last.value {
								let event_description_for_input_source = action.input_event_descriptions.iter().find(|description| description.input_source_handle == last.input_source_handle).unwrap();
		
								match event_description_for_input_source.mapping {
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
						}
					}
					_ => {
						log::error!("Not implemented!");
						return None;
					},
				};

				Value::Float(float)
			}
			Types::Vector3 => {
				match record.value {
					Value::Bool(_) => {
						log::error!("Not implemented!");
						return None;
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

								Value::Vector3(Vector3 { x, y, z } * transformation)
							} else {
								log::error!("Not implemented!");
								return None;
							}
						} else {
							Value::Vector3(Vector3 { x: record_value.x, y: record_value.y, z: 0f32 })
						}
					},
					Value::Vector3(value) => Value::Vector3(value),
					Value::Quaternion(_) => {
						log::error!("Not implemented!");
						return None;
					},
				}
			}
			_ => {
				log::error!("Not implemented!");
				return None;
			},
		};

		Some(value)
	}

	fn get_input_source_from_input_source_action(&self, input_source_action: &InputSourceAction) -> Option<&InputSource> {
		self.to_input_source_handle(input_source_action).and_then(|input_source_handle| self.input_sources.get(input_source_handle.0 as usize))
	}

	fn get_device(&self, device_handle: &DeviceHandle) -> &Device {
		&self.devices[device_handle.0 as usize]
	}

	fn to_input_source_handle(&self, input_source_action: &InputSourceAction) -> Option<InputSourceHandle> { // TODO: return Option
		match input_source_action {
			InputSourceAction::Handle(handle) => Some(*handle),
			InputSourceAction::Name(name) => {
				let tokens = (*name).split('.');

				let input_device_class = self.device_classes.iter().enumerate().find(|(_, device_class)| device_class.name == tokens.clone().next().unwrap());

				if let Some(_) = input_device_class {
					let input_source = self.input_sources.iter().enumerate().find(|(_, input_source)| input_source.name == tokens.clone().last().unwrap());

					if let Some(input_source) = input_source {
						Some(InputSourceHandle(input_source.0 as u32))
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

impl EntitySubscriber<Action<bool>> for InputManager {
	async fn on_create<'a>(&'a mut self, orchestrator: orchestrator::OrchestratorReference, handle: EntityHandle<Action<bool>>, action: &Action<bool>) {
		let (name, r#type, input_events,) = (action.name, Types::Bool, &action.bindings);

		let input_event = InputAction {
			name: name.to_string(),
			type_: r#type,
			input_event_descriptions: input_events.iter().map(|input_event| {
				Some(InputSourceMapping {
					input_source_handle: self.to_input_source_handle(&input_event.input_source)?,
					mapping: input_event.mapping,
					function: input_event.function,
				})
			}).filter_map(|input_event| input_event).collect::<Vec<_>>(),
			stack: Vec::new(),
			handle: TypedHandle::Bool(handle.clone()),
		};

		self.actions.push(input_event);
	}

	async fn on_update(&'static mut self, orchestrator: orchestrator::OrchestratorReference, handle: EntityHandle<Action<bool>>, params: &Action<bool>) {
	}
}

impl EntitySubscriber<Action<Vector2>> for InputManager {
	async fn on_create<'a>(&'a mut self, orchestrator: orchestrator::OrchestratorReference, handle: EntityHandle<Action<Vector2>>, action: &Action<maths_rs::vec::Vec2<f32>>) {
		// self.create_action(action.name, Types::Vector2, &action.bindings);
	}

	async fn on_update(&'static mut self, orchestrator: orchestrator::OrchestratorReference, handle: EntityHandle<Action<Vector2>>, params: &Action<Vector2>) {
	}
}

impl EntitySubscriber<Action<Vector3>> for InputManager {
	async fn on_create<'a>(&'a mut self, orchestrator: orchestrator::OrchestratorReference, handle: EntityHandle<Action<Vector3>>, action: &Action<maths_rs::vec::Vec3<f32>>) {
		// let internal_handle = self.create_action(action.name, Types::Vector3, &action.bindings);

		// self.actions_ie_map.insert(internal_handle, handle);
	}

	async fn on_update(&'static mut self, orchestrator: orchestrator::OrchestratorReference, handle: EntityHandle<Action<Vector3>>, params: &Action<Vector3>) {
	}
}

#[cfg(test)]
mod tests {
	use maths_rs::prelude::Base; // TODO: remove, make own

	use super::*;

	fn declare_keyboard_input_device_class(input_manager: &mut InputManager) -> DeviceClassHandle {
		let device_class_handle = input_manager.register_device_class("Keyboard");

		let key_source_description = InputTypes::Bool(InputSourceDescription::new(false, false, false, true));

		let _up_input_source = input_manager.register_input_source(&device_class_handle, "Up", key_source_description);
		let _down_input_source = input_manager.register_input_source(&device_class_handle, "Down", key_source_description);
		let _left_input_source = input_manager.register_input_source(&device_class_handle, "Left", key_source_description);
		let _right_input_source = input_manager.register_input_source(&device_class_handle, "Right", key_source_description);

		let key_source_description = InputTypes::Unicode(InputSourceDescription::new('\0', '\0', '\0', 'Z'));

		let _a_input_source = input_manager.register_input_source(&device_class_handle, "Character", key_source_description);

		device_class_handle
	}

	fn declare_gamepad_input_device_class(input_manager: &mut InputManager) -> DeviceClassHandle {
		let device_class_handle = input_manager.register_device_class("Gamepad");

		let key_source_description = InputTypes::Float(InputSourceDescription::new(0.0f32, 0.0f32, 0.0f32, 1.0f32));

		let _up_input_source = input_manager.register_input_source(&device_class_handle, "LeftTrigger", key_source_description);
		let _down_input_source = input_manager.register_input_source(&device_class_handle, "RighTrigger", key_source_description);

		let key_source_description = InputTypes::Vector2(InputSourceDescription::new(Vector2::zero(), Vector2::zero(), Vector2 { x: -1.0, y: -1.0, }, Vector2 { x: 1.0, y: 1.0, }));

		let _a_input_source = input_manager.register_input_source(&device_class_handle, "LeftStick", key_source_description);
		let _b_input_source = input_manager.register_input_source(&device_class_handle, "RightStick", key_source_description);

		let light_source_description = InputTypes::Rgba(InputSourceDescription::new(RGBA { r: 0.0f32, g: 0.0f32, b: 0.0f32, a: 0.0f32 }, RGBA { r: 0.0f32, g: 0.0f32, b: 0.0f32, a: 0.0f32 }, RGBA { r: 0.0f32, g: 0.0f32, b: 0.0f32, a: 0.0f32 }, RGBA { r: 1.0f32, g: 1.0f32, b: 1.0f32, a: 1.0f32 }));

		let _light_destination = input_manager.register_input_destination(&device_class_handle, "Light", light_source_description);

		device_class_handle
	}

	fn declare_vr_headset_input_device_class(input_manager: &mut InputManager) -> DeviceClassHandle {
		let device_class_handle = input_manager.register_device_class("Headset");

		let source_description = InputTypes::Vector3(InputSourceDescription::new(Vector3::new(0f32, 1.80f32, 0f32), Vector3::new(0f32, 0f32, 0f32), Vector3::min_value(), Vector3::max_value()));

		let _position_input_source = input_manager.register_input_source(&device_class_handle, "Position", source_description);

		let source_description = InputTypes::Quaternion(InputSourceDescription::new(Quaternion::identity(), Quaternion::identity(), Quaternion::identity(), Quaternion::identity()));

		let _rotation_input_source = input_manager.register_input_source(&device_class_handle, "Orientation", source_description);

		device_class_handle
	}

	fn declare_funky_input_device_class(input_manager: &mut InputManager) -> DeviceClassHandle {
		let device_class_handle = input_manager.register_device_class("Funky");

		let source_description = InputTypes::Int(InputSourceDescription::new(0, 0, 0, 3));

		let _funky_input_source = input_manager.register_input_source(&device_class_handle, "Int", source_description);

		let source_description = InputTypes::Rgba(InputSourceDescription::new(RGBA { r: 0.0f32, g: 0.0f32, b: 0.0f32, a: 0.0f32 }, RGBA { r: 0.0f32, g: 0.0f32, b: 0.0f32, a: 0.0f32 }, RGBA { r: 0.0f32, g: 0.0f32, b: 0.0f32, a: 0.0f32 }, RGBA { r: 1.0f32, g: 1.0f32, b: 1.0f32, a: 1.0f32 }));

		input_manager.register_input_source(&device_class_handle, "Rgba", source_description);

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

		let stick_source_description = InputTypes::Vector2(InputSourceDescription::new(Vector2::zero(), Vector2::zero(), Vector2 { x: -1.0, y: -1.0, }, Vector2 { x: 1.0, y: 1.0, }));

		let _gamepad_left_stick_input_source = input_manager.register_input_source(&gamepad_class_handle, "LeftStick", stick_source_description);
		let _gamepad_right_stick_input_source = input_manager.register_input_source(&gamepad_class_handle, "RightStick", stick_source_description);

		let trigger_source_description = InputTypes::Float(InputSourceDescription { default: 0.0, rest: 0.0, min: 0.0, max: 1.0 });

		let _trigger_input_source = input_manager.register_input_source(&gamepad_class_handle, "LeftTrigger", trigger_source_description);
	}

	#[test]
	fn record_bool_input_source_actions() {
		let mut input_manager = InputManager::new();

		let device_class_handle = declare_keyboard_input_device_class(&mut input_manager);

		let device = input_manager.create_device(&device_class_handle);

		let handle = InputSourceAction::Name("Keyboard.Up");

		assert_eq!(input_manager.get_input_source_value(&device, handle), Value::Bool(false)); // Must be false by default(declared in "declare_keyboard_input_device_class").

		input_manager.record_input_source_action(&device, handle, Value::Bool(true));

		assert_eq!(input_manager.get_input_source_value(&device, handle), Value::Bool(true)); // Must be true after recording.

		input_manager.record_input_source_action(&device, handle, Value::Bool(false));

		assert_eq!(input_manager.get_input_source_value(&device, handle), Value::Bool(false)); // Must be false after recording.

		input_manager.record_input_source_action(&device, handle, Value::Bool(false));

		assert_eq!(input_manager.get_input_source_value(&device, handle), Value::Bool(false)); // Must be false after recording.

		input_manager.record_input_source_action(&device, handle, Value::Bool(false));
		input_manager.record_input_source_action(&device, handle, Value::Bool(true));

		assert_eq!(input_manager.get_input_source_value(&device, handle), Value::Bool(true)); // Must be true after recording multiple times without querying the value.

		input_manager.record_input_source_action(&device, handle, Value::Float(98f32));

		assert_eq!(input_manager.get_input_source_value(&device, handle), Value::Bool(true)); // Must keep the previous value if the type is different.
	}

	#[test]
	fn record_unicode_input_source_actions() {
		let mut input_manager = InputManager::new();

		let device_class_handle = declare_keyboard_input_device_class(&mut input_manager);

		let device = input_manager.create_device(&device_class_handle);

		let handle = InputSourceAction::Name("Keyboard.Character");

		assert_eq!(input_manager.get_input_source_value(&device, handle), Value::Unicode('\0')); // Must be false by default(declared in "declare_keyboard_input_device_class").

		input_manager.record_input_source_action(&device, handle, Value::Unicode('a'));

		assert_eq!(input_manager.get_input_source_value(&device, handle), Value::Unicode('a')); // Must be true after recording.

		input_manager.record_input_source_action(&device, handle, Value::Unicode('\0'));

		assert_eq!(input_manager.get_input_source_value(&device, handle), Value::Unicode('\0')); // Must be false after recording.

		input_manager.record_input_source_action(&device, handle, Value::Unicode('\0'));

		assert_eq!(input_manager.get_input_source_value(&device, handle), Value::Unicode('\0')); // Must be false after recording.

		input_manager.record_input_source_action(&device, handle, Value::Unicode('\0'));
		input_manager.record_input_source_action(&device, handle, Value::Unicode('a'));

		assert_eq!(input_manager.get_input_source_value(&device, handle), Value::Unicode('a')); // Must be true after recording multiple times without querying the value.

		input_manager.record_input_source_action(&device, handle, Value::Bool(true));

		assert_eq!(input_manager.get_input_source_value(&device, handle), Value::Unicode('a')); // Must keep the previous value if the type is different.
	}

	#[test]
	fn record_int_input_source_actions() {
		let mut input_manager = InputManager::new();

		let device_class_handle = declare_funky_input_device_class(&mut input_manager);

		let device = input_manager.create_device(&device_class_handle);

		let handle = InputSourceAction::Name("Funky.Int");

		assert_eq!(input_manager.get_input_source_value(&device, handle), Value::Int(0)); // Must be false by default(declared in "declare_keyboard_input_device_class").

		input_manager.record_input_source_action(&device, handle, Value::Int(1));

		assert_eq!(input_manager.get_input_source_value(&device, handle), Value::Int(1)); // Must be true after recording.

		input_manager.record_input_source_action(&device, handle, Value::Int(0));

		assert_eq!(input_manager.get_input_source_value(&device, handle), Value::Int(0)); // Must be false after recording.

		input_manager.record_input_source_action(&device, handle, Value::Int(0));

		assert_eq!(input_manager.get_input_source_value(&device, handle), Value::Int(0)); // Must be false after recording.

		input_manager.record_input_source_action(&device, handle, Value::Int(0));
		input_manager.record_input_source_action(&device, handle, Value::Int(1));

		assert_eq!(input_manager.get_input_source_value(&device, handle), Value::Int(1)); // Must be true after recording multiple times without querying the value.

		input_manager.record_input_source_action(&device, handle, Value::Bool(true));

		assert_eq!(input_manager.get_input_source_value(&device, handle), Value::Int(1)); // Must keep the previous value if the type is different.
	}

	#[test]
	fn record_float_input_source_actions() {
		let mut input_manager = InputManager::new();

		let device_class_handle = declare_gamepad_input_device_class(&mut input_manager);

		let device = input_manager.create_device(&device_class_handle);

		let handle = InputSourceAction::Name("Gamepad.LeftTrigger");

		assert_eq!(input_manager.get_input_source_value(&device, handle), Value::Float(0.0f32)); // Must be false by default(declared in "declare_gamepad_input_device_class").

		input_manager.record_input_source_action(&device, handle, Value::Float(1f32));

		assert_eq!(input_manager.get_input_source_value(&device, handle), Value::Float(1f32)); // Must be true after recording.

		input_manager.record_input_source_action(&device, handle, Value::Float(0f32));

		assert_eq!(input_manager.get_input_source_value(&device, handle), Value::Float(0f32)); // Must be false after recording.

		input_manager.record_input_source_action(&device, handle, Value::Float(0f32));

		assert_eq!(input_manager.get_input_source_value(&device, handle), Value::Float(0f32)); // Must be false after recording.

		input_manager.record_input_source_action(&device, handle, Value::Float(0f32));
		input_manager.record_input_source_action(&device, handle, Value::Float(1f32));

		assert_eq!(input_manager.get_input_source_value(&device, handle), Value::Float(1f32)); // Must be true after recording multiple times without querying the value.

		input_manager.record_input_source_action(&device, handle, Value::Bool(true));

		assert_eq!(input_manager.get_input_source_value(&device, handle), Value::Float(1f32)); // Must keep the previous value if the type is different.
	}

	#[test]
	fn record_vector2_input_source_action() {
		let mut input_manager = InputManager::new();

		let device_class_handle = declare_gamepad_input_device_class(&mut input_manager);

		let device = input_manager.create_device(&device_class_handle);

		let handle = InputSourceAction::Name("Gamepad.LeftStick");

		assert_eq!(input_manager.get_input_source_value(&device, handle), Value::Vector2(Vector2 { x: 0f32, y: 0f32, })); // Must be false by default(declared in "declare_gamepad_input_device_class").

		input_manager.record_input_source_action(&device, handle, Value::Vector2(Vector2 { x: 1f32, y: 1f32, }));

		assert_eq!(input_manager.get_input_source_value(&device, handle), Value::Vector2(Vector2 { x: 1f32, y: 1f32, })); // Must be true after recording.

		input_manager.record_input_source_action(&device, handle, Value::Vector2(Vector2 { x: 0f32, y: 0f32, }));

		assert_eq!(input_manager.get_input_source_value(&device, handle), Value::Vector2(Vector2 { x: 0f32, y: 0f32, })); // Must be false after recording.

		input_manager.record_input_source_action(&device, handle, Value::Vector2(Vector2 { x: 0f32, y: 0f32, }));

		assert_eq!(input_manager.get_input_source_value(&device, handle), Value::Vector2(Vector2 { x: 0f32, y: 0f32, })); // Must be false after recording.

		input_manager.record_input_source_action(&device, handle, Value::Vector2(Vector2 { x: 0f32, y: 0f32, }));
		input_manager.record_input_source_action(&device, handle, Value::Vector2(Vector2 { x: 1f32, y: 1f32, }));

		assert_eq!(input_manager.get_input_source_value(&device, handle), Value::Vector2(Vector2 { x: 1f32, y: 1f32, })); // Must be true after recording multiple times without querying the value.

		input_manager.record_input_source_action(&device, handle, Value::Bool(true));

		assert_eq!(input_manager.get_input_source_value(&device, handle), Value::Vector2(Vector2 { x: 1f32, y: 1f32, })); // Must keep the previous value if the type is different.
	}

	#[test]
	fn record_vector3_input_source_actions() {
		let mut input_manager = InputManager::new();

		let device_class_handle = declare_vr_headset_input_device_class(&mut input_manager);

		let device = input_manager.create_device(&device_class_handle);

		let handle = InputSourceAction::Name("Headset.Position");

		assert_eq!(input_manager.get_input_source_value(&device, handle), Value::Vector3(Vector3 { x: 0f32, y: 1.8f32, z: 0f32 }));

		input_manager.record_input_source_action(&device, handle, Value::Vector3(Vector3 { x: 1f32, y: 1f32, z: 1f32 }));

		assert_eq!(input_manager.get_input_source_value(&device, handle), Value::Vector3(Vector3 { x: 1f32, y: 1f32, z: 1f32 })); // Must be true after recording.

		input_manager.record_input_source_action(&device, handle, Value::Vector3(Vector3 { x: 0f32, y: 0f32, z: 0f32 }));

		assert_eq!(input_manager.get_input_source_value(&device, handle), Value::Vector3(Vector3 { x: 0f32, y: 0f32, z: 0f32 })); // Must be false after recording.

		input_manager.record_input_source_action(&device, handle, Value::Vector3(Vector3 { x: 0f32, y: 0f32, z: 0f32 }));

		assert_eq!(input_manager.get_input_source_value(&device, handle), Value::Vector3(Vector3 { x: 0f32, y: 0f32, z: 0f32 })); // Must be false after recording.

		input_manager.record_input_source_action(&device, handle, Value::Vector3(Vector3 { x: 0f32, y: 0f32, z: 0f32 }));
		input_manager.record_input_source_action(&device, handle, Value::Vector3(Vector3 { x: 1f32, y: 1f32, z: 1f32 }));

		assert_eq!(input_manager.get_input_source_value(&device, handle), Value::Vector3(Vector3 { x: 1f32, y: 1f32, z: 1f32 })); // Must be true

		input_manager.record_input_source_action(&device, handle, Value::Bool(true));

		assert_eq!(input_manager.get_input_source_value(&device, handle), Value::Vector3(Vector3 { x: 1f32, y: 1f32, z: 1f32 })); // Must keep the previous value if the type is different.
	}

	#[test]
	fn record_quaternion_input_source_actions() {
		let mut input_manager = InputManager::new();

		let device_class_handle = declare_vr_headset_input_device_class(&mut input_manager);

		let device = input_manager.create_device(&device_class_handle);

		let handle = InputSourceAction::Name("Headset.Orientation");

		assert_eq!(input_manager.get_input_source_value(&device, handle), Value::Quaternion(Quaternion::from_euler_angles(0f32, 0f32, 0f32)));

		input_manager.record_input_source_action(&device, handle, Value::Quaternion(Quaternion::from_euler_angles(1f32, 1f32, 1f32)));

		assert_eq!(input_manager.get_input_source_value(&device, handle), Value::Quaternion(Quaternion::from_euler_angles(1f32, 1f32, 1f32))); // Must be true after recording.

		input_manager.record_input_source_action(&device, handle, Value::Quaternion(Quaternion::from_euler_angles(0f32, 0f32, 0f32)));

		assert_eq!(input_manager.get_input_source_value(&device, handle), Value::Quaternion(Quaternion::from_euler_angles(0f32, 0f32, 0f32))); // Must be false after recording.

		input_manager.record_input_source_action(&device, handle, Value::Quaternion(Quaternion::from_euler_angles(0f32, 0f32, 0f32)));

		assert_eq!(input_manager.get_input_source_value(&device, handle), Value::Quaternion(Quaternion::from_euler_angles(0f32, 0f32, 0f32))); // Must be false after recording.

		input_manager.record_input_source_action(&device, handle, Value::Quaternion(Quaternion::from_euler_angles(0f32, 0f32, 0f32)));
		input_manager.record_input_source_action(&device, handle, Value::Quaternion(Quaternion::from_euler_angles(1f32, 1f32, 1f32)));

		assert_eq!(input_manager.get_input_source_value(&device, handle), Value::Quaternion(Quaternion::from_euler_angles(1f32, 1f32, 1f32))); // Must be true

		input_manager.record_input_source_action(&device, handle, Value::Bool(true));

		assert_eq!(input_manager.get_input_source_value(&device, handle), Value::Quaternion(Quaternion::from_euler_angles(1f32, 1f32, 1f32))); // Must keep the previous value if the type is different.
	}

	#[test]
	fn record_rgba_input_source_actions() {
		let mut input_manager = InputManager::new();

		let device_class_handle = declare_funky_input_device_class(&mut input_manager);

		let device = input_manager.create_device(&device_class_handle);

		let handle = InputSourceAction::Name("Funky.Rgba");

		assert_eq!(input_manager.get_input_source_value(&device, handle), Value::Rgba(RGBA { r: 0f32, g: 0f32, b: 0f32, a: 0f32 }));

		input_manager.record_input_source_action(&device, handle, Value::Rgba(RGBA { r: 1f32, g: 1f32, b: 1f32, a: 1f32 }));

		assert_eq!(input_manager.get_input_source_value(&device, handle), Value::Rgba(RGBA { r: 1f32, g: 1f32, b: 1f32, a: 1f32 })); // Must be true after recording.

		input_manager.record_input_source_action(&device, handle, Value::Rgba(RGBA { r: 0f32, g: 0f32, b: 0f32, a: 0f32 }));

		assert_eq!(input_manager.get_input_source_value(&device, handle), Value::Rgba(RGBA { r: 0f32, g: 0f32, b: 0f32, a: 0f32 })); // Must be false after recording.

		input_manager.record_input_source_action(&device, handle, Value::Rgba(RGBA { r: 0f32, g: 0f32, b: 0f32, a: 0f32 }));

		assert_eq!(input_manager.get_input_source_value(&device, handle), Value::Rgba(RGBA { r: 0f32, g: 0f32, b: 0f32, a: 0f32 })); // Must be false after recording.

		input_manager.record_input_source_action(&device, handle, Value::Rgba(RGBA { r: 0f32, g: 0f32, b: 0f32, a: 0f32 }));
		input_manager.record_input_source_action(&device, handle, Value::Rgba(RGBA { r: 1f32, g: 1f32, b: 1f32, a: 1f32 }));

		assert_eq!(input_manager.get_input_source_value(&device, handle), Value::Rgba(RGBA { r: 1f32, g: 1f32, b: 1f32, a: 1f32 })); // Must be true

		input_manager.record_input_source_action(&device, handle, Value::Bool(true));

		assert_eq!(input_manager.get_input_source_value(&device, handle), Value::Rgba(RGBA { r: 1f32, g: 1f32, b: 1f32, a: 1f32 })); // Must keep the previous value if the type is different.
	}

	// #[test]
	// fn create_input_events() {
	// 	let mut input_manager = InputManager::new();

	// 	declare_keyboard_input_device_class(&mut input_manager);

	// 	input_manager.create_action("MoveLongitudinally", Types::Float, &[
	// 		ActionBindingDescription {
	// 			input_source: InputSourceAction::Name("Keyboard.Up"),
	// 			mapping: Value::Float(1.0),
	// 			function: Some(Function::Linear),
	// 		},
	// 		ActionBindingDescription {
	// 			input_source: InputSourceAction::Name("Keyboard.Down"),
	// 			mapping: Value::Float(-1.0),
	// 			function: Some(Function::Linear),
	// 		},]);
	// }

	// #[test]
	// fn get_float_input_event_from_bool_input_source() {
	// 	let mut input_manager = InputManager::new();

	// 	let device_class_handle = declare_keyboard_input_device_class(&mut input_manager);

	// 	let input_event = input_manager.create_action("MoveLongitudinally", Types::Float, &[
	// 		ActionBindingDescription {
	// 			input_source: InputSourceAction::Name("Keyboard.Up"),
	// 			mapping: Value::Float(1.0),
	// 			function: Some(Function::Linear),
	// 		},
	// 		ActionBindingDescription {
	// 			input_source: InputSourceAction::Name("Keyboard.Down"),
	// 			mapping: Value::Float(-1.0),
	// 			function: Some(Function::Linear),
	// 		},]);

	// 	let device_handle = input_manager.create_device(&device_class_handle);

	// 	let value = input_manager.get_action_state(input_event, &device_handle);

	// 	assert_eq!(value.device_handle, device_handle);
	// 	assert_eq!(value.value, Value::Float(0f32)); // Default value must be 0.

	// 	input_manager.record_input_source_action(&device_handle, InputSourceAction::Name("Keyboard.Up"), Value::Bool(true));

	// 	let value = input_manager.get_action_state(input_event, &device_handle);

	// 	assert_eq!(value.value, Value::Float(1.0)); // Must be 1.0 after recording.

	// 	input_manager.record_input_source_action(&device_handle, InputSourceAction::Name("Keyboard.Up"), Value::Bool(false));

	// 	let value = input_manager.get_action_state(input_event, &device_handle);

	// 	assert_eq!(value.value, Value::Float(0.0)); // Must be 0.0 after recording.

	// 	input_manager.record_input_source_action(&device_handle, InputSourceAction::Name("Keyboard.Down"), Value::Bool(true));

	// 	let value = input_manager.get_action_state(input_event, &device_handle);

	// 	assert_eq!(value.value, Value::Float(-1.0)); // Must be -1.0 after recording.

	// 	input_manager.record_input_source_action(&device_handle, InputSourceAction::Name("Keyboard.Down"), Value::Bool(false));

	// 	let value = input_manager.get_action_state(input_event, &device_handle);

	// 	assert_eq!(value.value, Value::Float(0.0)); // Must be 0.0 after recording.

	// 	input_manager.record_input_source_action(&device_handle, InputSourceAction::Name("Keyboard.Up"), Value::Bool(true));
	// 	input_manager.record_input_source_action(&device_handle, InputSourceAction::Name("Keyboard.Down"), Value::Bool(true));

	// 	let value = input_manager.get_action_state(input_event, &device_handle);

	// 	assert_eq!(value.value, Value::Float(-1.0)); // Must be -1.0 after recording down after up while up is still pressed.

	// 	input_manager.record_input_source_action(&device_handle, InputSourceAction::Name("Keyboard.Up"), Value::Bool(false));
	// 	input_manager.record_input_source_action(&device_handle, InputSourceAction::Name("Keyboard.Down"), Value::Bool(false));

	// 	let value = input_manager.get_action_state(input_event, &device_handle);

	// 	assert_eq!(value.value, Value::Float(0.0)); // Must be 0.0 after recording

	// 	input_manager.record_input_source_action(&device_handle, InputSourceAction::Name("Keyboard.Up"), Value::Bool(true));
	// 	input_manager.record_input_source_action(&device_handle, InputSourceAction::Name("Keyboard.Down"), Value::Bool(true));
	// 	input_manager.record_input_source_action(&device_handle, InputSourceAction::Name("Keyboard.Up"), Value::Bool(false));

	// 	let value = input_manager.get_action_state(input_event, &device_handle);

	// 	assert_eq!(value.value, Value::Float(-1.0)); // Must be -1.0 after releasing up while down down is still pressed.

	// 	input_manager.record_input_source_action(&device_handle, InputSourceAction::Name("Keyboard.Up"), Value::Bool(true));
	// 	input_manager.record_input_source_action(&device_handle, InputSourceAction::Name("Keyboard.Down"), Value::Bool(true));
	// 	input_manager.record_input_source_action(&device_handle, InputSourceAction::Name("Keyboard.Down"), Value::Bool(false));

	// 	let value = input_manager.get_action_state(input_event, &device_handle);

	// 	assert_eq!(value.value, Value::Float(1.0)); // Must be 1.0 after releasing down while up is still pressed.
	// }
}

pub trait Extract<T: InputValue> {
	fn extract(&self) -> T;
}

impl Extract<bool> for Value {
	fn extract(&self) -> bool {
		match self {
			Value::Bool(value) => *value,
			_ => panic!("Wrong type")
		}
	}
}

impl Extract<Vector2> for Value {
	fn extract(&self) -> Vector2 {
		match self {
			Value::Vector2(value) => *value,
			_ => panic!("Wrong type")
		}
	}
}

impl Extract<Vector3> for Value {
	fn extract(&self) -> Vector3 {
		match self {
			Value::Vector3(value) => *value,
			_ => panic!("Wrong type")
		}
	}
}

impl InputManager {
	pub fn get_action_value<T: InputValue + Clone + 'static>(&self, action_handle: &EntityHandle<Action<T>>) -> T where Value: Extract<T> {
		let state = self.get_action_state(ActionHandle(action_handle.get_external_key()), &DeviceHandle(0));
		state.value.extract()
	}

	pub fn set_action_value<T: InputValue + Clone + 'static>(&mut self, _action_handle: &EntityHandle<Action<T>>, _value: T) {

	}
}

impl Entity for InputManager {}
impl System for InputManager {}

trait ActionLike: Entity {
	fn get_bindings(&self) -> &[ActionBindingDescription];
	fn get_inputs(&self) -> &[InputSourceMapping];
}

pub struct Action<T: InputValue> {
	pub name: &'static str,
	pub bindings: Vec<ActionBindingDescription>,
	inputs: Vec<InputSourceMapping>,
	pub value: T,

	pub events: Vec<Box<dyn Event<T>>>,
}

impl <T: InputValue> orchestrator::Entity for Action<T> {}

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

impl InputValue for Vector2 {
	fn get_type() -> Types { Types::Vector2 }
}

impl InputValue for Vector3 {
	fn get_type() -> Types { Types::Vector3 }
}

impl <T: Clone + InputValue + 'static> orchestrator::Component for Action<T> {
	// type Parameters<'a> = ActionParameters<'a>;
}

impl <T: InputValue + Clone + 'static> Action<T> {
	pub fn new(name: &'static str, bindings: &[ActionBindingDescription]) -> Action<T> {
		Action {
			name,
			bindings: bindings.to_vec(),
			value: T::default(),
			inputs: Vec::new(),

			events: Vec::new(),
		}
	}

	pub fn get_value(&self) -> T { self.value }
	pub fn set_value(&mut self, value: T) { self.value = value; }
	pub const fn value() -> orchestrator::EventDescription<Action<T>, T> { return orchestrator::EventDescription::new() }

	pub fn subscribe<E: Entity>(&mut self, subscriber: &EntityHandle<E>, endpoint: fn(&mut E, &T)) {
		self.events.push(Box::new(EventImplementation::new(subscriber.clone(), endpoint)));
	}

	// pub fn subscribe_async<E: Entity, R>(&mut self, subscriber: &EntityHandle<E>, endpoint: fn(&mut E, &T) -> R) where R: std::future::Future<Output = ()> {
	// 	AsyncEventImplementation::new(subscriber.clone(), endpoint);
	// }
}