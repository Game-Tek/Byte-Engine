//! Finds human interface devices (HID), such as gamepads, through the operating system's own device registry.
//!
//! Input systems list the [`Usage`] values they can read, build a [`Scanner`] once, and call [`Scanner::scan`] at
//! startup. Each platform filters by usage before it reads anything expensive, so only matching devices pay for
//! their product name. A scan reports borrowed [`DeviceInfo`] values and allocates nothing per device; call
//! [`DevicePathRef::to_owned`] only for the devices you keep.

#[cfg(target_os = "linux")]
mod linux;
#[cfg(target_os = "macos")]
mod macos;
#[cfg(target_os = "linux")]
mod report_descriptor;
#[cfg(target_os = "windows")]
mod windows;

use std::borrow::Borrow as _;

#[cfg(target_os = "linux")]
use linux as os;
#[cfg(target_os = "macos")]
use macos as os;
#[cfg(target_os = "windows")]
use windows as os;

/// The `Usage` struct names what a device is for, such as `0x01`/`0x05` for a gamepad, so input systems can ask
/// for the devices they can read.
///
/// Pass the usages you want to [`Scanner::new`]. A device matches when one of its top-level collections declares
/// one of them.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Usage {
	pub page: u16,
	pub usage: u16,
}

/// The `Scanner` struct keeps the buffers and platform queries that discovery reuses, so repeated scans do not
/// allocate.
///
/// Build it once from the input system's usages, then call [`Scanner::scan`].
pub struct Scanner {
	os: os::Scanner,
}

impl Scanner {
	/// Prepares discovery for the devices that declare any of `usages`.
	///
	/// Next, call [`Scanner::scan`].
	pub fn new(usages: &'static [Usage]) -> Self {
		Self {
			os: os::Scanner::new(usages),
		}
	}

	/// Calls `found` once for every connected device that matches.
	///
	/// Unmatched devices cost only a usage query. The [`DeviceInfo`] borrows scan buffers, so copy what you keep,
	/// for example with [`DevicePathRef::to_owned`].
	pub fn scan(&mut self, found: impl FnMut(DeviceInfo<'_>)) -> Result<(), String> {
		self.os.scan(found)
	}
}

/// The `DeviceInfo` struct describes one matching device interface, so the input layer can classify it.
///
/// It borrows the scan's buffers and lives only for one call of the `found` callback of [`Scanner::scan`].
#[derive(Debug, Clone, Copy)]
pub struct DeviceInfo<'a> {
	pub path: DevicePathRef<'a>,
	pub vendor_id: u16,
	pub product_id: u16,
	/// The requested usage the device declares.
	pub usage: Usage,
	pub product_name: Option<&'a str>,
}

/// The `DevicePath` struct names one device interface so input systems can tell devices apart and open them.
///
/// Get one from [`DevicePathRef::to_owned`]. It allocates only on Windows, where it holds the interface path.
/// It displays as the path hidapi reports on the same platform: `/dev/hidrawN` on Linux, the device interface
/// path on Windows, and `DevSrvsID:<registry entry ID>` on macOS.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct DevicePath(<os::PathData as ToOwned>::Owned);

impl std::fmt::Display for DevicePath {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		os::write_path(self.0.borrow(), f)
	}
}

/// The `DevicePathRef` struct borrows a device path from a scan, so a scan can report devices without allocating.
///
/// Compare it with a kept [`DevicePath`] to skip known devices, and call [`Self::to_owned`] for new ones.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DevicePathRef<'a>(&'a os::PathData);

impl DevicePathRef<'_> {
	/// Copies the path so it outlives the scan.
	pub fn to_owned(self) -> DevicePath {
		DevicePath(self.0.to_owned())
	}
}

impl PartialEq<DevicePath> for DevicePathRef<'_> {
	fn eq(&self, other: &DevicePath) -> bool {
		let other: &os::PathData = other.0.borrow();
		self.0 == other
	}
}

impl std::fmt::Display for DevicePathRef<'_> {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		os::write_path(self.0, f)
	}
}
