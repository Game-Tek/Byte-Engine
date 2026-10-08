//! Finds human interface devices (HID), such as gamepads, through the operating system's own device registry.
//!
//! Input systems describe the devices they want with [`Match`] rules and call [`scan`] once at startup. Each
//! platform applies the rules before it reads anything expensive, so only matching devices pay for their product
//! name. Classify the returned [`Device`] values by vendor, product, and name in the input layer.

#[cfg(target_os = "linux")]
mod linux;
#[cfg(target_os = "macos")]
mod macos;
#[cfg(target_os = "linux")]
pub(crate) mod report_descriptor;
#[cfg(target_os = "windows")]
mod windows;

#[cfg(target_os = "linux")]
use linux as os;
#[cfg(target_os = "macos")]
use macos as os;
#[cfg(target_os = "windows")]
use windows as os;

/// The `Match` enum lets an input system describe the devices it can read without seeing every device.
///
/// Pass a slice of rules to [`scan`]. A device matches when any rule matches.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Match {
	/// Matches devices that declare this usage in a top-level collection, such as `0x01`/`0x05` for a gamepad.
	Usage { page: u16, usage: u16 },
	/// Matches every device from this USB vendor ID.
	Vendor(u16),
}

/// The `DevicePath` struct names one device interface so input systems can open it and tell devices apart.
///
/// The text matches the path hidapi reports on the same platform: `/dev/hidrawN` on Linux, the device interface
/// path on Windows, and `DevSrvsID:<registry entry ID>` on macOS.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct DevicePath(String);

impl DevicePath {
	/// Returns the platform path text.
	pub fn as_str(&self) -> &str {
		&self.0
	}
}

impl std::fmt::Display for DevicePath {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		f.write_str(&self.0)
	}
}

/// The `Device` struct describes one device interface that matched a [`Match`] rule, so the input layer can
/// classify it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Device {
	pub path: DevicePath,
	pub vendor_id: u16,
	pub product_id: u16,
	/// The usage page of the matching top-level collection, or of the first one for a [`Match::Vendor`] match.
	pub usage_page: u16,
	/// The usage of the matching top-level collection, or of the first one for a [`Match::Vendor`] match.
	pub usage: u16,
	pub product_name: Option<String>,
}

/// Lists the connected devices that match any of `matches`.
///
/// Call it once at startup. It reads only the vendor, product, and usages of devices that do not match.
pub fn scan(matches: &[Match]) -> Result<Vec<Device>, String> {
	os::scan(matches)
}

/// Returns the usage pair that satisfies `matches` for a device, or `None` when no rule matches.
///
/// `usages` lists the device's top-level collections. A vendor match reports the first collection, or `(0, 0)`
/// when the device declares none.
pub(crate) fn find_match(
	matches: &[Match],
	vendor_id: u16,
	usages: impl Iterator<Item = (u16, u16)>,
) -> Option<(u16, u16)> {
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
