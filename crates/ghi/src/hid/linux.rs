//! Lists hidraw devices from sysfs. Reading sysfs opens no device, so an unmatched device costs one small file read.
//!
//! The scan lists the class directory into a stack buffer and opens attributes relative to it, so it does not
//! allocate.

use std::{ffi::CStr, io::Write as _, mem::MaybeUninit, path::Path};

use rustix::{
	fd::{AsFd, BorrowedFd, OwnedFd},
	fs::{Mode, OFlags, RawDir, inotify},
	io::Errno,
};

use super::{DeviceInfo, DevicePathRef, Usage, report_descriptor::top_level_usages};

/// The largest report descriptor the kernel accepts (`HID_MAX_DESCRIPTOR_SIZE`).
const MAX_DESCRIPTOR_SIZE: usize = 4096;

/// A device path is the `N` of `/dev/hidrawN`.
pub(super) type PathData = u32;

pub(super) fn write_path(path: &PathData, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
	write!(f, "/dev/hidraw{path}")
}

pub(super) struct Scanner {
	usages: &'static [Usage],
}

impl Scanner {
	pub(super) fn new(usages: &'static [Usage]) -> Self {
		Self { usages }
	}

	pub(super) fn scan(&mut self, found: impl FnMut(DeviceInfo<'_>)) -> Result<(), String> {
		scan_class_dir(Path::new("/sys/class/hidraw"), self.usages, found)
	}
}

/// The `Monitor` struct watches `/dev` for hidraw nodes that appear, disappear, or change permissions.
///
/// Nothing in Linux calls back on a change, so the window event loop watches [`Monitor::fd`] instead.
pub(crate) struct Monitor {
	inotify: OwnedFd,
}

impl Monitor {
	pub(super) fn new() -> Result<Self, String> {
		Self::watching(Path::new("/dev"))
	}

	/// Watches `directory` for entries named `hidraw*`.
	fn watching(directory: &Path) -> Result<Self, String> {
		let inotify = inotify::init(inotify::CreateFlags::NONBLOCK | inotify::CreateFlags::CLOEXEC).map_err(|error| {
			format!("Failed to create a HID device watch: {error}. The most likely cause is the inotify instance limit.")
		})?;
		// udev applies access rights after it creates a node, so attribute changes also count: a device that could
		// not be opened at creation can be opened after them.
		let flags = inotify::WatchFlags::CREATE | inotify::WatchFlags::DELETE | inotify::WatchFlags::ATTRIB;
		inotify::add_watch(&inotify, directory, flags | inotify::WatchFlags::ONLYDIR).map_err(|error| {
			format!(
				"Failed to watch {} for HID devices: {error}. The most likely cause is the inotify watch limit.",
				directory.display()
			)
		})?;
		Ok(Self { inotify })
	}

	/// Reads every queued event and reports whether one named a hidraw node.
	pub(super) fn take_changed(&mut self) -> bool {
		let mut buffer = [MaybeUninit::<u8>::uninit(); 1024];
		let mut events = inotify::Reader::new(&self.inotify, &mut buffer);
		let mut changed = false;
		// The descriptor does not block, so reading ends with `AGAIN` once the queue is empty.
		while let Ok(event) = events.next() {
			changed |= event.file_name().is_some_and(|name| name.to_bytes().starts_with(b"hidraw"));
		}
		changed
	}

	/// Returns the descriptor that becomes readable when `/dev` changes.
	pub(crate) fn fd(&self) -> BorrowedFd<'_> {
		self.inotify.as_fd()
	}
}

pub(super) struct Device(OwnedFd);

impl Device {
	pub(super) fn open(path: &PathData) -> Result<Self, String> {
		// The node is at most `/dev/hidraw4294967295` plus a NUL terminator.
		let mut node = [0u8; 32];
		let mut cursor = &mut node[..];
		write!(cursor, "/dev/hidraw{path}\0").expect("A hidraw node path always fits its buffer.");
		let node = CStr::from_bytes_until_nul(&node).expect("The hidraw node path was written with its terminator.");

		rustix::fs::open(node, OFlags::RDWR | OFlags::NONBLOCK | OFlags::CLOEXEC, Mode::empty())
			.map(Self)
			.map_err(|error| {
				format!(
					"Failed to open /dev/hidraw{path}: {error}. The most likely cause is that a udev rule does not grant your user access to the device."
				)
			})
	}

