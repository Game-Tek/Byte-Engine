//! Describes the operating system dialogs an [`App`](crate::window::App) opens and the results they send back.
//!
//! Open one with [`App::open_dialog`](crate::window::App::open_dialog), then match
//! [`AppEvents::Dialog`](crate::window::AppEvents::Dialog) events from [`App::poll`](crate::window::App::poll) by
//! their [`DialogId`].

use std::path::{Path, PathBuf};

/// The `DialogId` struct ties the [`AppEvents::Dialog`](crate::window::AppEvents::Dialog) events of one dialog to
/// the [`App::open_dialog`](crate::window::App::open_dialog) call that opened it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(transparent)]
pub struct DialogId(pub(crate) std::num::NonZeroU64);

impl DialogId {
	pub(crate) fn from_raw(raw: u64) -> Self {
		Self(std::num::NonZeroU64::new(raw).expect(
			"Dialog identity is zero. The dialog counter wrapped or was never started. Start dialog identities at one.",
		))
	}
}

/// A dialog request for [`App::open_dialog`](crate::window::App::open_dialog).
///
/// The request borrows its text and filters only for the call; nothing is kept after it returns.
#[derive(Debug, Clone, Copy)]
pub enum Dialog<'a> {
	/// Picks existing files or folders. Its final result is [`DialogResult::Files`] with at least one path.
	Open {
		title: &'a str,
		/// The folder the dialog starts in. `None` lets the platform choose.
		directory: Option<&'a Path>,
		/// The file kinds the user may pick. An empty slice accepts every file.
		filters: &'a [FileFilter<'a>],
		/// Whether the user may pick more than one item.
		multiple: bool,
		/// Whether the user picks folders instead of files. Filters do not apply to folders.
		folders: bool,
	},
	/// Picks the path of a file to write. Its final result is [`DialogResult::Files`] with exactly one path.
	Save {
		title: &'a str,
		/// The folder the dialog starts in. `None` lets the platform choose.
		directory: Option<&'a Path>,
		/// The file name the dialog suggests.
		file_name: &'a str,
		/// The file kinds the user may save as. An empty slice accepts every file.
		filters: &'a [FileFilter<'a>],
	},
	/// Picks a color. It sends [`DialogResult::Color`] results, the last of them with `done` set.
	Color {
		/// The color the dialog starts with, as sRGB-encoded red, green, blue, and alpha from 0 to 1.
		initial: [f32; 4],
		/// Whether the user may change alpha. When not, every result keeps `initial`'s alpha.
		alpha: bool,
	},
	/// Shows a message and asks the user to press a button. Its final result is [`DialogResult::Button`].
	Message {
		level: MessageLevel,
		title: &'a str,
		text: &'a str,
		buttons: Buttons,
	},
}

/// The `FileFilter` struct names one kind of file an [`Dialog::Open`] or [`Dialog::Save`] dialog offers.
#[derive(Debug, Clone, Copy)]
pub struct FileFilter<'a> {
	/// The name the dialog shows, such as `"Images"`.
	pub name: &'a str,
	/// The file name extensions the kind covers, without the leading period, such as `["png", "jpg"]`.
	pub extensions: &'a [&'a str],
}

/// How serious a [`Dialog::Message`] is. Platforms pick the dialog's icon from it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MessageLevel {
	Info,
	Warning,
	Error,
}

/// The buttons a [`Dialog::Message`] offers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Buttons {
	Ok,
	OkCancel,
	YesNo,
	YesNoCancel,
}

/// A button the user pressed on a [`Dialog::Message`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Button {
	Ok,
	Cancel,
	Yes,
	No,
}

/// A result a dialog sends through [`AppEvents::Dialog`](crate::window::AppEvents::Dialog).
///
/// Every dialog sends exactly one final result: [`Self::Files`], [`Self::Button`], [`Self::Cancelled`],
/// [`Self::Failed`], or [`Self::Color`] with `done` set. A color dialog may send results with `done` unset before it.
#[derive(Debug, Clone, PartialEq)]
pub enum DialogResult {
	/// The paths the user picked in an [`Dialog::Open`] or [`Dialog::Save`] dialog.
	Files(Vec<PathBuf>),
	/// A color from a [`Dialog::Color`] dialog, as sRGB-encoded red, green, blue, and alpha from 0 to 1.
	///
	/// `done` is unset while the user still edits the color, and set on the dialog's final result.
	Color { color: [f32; 4], done: bool },
	/// The button the user pressed on a [`Dialog::Message`].
	Button(Button),
	/// The user dismissed the dialog without choosing.
	Cancelled,
	/// The dialog opened but the operating system failed to show or read it, so the user could not choose.
	Failed(DialogError),
}

/// Why [`App::open_dialog`](crate::window::App::open_dialog) could not open a dialog, or why an open one
/// [failed](DialogResult::Failed).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DialogError {
	/// The platform has no dialog of the requested kind.
	Unsupported,
	/// The platform shows one dialog of the requested kind at a time, and one is open.
	Busy,
	/// The operating system refused to show the dialog. Carries its message.
	Os(String),
}

impl std::fmt::Display for DialogError {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		match self {
			DialogError::Unsupported => write!(
				f,
				"The dialog could not open. This platform has no dialog of the requested kind. Show the choice in the application's own interface instead."
			),
			DialogError::Busy => write!(
				f,
				"The dialog could not open. The platform shows one dialog of this kind at a time and one is already open. Wait for the open dialog's final result, then open the next."
			),
			DialogError::Os(message) => write!(
				f,
				"The dialog could not open: {message}. The operating system refused to show it, usually because its dialog service is missing or not running. Check that the platform's dialog service is installed and running."
			),
		}
	}
}

impl std::error::Error for DialogError {}
