//! Keeps the application's recent log records in memory, so in-process tools can show them.
//!
//! The engine's logger keeps every record it lets through here, numbered in order, and also writes it to standard
//! error. Tools read what arrived since the last number they saw with [`entries_since`], such as the
//! [editor](crate::editor), which shows warnings and errors as toasts.
//!
//! Standard error would draw over a full-screen [console](crate::inspector::console). While the console runs, it calls
//! [`begin`], and records are only kept; the console draws the kept entries. [`end`] hands back those that weren't
//! written, so the console can write them to standard error once the terminal is restored.

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

/// Whether log records are only kept, not written to standard error. Read on every record, so it stays outside the
/// lock.
static ACTIVE: AtomicBool = AtomicBool::new(false);
/// Counts every entry pushed, which numbers them: the newest kept entry is number `REVISION`. Readers compare it to
/// the last number they saw to tell what arrived.
static REVISION: AtomicU64 = AtomicU64::new(0);
/// The revision [`begin`] started at, so [`end`] hands back only what standard error missed.
static BEGUN_AT: AtomicU64 = AtomicU64::new(0);
static ENTRIES: Mutex<VecDeque<Entry>> = Mutex::new(VecDeque::new());

/// Only keeps log records from now on, without writing them to standard error.
pub(crate) fn begin() {
	BEGUN_AT.store(revision(), Ordering::SeqCst);
	ACTIVE.store(true, Ordering::SeqCst);
}

/// Writes log records to standard error again and returns the entries kept since [`begin`], oldest first.
pub(crate) fn end() -> Vec<Entry> {
	ACTIVE.store(false, Ordering::SeqCst);
	let mut missed = Vec::new();
	entries_since(BEGUN_AT.load(Ordering::SeqCst), |entry| missed.push(entry.clone()));
	missed
}

/// Reports whether log records are only kept, not written to standard error.
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

/// Calls `f` with each kept entry pushed after number `seen`, oldest first, and returns the newest entry's number, to
/// pass as `seen` next time. Entries dropped to make room are skipped. Don't log from `f`, as for [`with_entries`].
pub(crate) fn entries_since(seen: u64, mut f: impl FnMut(&Entry)) -> u64 {
	let kept = entries();
	// Read under the lock, which pushes hold while they count, so the number matches the entries.
	let newest = revision();
	let unseen = newest.saturating_sub(seen).min(kept.len() as u64) as usize;
	kept.range(kept.len() - unseen..).for_each(&mut f);
	newest
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

impl Entry {
	/// Returns a log record's message, without the time, level, and target [`push_record`] put before it. A console
	/// entry is all message.
	pub(crate) fn message(&self) -> &str {
		match self.kind {
			LineKind::Log(_) => self
				.text
				.get(LEVEL.end..)
				.and_then(|rest| rest.split_once("  "))
				.map_or(self.text.as_str(), |(_, message)| message),
			LineKind::Console => &self.text,
		}
	}
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

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn a_reader_gets_each_entry_once_in_order_with_its_message() {
		let seen = entries_since(revision(), |_| {});
		push(
			LineKind::Log(log::Level::Warn),
			"12:00:00 WARN  app::scene  It broke. Fix it.".to_string(),
		);
		push(LineKind::Console, "> help".to_string());

		let mut read = Vec::new();
		let seen = entries_since(seen, |entry| read.push((entry.kind, entry.message().to_string())));
		assert_eq!(
			read,
			[
				(LineKind::Log(log::Level::Warn), "It broke. Fix it.".to_string()),
				(LineKind::Console, "> help".to_string()),
			]
		);

		let mut again = 0;
		entries_since(seen, |_| again += 1);
		assert_eq!(again, 0, "a reader that caught up reads nothing again");
	}
}
