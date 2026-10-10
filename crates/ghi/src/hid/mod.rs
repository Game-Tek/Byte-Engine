//! Finds human interface devices (HID), such as gamepads, through the operating system's own device registry.
//!
//! Input systems list the [`Usage`] values they can read, build a [`Scanner`] once, and call [`Scanner::scan`] at
//! startup. Each platform filters by usage before it reads anything expensive, so only matching devices pay for
//! their product name. A scan reports borrowed [`DeviceInfo`] values and allocates nothing per device; call
//! [`DevicePathRef::to_owned`] only for the devices you keep, then read them through a [`Device`]. A [`Monitor`]
//! reports when devices connect or disconnect, so you scan again only then.

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

/// The `Monitor` struct tells an input system when HID devices connect or disconnect, so it scans again only after
/// a change instead of on a timer.
///
/// Create it before the startup [`Scanner::scan`] so no change slips between the two, call [`Monitor::take_changed`]
/// once per frame, and pass it to [`crate::window::App::wake_on_hid_changes`] so a waiting application loop wakes
/// up for a change.
pub struct Monitor {
	pub(crate) os: os::Monitor,
}

impl Monitor {
	/// Subscribes to the operating system's device notifications.
	///
	/// Next, call [`Scanner::scan`] for the devices already connected.
	pub fn new() -> Result<Self, String> {
		Ok(Self { os: os::Monitor::new()? })
	}

	/// Returns whether a device connected or disconnected since the last call, then forgets the change.
	///
	/// When it returns `true`, call [`Scanner::scan`] and compare its devices with the ones you keep.
	pub fn take_changed(&mut self) -> bool {
		self.os.take_changed()
	}
}

/// The `ChangeSignal` struct carries device notifications from the operating system's callback thread to the
/// [`Monitor`] owner, and wakes the application loop that watches them.
#[cfg(not(target_os = "linux"))]
#[derive(Default)]
pub(crate) struct ChangeSignal {
	changed: std::sync::atomic::AtomicBool,
	waker: std::sync::Mutex<Option<crate::window::AppWaker>>,
}

#[cfg(not(target_os = "linux"))]
impl ChangeSignal {
	/// Records a change and wakes the watching application loop. Notification callbacks call it.
	fn signal(&self) {
		self.changed.store(true, std::sync::atomic::Ordering::Release);
		if let Some(waker) = &*self.waker.lock().unwrap_or_else(std::sync::PoisonError::into_inner) {
			waker.wake();
		}
	}

	fn take(&self) -> bool {
		self.changed.swap(false, std::sync::atomic::Ordering::Acquire)
	}

	/// Makes later changes wake `waker`'s application loop.
	pub(crate) fn set_waker(&self, waker: crate::window::AppWaker) {
		*self.waker.lock().unwrap_or_else(std::sync::PoisonError::into_inner) = Some(waker);
	}
}

/// The `Device` struct keeps one device interface open so an input system can read its reports every frame.
///
/// Open it with a path from [`Scanner::scan`], then call [`Device::read`] until it returns `None`. Use it on the
/// thread that opened it: on macOS, reports arrive through that thread's run loop.
pub struct Device {
	os: os::Device,
}

impl Device {
	/// Opens the device for reading without blocking.
	///
	/// Next, call [`Device::read`] once per frame.
	pub fn open(path: &DevicePath) -> Result<Self, String> {
		Ok(Self {
			os: os::Device::open(path.0.borrow())?,
		})
	}

	/// Copies the next waiting input report into `report` and returns its length, or `None` when no report waits.
	///
	/// A report starts with its report ID only when the device numbers its reports. A report longer than
	/// `report` is cut to fit. An error means the device stopped working, usually because it was unplugged.
	pub fn read(&mut self, report: &mut [u8]) -> Result<Option<usize>, String> {
		self.os.read(report)
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
