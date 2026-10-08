//! Lists HID services from the I/O Registry.
//!
//! hidapi creates an `IOHIDManager` that builds a device object for every HID service, then copies its
//! properties. This scan lets the kernel apply each requested usage as a matching dictionary and reads registry
//! properties of the matching services only. The dictionaries and property keys are built once per
//! `Scanner`, so a scan allocates only the property values IOKit copies out.

use std::ffi::{CStr, c_void};

use objc2_core_foundation::{CFDictionary, CFMutableDictionary, CFNumber, CFRetained, CFString, CFType};
use objc2_io_kit::{
	IOIteratorNext, IOObjectRelease, IORegistryEntryCreateCFProperty, IORegistryEntryGetRegistryEntryID,
	IOServiceGetMatchingServices, IOServiceMatching, io_object_t, kIOMainPortDefault,
};
use smallvec::SmallVec;

use super::{DeviceInfo, DevicePathRef, Usage};

/// `kCFStringEncodingUTF8`.
const UTF8: u32 = 0x0800_0100;

/// A device path is the service's registry entry ID.
pub(super) type PathData = u64;

pub(super) fn write_path(path: &PathData, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
	write!(f, "DevSrvsID:{path}")
}

pub(super) struct Scanner {
	/// Each usage with its matching dictionary, which every scan passes to IOKit again.
	usages: SmallVec<[(Usage, CFRetained<CFDictionary>); 4]>,
	/// The registry property names a scan reads, created once instead of per read.
	vendor_id: CFRetained<CFString>,
	product_id: CFRetained<CFString>,
	product: CFRetained<CFString>,
}

impl Scanner {
	pub(super) fn new(usages: &'static [Usage]) -> Self {
		Self {
			usages: usages.iter().map(|usage| (*usage, matching_dictionary(*usage))).collect(),
			vendor_id: CFString::from_static_str("VendorID"),
			product_id: CFString::from_static_str("ProductID"),
			product: CFString::from_static_str("Product"),
		}
	}

	/// Looks up the services of every usage and reports each service once, even when it declares several usages.
	pub(super) fn scan(&mut self, mut found: impl FnMut(DeviceInfo<'_>)) -> Result<(), String> {
		let mut seen = SmallVec::<[u64; 8]>::new();
		let mut name = [0u8; 512];

		for (usage, matching) in &self.usages {
			let services = matching_services(matching)?;
			while let Some(service) = services.next() {
				let mut entry_id = 0u64;
				// SAFETY: `service` is a live registry entry and `entry_id` is valid for writes.
				if unsafe { IORegistryEntryGetRegistryEntryID(service.0, &mut entry_id) } != 0 || seen.contains(&entry_id) {
					continue;
				}
				seen.push(entry_id);

				found(DeviceInfo {
					path: DevicePathRef(&entry_id),
					vendor_id: service.number(&self.vendor_id).unwrap_or(0) as u16,
					product_id: service.number(&self.product_id).unwrap_or(0) as u16,
					usage: *usage,
					product_name: service.string(&self.product, &mut name),
				});
			}
		}

		Ok(())
	}
}

/// Builds the `IOHIDDevice` matching dictionary for `usage`.
///
/// `DeviceUsagePage` and `DeviceUsage` match any usage pair a device declares, like `IOHIDManager` matching does.
fn matching_dictionary(usage: Usage) -> CFRetained<CFDictionary> {
	// SAFETY: the class name is a NUL-terminated string.
	let matching = unsafe { IOServiceMatching(c"IOHIDDevice".as_ptr()) }.expect(
		"Failed to create an IOHIDDevice matching dictionary. The most likely cause is that the process is out of memory.",
	);
	for (key, value) in [("DeviceUsagePage", usage.page), ("DeviceUsage", usage.usage)] {
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
	}
	CFRetained::<CFDictionary>::from(&*matching)
}

/// Asks the kernel for the services that satisfy a matching dictionary from [`matching_dictionary`].
fn matching_services(matching: &CFRetained<CFDictionary>) -> Result<Object, String> {
	let mut iterator: io_object_t = 0;
	// SAFETY: the call consumes the extra reference passed to it and writes the iterator, which `Object` releases.
	let result = unsafe { IOServiceGetMatchingServices(kIOMainPortDefault, Some(matching.clone()), &mut iterator) };
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

	fn property(&self, key: &CFString) -> Option<CFRetained<CFType>> {
		// SAFETY: `self.0` is a live registry entry; the default allocator is passed as `None`.
		unsafe { IORegistryEntryCreateCFProperty(self.0, Some(key), None, 0) }
	}

	fn number(&self, key: &CFString) -> Option<i32> {
		self.property(key)?.downcast_ref::<CFNumber>()?.as_i32()
	}

	/// Copies a string property into `buffer` as UTF-8 and returns it.
	fn string<'b>(&self, key: &CFString, buffer: &'b mut [u8]) -> Option<&'b str> {
		let property = self.property(key)?;
		let string = property.downcast_ref::<CFString>()?;
		// SAFETY: the buffer size passed is the length of `buffer`.
		let copied = unsafe { string.c_string(buffer.as_mut_ptr().cast(), buffer.len() as isize, UTF8) };
		if !copied {
			return None;
		}
		CStr::from_bytes_until_nul(buffer).ok()?.to_str().ok()
	}
}

impl Drop for Object {
	fn drop(&mut self) {
		IOObjectRelease(self.0);
	}
}
