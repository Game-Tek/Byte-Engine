//! Device Removed Extended Data (DRED) setup and reporting.
//!
//! DRED makes a device removal debuggable. With it on, D3D12 records the last GPU operations each command list
//! completed (auto-breadcrumbs) and, on a page fault, which allocations lived at the faulting address. The engine turns
//! it on before it creates the device, then calls [`device_removed_report`] wherever it detects a removal.

use std::fmt::Write as _;

use windows::Win32::Graphics::Direct3D12::{
	CLSID_D3D12DeviceRemovedExtendedData, D3D12_AUTO_BREADCRUMB_NODE1, D3D12_DRED_ALLOCATION_NODE1,
	D3D12_DRED_ENABLEMENT_FORCED_ON, ID3D12Device10, ID3D12DeviceFactory, ID3D12DeviceRemovedExtendedData1,
	ID3D12DeviceRemovedExtendedDataSettings1,
};
use windows::core::{Interface, PCWSTR};

/// Operations listed around the last completed one for each command list, so the report stays readable.
const BREADCRUMB_CONTEXT: u32 = 8;

/// Turns on auto-breadcrumbs, breadcrumb context and page-fault reporting for devices the factory creates.
///
/// Must run before the factory creates the device. A runtime without DRED settings is not an error, because DRED only
/// adds diagnostics.
pub(super) fn enable(factory: &ID3D12DeviceFactory) {
	// SAFETY: The factory is live, and the configuration interface is only used for this call.
	let Ok(settings) = (unsafe {
		factory.GetConfigurationInterface::<ID3D12DeviceRemovedExtendedDataSettings1>(&CLSID_D3D12DeviceRemovedExtendedData)
	}) else {
		return;
	};
	// SAFETY: The settings interface is live and these calls only store enablement flags.
	unsafe {
		settings.SetAutoBreadcrumbsEnablement(D3D12_DRED_ENABLEMENT_FORCED_ON);
		settings.SetPageFaultEnablement(D3D12_DRED_ENABLEMENT_FORCED_ON);
		settings.SetBreadcrumbContextEnablement(D3D12_DRED_ENABLEMENT_FORCED_ON);
	}
}

/// Describes why the device was removed: the removal reason, the last operations each command list completed, and the
/// allocations near a faulting GPU address.
///
/// Call it after the device was removed. Without DRED, the report holds only the removal reason.
pub(crate) fn device_removed_report(device: &ID3D12Device10) -> String {
	let mut report = String::new();
	// SAFETY: The device is live for the call.
	let reason = unsafe { device.GetDeviceRemovedReason() };
	let _ = write!(report, "Device removed reason: {reason:?}.");

	let Ok(dred) = device.cast::<ID3D12DeviceRemovedExtendedData1>() else {
		report.push_str(" DRED is unavailable, so there are no breadcrumbs.");
		return report;
	};
	// SAFETY: The device was removed, so DRED's output is final. D3D12 owns the nodes until the device is released,
	// which outlives this call.
	unsafe {
		if let Ok(breadcrumbs) = dred.GetAutoBreadcrumbsOutput1() {
			write_breadcrumbs(&mut report, breadcrumbs.pHeadAutoBreadcrumbNode);
		}
		if let Ok(page_fault) = dred.GetPageFaultAllocationOutput1() {
			if page_fault.PageFaultVA != 0 {
				let _ = write!(report, "\nPage fault at GPU address {:#x}.", page_fault.PageFaultVA);
				write_allocations(
					&mut report,
					"Allocations at that address",
					page_fault.pHeadExistingAllocationNode,
				);
				write_allocations(
					&mut report,
					"Recently freed allocations at that address",
					page_fault.pHeadRecentFreedAllocationNode,
				);
			}
		}
	}
	report
}

/// Lists, for each command list that did not finish, the operations around the last one the GPU completed.
///
/// # Safety
/// `node` must be null or the head of a live DRED breadcrumb list.
unsafe fn write_breadcrumbs(report: &mut String, mut node: *const D3D12_AUTO_BREADCRUMB_NODE1) {
	while let Some(current) = unsafe { node.as_ref() } {
		node = current.pNext;
		let count = current.BreadcrumbCount;
		// SAFETY: DRED points the last value at live memory for each listed command list.
		let completed = unsafe { current.pLastBreadcrumbValue.as_ref() }.copied().unwrap_or(0);
		// Lists that completed every operation did not cause the removal.
		if completed >= count || current.pCommandHistory.is_null() {
			continue;
		}
		let list = unsafe { wide_name(current.pCommandListDebugNameW) };
		let queue = unsafe { wide_name(current.pCommandQueueDebugNameW) };
		let _ = write!(
			report,
			"\nCommand list '{list}' on queue '{queue}' completed {completed} of {count} operations:"
		);
		// SAFETY: The history holds `count` operations.
		let history = unsafe { std::slice::from_raw_parts(current.pCommandHistory, count as usize) };
		let contexts = if current.pBreadcrumbContexts.is_null() {
			&[][..]
		} else {
			// SAFETY: DRED sizes the context array by its count.
			unsafe { std::slice::from_raw_parts(current.pBreadcrumbContexts, current.BreadcrumbContextsCount as usize) }
		};
		let start = completed.saturating_sub(BREADCRUMB_CONTEXT);
		let end = (completed + BREADCRUMB_CONTEXT).min(count);
		for index in start..end {
			let marker = if index == completed {
				" <- first operation that did not complete"
			} else {
				""
			};
			let _ = write!(report, "\n  [{index}] {:?}", history[index as usize]);
			if let Some(context) = contexts.iter().find(|context| context.BreadcrumbIndex == index) {
				let _ = write!(report, " ({})", unsafe { wide_name(context.pContextString) });
			}
			report.push_str(marker);
		}
	}
}

/// Lists the allocations of a DRED allocation list under a heading.
///
/// # Safety
/// `node` must be null or the head of a live DRED allocation list.
unsafe fn write_allocations(report: &mut String, heading: &str, mut node: *const D3D12_DRED_ALLOCATION_NODE1) {
	if node.is_null() {
		return;
	}
	let _ = write!(report, "\n{heading}:");
	while let Some(current) = unsafe { node.as_ref() } {
		node = current.pNext;
		let _ = write!(
			report,
			"\n  '{}' ({:?})",
			unsafe { wide_name(current.ObjectNameW) },
			current.AllocationType
		);
	}
}

/// Reads a debug name DRED recorded, or a placeholder for an unnamed object.
///
/// # Safety
/// `name` must be null or a live NUL-terminated UTF-16 string.
unsafe fn wide_name(name: PCWSTR) -> String {
	if name.is_null() {
		return "unnamed".to_owned();
	}
	unsafe { name.to_string() }.unwrap_or_else(|_| "invalid name".to_owned())
}
