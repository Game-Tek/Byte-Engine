//! Facts about the machine the process runs on, for sizing parallel work.

use std::num::NonZero;

/// Returns the number of hardware threads on the machine's fastest cores.
///
/// Many processors pair a few fast cores with slower ones. Work that every lane has to finish before the next
/// stage starts, such as an [`Alley::execute_phased`](crate::core::alley::Alley::execute_phased) algorithm, runs
/// at the pace of its slowest lane, so it should use no more lanes than this. When the cores are all alike, or the
/// operating system does not tell them apart, this is every hardware thread available to the process.
///
/// Apple silicon reports its performance cores. On Linux, the count comes from the kernel's lists of hybrid cores
/// or its per-core capacities. Other systems report every hardware thread.
pub fn performance_parallelism() -> usize {
	let available = std::thread::available_parallelism().map_or(1, NonZero::get);
	fastest_core_threads()
		.or_else(sysfs_fastest_core_threads)
		// A process restricted to fewer threads than the machine has fast ones cannot use more than its own.
		.map_or(available, |fastest| fastest.clamp(1, available))
}

/// Returns the number of hardware threads on Apple silicon's performance cores.
#[cfg(target_os = "macos")]
#[allow(unsafe_code, reason = "Reading a system control value takes a call into the C library.")]
fn fastest_core_threads() -> Option<usize> {
	let mut count: libc::c_int = 0;
	let mut size = size_of::<libc::c_int>();
	// Performance level zero is the fastest class of cores. The name does not exist on Intel Macs.
	// SAFETY: The name is a NUL-terminated string, `count` is valid for `size` bytes, and `size` holds that length.
	// No new value is passed, so the null pointer and zero length are what the call expects.
	let status = unsafe {
		libc::sysctlbyname(
			c"hw.perflevel0.logicalcpu".as_ptr(),
			(&raw mut count).cast(),
			&raw mut size,
			std::ptr::null_mut(),
			0,
		)
	};
	(status == 0 && size == size_of::<libc::c_int>())
		.then(|| usize::try_from(count).ok())
		.flatten()
		.filter(|count| *count > 0)
}

/// Returns `None` where no system call tells fast cores from slow ones.
#[cfg(not(target_os = "macos"))]
fn fastest_core_threads() -> Option<usize> {
	None
}

/// Returns the number of hardware threads on the fastest cores from the Linux kernel's device files.
///
/// Returns `None` when the files are missing, as on other systems and on kernels that do not rank cores.
fn sysfs_fastest_core_threads() -> Option<usize> {
	// Hybrid x86 processors list the hardware threads of their performance cores.
	if let Ok(list) = std::fs::read_to_string("/sys/devices/cpu_core/cpus") {
		return count_listed_cpus(&list);
	}
	// Arm processors rank each core by capacity instead. Cores are numbered from zero without gaps.
	let capacities = (0..)
		.map_while(|cpu| std::fs::read_to_string(format!("/sys/devices/system/cpu/cpu{cpu}/cpu_capacity")).ok())
		.filter_map(|capacity| capacity.trim().parse::<u32>().ok())
		.collect::<Vec<_>>();
	count_fastest(&capacities)
}

/// Returns how many processors a kernel processor list such as `0-7,16-23` names.
fn count_listed_cpus(list: &str) -> Option<usize> {
	let mut count = 0;
	for range in list.trim().split(',').filter(|range| !range.is_empty()) {
		let (first, last) = range.split_once('-').unwrap_or((range, range));
		let (first, last) = (first.trim().parse::<usize>().ok()?, last.trim().parse::<usize>().ok()?);
		count += last.checked_sub(first)? + 1;
	}
	(count > 0).then_some(count)
}

/// Returns how many of `capacities` equal the largest one.
fn count_fastest(capacities: &[u32]) -> Option<usize> {
	let fastest = capacities.iter().copied().max()?;
	Some(capacities.iter().filter(|capacity| **capacity == fastest).count())
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn performance_parallelism_is_within_the_available_threads() {
		let available = std::thread::available_parallelism().map_or(1, NonZero::get);

		assert!((1..=available).contains(&performance_parallelism()));
	}

	#[test]
	fn processor_lists_count_ranges_and_single_processors() {
		assert_eq!(count_listed_cpus("0-7,16-23\n"), Some(16));
		assert_eq!(count_listed_cpus("0,2,4-5"), Some(4));
		assert_eq!(count_listed_cpus("3"), Some(1));
	}

	#[test]
	fn malformed_processor_lists_count_nothing() {
		assert_eq!(count_listed_cpus(""), None);
		assert_eq!(count_listed_cpus("fast"), None);
		assert_eq!(count_listed_cpus("7-0"), None);
	}

	#[test]
	fn the_fastest_cores_share_the_largest_capacity() {
		assert_eq!(count_fastest(&[512, 512, 512, 512, 1024, 1024]), Some(2));
		assert_eq!(count_fastest(&[1024, 1024, 1024, 1024]), Some(4));
		assert_eq!(count_fastest(&[]), None);
	}
}
