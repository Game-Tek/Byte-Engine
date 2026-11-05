//! Opens each operating system dialog from a window and prints the results it sends.
//!
//! Press O to open files, F to pick a folder, S to save, C to pick a color, and M to show a message. Hold Shift to
//! open the dialog without a parent window. Press Escape or close the window to quit.

use ghi::window::{App, AppEvents, Buttons, Dialog, Event, Events, Features, FileFilter, MessageLevel, Wait, input::Keys};

fn main() -> Result<(), String> {
	let mut app = App::new("dialogs")?;
	let window = app.create_window("Dialogs", utils::Extent::rectangle(640, 360), Features::DECORATIONS)?;
	let filters = [
		FileFilter {
			name: "Images",
			extensions: &["png", "jpg"],
		},
		FileFilter {
			name: "Text",
			extensions: &["txt", "md"],
		},
	];
	let mut shift = false;

	loop {
		// Events are collected first, since opening a dialog needs the app the poll borrows.
		let events: Vec<Event> = app.poll(Wait::Forever).collect();
		for event in events {
			let key = match event {
				Event::App(AppEvents::Quit) => return Ok(()),
				Event::App(AppEvents::Dialog { id, result }) => {
					println!("{id:?}: {result:?}");
					continue;
				}
				Event::Window {
					event: Events::Close, ..
				} => return Ok(()),
				Event::Window {
					event: Events::Key { pressed, key, .. },
					..
				} => {
					if matches!(key, Keys::ShiftLeft | Keys::ShiftRight) {
						shift = pressed;
					}
					if !pressed {
						continue;
					}
					key
				}
				Event::Window { .. } => continue,
			};
			let dialog = match key {
				Keys::Escape => return Ok(()),
				Keys::O | Keys::F => Dialog::Open {
					title: "Open",
					directory: None,
					filters: &filters,
					multiple: true,
					folders: matches!(key, Keys::F),
				},
				Keys::S => Dialog::Save {
					title: "Save",
					directory: None,
					file_name: "untitled.txt",
					filters: &filters,
				},
				Keys::C => Dialog::Color {
					initial: [1.0, 0.5, 0.0, 0.75],
					alpha: true,
				},
				Keys::M => Dialog::Message {
					level: MessageLevel::Warning,
					title: "Discard changes?",
					text: "The scene has unsaved changes.",
					buttons: Buttons::YesNoCancel,
				},
				_ => continue,
			};
			let parent = (!shift).then_some(&window);
			match app.open_dialog(&dialog, parent) {
				Ok(id) => println!("{id:?}: opened {key:?}, parent: {}", parent.is_some()),
				Err(error) => println!("{error}"),
			}
		}
	}
}
