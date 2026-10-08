//! Lists HID services from the I/O Registry.
//!
//! hidapi creates an `IOHIDManager` that builds a device object for every HID service, then copies its
//! properties. This scan lets the kernel apply each [`Match`] rule as a matching dictionary and reads registry
//! properties of the matching services only.

use std::ffi::c_void;

use objc2_core_foundation::{CFDictionary, CFMutableDictionary, CFNumber, CFRetained, CFString, CFType};
use objc2_io_kit::{
	IOIteratorNext, IOObjectRelease, IORegistryEntryCreateCFProperty, IORegistryEntryGetRegistryEntryID,
	IOServiceGetMatchingServices, IOServiceMatching, io_object_t, kIOMainPortDefault,
};

use super::{Device, DevicePath, Match};

pub(super) fn scan(matches: &[Match]) -> Result<Vec<Device>, String> {
	let mut devices = Vec::new();
	let mut seen = Vec::new();

	for rule in matches {
		let services = matching_services(*rule)?;
		while let Some(service) = services.next() {
			let mut entry_id = 0u64;
			// SAFETY: `service` is a live registry entry and `entry_id` is valid for writes.
			if unsafe { IORegistryEntryGetRegistryEntryID(service.0, &mut entry_id) } != 0 || seen.contains(&entry_id) {
				continue;
			}
			seen.push(entry_id);

			let (usage_page, usage) = match *rule {
				Match::Usage { page, usage } => (page, usage),
				Match::Vendor(_) => (
					service.number("PrimaryUsagePage").unwrap_or(0) as u16,
					service.number("PrimaryUsage").unwrap_or(0) as u16,
				),
			};
			devices.push(Device {
				path: DevicePath(format!("DevSrvsID:{entry_id}")),
				vendor_id: service.number("VendorID").unwrap_or(0) as u16,
				product_id: service.number("ProductID").unwrap_or(0) as u16,
				usage_page,
				usage,
				product_name: service.string("Product"),
			});
		}
	}

	Ok(devices)
}

/// Asks the kernel for the `IOHIDDevice` services that satisfy `rule`.
///
/// `DeviceUsagePage` and `DeviceUsage` match any usage pair a device declares, like `IOHIDManager` matching does.
fn matching_services(rule: Match) -> Result<Object, String> {
	// SAFETY: the class name is a NUL-terminated string.
	let Some(matching) = (unsafe { IOServiceMatching(c"IOHIDDevice".as_ptr()) }) else {
		return Err(
			"Failed to create an IOHIDDevice matching dictionary. The most likely cause is that IOKit is unavailable.".into(),
		);
	};
	let set = |key: &'static str, value: u16| {
		let key = CFString::from_static_str(key);
		let value = CFNumber::new_i32(value as i32);
		// SAFETY: the dictionary retains both CF objects, so they may be released after the call.
		unsafe {
			CFMutableDictionary::set_value(
				Some(&matching),
				(&*key as *const CFString).cast::<c_void>(),
				(&*value as *const CFNumber).cast::<c_void>(),
			)
		};
	};
	match rule {
		Match::Usage { page, usage } => {
			set("DeviceUsagePage", page);
			set("DeviceUsage", usage);
		}
		Match::Vendor(vendor) => set("VendorID", vendor),
	}

	let mut iterator: io_object_t = 0;
	// SAFETY: the call consumes the extra reference passed to it and writes the iterator, which `Object` releases.
	let result = unsafe {
		IOServiceGetMatchingServices(
			kIOMainPortDefault,
			Some(CFRetained::<CFDictionary>::from(&*matching)),
			&mut iterator,
		)
	};
	if result != 0 {
		return Err(format!(
			"Failed to look up HID services: kern_return_t {result}. The most likely cause is that IOKit is unavailable."
		));
	}
	Ok(Object(iterator))
}

/// The `Object` struct owns an IOKit object handle, such as a service or an iterator, releasing it when dropped.
struct Object(io_object_t);

impl Object {
	/// Returns the next object of an iterator, or `None` at its end.
	fn next(&self) -> Option<Object> {
		let next = IOIteratorNext(self.0);
		(next != 0).then_some(Object(next))
	}

	fn property(&self, key: &'static str) -> Option<CFRetained<CFType>> {
		let key = CFString::from_static_str(key);
		// SAFETY: `self.0` is a live registry entry; the default allocator is passed as `None`.
		unsafe { IORegistryEntryCreateCFProperty(self.0, Some(&key), None, 0) }
	}

	fn number(&self, key: &'static str) -> Option<i32> {
		self.property(key)?.downcast_ref::<CFNumber>()?.as_i32()
	}

	fn string(&self, key: &'static str) -> Option<String> {
		Some(self.property(key)?.downcast_ref::<CFString>()?.to_string())
	}
}

impl Drop for Object {
	fn drop(&mut self) {
		IOObjectRelease(self.0);
	}
}
