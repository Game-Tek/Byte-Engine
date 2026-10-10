//! Lists HID device interfaces through the Configuration Manager.
//!
//! hidapi opens every HID interface and reads its strings and device-tree properties. This scan opens each
//! interface without access rights, reads only its top-level usage and IDs, and reads the product string
//! only for interfaces with a requested usage.

use windows::{
	Win32::{
		Devices::{
			DeviceAndDriverInstallation::{
				CM_GET_DEVICE_INTERFACE_LIST_PRESENT, CM_Get_Device_Interface_List_SizeW, CM_Get_Device_Interface_ListW,
				CM_NOTIFY_ACTION, CM_NOTIFY_EVENT_DATA, CM_NOTIFY_FILTER, CM_NOTIFY_FILTER_0, CM_NOTIFY_FILTER_0_0,
				CM_NOTIFY_FILTER_TYPE_DEVICEINTERFACE, CM_Register_Notification, CM_Unregister_Notification, CR_BUFFER_SMALL,
				CR_SUCCESS, HCMNOTIFICATION,
			},
			HumanInterfaceDevice::{
				HIDD_ATTRIBUTES, HIDP_CAPS, HidD_FreePreparsedData, HidD_GetAttributes, HidD_GetHidGuid, HidD_GetPreparsedData,
				HidD_GetProductString, HidP_GetCaps, PHIDP_PREPARSED_DATA,
			},
		},
		Foundation::{CloseHandle, ERROR_IO_INCOMPLETE, ERROR_IO_PENDING, GENERIC_READ, GENERIC_WRITE, HANDLE},
		Storage::FileSystem::{
			CreateFileW, FILE_FLAG_OVERLAPPED, FILE_FLAGS_AND_ATTRIBUTES, FILE_SHARE_READ, FILE_SHARE_WRITE, OPEN_EXISTING,
			ReadFile,
		},
		System::{
			IO::{CancelIoEx, GetOverlappedResult, OVERLAPPED},
			Threading::CreateEventW,
		},
	},
	core::PCWSTR,
};

use super::{ChangeSignal, DeviceInfo, DevicePathRef, Usage};

/// `HIDP_STATUS_SUCCESS`, which `HidP_GetCaps` returns on success.
const HIDP_STATUS_SUCCESS: i32 = 0x0011_0000;

/// The most UTF-16 units a product string can hold, the USB string descriptor limit.
const NAME_UNITS: usize = 126;
/// The UTF-8 size of the longest product string.
const NAME_CAPACITY: usize = NAME_UNITS * 3;

/// A device path is the interface path in UTF-16, without a NUL terminator.
pub(super) type PathData = [u16];

pub(super) fn write_path(path: &PathData, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
	use std::fmt::Write as _;
	for character in char::decode_utf16(path.iter().copied()) {
		f.write_char(character.unwrap_or(char::REPLACEMENT_CHARACTER))?;
	}
	Ok(())
}

pub(super) struct Scanner {
	usages: &'static [Usage],
	/// The interface list of the last scan, kept so later scans reuse its capacity.
	list: Vec<u16>,
}

impl Scanner {
	pub(super) fn new(usages: &'static [Usage]) -> Self {
		Self {
			usages,
			list: Vec::new(),
		}
	}

