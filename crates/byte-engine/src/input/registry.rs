//! Registered input layouts: device classes, their triggers, and concrete devices.
//!
//! The registry is plain data with name resolution. It knows nothing about
//! queued values or sinks, so [`InputCollector`](super::InputCollector) owns one
//! to validate records, and [`InputSink`](super::InputSink) reads it once to
//! resolve binding names into handles.

use std::alloc::Allocator;

use log::warn;

use super::device::{Device, DeviceClass, DeviceClassHandle};
use super::gesture::{Gate, TriggerMapping};
use super::trigger::{Trigger, TriggerDescription, TriggerReference};
use super::{ActionBindingDescription, DeviceHandle, InputValue, TriggerHandle, TriggerMode, Types, Value};

/// The `Registry` struct keeps the declared input layout separate from the values it produces.
pub(super) struct Registry<A: Allocator + Clone> {
	device_classes: Vec<DeviceClass<A>, A>,
	triggers: Vec<Trigger<A>, A>,
	devices: Vec<Device, A>,
}

impl<A: Allocator + Clone> Registry<A> {
	pub(super) fn new_in(allocator: A) -> Self {
		Self {
			device_classes: Vec::new_in(allocator.clone()),
			triggers: Vec::new_in(allocator.clone()),
			devices: Vec::new_in(allocator),
		}
	}

	pub(super) fn register_device_class(&mut self, name: &str) -> DeviceClassHandle {
		debug_assert!(
			self.device_classes.len() < u32::MAX as usize,
			"Device-class handle space is exhausted. The most likely cause is registering classes continuously instead of reusing them."
		);

		let handle = DeviceClassHandle(self.device_classes.len() as u32);
		self.device_classes.push(DeviceClass {
			name: Box::clone_from_ref_in(name, self.device_classes.allocator().clone()),
		});
		handle
	}

	pub(super) fn register_trigger<T: InputValue + Into<Value>>(
		&mut self,
		device_class: &DeviceClassHandle,
		name: &str,
		description: TriggerDescription<T>,
	) -> TriggerHandle {
		let default: Value = description.default.into();
		let default_value_type: Types = default.into();

		assert_eq!(
			default_value_type,
			T::get_type(),
			"Default value type does not match input source type"
		);
		debug_assert!(
			(device_class.0 as usize) < self.device_classes.len(),
			"Trigger device class is unknown. The most likely cause is using a handle from another input pipeline."
		);
		debug_assert!(
			self.triggers.len() < u32::MAX as usize,
			"Trigger handle space is exhausted. The most likely cause is registering triggers continuously instead of reusing them."
		);

		let handle = TriggerHandle(self.triggers.len() as u32);
		// Names resolve by one scan of `Class.Trigger` strings, without splitting the query.
		let full_name = format!("{}.{name}", self.device_classes[device_class.0 as usize].name);
		self.triggers.push(Trigger {
			device_class_handle: *device_class,
			name: Box::clone_from_ref_in(full_name.as_str(), self.triggers.allocator().clone()),
			r#type: T::get_type(),
			default,
			transient: description.transient,
		});
		handle
	}

	pub(super) fn create_device(&mut self, device_class: &DeviceClassHandle) -> DeviceHandle {
		debug_assert!(
			(device_class.0 as usize) < self.device_classes.len(),
			"Device class is unknown. The most likely cause is using a handle from another input pipeline."
		);
		debug_assert!(
			self.devices.len() < u32::MAX as usize,
			"Device handle space is exhausted. The most likely cause is creating devices without retiring old state."
		);

		let handle = DeviceHandle(self.devices.len() as u32);
		self.devices.push(Device {
			device_class_handle: *device_class,
		});
		handle
	}

	/// Returns every device of the named class, in creation order.
	pub(super) fn devices_by_class_name(&self, class_name: &str) -> Option<impl Iterator<Item = DeviceHandle> + '_> {
		let class = self.class_by_name(class_name)?;

		Some(
			self.devices
				.iter()
				.enumerate()
				.filter_map(move |(index, device)| (device.device_class_handle == class).then_some(DeviceHandle(index as u32))),
		)
	}

	/// Resolves a trigger by handle or by its `DeviceClass.Trigger` name.
	pub(super) fn resolve(&self, reference: &TriggerReference) -> Option<(TriggerHandle, &Trigger<A>)> {
		let handle = match reference {
			TriggerReference::Handle(handle) => *handle,
			TriggerReference::Name(name) => {
				let index = self.triggers.iter().position(|trigger| trigger.name.as_ref() == *name)?;
				TriggerHandle(index as u32)
			}
		};

		self.triggers.get(handle.0 as usize).map(|trigger| (handle, trigger))
	}

	/// Resolves one binding's value source and its optional gating button.
	///
	/// Returns `None` for an unknown name, or for a button that is not a retained
	/// boolean control of the value source's own device class.
	pub(super) fn resolve_binding(&self, binding: &ActionBindingDescription) -> Option<TriggerMapping> {
		let (source, source_trigger) = self.resolve(&binding.input_source)?;
		let gate = match &binding.trigger {
			Some(reference) => {
				let (button, gate) = self.resolve(reference)?;
				if gate.r#type != Types::Boolean
					|| gate.device_class_handle != source_trigger.device_class_handle
					|| (binding.trigger_mode == TriggerMode::Drag && gate.transient)
				{
					warn!(
						"Input binding is invalid. The trigger must be boolean, share the value source's device class, and be retained for drags."
					);
					return None;
				}
				match binding.trigger_mode {
					TriggerMode::Press => Gate::Press(button),
					TriggerMode::Release => Gate::Release(button),
					TriggerMode::Drag => Gate::Drag(button),
				}
			}
			None => Gate::None,
		};

		Some(TriggerMapping {
			source,
			gate,
			mapping: binding.mapping,
		})
	}

	pub(super) fn device_count(&self) -> u32 {
		self.devices.len() as u32
	}

	pub(super) fn trigger_count(&self) -> u32 {
		self.triggers.len() as u32
	}

	fn class_by_name(&self, name: &str) -> Option<DeviceClassHandle> {
		self.device_classes
			.iter()
			.position(|class| class.name.as_ref() == name)
			.map(|index| DeviceClassHandle(index as u32))
	}
}
