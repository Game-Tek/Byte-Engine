//! Lists HID services from the I/O Registry.
//!
//! hidapi creates an `IOHIDManager` that builds a device object for every HID service, then copies its
//! properties. This scan lets the kernel apply each requested usage as a matching dictionary and reads registry
//! properties of the matching services only. The dictionaries and property keys are built once per
//! `Scanner`, so a scan allocates only the property values IOKit copies out.

use std::{
	ffi::{CStr, c_void},
	ptr::NonNull,
};

use dispatch2::{DispatchQueue, DispatchRetained};
use objc2_core_foundation::{CFDictionary, CFIndex, CFMutableDictionary, CFNumber, CFRetained, CFRunLoop, CFString, CFType};
use objc2_io_kit::{
	IOHIDDevice, IOHIDReportType, IOIteratorNext, IONotificationPort, IONotificationPortRef, IOObjectRelease,
	IORegistryEntryCreateCFProperty, IORegistryEntryGetRegistryEntryID, IORegistryEntryIDMatching, IOReturn,
	IOServiceAddMatchingNotification, IOServiceGetMatchingService, IOServiceGetMatchingServices, IOServiceMatching,
	io_iterator_t, io_object_t, kIOMainPortDefault, kIOReturnSuccess,
};
use smallvec::SmallVec;

use super::{ChangeSignal, DeviceInfo, DevicePathRef, Usage};

/// `kCFStringEncodingUTF8`.
const UTF8: u32 = 0x0800_0100;

/// A device path is the service's registry entry ID.
pub(super) type PathData = u64;

pub(super) fn write_path(path: &PathData, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
	write!(f, "DevSrvsID:{path}")
}

pub(super) struct Scanner {
	/// Each usage with its matching dictionary, which every scan passes to IOKit again.
	usages: SmallVec<[(Usage, CFRetained<CFDictionary>); 4]>,
	/// The registry property names a scan reads, created once instead of per read.
	vendor_id: CFRetained<CFString>,
	product_id: CFRetained<CFString>,
	product: CFRetained<CFString>,
}

impl Scanner {
	pub(super) fn new(usages: &'static [Usage]) -> Self {
		Self {
			usages: usages.iter().map(|usage| (*usage, matching_dictionary(*usage))).collect(),
			vendor_id: CFString::from_static_str("VendorID"),
			product_id: CFString::from_static_str("ProductID"),
			product: CFString::from_static_str("Product"),
		}
	}

