//! Running out of file handles: telling it apart from other read and write failures, so each failure names it.
//!
//! Once the process has no free file handle, every file it opens fails. An error that only says what it was doing
//! would blame a missing asset, a misspelled module, or a full disk, so every failure that [`exhausted`] recognizes
//! says that the process ran out of file handles, with [`CAUSE_AND_FIX`]. Where the failure carries no message of
//! its own, [`report`] logs one.

/// The cause and fix that every error caused by running out of file handles shares.
pub(crate) const CAUSE_AND_FIX: &str = "The most likely cause is the operating system's limit on open files, which macOS terminals set to 256 by default, or files the process left open. Raise the limit before you start the application, such as with `ulimit -n 10240`; if it happens again with a high limit, the engine leaks files.";

/// Returns whether `error` says that the process, or the whole system, has no free file handle.
pub(crate) fn exhausted(error: &std::io::Error) -> bool {
	// EMFILE is the process's limit and ENFILE the system's. Windows reports both as ERROR_TOO_MANY_OPEN_FILES.
	#[cfg(unix)]
	const CODES: &[i32] = &[24, 23];
	#[cfg(windows)]
	const CODES: &[i32] = &[4];
	#[cfg(not(any(unix, windows)))]
	const CODES: &[i32] = &[];
	error.raw_os_error().is_some_and(|code| CODES.contains(&code))
}

/// Logs that `action` failed because the process ran out of file handles, when `error` says so, and returns whether
/// it did.
///
/// Call it where an I/O error is turned into one that carries no message, so the failure still names its cause.
/// `action` completes "Could not …", such as `create the staging file 'x.tmp'`.
pub(crate) fn report(error: &std::io::Error, action: impl std::fmt::Display) -> bool {
	if !exhausted(error) {
		return false;
	}
	log::error!("Could not {action}, because the process ran out of file handles. {CAUSE_AND_FIX} Error: {error}");
	true
}

#[cfg(test)]
mod tests {
	use super::exhausted;

	#[test]
	fn only_the_open_file_limits_count_as_exhausted() {
		#[cfg(unix)]
		let limits = [24, 23];
		#[cfg(windows)]
		let limits = [4];
		for code in limits {
			assert!(exhausted(&std::io::Error::from_raw_os_error(code)));
		}
		assert!(!exhausted(&std::io::Error::from(std::io::ErrorKind::NotFound)));
		assert!(!exhausted(&std::io::Error::from(std::io::ErrorKind::PermissionDenied)));
	}
}
