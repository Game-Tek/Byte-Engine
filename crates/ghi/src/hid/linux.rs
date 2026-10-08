//! Lists hidraw devices from sysfs. Reading sysfs opens no device, so unmatched devices cost two small file reads.
//!
//! The scan lists the class directory into a stack buffer and opens attributes relative to it, so it does not
//! allocate.

use std::{ffi::CStr, io::Write as _, mem::MaybeUninit};

use rustix::{
	fd::{AsFd, OwnedFd},
	fs::{Mode, OFlags, RawDir},
	io::Errno,
};

use super::{DeviceInfo, DevicePathRef, Match, find_match, report_descriptor::top_level_usages};

/// The largest report descriptor the kernel accepts (`HID_MAX_DESCRIPTOR_SIZE`).
const MAX_DESCRIPTOR_SIZE: usize = 4096;

/// A device path is the `N` of `/dev/hidrawN`.
pub(super) type Path = u32;
pub(super) type PathRef<'a> = u32;

pub(super) fn path_ref(path: &Path) -> PathRef<'_> {
	*path
}

pub(super) fn owned_path(path: PathRef<'_>) -> Path {
	path
}

pub(super) fn write_path(path: PathRef<'_>, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
	write!(f, "/dev/hidraw{path}")
}

pub(super) struct Scanner<'a> {
	matches: &'a [Match],
	/// The hidraw class directory, `/sys/class/hidraw` outside tests.
	class_dir: &'a std::path::Path,
}

impl<'a> Scanner<'a> {
	pub(super) fn new(matches: &'a [Match]) -> Result<Self, String> {
		Ok(Self::with_class_dir(matches, std::path::Path::new("/sys/class/hidraw")))
	}

	fn with_class_dir(matches: &'a [Match], class_dir: &'a std::path::Path) -> Self {
		Self { matches, class_dir }
	}

	/// Reports the hidraw entries whose `device/uevent` and `device/report_descriptor` attributes match.
	///
	/// The uevent gives the vendor, product, and name, and the report descriptor gives the usages. Entries that
	/// disappear or cannot be read mid-scan are skipped.
	pub(super) fn scan(&mut self, mut found: impl FnMut(DeviceInfo<'_>)) -> Result<(), String> {
		let directory = match rustix::fs::open(
			self.class_dir,
			OFlags::RDONLY | OFlags::DIRECTORY | OFlags::CLOEXEC,
			Mode::empty(),
		) {
			Ok(directory) => directory,
			// Kernels without hidraw support have no class directory, which only means there are no devices.
			Err(Errno::NOENT) => return Ok(()),
			Err(error) => {
				return Err(format!(
					"Failed to open the hidraw class directory: {error}. The most likely cause is that sysfs is not mounted at /sys."
				));
			}
		};

		let mut listing = [MaybeUninit::<u8>::uninit(); 4096];
		let mut entries = RawDir::new(&directory, &mut listing);
		let mut uevent = [0u8; 1024];
		let mut descriptor = [0u8; MAX_DESCRIPTOR_SIZE];

		while let Some(entry) = entries.next() {
			let entry = entry.map_err(|error| {
				format!(
					"Failed to list hidraw devices: {error}. The most likely cause is that a device was removed during the scan."
				)
			})?;
			let Some(index) = hidraw_index(entry.file_name()) else {
				continue;
			};

			let Some(uevent) = read_attribute(&directory, index, "uevent", &mut uevent) else {
				continue;
			};
			let Some(identity) = parse_uevent(uevent) else {
				continue;
			};
			let descriptor = read_attribute(&directory, index, "report_descriptor", &mut descriptor).unwrap_or_default();
			let Some((usage_page, usage)) = find_match(self.matches, identity.vendor_id, top_level_usages(descriptor)) else {
				continue;
			};

			found(DeviceInfo {
				path: DevicePathRef(index, std::marker::PhantomData),
				vendor_id: identity.vendor_id,
				product_id: identity.product_id,
				usage_page,
				usage,
				product_name: identity.name,
			});
		}

		Ok(())
	}
}

/// Returns `N` for a directory entry named `hidrawN`, or `None` for `.`, `..`, and anything else.
fn hidraw_index(name: &CStr) -> Option<u32> {
	std::str::from_utf8(name.to_bytes().strip_prefix(b"hidraw")?)
		.ok()?
		.parse()
		.ok()
}

/// Reads the `hidraw<index>/device/<attribute>` file below `directory` into `buffer`.
///
/// Returns the bytes read, or `None` when the attribute cannot be read.
fn read_attribute<'a>(directory: impl AsFd, index: u32, attribute: &str, buffer: &'a mut [u8]) -> Option<&'a [u8]> {
	// The path is at most `hidraw4294967295/device/report_descriptor` plus a NUL terminator.
	let mut path = [0u8; 64];
	let mut cursor = &mut path[..];
	write!(cursor, "hidraw{index}/device/{attribute}\0").ok()?;
	let path = CStr::from_bytes_until_nul(&path).ok()?;

	let file: OwnedFd = rustix::fs::openat(directory, path, OFlags::RDONLY | OFlags::CLOEXEC, Mode::empty()).ok()?;
	let mut length = 0;
	// Sysfs attributes can arrive over several reads; stop at end of file or when the buffer is full.
	while length < buffer.len() {
		match rustix::io::read(&file, &mut buffer[length..]) {
			Ok(0) => break,
			Ok(read) => length += read,
			Err(Errno::INTR) => {}
			Err(_) => return None,
		}
	}
	Some(&buffer[..length])
}