	/// Looks up the services of every usage and reports each service once, even when it declares several usages.
	pub(super) fn scan(&mut self, mut found: impl FnMut(DeviceInfo<'_>)) -> Result<(), String> {
		let mut seen = SmallVec::<[u64; 8]>::new();
		let mut name = [0u8; 512];

		for (usage, matching) in &self.usages {
			let services = matching_services(matching)?;
			while let Some(service) = services.next() {
				let mut entry_id = 0u64;
				// SAFETY: `service` is a live registry entry and `entry_id` is valid for writes.
				if unsafe { IORegistryEntryGetRegistryEntryID(service.0, &mut entry_id) } != 0 || seen.contains(&entry_id) {
					continue;
				}
				seen.push(entry_id);

				found(DeviceInfo {
					path: DevicePathRef(&entry_id),
					vendor_id: service.number(&self.vendor_id).unwrap_or(0) as u16,
					product_id: service.number(&self.product_id).unwrap_or(0) as u16,
					usage: *usage,
					product_name: service.string(&self.product, &mut name),
				});
			}
		}

		Ok(())
	}
}

/// Builds the `IOHIDDevice` matching dictionary for `usage`.
///
/// `DeviceUsagePage` and `DeviceUsage` match any usage pair a device declares, like `IOHIDManager` matching does.
fn matching_dictionary(usage: Usage) -> CFRetained<CFDictionary> {
	let matching = hid_device_matching();
	for (key, value) in [("DeviceUsagePage", usage.page), ("DeviceUsage", usage.usage)] {
		let key = CFString::from_static_str(key);
		let value = CFNumber::new_i32(value as i32);
		// SAFETY: the dictionary retains both CF objects, so they may be released after the call.
		unsafe {
			CFMutableDictionary::set_value(
				Some(&matching),
				(&*key as *const CFString).cast::<c_void>(),
				(&*value as *const CFNumber).cast::<c_void>(),
			)
		};
	}
	CFRetained::<CFDictionary>::from(&*matching)
}

/// Builds a dictionary that matches every `IOHIDDevice` service.
fn hid_device_matching() -> CFRetained<CFMutableDictionary> {
	// SAFETY: the class name is a NUL-terminated string.
	unsafe { IOServiceMatching(c"IOHIDDevice".as_ptr()) }.expect(
		"Failed to create an IOHIDDevice matching dictionary. The most likely cause is that the process is out of memory.",
	)
}

/// The `Monitor` struct holds IOKit notifications for HID services that appear or terminate.
///
/// They arrive on a private dispatch queue, whose callback records the change and wakes the application loop.
pub(crate) struct Monitor {
	pub(crate) signal: std::sync::Arc<ChangeSignal>,
	/// Runs every notification callback, one at a time.
	queue: DispatchRetained<DispatchQueue>,
	port: IONotificationPortRef,
	/// The first-match and termination iterators, which IOKit re-arms each time they are drained.
	iterators: [io_iterator_t; 2],
}

impl Monitor {
	pub(super) fn new() -> Result<Self, String> {
		let signal = std::sync::Arc::new(ChangeSignal::default());
		// SAFETY: reading the default main port constant has no other effect.
		let port = IONotificationPort::create(unsafe { kIOMainPortDefault });
		if port.is_null() {
			return Err(
				"Failed to create an IOKit notification port. The most likely cause is that IOKit is unavailable.".into(),
			);
		}
		let mut monitor = Self {
			signal,
			queue: DispatchQueue::new("com.byte-engine.hid", None),
			port,
			iterators: [0; 2],
		};

		for (iterator, kind) in monitor
			.iterators
			.iter_mut()
			.zip([c"IOServiceFirstMatch", c"IOServiceTerminate"])
		{
			let mut name = [0; 128];
			for (to, from) in name.iter_mut().zip(kind.to_bytes_with_nul()) {
				*to = *from as std::ffi::c_char;
			}
			let matching = hid_device_matching();
			// SAFETY: the call consumes the extra dictionary reference; the refcon is the signal `Monitor` keeps alive
			// until `Drop` destroys the port.
			let result = unsafe {
				IOServiceAddMatchingNotification(
					port,
					&mut name,
					Some(CFRetained::<CFDictionary>::from(&*matching)),
					Some(on_service_change),
					std::sync::Arc::as_ptr(&monitor.signal).cast_mut().cast(),
					iterator,
				)
			};
			if result != kIOReturnSuccess {
				return Err(format!(
					"Failed to subscribe to HID device changes: kern_return_t {result}. The most likely cause is that IOKit is unavailable."
				));
			}
			// Draining arms the notification. The services already present are not changes.
			drain(*iterator);
		}

		// Callbacks start only once the port has a queue, after the iterators above are armed.
		// SAFETY: the port is live and the queue outlives it.
		unsafe { IONotificationPort::set_dispatch_queue(port, Some(&monitor.queue)) };
		Ok(monitor)
	}

