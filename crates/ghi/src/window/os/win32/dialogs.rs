//! Opens Win32 common dialogs for [`crate::window::App::open_dialog`].
//!
//! Every common dialog runs its own modal message loop until the user closes it, so each one runs on a thread of its
//! own. The thread sends its result through a channel and wakes the pump, which turns it into an event.

use std::path::PathBuf;
use std::sync::mpsc::Sender;

use windows::Win32::Foundation::{COLORREF, HWND};
use windows::Win32::System::Com::{
	CLSCTX_INPROC_SERVER, COINIT_APARTMENTTHREADED, COINIT_DISABLE_OLE1DDE, CoCreateInstance, CoInitializeEx, CoTaskMemFree,
	CoUninitialize,
};
use windows::Win32::UI::Controls::Dialogs::{CC_FULLOPEN, CC_RGBINIT, CHOOSECOLORW, ChooseColorW};
use windows::Win32::UI::Shell::Common::COMDLG_FILTERSPEC;
use windows::Win32::UI::Shell::{
	FOS_ALLOWMULTISELECT, FOS_FORCEFILESYSTEM, FOS_PICKFOLDERS, FileOpenDialog, FileSaveDialog, IFileDialog, IFileOpenDialog,
	IFileSaveDialog, IShellItem, SHCreateItemFromParsingName, SIGDN_FILESYSPATH,
};
use windows::Win32::UI::WindowsAndMessaging::{
	IDNO, IDOK, IDYES, MB_ICONERROR, MB_ICONINFORMATION, MB_ICONWARNING, MB_OK, MB_OKCANCEL, MB_YESNO, MB_YESNOCANCEL,
	MESSAGEBOX_STYLE, MessageBoxW,
};
use windows::core::{HSTRING, Interface, PCWSTR};

use super::AppWaker;
use crate::window::{Button, Buttons, Dialog, DialogError, DialogId, DialogResult, MessageLevel};

/// The results dialog threads send to the pump.
pub(super) type Results = Sender<(DialogId, DialogResult)>;

/// An owned copy of a [`Dialog`], since the request's borrows end before its thread runs.
enum Request {
	Open {
		title: HSTRING,
		directory: Option<HSTRING>,
		filters: Vec<(HSTRING, HSTRING)>,
		multiple: bool,
		folders: bool,
	},
	Save {
		title: HSTRING,
		directory: Option<HSTRING>,
		file_name: HSTRING,
		filters: Vec<(HSTRING, HSTRING)>,
	},
	Color {
		initial: [f32; 4],
	},
	Message {
		style: MESSAGEBOX_STYLE,
		title: HSTRING,
		text: HSTRING,
	},
}

/// Starts `dialog` on its own thread, owned by `parent` when given.
pub(super) fn open(
	results: &Results,
	waker: AppWaker,
	id: DialogId,
	dialog: &Dialog,
	parent: Option<HWND>,
) -> Result<(), DialogError> {
	let request = request(dialog);
	let results = results.clone();
	// Window handles are not `Send`, but any thread may own a dialog with one, so the raw value crosses.
	let parent = parent.map(|hwnd| hwnd.0 as isize);
	std::thread::Builder::new()
		.name("dialog".into())
		.spawn(move || {
			let parent = parent.map(|hwnd| HWND(hwnd as _));
			let result = run(&request, parent);
			// A closed receiver means the app is gone, so nobody waits for the result.
			if results.send((id, result)).is_ok() {
				waker.wake();
			}
		})
		.map_err(|error| DialogError::Os(error.to_string()))?;
	Ok(())
}

fn request(dialog: &Dialog) -> Request {
	let filters = |filters: &[crate::window::FileFilter]| {
		filters
			.iter()
			.map(|filter| {
				let spec = filter
					.extensions
					.iter()
					.map(|extension| format!("*.{extension}"))
					.collect::<Vec<_>>()
					.join(";");
				(HSTRING::from(filter.name), HSTRING::from(spec))
			})
			.collect()
	};
	match *dialog {
		Dialog::Open {
			title,
			directory,
			filters: file_filters,
			multiple,
			folders,
		} => Request::Open {
			title: title.into(),
			directory: directory.map(HSTRING::from),
			filters: if folders { Vec::new() } else { filters(file_filters) },
			multiple,
			folders,
		},
		Dialog::Save {
			title,
			directory,
			file_name,
			filters: file_filters,
		} => Request::Save {
			title: title.into(),
			directory: directory.map(HSTRING::from),
			file_name: file_name.into(),
			filters: filters(file_filters),
		},
		Dialog::Color { initial, .. } => Request::Color { initial },
		Dialog::Message {
			level,
			title,
			text,
			buttons,
		} => Request::Message {
			style: match level {
				MessageLevel::Info => MB_ICONINFORMATION,
				MessageLevel::Warning => MB_ICONWARNING,
				MessageLevel::Error => MB_ICONERROR,
			} | match buttons {
				Buttons::Ok => MB_OK,
				Buttons::OkCancel => MB_OKCANCEL,
				Buttons::YesNo => MB_YESNO,
				Buttons::YesNoCancel => MB_YESNOCANCEL,
			},
			title: title.into(),
			text: text.into(),
		},
	}
}

