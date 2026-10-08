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

use super::{DeviceInfo, DevicePathRef, Match, find_match};

/// `HIDP_STATUS_SUCCESS`, which `HidP_GetCaps` returns on success.
const HIDP_STATUS_SUCCESS: i32 = 0x0011_0000;

/// The most UTF-16 units a product string can hold, the USB string descriptor limit.
const NAME_UNITS: usize = 126;
/// The UTF-8 size of the longest product string.
const NAME_CAPACITY: usize = NAME_UNITS * 3;

/// A device path is the interface path in UTF-16, with its NUL terminator so it can be opened directly.
pub(super) type Path = Box<[u16]>;
/// A borrowed device path is the interface path in UTF-16, without its NUL terminator.
pub(super) type PathRef<'a> = &'a [u16];

pub(super) fn path_ref(path: &Path) -> PathRef<'_> {
	&path[..path.len() - 1]
}

pub(super) fn owned_path(path: PathRef<'_>) -> Path {
	path.iter().copied().chain([0]).collect()
}

pub(super) fn write_path(path: PathRef<'_>, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
	use std::fmt::Write as _;
	for character in char::decode_utf16(path.iter().copied()) {
		f.write_char(character.unwrap_or(char::REPLACEMENT_CHARACTER))?;
	}
	Ok(())
}

pub(super) struct Scanner<'a> {
	matches: &'a [Match],
	guid: windows::core::GUID,
	/// The interface list of the last scan, kept so later scans reuse its capacity.
	list: Vec<u16>,
}

impl<'a> Scanner<'a> {
	pub(super) fn new(matches: &'a [Match]) -> Result<Self, String> {
		Ok(Self {
			matches,
			// SAFETY: `HidD_GetHidGuid` only writes the HID interface class GUID.
			guid: unsafe { HidD_GetHidGuid() },
			list: Vec::new(),
		})
	}

	/// Opens every present HID interface for queries and reports the ones whose attributes and usage match.
	pub(super) fn scan(&mut self, mut found: impl FnMut(DeviceInfo<'_>)) -> Result<(), String> {
		fill_interface_list(&self.guid, &mut self.list)?;
		let mut name = [0u8; NAME_CAPACITY];

		// The list holds NUL-terminated paths and ends with an empty one.
		for path in self.list.split(|&unit| unit == 0).take_while(|path| !path.is_empty()) {
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
			let Some((usage_page, usage)) = find_match(self.matches, vendor_id, std::iter::once(usages)) else {
				continue;
			};

			found(DeviceInfo {
				path: DevicePathRef(path, std::marker::PhantomData),
				vendor_id,
				product_id,
				usage_page,
				usage,
				product_name: interface.product_name(&mut name),
			});
		}

		Ok(())
	}
}

/// Fills `list` with the present interfaces of `guid` as NUL-terminated wide strings, reusing its capacity.
///
/// The list can grow between the size query and the copy, so a too-small buffer retries with the new size.
fn fill_interface_list(guid: &windows::core::GUID, list: &mut Vec<u16>) -> Result<(), String> {
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

		list.clear();
		list.resize(length as usize, 0);
		// SAFETY: `list` holds `length` units, the size the previous call reported.
		let result = unsafe { CM_Get_Device_Interface_ListW(guid, PCWSTR::null(), list, CM_GET_DEVICE_INTERFACE_LIST_PRESENT) };
		match result {
			CR_SUCCESS => return Ok(()),
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

	/// Decodes the product string into `buffer` and returns it, or `None` when the device reports none.
	fn product_name<'b>(&self, buffer: &'b mut [u8; NAME_CAPACITY]) -> Option<&'b str> {
		// The extra unit keeps a terminator after the longest name.
		let mut name = [0u16; NAME_UNITS + 1];
		// SAFETY: the buffer length passed is the size of `name` in bytes.
		let read = unsafe { HidD_GetProductString(self.0, name.as_mut_ptr().cast(), size_of_val(&name) as u32) };
		let units = name.iter().position(|&unit| unit == 0).unwrap_or(NAME_UNITS);
		if !read || units == 0 {
			return None;
		}

		// Each UTF-16 unit becomes at most three UTF-8 bytes, and a surrogate pair four, so the name fits in `buffer`.
		let mut length = 0;
		for character in char::decode_utf16(name[..units].iter().copied()) {
			let character = character.unwrap_or(char::REPLACEMENT_CHARACTER);
			length += character.encode_utf8(&mut buffer[length..]).len();
		}
		std::str::from_utf8(&buffer[..length]).ok()
	}
}

impl Drop for Interface {
	fn drop(&mut self) {
		// SAFETY: the handle came from `CreateFileW` and is closed exactly once.
		let _ = unsafe { CloseHandle(self.0) };
	}
}