	pub(super) fn take_changed(&mut self) -> bool {
		self.signal.take()
	}
}

impl Drop for Monitor {
	fn drop(&mut self) {
		/// The `Notifications` struct moves the port and iterators onto the queue that owns their callbacks.
		struct Notifications(IONotificationPortRef, [io_iterator_t; 2]);
		// SAFETY: the port and iterators are only touched on the queue below, which serializes them with callbacks.
		unsafe impl Send for Notifications {}

		impl Notifications {
			fn destroy(self) {
				for iterator in self.1 {
					IOObjectRelease(iterator);
				}
				// SAFETY: the port came from `IONotificationPort::create` and is destroyed exactly once.
				unsafe { IONotificationPort::destroy(self.0) };
			}
		}

		let notifications = Notifications(self.port, self.iterators);
		// Tearing down on the queue waits for a running callback, so none runs after the signal is freed.
		self.queue.exec_sync(move || notifications.destroy());
	}
}

/// Releases every service an iterator holds, which also re-arms its notification.
fn drain(iterator: io_iterator_t) {
	while let Some(service) = Some(IOIteratorNext(iterator)).filter(|service| *service != 0) {
		IOObjectRelease(service);
	}
}

/// Records a HID service that appeared or terminated.
unsafe extern "C-unwind" fn on_service_change(refcon: *mut c_void, iterator: io_iterator_t) {
	drain(iterator);
	// SAFETY: `refcon` is the `ChangeSignal` that `Monitor` keeps alive until its port is destroyed.
	unsafe { &*refcon.cast::<ChangeSignal>() }.signal();
}

/// How many reports a device keeps between reads; older ones are dropped first.
const QUEUED_REPORTS: usize = 16;

/// The `Device` struct keeps an `IOHIDDevice` open with its reports delivered in a private run loop mode.
///
/// Only [`Device::read`] runs that mode, so reports arrive on the opening thread and only while it reads.
pub(super) struct Device {
	device: CFRetained<IOHIDDevice>,
	run_loop: CFRetained<CFRunLoop>,
	mode: CFRetained<CFString>,
	/// The buffer IOKit writes each report into before it calls [`queue_report`].
	incoming: NonNull<[u8]>,
	/// The reports waiting to be read, owned through this pointer because the callback also writes to them.
	queue: NonNull<ReportQueue>,
}

impl Device {
	pub(super) fn open(path: &PathData) -> Result<Self, String> {
		// SAFETY: `IORegistryEntryIDMatching` only builds a dictionary from the ID.
		let matching = unsafe { IORegistryEntryIDMatching(*path) }.ok_or_else(|| {
			"Failed to create a HID device lookup. The most likely cause is that the process is out of memory.".to_string()
		})?;
		// SAFETY: the call consumes the extra reference passed to it; `Object` releases the service.
		let service = Object(unsafe {
			IOServiceGetMatchingService(kIOMainPortDefault, Some(CFRetained::<CFDictionary>::from(&*matching)))
		});
		if service.0 == 0 {
			return Err(format!(
				"Failed to find HID device DevSrvsID:{path}. The most likely cause is that the device was unplugged."
			));
		}
		let device = IOHIDDevice::new(None, service.0).ok_or_else(|| {
			format!("Failed to create HID device DevSrvsID:{path}. The most likely cause is that the device was unplugged.")
		})?;
		let result = device.open(0);
		if result != kIOReturnSuccess {
			return Err(format!(
				"Failed to open HID device DevSrvsID:{path}: IOReturn {result:#x}. The most likely cause is that another application seized the device."
			));
		}

		let report_size = device
			.property(&CFString::from_static_str("MaxInputReportSize"))
			.and_then(|size| size.downcast_ref::<CFNumber>()?.as_i32())
			.filter(|size| *size > 0)
			.unwrap_or(64) as usize;
		let incoming = NonNull::from(Box::leak(vec![0u8; report_size].into_boxed_slice()));
		let queue = NonNull::from(Box::leak(Box::new(ReportQueue::new(report_size))));
		let run_loop = CFRunLoop::current().expect("Every thread has a run loop.");
		let mode = CFString::from_static_str("ByteEngineHID");

		// SAFETY: `incoming` and `queue` stay allocated until `Drop` unregisters the callback, and the callback only
		// runs inside `read`, which does not touch them while the run loop runs.
		unsafe {
			device.schedule_with_run_loop(&run_loop, &mode);
			device.register_input_report_callback(
				incoming.cast(),
				report_size as CFIndex,
				Some(queue_report),
				queue.as_ptr().cast(),
			);
		}

		Ok(Self {
			device,
			run_loop,
			mode,
			incoming,
			queue,
		})
	}

	/// Delivers the reports IOKit has waiting when none are queued, then returns the oldest queued report.
	pub(super) fn read(&mut self, report: &mut [u8]) -> Result<Option<usize>, String> {
		// SAFETY: the callback writes to the queue only while the run loop runs below, when no reference is held.
		if unsafe { self.queue.as_ref() }.is_empty() {
			CFRunLoop::run_in_mode(Some(&self.mode), 0.0, false);
		}
		// SAFETY: the run loop has returned, so the callback cannot write to the queue during this borrow.
		Ok(unsafe { self.queue.as_mut() }.pop(report))
	}
}

impl Drop for Device {
	fn drop(&mut self) {
		// SAFETY: unregistering and unscheduling stop further callbacks before their buffers are freed.
		unsafe {
			self.device.register_input_report_callback(
				self.incoming.cast(),
				self.incoming.len() as CFIndex,
				None,
				std::ptr::null_mut(),
			);
			self.device.unschedule_from_run_loop(&self.run_loop, &self.mode);
		}
		self.device.close(0);
		// SAFETY: both pointers came from `Box::leak` in `open` and nothing refers to them anymore.
		unsafe {
			drop(Box::from_raw(self.incoming.as_ptr()));
			drop(Box::from_raw(self.queue.as_ptr()));
		}
	}
}

/// The `ReportQueue` struct holds reports between reads in fixed slots, so receiving a report never allocates.
struct ReportQueue {
	/// `QUEUED_REPORTS` slots of `slot_size` bytes each.
	slots: Box<[u8]>,
	slot_size: usize,
	lengths: [usize; QUEUED_REPORTS],
	/// The slot of the oldest report.
	head: usize,
	count: usize,
}

impl ReportQueue {
	fn new(slot_size: usize) -> Self {
		Self {
			slots: vec![0; slot_size * QUEUED_REPORTS].into_boxed_slice(),
			slot_size,
			lengths: [0; QUEUED_REPORTS],
			head: 0,
			count: 0,
		}
	}