	pub(super) fn read(&mut self, report: &mut [u8]) -> Result<Option<usize>, String> {
		loop {
			match rustix::io::read(&self.0, &mut *report) {
				Ok(length) => return Ok(Some(length)),
				Err(Errno::AGAIN) => return Ok(None),
				Err(Errno::INTR) => {}
				Err(error) => {
					return Err(format!(
						"Failed to read a hidraw report: {error}. The most likely cause is that the device was unplugged."
					));
				}
			}
		}
	}
}

/// Reports the hidraw entries of `class_dir` whose `device/report_descriptor` declares one of `usages`.
///
/// The descriptor gives the usages, and `device/uevent` gives the vendor, product, and name of matches. Entries
/// that disappear or cannot be read mid-scan are skipped.
fn scan_class_dir(class_dir: &Path, usages: &[Usage], mut found: impl FnMut(DeviceInfo<'_>)) -> Result<(), String> {
	let directory = match rustix::fs::open(class_dir, OFlags::RDONLY | OFlags::DIRECTORY | OFlags::CLOEXEC, Mode::empty()) {
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

		let Some(descriptor) = read_attribute(&directory, index, "report_descriptor", &mut descriptor) else {
			continue;
		};
		let Some(usage) = top_level_usages(descriptor).find(|usage| usages.contains(usage)) else {
			continue;
		};
		let Some((vendor_id, product_id, product_name)) =
			read_attribute(&directory, index, "uevent", &mut uevent).and_then(parse_uevent)
		else {
			continue;
		};

		found(DeviceInfo {
			path: DevicePathRef(&index),
			vendor_id,
			product_id,
			usage,
			product_name,
		});
	}

	Ok(())
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

/// Reads `HID_ID=<bus>:<vendor>:<product>` and `HID_NAME=<name>` from a HID device's `uevent` attribute.
fn parse_uevent(uevent: &[u8]) -> Option<(u16, u16, Option<&str>)> {
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
	Some((vendor_id, product_id, name))
}

#[cfg(test)]
mod tests {
	use std::path::PathBuf;

	use super::*;

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

	const GAMEPAD: &[Usage] = &[Usage { page: 0x01, usage: 0x05 }];

	/// Scans `class_dir` for gamepads and returns `(path, vendor, product, name)` for each match.
	fn scan(class_dir: &Path) -> Vec<(String, u16, u16, Option<String>)> {
		let mut devices = Vec::new();
		scan_class_dir(class_dir, GAMEPAD, |device| {
			devices.push((
				device.path.to_string(),
				device.vendor_id,
				device.product_id,
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

		let devices = scan(&root);

		assert_eq!(
			devices,
			[(
				"/dev/hidraw1".to_string(),
				0x045E,
				0x0B13,
				Some("Xbox Wireless Controller".to_string())
			)]
		);
		std::fs::remove_dir_all(root).unwrap();
	}

	#[test]
	fn scan_skips_entries_without_a_hid_id() {
		let root = fake_sysfs("no-id", &[("hidraw0", "DRIVER=hid-generic\n", GAMEPAD_DESCRIPTOR)]);

		let devices = scan(&root);

		assert!(devices.is_empty());
		std::fs::remove_dir_all(root).unwrap();
	}

	#[test]
	fn monitor_reports_only_hidraw_changes() {
		let directory = std::env::temp_dir().join(format!("byte-engine-ghi-hid-monitor-{}", std::process::id()));
		let _ = std::fs::remove_dir_all(&directory);
		std::fs::create_dir_all(&directory).unwrap();
		let mut monitor = Monitor::watching(&directory).unwrap();

		std::fs::write(directory.join("tty9"), "").unwrap();
		assert!(!monitor.take_changed());

		std::fs::write(directory.join("hidraw3"), "").unwrap();
		assert!(monitor.take_changed());
		assert!(!monitor.take_changed());

		std::fs::remove_file(directory.join("hidraw3")).unwrap();
		assert!(monitor.take_changed());
		std::fs::remove_dir_all(directory).unwrap();
	}

	#[test]
	fn missing_class_directory_means_no_devices() {
		let devices = scan(Path::new("/nonexistent/hidraw"));

		assert!(devices.is_empty());
	}
}
