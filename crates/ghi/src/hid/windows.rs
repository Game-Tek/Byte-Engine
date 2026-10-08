//! Lists HID device interfaces through the Configuration Manager.
//!
//! hidapi opens every HID interface and reads its strings and device-tree properties. This scan opens each
//! interface without access rights, reads only its attributes and top-level usage, and reads the product string
//! only for matching interfaces.

use windows::{
	Win32::{
		Devices::{
			DeviceAndDriverInstallation::{
				CM_GET_DEVICE_INTERFACE_LIST_PRESENT, CM_Get_Device_Interface_List_SizeW, CM_Get_Device_Interface_ListW,
				CR_BUFFER_SMALL, CR_SUCCESS,
			},
			HumanInterfaceDevice::{
				HIDD_ATTRIBUTES, HIDP_CAPS, HidD_FreePreparsedData, HidD_GetAttributes, HidD_GetHidGuid, HidD_GetPreparsedData,
				HidD_GetProductString, HidP_GetCaps, PHIDP_PREPARSED_DATA,
			},
		},
		Foundation::{CloseHandle, HANDLE},
		Storage::FileSystem::{CreateFileW, FILE_FLAGS_AND_ATTRIBUTES, FILE_SHARE_READ, FILE_SHARE_WRITE, OPEN_EXISTING},
	},
	core::PCWSTR,
};

use super::{Device, DevicePath, Match, find_match};

/// `HIDP_STATUS_SUCCESS`, which `HidP_GetCaps` returns on success.
const HIDP_STATUS_SUCCESS: i32 = 0x0011_0000;

pub(super) fn scan(matches: &[Match]) -> Result<Vec<Device>, String> {
	// SAFETY: `HidD_GetHidGuid` only writes the HID interface class GUID.
	let guid = unsafe { HidD_GetHidGuid() };
	let list = interface_list(&guid)?;
	let mut devices = Vec::new();

	// The list holds NUL-terminated paths and ends with an empty one.
	for path in list.split(|&unit| unit == 0).take_while(|path| !path.is_empty()) {
		// SAFETY: `path` is followed by the NUL that `split` removed, so the pointer names a terminated string.
		let Some(interface) = Interface::open(PCWSTR(path.as_ptr())) else {
			continue;
		};
		let Some((vendor_id, product_id)) = interface.ids() else {
			continue;
		};
		let Some(usages) = interface.usage() else {
			continue;
		};
		let Some((usage_page, usage)) = find_match(matches, vendor_id, std::iter::once(usages)) else {
			continue;
		};

		devices.push(Device {
			path: DevicePath(String::from_utf16_lossy(path)),
			vendor_id,
			product_id,
			usage_page,
			usage,
			product_name: interface.product_name(),
		});
	}

	Ok(devices)
}

/// Returns the present interfaces of `guid` as one buffer of NUL-terminated wide strings.
///
/// The list can grow between the size query and the copy, so a too-small buffer retries with the new size.
fn interface_list(guid: &windows::core::GUID) -> Result<Vec<u16>, String> {
	loop {
		let mut length = 0u32;
		// SAFETY: `length` and `guid` are valid for the call, and a null device ID asks for every device.
		let result = unsafe {
			CM_Get_Device_Interface_List_SizeW(&mut length, guid, PCWSTR::null(), CM_GET_DEVICE_INTERFACE_LIST_PRESENT)
		};
		if result != CR_SUCCESS {
			return Err(format!(
				"Failed to size the HID interface list: CONFIGRET {}. The most likely cause is that the Configuration Manager is unavailable.",
				result.0
			));
		}

		let mut list = vec![0u16; length as usize];
		// SAFETY: `list` holds `length` units, the size the previous call reported.
		let result =
			unsafe { CM_Get_Device_Interface_ListW(guid, PCWSTR::null(), &mut list, CM_GET_DEVICE_INTERFACE_LIST_PRESENT) };
		match result {
			CR_SUCCESS => return Ok(list),
			CR_BUFFER_SMALL => continue,
			result => {
				return Err(format!(
					"Failed to list HID interfaces: CONFIGRET {}. The most likely cause is that the Configuration Manager is unavailable.",
					result.0
				));
			}
		}
	}
}

/// The `Interface` struct owns a HID interface handle opened only for queries, closing it when dropped.
struct Interface(HANDLE);

impl Interface {
	/// Opens `path` without read or write access, which is enough for attribute queries and never conflicts with
	/// applications that hold the device.
	fn open(path: PCWSTR) -> Option<Self> {
		// SAFETY: `path` is a NUL-terminated interface path and no security attributes or template are passed.
		let handle = unsafe {
			CreateFileW(
				path,
				0,
				FILE_SHARE_READ | FILE_SHARE_WRITE,
				None,
				OPEN_EXISTING,
				FILE_FLAGS_AND_ATTRIBUTES(0),
				None,
			)
		};
		handle.ok().map(Self)
	}

	fn ids(&self) -> Option<(u16, u16)> {
		let mut attributes = HIDD_ATTRIBUTES {
			Size: size_of::<HIDD_ATTRIBUTES>() as u32,
			..Default::default()
		};
		// SAFETY: `self.0` is an open HID handle and `attributes` declares its size.
		unsafe { HidD_GetAttributes(self.0, &mut attributes) }.then_some((attributes.VendorID, attributes.ProductID))
	}

	/// Returns the usage page and usage of the interface's top-level collection.
	fn usage(&self) -> Option<(u16, u16)> {
		let mut preparsed = PHIDP_PREPARSED_DATA::default();
		// SAFETY: `self.0` is an open HID handle; the data is freed below.
		if !unsafe { HidD_GetPreparsedData(self.0, &mut preparsed) } {
			return None;
		}
		let mut caps = HIDP_CAPS::default();
		// SAFETY: `preparsed` came from `HidD_GetPreparsedData` and has not been freed.
		let status = unsafe { HidP_GetCaps(preparsed, &mut caps) };
		// SAFETY: `preparsed` is freed once and not used afterwards.
		unsafe { HidD_FreePreparsedData(preparsed) };
		(status.0 == HIDP_STATUS_SUCCESS).then_some((caps.UsagePage, caps.Usage))
	}

	fn product_name(&self) -> Option<String> {
		// USB string descriptors hold at most 126 UTF-16 units; the rest of the buffer keeps a terminator.
		let mut name = [0u16; 128];
		// SAFETY: the buffer length passed is the size of `name` in bytes.
		let read = unsafe { HidD_GetProductString(self.0, name.as_mut_ptr().cast(), size_of_val(&name) as u32) };
		let length = name.iter().position(|&unit| unit == 0).unwrap_or(name.len());
		(read && length > 0).then(|| String::from_utf16_lossy(&name[..length]))
	}
}

impl Drop for Interface {
	fn drop(&mut self) {
		// SAFETY: the handle came from `CreateFileW` and is closed exactly once.
		let _ = unsafe { CloseHandle(self.0) };
	}
}