	/// Opens every present HID interface for queries and reports the ones whose usage matches.
	pub(super) fn scan(&mut self, mut found: impl FnMut(DeviceInfo<'_>)) -> Result<(), String> {
		fill_interface_list(&mut self.list)?;
		let mut name = [0u8; NAME_CAPACITY];

		// The list holds NUL-terminated paths and ends with an empty one.
		for path in self.list.split(|&unit| unit == 0).take_while(|path| !path.is_empty()) {
			// SAFETY: `path` is followed by the NUL that `split` removed, so the pointer names a terminated string.
			let Some(interface) = Interface::open(PCWSTR(path.as_ptr()), 0, FILE_FLAGS_AND_ATTRIBUTES(0)) else {
				continue;
			};
			let Some(usage) = interface.usage().filter(|usage| self.usages.contains(usage)) else {
				continue;
			};
			let Some((vendor_id, product_id)) = interface.ids() else {
				continue;
			};

			found(DeviceInfo {
				path: DevicePathRef(path),
				vendor_id,
				product_id,
				usage,
				product_name: interface.product_name(&mut name),
			});
		}

		Ok(())
	}
}

/// The `Monitor` struct holds a Configuration Manager subscription to HID interface arrivals and removals.
///
/// Windows runs the callback on a thread pool thread, which records the change and wakes the application loop.
pub(crate) struct Monitor {
	signal: std::sync::Arc<ChangeSignal>,
	registration: HCMNOTIFICATION,
}

impl Monitor {
	pub(super) fn new() -> Result<Self, String> {
		let signal = std::sync::Arc::new(ChangeSignal::default());
		let filter = CM_NOTIFY_FILTER {
			cbSize: size_of::<CM_NOTIFY_FILTER>() as u32,
			FilterType: CM_NOTIFY_FILTER_TYPE_DEVICEINTERFACE,
			u: CM_NOTIFY_FILTER_0 {
				DeviceInterface: CM_NOTIFY_FILTER_0_0 {
					// SAFETY: `HidD_GetHidGuid` only writes the HID interface class GUID.
					ClassGuid: unsafe { HidD_GetHidGuid() },
				},
			},
			..Default::default()
		};
		let mut registration = HCMNOTIFICATION::default();
		// SAFETY: the context points at `signal`, which `Monitor` keeps alive until `Drop` unregisters the callback.
		let result = unsafe {
			CM_Register_Notification(
				&filter,
				Some(std::sync::Arc::as_ptr(&signal).cast()),
				Some(on_interface_change),
				&mut registration,
			)
		};
		if result != CR_SUCCESS {
			return Err(format!(
				"Failed to subscribe to HID device changes: CONFIGRET {}. The most likely cause is that the Configuration Manager is unavailable.",
				result.0
			));
		}
		Ok(Self { signal, registration })
	}

	pub(super) fn take_changed(&mut self) -> bool {
		self.signal.take()
	}

