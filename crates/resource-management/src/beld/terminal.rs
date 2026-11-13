//! Adapt BELD's text to the streams it writes to: color, width, and a progress line that stays below log messages.
//!
//! The behavior is specified in the BELD command-line reference, under Terminals.

use std::{
	io::{IsTerminal, Write as _},
	sync::{Mutex, PoisonError},
};

use anstream::{AutoStream, ColorChoice, stream::RawStream};
use clap::builder::styling::Style;
use log::{Log, Metadata, Record};

/// The progress line at the bottom of standard error, or empty when none is shown.
static STATUS: Mutex<String> = Mutex::new(String::new());

/// Installs BELD's logger, which writes to standard error, reads its filter from `RUST_LOG`, and colors level names.
///
/// Set the process's color choice with [`anstream::ColorChoice::write_global`] first, because the logger reads it
/// once. Commands that show a progress line, such as [`crate::beld::bake`], need this logger to keep log messages above it.
pub fn init_logging() {
	let color = has_color(&std::io::stderr());
	// `colored`, which colors the level names, checks the terminal on its own; BELD's decision replaces that check.
	colored::control::set_override(color);
	let logger = simple_logger::SimpleLogger::new().env().with_colors(color);
	log::set_max_level(logger.max_level());
	let _ = log::set_boxed_logger(Box::new(StatusLogger(logger)));
}

/// Returns whether text written to `stream` has color, as `--color` and the color environment variables decide.
pub(crate) fn has_color(stream: &impl RawStream) -> bool {
	AutoStream::choice(stream) != ColorChoice::Never
}

/// Returns `style` when the output has color, or a style that writes no escape codes otherwise.
pub(crate) fn paint(style: Style, color: bool) -> Style {
	if color { style } else { Style::new() }
}

/// Returns the width in columns of the terminal `stream` writes to, or `None` when it isn't a terminal.
pub(crate) fn width(stream: &impl IsTerminal) -> Option<usize> {
	if !stream.is_terminal() {
		return None;
	}
	ratatui::crossterm::terminal::size()
		.ok()
		.map(|(columns, _)| usize::from(columns))
		.filter(|&columns| columns > 0)
}

/// Shows `line` as the progress line at the bottom of standard error, replacing the previous one. An empty line
/// removes it.
///
/// Write only lines that fit the terminal: a wrapped line can't be erased.
pub(crate) fn show_status(line: &str) {
	let mut status = STATUS.lock().unwrap_or_else(PoisonError::into_inner);
	status.clear();
	status.push_str(line);
	let mut stderr = std::io::stderr().lock();
	// Return to the start of the line and erase it before writing the new one.
	let _ = write!(stderr, "\r\x1b[2K{line}");
	let _ = stderr.flush();
}

/// The `StatusLogger` struct lets log messages and the progress line share standard error, so a message never lands
/// in the middle of the line.
struct StatusLogger(simple_logger::SimpleLogger);

impl Log for StatusLogger {
	fn enabled(&self, metadata: &Metadata) -> bool {
		self.0.enabled(metadata)
	}

	/// Erases the progress line, writes the message, and draws the line again below it.
	fn log(&self, record: &Record) {
		if !self.enabled(record.metadata()) {
			return;
		}
		let status = STATUS.lock().unwrap_or_else(PoisonError::into_inner);
		if status.is_empty() {
			self.0.log(record);
			return;
		}
		// The standard error lock is reentrant, so the inner logger can write while this thread holds it.
		let mut stderr = std::io::stderr().lock();
		let _ = write!(stderr, "\r\x1b[2K");
		self.0.log(record);
		let _ = write!(stderr, "{status}");
		let _ = stderr.flush();
	}

	fn flush(&self) {
		self.0.flush();
	}
}
