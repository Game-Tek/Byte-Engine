//! Finds human interface devices (HID), such as gamepads, through the operating system's own device registry.
//!
//! Input systems describe the devices they want with [`Match`] rules, build a [`Scanner`] once, and call
//! [`Scanner::scan`] at startup. Each platform applies the rules before it reads anything expensive, so only
//! matching devices pay for their product name. A scan reports borrowed [`DeviceInfo`] values and allocates
//! nothing per device; call [`DevicePathRef::to_owned`] only for the devices you keep.

#[cfg(target_os = "linux")]
mod linux;
#[cfg(target_os = "macos")]
mod macos;
#[cfg(target_os = "linux")]
pub(crate) mod report_descriptor;
#[cfg(target_os = "windows")]
mod windows;

use std::marker::PhantomData;

#[cfg(target_os = "linux")]
use linux as os;
#[cfg(target_os = "macos")]
use macos as os;
#[cfg(target_os = "windows")]
use windows as os;

/// The `Match` enum lets an input system describe the devices it can read without seeing every device.
///
/// Pass a slice of rules to [`Scanner::new`]. A device matches when any rule matches.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Match {
	/// Matches devices that declare this usage in a top-level collection, such as `0x01`/`0x05` for a gamepad.
	Usage { page: u16, usage: u16 },
	/// Matches every device from this USB vendor ID.
	Vendor(u16),
}

/// The `Scanner` struct keeps the buffers and platform queries that discovery reuses, so repeated scans do not
/// allocate.
///
/// Build it once from the input system's [`Match`] rules, then call [`Scanner::scan`].
pub struct Scanner<'a> {
	os: os::Scanner<'a>,
}

impl<'a> Scanner<'a> {
	/// Prepares discovery for the devices that match any of `matches`.
	///
	/// Next, call [`Scanner::scan`].
	pub fn new(matches: &'a [Match]) -> Result<Self, String> {
		Ok(Self {
			os: os::Scanner::new(matches)?,
		})
	}

	/// Calls `found` once for every connected device that matches.
	///
	/// Unmatched devices cost only a vendor, product, and usage query. The [`DeviceInfo`] borrows scan buffers, so
	/// copy what you keep, for example with [`DevicePathRef::to_owned`].
	pub fn scan(&mut self, found: impl FnMut(DeviceInfo<'_>)) -> Result<(), String> {
		self.os.scan(found)
	}
}

/// The `DeviceInfo` struct describes one device interface that matched a [`Match`] rule, so the input layer can
/// classify it.
///
/// It borrows the scan's buffers and lives only for one call of the `found` callback of [`Scanner::scan`].
#[derive(Debug, Clone, Copy)]
pub struct DeviceInfo<'a> {
	pub path: DevicePathRef<'a>,
	pub vendor_id: u16,
	pub product_id: u16,
	/// The usage page of the matching top-level collection, or of the first one for a [`Match::Vendor`] match.
	pub usage_page: u16,
	/// The usage of the matching top-level collection, or of the first one for a [`Match::Vendor`] match.
	pub usage: u16,
	pub product_name: Option<&'a str>,
}

/// The `DevicePath` struct names one device interface so input systems can tell devices apart and open them.
///
/// Get one from [`DevicePathRef::to_owned`]. It allocates only on Windows, where it holds the interface path.
/// It displays as the path hidapi reports on the same platform: `/dev/hidrawN` on Linux, the device interface
/// path on Windows, and `DevSrvsID:<registry entry ID>` on macOS.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct DevicePath(os::Path);

impl DevicePath {
	/// Borrows the path, for example to compare it with a [`DeviceInfo::path`].
	pub fn as_path_ref(&self) -> DevicePathRef<'_> {
		DevicePathRef(os::path_ref(&self.0), PhantomData)
	}
}

impl std::fmt::Display for DevicePath {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		self.as_path_ref().fmt(f)
	}
}

/// The `DevicePathRef` struct borrows a device path from a scan, so a scan can report devices without allocating.
///
/// Compare it with a kept [`DevicePath`] to skip known devices, and call [`Self::to_owned`] for new ones.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DevicePathRef<'a>(os::PathRef<'a>, PhantomData<&'a ()>);

impl DevicePathRef<'_> {
	/// Copies the path so it outlives the scan.
	pub fn to_owned(self) -> DevicePath {
		DevicePath(os::owned_path(self.0))
	}
}

impl PartialEq<DevicePath> for DevicePathRef<'_> {
	fn eq(&self, other: &DevicePath) -> bool {
		*self == other.as_path_ref()
	}
}

impl std::fmt::Display for DevicePathRef<'_> {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		os::write_path(self.0, f)
	}
}

/// Returns the usage pair that satisfies `matches` for a device, or `None` when no rule matches.
///
/// `usages` lists the device's top-level collections. A vendor match reports the first collection, or `(0, 0)`
/// when the device declares none.
pub(crate) fn find_match(matches: &[Match], vendor_id: u16, usages: impl Iterator<Item = (u16, u16)>) -> Option<(u16, u16)> {
	let mut first = None;
	for (usage_page, usage) in usages {
		first.get_or_insert((usage_page, usage));
		let wanted = Match::Usage { page: usage_page, usage };
		if matches.contains(&wanted) {
			return Some((usage_page, usage));
		}
	}
	matches.contains(&Match::Vendor(vendor_id)).then(|| first.unwrap_or((0, 0)))
}

#[cfg(test)]
mod tests {
	use super::*;

	const GAMEPADS: &[Match] = &[Match::Usage { page: 0x01, usage: 0x05 }, Match::Vendor(0x054C)];

	#[test]
	fn usage_rule_reports_the_matching_collection() {
		let usages = [(0x01, 0x02), (0x01, 0x05)];

		assert_eq!(find_match(GAMEPADS, 0x1234, usages.into_iter()), Some((0x01, 0x05)));
	}

	#[test]
	fn vendor_rule_reports_the_first_collection() {
		let usages = [(0xFF00, 0x01), (0x01, 0x02)];

		assert_eq!(find_match(GAMEPADS, 0x054C, usages.into_iter()), Some((0xFF00, 0x01)));
	}

	#[test]
	fn unmatched_devices_are_skipped() {
		let usages = [(0x01, 0x06)];

		assert_eq!(find_match(GAMEPADS, 0x1234, usages.into_iter()), None);
	}
}