	pub(crate) fn signal(&self) -> &ChangeSignal {
		&self.signal
	}
}

impl Drop for Monitor {
	fn drop(&mut self) {
		// SAFETY: the registration came from `CM_Register_Notification`. Unregistering waits for running callbacks, so
		// `signal` outlives every callback.
		unsafe { CM_Unregister_Notification(self.registration) };
	}
}

/// Records an HID interface arrival or removal. The filter only delivers those two actions.
unsafe extern "system" fn on_interface_change(
	_registration: HCMNOTIFICATION,
	context: *const std::ffi::c_void,
	_action: CM_NOTIFY_ACTION,
	_data: *const CM_NOTIFY_EVENT_DATA,
	_size: u32,
) -> u32 {
	// SAFETY: `context` is the `ChangeSignal` that `Monitor` keeps alive while registered.
	unsafe { &*context.cast::<ChangeSignal>() }.signal();
	0
}

/// Fills `list` with the present HID interfaces as NUL-terminated wide strings, reusing its capacity.
///
/// The list can grow between the size query and the copy, so a too-small buffer retries with the new size.
fn fill_interface_list(list: &mut Vec<u16>) -> Result<(), String> {
	// SAFETY: `HidD_GetHidGuid` only writes the HID interface class GUID.
	let guid = &unsafe { HidD_GetHidGuid() };
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
	/// Opens `path` with `access`, sharing it with other applications.
	///
	/// Queries need no access rights, which never conflicts with applications that hold the device.
	fn open(path: PCWSTR, access: u32, flags: FILE_FLAGS_AND_ATTRIBUTES) -> Option<Self> {
		// SAFETY: `path` is a NUL-terminated interface path and no security attributes or template are passed.
		let handle = unsafe {
			CreateFileW(
				path,
				access,
				FILE_SHARE_READ | FILE_SHARE_WRITE,
				None,
				OPEN_EXISTING,
				flags,
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

	/// Returns the usage of the interface's top-level collection.
	fn usage(&self) -> Option<Usage> {
		self.caps().map(|caps| Usage {
			page: caps.UsagePage,
			usage: caps.Usage,
		})
	}

	/// Returns the capabilities of the interface's top-level collection, such as its usage and report sizes.
	fn caps(&self) -> Option<HIDP_CAPS> {
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
		(status.0 == HIDP_STATUS_SUCCESS).then_some(caps)
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

/// The `Device` struct keeps an interface open with one overlapped read in flight, so reads never block.
pub(super) struct Device {
	interface: Interface,
	/// The read in flight, boxed so its address stays fixed while the kernel writes to it.
	overlapped: Box<OVERLAPPED>,
	/// Receives each report, sized to the interface's longest input report.
	buffer: Box<[u8]>,
	pending: bool,
}

impl Device {
	pub(super) fn open(path: &PathData) -> Result<Self, String> {
		let terminated = path.iter().copied().chain([0]).collect::<Vec<u16>>();
		let interface = Interface::open(
			PCWSTR(terminated.as_ptr()),
			GENERIC_READ.0 | GENERIC_WRITE.0,
			FILE_FLAG_OVERLAPPED,
		)
		.ok_or_else(|| {
			format!(
				"Failed to open HID device {}. The most likely cause is that another application holds it exclusively.",
				String::from_utf16_lossy(path)
			)
		})?;
		let caps = interface.caps().ok_or_else(|| {
			"Failed to read HID device capabilities. The most likely cause is that the device was unplugged.".to_string()
		})?;
		// SAFETY: no security attributes or name are passed; the event is closed in `Drop`.
		let event = unsafe { CreateEventW(None, true, false, PCWSTR::null()) }.map_err(|error| {
			format!("Failed to create a HID read event: {error}. The most likely cause is that the process ran out of handles.")
		})?;

		Ok(Self {
			interface,
			overlapped: Box::new(OVERLAPPED {
				hEvent: event,
				..Default::default()
			}),
			buffer: vec![0; caps.InputReportByteLength as usize].into_boxed_slice(),
			pending: false,
		})
	}

	/// Starts a read when none is in flight, then returns its report if it has completed.
	pub(super) fn read(&mut self, report: &mut [u8]) -> Result<Option<usize>, String> {
		if !self.pending {
			// SAFETY: `buffer` and `overlapped` live in boxes that outlive the read; `Drop` waits for it to finish.
			let started = unsafe { ReadFile(self.interface.0, Some(&mut self.buffer), None, Some(&mut *self.overlapped)) };
			if let Err(error) = started
				&& error.code() != ERROR_IO_PENDING.to_hresult()
			{
				return Err(read_error(error));
			}
			self.pending = true;
		}

		let mut length = 0u32;
		// SAFETY: `overlapped` belongs to the read in flight on this handle.
		match unsafe { GetOverlappedResult(self.interface.0, &*self.overlapped, &mut length, false) } {
			Ok(()) => {
				self.pending = false;
				let mut data = &self.buffer[..length as usize];
				// Windows always starts a report with its report ID, and with 0 when the device does not number them.
				if let [0, rest @ ..] = data {
					data = rest;
				}
				let copied = data.len().min(report.len());
				report[..copied].copy_from_slice(&data[..copied]);
				Ok(Some(copied))
			}
			Err(error) if error.code() == ERROR_IO_INCOMPLETE.to_hresult() => Ok(None),
			Err(error) => {
				self.pending = false;
				Err(read_error(error))
			}
		}
	}
}

fn read_error(error: windows::core::Error) -> String {
	format!("Failed to read a HID report: {error}. The most likely cause is that the device was unplugged.")
}

impl Drop for Device {
	fn drop(&mut self) {
		if self.pending {
			let mut length = 0u32;
			// SAFETY: the read in flight uses `overlapped`; waiting for it keeps the kernel from writing to freed memory.
			unsafe {
				let _ = CancelIoEx(self.interface.0, Some(&*self.overlapped));
				let _ = GetOverlappedResult(self.interface.0, &*self.overlapped, &mut length, true);
			}
		}
		// SAFETY: the event came from `CreateEventW` and is closed exactly once.
		let _ = unsafe { CloseHandle(self.overlapped.hEvent) };
	}
}
