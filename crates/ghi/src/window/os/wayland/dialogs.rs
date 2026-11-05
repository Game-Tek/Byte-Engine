//! Opens desktop portal dialogs for [`crate::window::App::open_dialog`].
//!
//! Wayland has no dialogs of its own, so requests go over the D-Bus session bus to `xdg-desktop-portal`, which shows
//! the desktop's own file chooser and color picker. The portal answers each request with a `Response` signal on a
//! request object; the pump watches the bus connection's descriptor and turns those signals into events.

use std::{
	cell::RefCell,
	os::{fd::BorrowedFd, unix::ffi::OsStringExt as _},
	path::PathBuf,
	rc::Rc,
	time::Duration,
};

use dbus::{
	Message,
	arg::{PropMap, RefArg, Variant},
	blocking::{BlockingSender as _, LocalConnection},
	channel::{BusType, Channel},
	message::MatchRule,
};

use crate::window::{Dialog, DialogError, DialogId, DialogResult, FileFilter};

const PORTAL: &str = "org.freedesktop.portal.Desktop";
const PORTAL_PATH: &str = "/org/freedesktop/portal/desktop";
const FILE_CHOOSER: &str = "org.freedesktop.portal.FileChooser";
/// How long to wait for the portal to accept a request. The dialog itself answers later, through a signal.
const REQUEST_TIMEOUT: Duration = Duration::from_secs(5);

/// The `Portal` struct keeps the session bus connection that dialogs share, opened by the first dialog, and the
/// results its signal handlers collect until the pump takes them.
pub(super) struct Portal {
	connection: LocalConnection,
	results: Rc<RefCell<Vec<(DialogId, DialogResult)>>>,
}

impl Portal {
	pub(super) fn connect() -> Result<Self, DialogError> {
		let mut channel = Channel::get_private(BusType::Session).map_err(os_error)?;
		// The pump polls the connection's descriptor next to the display socket.
		channel.set_watch_enabled(true);
		Ok(Portal {
			connection: LocalConnection::from(channel),
			results: Rc::default(),
		})
	}

	/// The descriptor a waiting poll watches for portal signals.
	pub(super) fn fd(&self) -> BorrowedFd<'_> {
		// SAFETY: The descriptor belongs to the connection, which outlives the borrow.
		unsafe { BorrowedFd::borrow_raw(self.connection.channel().watch().fd) }
	}

	/// Handles every message the bus has delivered, without waiting, and returns the dialog results they carried.
	pub(super) fn drain(&self) -> impl Iterator<Item = (DialogId, DialogResult)> + '_ {
		// A broken connection leaves the remaining dialogs without results; there is no caller to report it to.
		while self.connection.process(Duration::ZERO).unwrap_or(false) {}
		let results = std::mem::take(&mut *self.results.borrow_mut());
		results.into_iter()
	}

	/// Sends `dialog` to the portal and arranges for its response to become a result.
	pub(super) fn open(&self, id: DialogId, dialog: &Dialog) -> Result<(), DialogError> {
		let token = format!("byte_engine_dialog_{}", id.0);
		let mut options = PropMap::new();
		options.insert("handle_token".into(), variant(token.clone()));

		// Each portal method takes the parent window, then the title when it shows one, then the options. An empty
		// parent opens the dialog on its own; attaching it needs the toplevel exported through xdg-foreign.
		let call = |interface: &str, method: &str| {
			Message::new_method_call(PORTAL, PORTAL_PATH, interface, method)
				.expect("Portal method names are invalid. The constants in this module are malformed. Fix their spelling.")
				.append1("")
		};
		let (message, read): (Message, Reader) = match *dialog {
			Dialog::Open {
				title,
				directory,
				filters,
				multiple,
				folders,
			} => {
				options.insert("multiple".into(), variant(multiple));
				options.insert("directory".into(), variant(folders));
				insert_files(&mut options, directory, if folders { &[] } else { filters });
				(call(FILE_CHOOSER, "OpenFile").append1(title), Box::new(files))
			}
			Dialog::Save {
				title,
				directory,
				file_name,
				filters,
			} => {
				options.insert("current_name".into(), variant(file_name.to_owned()));
				insert_files(&mut options, directory, filters);
				(call(FILE_CHOOSER, "SaveFile").append1(title), Box::new(files))
			}
			Dialog::Color { initial, .. } => {
				// The portal picks a color from the screen and has no alpha, so alpha stays as it started.
				let alpha = initial[3];
				(
					call("org.freedesktop.portal.Screenshot", "PickColor"),
					Box::new(move |results: &PropMap| color(results, alpha)),
				)
			}
			Dialog::Message { .. } => return Err(DialogError::Unsupported),
		};

		// The portal names the request object after the sender and the token. Subscribing before the call ensures a
		// fast response is not missed.
		let sender = self.connection.unique_name().trim_start_matches(':').replace('.', "_");
		let rule = MatchRule::new_signal("org.freedesktop.portal.Request", "Response")
			.with_path(format!("{PORTAL_PATH}/request/{sender}/{token}"));
		let results = self.results.clone();
		let subscription = self
			.connection
			.add_match(rule, move |(response, values): (u32, PropMap), _, _| {
				// Response 0 means the user chose, 1 that they cancelled, and 2 that the portal ended the request.
				let result = match response {
					0 => read(&values),
					1 => DialogResult::Cancelled,
					_ => DialogResult::Failed(DialogError::Os("the desktop portal ended the request".into())),
				};
				results.borrow_mut().push((id, result));
				// Each request answers once, so the match is removed.
				false
			})
			.map_err(os_error)?;

		if let Err(error) = self
			.connection
			.send_with_reply_and_block(message.append1(options), REQUEST_TIMEOUT)
		{
			let _ = self.connection.remove_match(subscription);
			return Err(os_error(error));
		}
		Ok(())
	}
}