/// Shows the dialog and waits for the user. Failures become [`DialogResult::Failed`], since the request already
/// returned and the dialog's single final result must still arrive.
fn run(request: &Request, parent: Option<HWND>) -> DialogResult {
	match request {
		Request::Open {
			title,
			directory,
			filters,
			multiple,
			folders,
		} => with_com(|| {
			let dialog: IFileOpenDialog = unsafe { CoCreateInstance(&FileOpenDialog, None, CLSCTX_INPROC_SERVER)? };
			let mut options = unsafe { dialog.GetOptions()? } | FOS_FORCEFILESYSTEM;
			if *multiple {
				options |= FOS_ALLOWMULTISELECT;
			}
			if *folders {
				options |= FOS_PICKFOLDERS;
			}
			unsafe { dialog.SetOptions(options)? };
			configure(&dialog.cast()?, title, directory.as_ref(), filters)?;
			if unsafe { dialog.Show(parent) }.is_err() {
				return Ok(DialogResult::Cancelled);
			}
			let items = unsafe { dialog.GetResults()? };
			let count = unsafe { items.GetCount()? };
			let paths = (0..count)
				.map(|index| unsafe { items.GetItemAt(index) }.and_then(|item| file_path(&item)))
				.collect::<windows::core::Result<_>>()?;
			Ok(DialogResult::Files(paths))
		}),
		Request::Save {
			title,
			directory,
			file_name,
			filters,
		} => with_com(|| {
			let dialog: IFileSaveDialog = unsafe { CoCreateInstance(&FileSaveDialog, None, CLSCTX_INPROC_SERVER)? };
			let file_dialog: IFileDialog = dialog.cast()?;
			configure(&file_dialog, title, directory.as_ref(), filters)?;
			unsafe { file_dialog.SetFileName(PCWSTR(file_name.as_ptr()))? };
			if unsafe { dialog.Show(parent) }.is_err() {
				return Ok(DialogResult::Cancelled);
			}
			Ok(DialogResult::Files(vec![file_path(&unsafe { dialog.GetResult()? })?]))
		}),
		Request::Color { initial } => {
			let byte = |value: f32| (value.clamp(0.0, 1.0) * 255.0).round() as u32;
			// The dialog keeps the user's custom colors in this array while it is open.
			let mut custom = [COLORREF(0x00FF_FFFF); 16];
			let mut choose = CHOOSECOLORW {
				lStructSize: size_of::<CHOOSECOLORW>() as u32,
				hwndOwner: parent.unwrap_or_default(),
				rgbResult: COLORREF(byte(initial[0]) | byte(initial[1]) << 8 | byte(initial[2]) << 16),
				lpCustColors: custom.as_mut_ptr(),
				Flags: CC_FULLOPEN | CC_RGBINIT,
				..Default::default()
			};
			if !unsafe { ChooseColorW(&mut choose) }.as_bool() {
				return DialogResult::Cancelled;
			}
			let channel = |shift: u32| ((choose.rgbResult.0 >> shift) & 0xFF) as f32 / 255.0;
			DialogResult::Color {
				// Win32 has no alpha picker, so alpha stays as it started.
				color: [channel(0), channel(8), channel(16), initial[3]],
				done: true,
			}
		}
		Request::Message { style, title, text } => {
			let pressed = unsafe { MessageBoxW(parent, text, title, *style) };
			DialogResult::Button(match pressed {
				IDOK => Button::Ok,
				IDYES => Button::Yes,
				IDNO => Button::No,
				// Closing a box with a Cancel button answers `IDCANCEL`; boxes without one cannot be closed otherwise.
				_ => Button::Cancel,
			})
		}
	}
}

/// Runs `dialog` in a single-threaded COM apartment, which the shell dialogs require.
fn with_com(dialog: impl FnOnce() -> windows::core::Result<DialogResult>) -> DialogResult {
	let initialized = unsafe { CoInitializeEx(None, COINIT_APARTMENTTHREADED | COINIT_DISABLE_OLE1DDE) }.is_ok();
	let result = dialog().unwrap_or_else(|error| DialogResult::Failed(DialogError::Os(error.message())));
	if initialized {
		unsafe { CoUninitialize() };
	}
	result
}

/// Applies the settings open and save dialogs share.
fn configure(
	dialog: &IFileDialog,
	title: &HSTRING,
	directory: Option<&HSTRING>,
	filters: &[(HSTRING, HSTRING)],
) -> windows::core::Result<()> {
	unsafe { dialog.SetTitle(PCWSTR(title.as_ptr()))? };
	if let Some(directory) = directory {
		let folder: IShellItem = unsafe { SHCreateItemFromParsingName(PCWSTR(directory.as_ptr()), None)? };
		unsafe { dialog.SetFolder(&folder)? };
	}
	if !filters.is_empty() {
		let specs: Vec<COMDLG_FILTERSPEC> = filters
			.iter()
			.map(|(name, spec)| COMDLG_FILTERSPEC {
				pszName: PCWSTR(name.as_ptr()),
				pszSpec: PCWSTR(spec.as_ptr()),
			})
			.collect();
		unsafe { dialog.SetFileTypes(&specs)? };
	}
	Ok(())
}

fn file_path(item: &IShellItem) -> windows::core::Result<PathBuf> {
	let name = unsafe { item.GetDisplayName(SIGDN_FILESYSPATH)? };
	let path = unsafe { name.to_string() };
	unsafe { CoTaskMemFree(Some(name.0 as _)) };
	Ok(PathBuf::from(path.map_err(|_| windows::core::Error::empty())?))
}