/// The `UeventIdentity` struct holds the fields of a HID `uevent` file the scan needs.
struct UeventIdentity<'a> {
	vendor_id: u16,
	product_id: u16,
	name: Option<&'a str>,
}

/// Reads `HID_ID=<bus>:<vendor>:<product>` and `HID_NAME=<name>` from a HID device's `uevent` attribute.
fn parse_uevent(uevent: &[u8]) -> Option<UeventIdentity<'_>> {
	let uevent = std::str::from_utf8(uevent).ok()?;
	let mut ids = None;
	let mut name = None;

	for line in uevent.lines() {
		if let Some(id) = line.strip_prefix("HID_ID=") {
			let mut fields = id.split(':').skip(1);
			let vendor = u32::from_str_radix(fields.next()?, 16).ok()?;
			let product = u32::from_str_radix(fields.next()?, 16).ok()?;
			ids = Some((vendor as u16, product as u16));
		} else if let Some(value) = line.strip_prefix("HID_NAME=") {
			name = Some(value);
		}
	}

	let (vendor_id, product_id) = ids?;
	Some(UeventIdentity {
		vendor_id,
		product_id,
		name,
	})
}

#[cfg(test)]
mod tests {
	use std::path::PathBuf;

	use super::*;

	type FsPath = std::path::Path;

	/// Creates a sysfs-like hidraw class directory with one entry per `(node, uevent, descriptor)`.
	fn fake_sysfs(test: &str, devices: &[(&str, &str, &[u8])]) -> PathBuf {
		let root = std::env::temp_dir().join(format!("byte-engine-ghi-hid-{test}-{}", std::process::id()));
		let _ = std::fs::remove_dir_all(&root);
		std::fs::create_dir_all(&root).unwrap();
		for (node, uevent, descriptor) in devices {
			let device = root.join(node).join("device");
			std::fs::create_dir_all(&device).unwrap();
			std::fs::write(device.join("uevent"), uevent).unwrap();
			std::fs::write(device.join("report_descriptor"), descriptor).unwrap();
		}
		root
	}

	/// Scans `class_dir` and returns `(path, vendor, product, usage page, usage, name)` for each match.
	fn scan(class_dir: &FsPath, matches: &[Match]) -> Vec<(String, u16, u16, u16, u16, Option<String>)> {
		let mut devices = Vec::new();
		Scanner::with_class_dir(matches, class_dir)
			.scan(|device| {
				devices.push((
					device.path.to_string(),
					device.vendor_id,
					device.product_id,
					device.usage_page,
					device.usage,
					device.product_name.map(str::to_owned),
				))
			})
			.unwrap();
		devices
	}

	// Usage Page (Generic Desktop), Usage (Game Pad), Collection (Application), End Collection
	const GAMEPAD_DESCRIPTOR: &[u8] = &[0x05, 0x01, 0x09, 0x05, 0xA1, 0x01, 0xC0];
	// Usage Page (Generic Desktop), Usage (Keyboard), Collection (Application), End Collection
	const KEYBOARD_DESCRIPTOR: &[u8] = &[0x05, 0x01, 0x09, 0x06, 0xA1, 0x01, 0xC0];

	#[test]
	fn scan_reports_only_matching_devices() {
		let root = fake_sysfs(
			"matching",
			&[
				(
					"hidraw0",
					"DRIVER=hid-generic\nHID_ID=0003:0000046D:0000C31C\nHID_NAME=Logitech Keyboard\n",
					KEYBOARD_DESCRIPTOR,
				),
				(
					"hidraw1",
					"DRIVER=microsoft\nHID_ID=0005:0000045E:00000B13\nHID_NAME=Xbox Wireless Controller\n",
					GAMEPAD_DESCRIPTOR,
				),
			],
		);

		let devices = scan(&root, &[Match::Usage { page: 0x01, usage: 0x05 }]);

		assert_eq!(
			devices,
			[(
				"/dev/hidraw1".to_string(),
				0x045E,
				0x0B13,
				0x01,
				0x05,
				Some("Xbox Wireless Controller".to_string())
			)]
		);
		std::fs::remove_dir_all(root).unwrap();
	}

	#[test]
	fn scan_skips_entries_without_a_hid_id() {
		let root = fake_sysfs("no-id", &[("hidraw0", "DRIVER=hid-generic\n", GAMEPAD_DESCRIPTOR)]);

		let devices = scan(&root, &[Match::Usage { page: 0x01, usage: 0x05 }]);

		assert!(devices.is_empty());
		std::fs::remove_dir_all(root).unwrap();
	}

	#[test]
	fn missing_class_directory_means_no_devices() {
		let devices = scan(FsPath::new("/nonexistent/hidraw"), &[Match::Vendor(0x054C)]);

		assert!(devices.is_empty());
	}
}