/// Turns a successful response's values into the dialog's result.
type Reader = Box<dyn Fn(&PropMap) -> DialogResult>;

fn variant<T: RefArg + 'static>(value: T) -> Variant<Box<dyn RefArg>> {
	Variant(Box::new(value))
}

fn os_error(error: dbus::Error) -> DialogError {
	DialogError::Os(error.message().unwrap_or("the session bus refused the request").to_owned())
}

/// Adds the starting folder and the filters that open and save requests share.
fn insert_files(options: &mut PropMap, directory: Option<&std::path::Path>, filters: &[FileFilter]) {
	if let Some(directory) = directory {
		// The portal takes the folder as a null-terminated byte string.
		let mut bytes = directory.as_os_str().as_encoded_bytes().to_vec();
		bytes.push(0);
		options.insert("current_folder".into(), variant(bytes));
	}
	if !filters.is_empty() {
		// Each filter is a name and a list of patterns, where kind 0 is a glob.
		let filters: Vec<(String, Vec<(u32, String)>)> = filters
			.iter()
			.map(|filter| {
				let globs = filter
					.extensions
					.iter()
					.map(|extension| (0, format!("*.{extension}")))
					.collect();
				(filter.name.to_owned(), globs)
			})
			.collect();
		options.insert("filters".into(), variant(filters));
	}
}

/// Reads the `uris` of a file chooser response as paths.
fn files(results: &PropMap) -> DialogResult {
	let paths = results
		.get("uris")
		.and_then(|uris| uris.0.as_iter())
		.into_iter()
		.flatten()
		.filter_map(|uri| uri.as_str())
		.filter_map(file_uri_path)
		.collect();
	DialogResult::Files(paths)
}

/// Reads the `color` of a color picker response, three doubles from 0 to 1.
fn color(results: &PropMap, alpha: f32) -> DialogResult {
	let mut channels = results
		.get("color")
		.and_then(|color| color.0.as_iter())
		.into_iter()
		.flatten()
		.filter_map(|channel| channel.as_f64());
	let mut next = || channels.next().unwrap_or(0.0) as f32;
	DialogResult::Color {
		color: [next(), next(), next(), alpha],
		done: true,
	}
}

/// Decodes a `file://` URI into a path, undoing percent escapes byte by byte, since paths need not be UTF-8.
fn file_uri_path(uri: &str) -> Option<PathBuf> {
	let encoded = uri.strip_prefix("file://")?;
	let mut bytes = Vec::with_capacity(encoded.len());
	let mut rest = encoded.as_bytes();
	while let Some((&byte, tail)) = rest.split_first() {
		if byte == b'%'
			&& let Some(hex) = tail.get(..2)
			&& let Ok(value) = u8::from_str_radix(std::str::from_utf8(hex).ok()?, 16)
		{
			bytes.push(value);
			rest = &tail[2..];
		} else {
			bytes.push(byte);
			rest = tail;
		}
	}
	Some(PathBuf::from(std::ffi::OsString::from_vec(bytes)))
}