	fn is_empty(&self) -> bool {
		self.count == 0
	}

	/// Queues a copy of `report`, dropping the oldest report when every slot is full.
	fn push(&mut self, report: &[u8]) {
		if self.count == QUEUED_REPORTS {
			self.head = (self.head + 1) % QUEUED_REPORTS;
			self.count -= 1;
		}
		let slot = (self.head + self.count) % QUEUED_REPORTS;
		let length = report.len().min(self.slot_size);
		self.slots[slot * self.slot_size..][..length].copy_from_slice(&report[..length]);
		self.lengths[slot] = length;
		self.count += 1;
	}

	/// Moves the oldest report into `report` and returns its length, or `None` when the queue is empty.
	fn pop(&mut self, report: &mut [u8]) -> Option<usize> {
		if self.count == 0 {
			return None;
		}
		let length = self.lengths[self.head].min(report.len());
		report[..length].copy_from_slice(&self.slots[self.head * self.slot_size..][..length]);
		self.head = (self.head + 1) % QUEUED_REPORTS;
		self.count -= 1;
		Some(length)
	}
}

/// Queues an input report that IOKit delivered while [`Device::read`] ran the device's run loop mode.
unsafe extern "C-unwind" fn queue_report(
	context: *mut c_void,
	result: IOReturn,
	_sender: *mut c_void,
	_kind: IOHIDReportType,
	_report_id: u32,
	report: NonNull<u8>,
	length: CFIndex,
) {
	if result != kIOReturnSuccess || length <= 0 {
		return;
	}
	// SAFETY: `context` is the device's queue, which outlives the callback registration and has no other borrow
	// while the run loop runs; `report` holds `length` bytes for the duration of the call.
	unsafe {
		let report = std::slice::from_raw_parts(report.as_ptr(), length as usize);
		(*context.cast::<ReportQueue>()).push(report);
	}
}

/// Asks the kernel for the services that satisfy a matching dictionary from [`matching_dictionary`].
fn matching_services(matching: &CFRetained<CFDictionary>) -> Result<Object, String> {
	let mut iterator: io_object_t = 0;
	// SAFETY: the call consumes the extra reference passed to it and writes the iterator, which `Object` releases.
	let result = unsafe { IOServiceGetMatchingServices(kIOMainPortDefault, Some(matching.clone()), &mut iterator) };
	if result != 0 {
		return Err(format!(
			"Failed to look up HID services: kern_return_t {result}. The most likely cause is that IOKit is unavailable."
		));
	}
	Ok(Object(iterator))
}

/// The `Object` struct owns an IOKit object handle, such as a service or an iterator, releasing it when dropped.
struct Object(io_object_t);

impl Object {
	/// Returns the next object of an iterator, or `None` at its end.
	fn next(&self) -> Option<Object> {
		let next = IOIteratorNext(self.0);
		(next != 0).then_some(Object(next))
	}

	fn property(&self, key: &CFString) -> Option<CFRetained<CFType>> {
		// SAFETY: `self.0` is a live registry entry; the default allocator is passed as `None`.
		unsafe { IORegistryEntryCreateCFProperty(self.0, Some(key), None, 0) }
	}

	fn number(&self, key: &CFString) -> Option<i32> {
		self.property(key)?.downcast_ref::<CFNumber>()?.as_i32()
	}

	/// Copies a string property into `buffer` as UTF-8 and returns it.
	fn string<'b>(&self, key: &CFString, buffer: &'b mut [u8]) -> Option<&'b str> {
		let property = self.property(key)?;
		let string = property.downcast_ref::<CFString>()?;
		// SAFETY: the buffer size passed is the length of `buffer`.
		let copied = unsafe { string.c_string(buffer.as_mut_ptr().cast(), buffer.len() as isize, UTF8) };
		if !copied {
			return None;
		}
		CStr::from_bytes_until_nul(buffer).ok()?.to_str().ok()
	}
}

impl Drop for Object {
	fn drop(&mut self) {
		IOObjectRelease(self.0);
	}
}
