//! Keeps log records in memory while a full-screen console owns the terminal.
//!
//! The engine's logger writes to standard error, which would draw over a full-screen
//! [console](crate::inspector::console). While the console runs, it calls [`begin`], the logger sends each record
//! here instead, and the console draws the kept entries. [`end`] hands them back so the console can write them to
//! standard error once the terminal is restored.

use std::{
	collections::VecDeque,
	fmt::Write as _,
	sync::{
		Mutex, PoisonError,
		atomic::{AtomicBool, AtomicU64, Ordering},
	},
	time::{SystemTime, UNIX_EPOCH},
};

/// The most entries kept; older entries are dropped first.
pub(crate) const KEPT_ENTRIES: usize = 10_000;

/// The `LineKind` enum tells the console how to style one kept entry.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum LineKind {
	/// A log record at this level.
	Log(log::Level),
	/// A command the console ran, or what it reported.
	Console,
}

/// The `Entry` struct is one kept entry: a log record or a console report, whose text may span several lines.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Entry {
	pub(crate) kind: LineKind,
	pub(crate) text: String,
}

/// Whether log records come here instead of standard error. Read on every record, so it stays outside the lock.
static ACTIVE: AtomicBool = AtomicBool::new(false);
/// Counts every entry pushed, so the console redraws only when something arrived.
static REVISION: AtomicU64 = AtomicU64::new(0);
static ENTRIES: Mutex<VecDeque<Entry>> = Mutex::new(VecDeque::new());

/// Sends log records here from now on.
pub(crate) fn begin() {
	ACTIVE.store(true, Ordering::SeqCst);
}

/// Sends log records to standard error again and returns every kept entry, oldest first.
pub(crate) fn end() -> VecDeque<Entry> {
	ACTIVE.store(false, Ordering::SeqCst);
	std::mem::take(&mut *entries())
}

/// Reports whether log records come here instead of standard error.
pub(crate) fn is_active() -> bool {
	ACTIVE.load(Ordering::Relaxed)
}

/// Returns a number that changes whenever an entry is pushed.
pub(crate) fn revision() -> u64 {
	REVISION.load(Ordering::SeqCst)
}

/// Calls `f` with the kept entries, oldest first. Don't log from `f`: the logger would wait for the entries `f` holds.
pub(crate) fn with_entries<R>(f: impl FnOnce(&VecDeque<Entry>) -> R) -> R {
	f(&entries())
}

/// Where the level sits in a record's text: after the `HH:MM:SS ` time, padded to five characters.
pub(crate) const LEVEL: std::ops::Range<usize> = 9..14;

/// Keeps `record`, prefixed with the time, the level at [`LEVEL`], and the target.
pub(crate) fn push_record(record: &log::Record) {
	let mut text = String::new();
	// UTC wall-clock time of day, as the standard error logger shows it.
	let seconds = SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |time| time.as_secs()) % 86_400;
	let _ = write!(
		text,
		"{:02}:{:02}:{:02} {:<5} {}  {}",
		seconds / 3_600,
		seconds / 60 % 60,
		seconds % 60,
		record.level(),
		record.target(),
		record.args()
	);
	push(LineKind::Log(record.level()), text);
}

/// Keeps `text` as one entry.
pub(crate) fn push(kind: LineKind, text: String) {
	let mut kept = entries();
	if kept.len() == KEPT_ENTRIES {
		kept.pop_front();
	}
	kept.push_back(Entry { kind, text });
	REVISION.fetch_add(1, Ordering::SeqCst);
}

/// Locks the kept entries; a thread that panicked while holding them left them whole, so the poison is ignored.
fn entries() -> std::sync::MutexGuard<'static, VecDeque<Entry>> {
	ENTRIES.lock().unwrap_or_else(PoisonError::into_inner)
}
