//! Lists hidraw devices from sysfs. Reading sysfs opens no device, so unmatched devices cost two small file reads.

use std::{
	io::{ErrorKind, Read as _},
	path::{Path, PathBuf},
};

use super::{Device, DevicePath, Match, find_match, report_descriptor::top_level_usages};

/// The largest report descriptor the kernel accepts (`HID_MAX_DESCRIPTOR_SIZE`).
const MAX_DESCRIPTOR_SIZE: usize = 4096;

pub(super) fn scan(matches: &[Match]) -> Result<Vec<Device>, String> {
	scan_sysfs(Path::new("/sys/class/hidraw"), Path::new("/dev"), matches)
}

/// Lists the hidraw devices under `class_dir` that match `matches`, naming each node inside `dev_dir`.
///
/// Each entry's `device/uevent` gives its vendor, product, and name, and `device/report_descriptor` gives its
/// usages. Entries that disappear or cannot be read mid-scan are skipped.
pub(super) fn scan_sysfs(class_dir: &Path, dev_dir: &Path, matches: &[Match]) -> Result<Vec<Device>, String> {
	let entries = match std::fs::read_dir(class_dir) {
		Ok(entries) => entries,
		// Kernels without hidraw support have no class directory, which only means there are no devices.
		Err(error) if error.kind() == ErrorKind::NotFound => return Ok(Vec::new()),
		Err(error) => {
			return Err(format!(
				"Failed to list hidraw devices: {error}. The most likely cause is that sysfs is not mounted at /sys."
			));
		}
	};

	let mut devices = Vec::new();
	let mut path = PathBuf::new();
	let mut uevent = [0u8; 1024];
	let mut descriptor = [0u8; MAX_DESCRIPTOR_SIZE];

	for entry in entries.flatten() {
		let name = entry.file_name();

		path.clear();
		path.extend([class_dir, Path::new(&name), Path::new("device/uevent")]);
		let Some(uevent) = read_into(&path, &mut uevent) else {
			continue;
		};
		let Some(identity) = parse_uevent(uevent) else {
			continue;
		};

		path.set_file_name("report_descriptor");
		let descriptor = read_into(&path, &mut descriptor).unwrap_or_default();
		let Some((usage_page, usage)) = find_match(matches, identity.vendor_id, top_level_usages(descriptor)) else {
			continue;
		};

		devices.push(Device {
			path: DevicePath(dev_dir.join(&name).to_string_lossy().into_owned()),
			vendor_id: identity.vendor_id,
			product_id: identity.product_id,
			usage_page,
			usage,
			product_name: identity.name.map(str::to_owned),
		});
	}

	Ok(devices)
}

/// Reads a whole sysfs attribute into `buffer` and returns the bytes read, or `None` when it cannot be read.
fn read_into<'a>(path: &Path, buffer: &'a mut [u8]) -> Option<&'a [u8]> {
	let mut file = std::fs::File::open(path).ok()?;
	let mut length = 0;
	// Sysfs attributes can arrive over several reads; stop at end of file or when the buffer is full.
	while length < buffer.len() {
		match file.read(&mut buffer[length..]) {
			Ok(0) => break,
			Ok(read) => length += read,
			Err(error) if error.kind() == ErrorKind::Interrupted => {}
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
	use super::*;

	/// Creates a sysfs-like hidraw class directory with one entry per `(node, uevent, descriptor)`.
	fn fake_sysfs(test: &str, devices: &[(&str, &str, &[u8])]) -> PathBuf {
		let root = std::env::temp_dir().join(format!("byte-engine-ghi-hid-{test}-{}", std::process::id()));
		let _ = std::fs::remove_dir_all(&root);
		for (node, uevent, descriptor) in devices {
			let device = root.join(node).join("device");
			std::fs::create_dir_all(&device).unwrap();
			std::fs::write(device.join("uevent"), uevent).unwrap();
			std::fs::write(device.join("report_descriptor"), descriptor).unwrap();
		}
		root
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

		let devices = scan_sysfs(&root, Path::new("/dev"), &[Match::Usage { page: 0x01, usage: 0x05 }]).unwrap();

		assert_eq!(
			devices,
			[Device {
				path: DevicePath("/dev/hidraw1".into()),
				vendor_id: 0x045E,
				product_id: 0x0B13,
				usage_page: 0x01,
				usage: 0x05,
				product_name: Some("Xbox Wireless Controller".into()),
			}]
		);
		std::fs::remove_dir_all(root).unwrap();
	}

	#[test]
	fn scan_skips_entries_without_a_hid_id() {
		let root = fake_sysfs("no-id", &[("hidraw0", "DRIVER=hid-generic\n", GAMEPAD_DESCRIPTOR)]);

		let devices = scan_sysfs(&root, Path::new("/dev"), &[Match::Usage { page: 0x01, usage: 0x05 }]).unwrap();

		assert!(devices.is_empty());
		std::fs::remove_dir_all(root).unwrap();
	}

	#[test]
	fn missing_class_directory_means_no_devices() {
		let devices = scan_sysfs(Path::new("/nonexistent/hidraw"), Path::new("/dev"), &[Match::Vendor(0x054C)]).unwrap();

		assert!(devices.is_empty());
	}
}
