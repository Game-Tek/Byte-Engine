use utils::Extent;

use crate::window::{AppEvents, Dialog, DialogError, DialogId, Event, Events, Features, Preferences, Wait, Window, os};

/// The `App` struct exists because platforms deliver events for the whole process through one queue.
/// Keep one per process, create every [`Window`] from it, and drain it once per frame with [`App::poll`].
///
/// Drop every [`Window`] before the `App` that created it.
pub struct App {
	os_app: os::App,
	/// The raw value of the last [`DialogId`] handed out. Identities start at one.
	last_dialog: u64,
	/// The last [`AppEvents::Preferences`] reported, so a backend's repeat of the same preferences isn't.
	preferences: Option<Preferences>,
}

impl App {
	/// Connects to the platform windowing system. `id_name` identifies the application to the OS.
	///
	/// Next, call [`App::create_window`].
	pub fn new(id_name: &str) -> Result<App, String> {
		Ok(App {
			os_app: os::App::try_new(id_name)?,
			last_dialog: 0,
			preferences: None,
		})
	}

	/// Creates a window and shows it, unless `features` contains [`Features::BACKGROUND`].
	///
	/// Next, pass [`Window::os_handles`] to [`crate::context::Context::bind_to_window`] and match
	/// [`Window::id`] against the events from [`App::poll`].
	pub fn create_window(&mut self, name: &str, extent: Extent, features: Features) -> Result<Window, String> {
		let os_window = self.os_app.create_window(name, extent, features)?;
		Ok(Window::new(name, extent, os_window))
	}

	/// Waits as `wait` allows for the first event, then drains every pending application and window event.
	///
	/// Pass [`Wait::Immediate`] to only drain. A waiting call also returns when an [`AppWaker`] from
	/// [`Self::waker`] wakes the app, which then yields no event.
	pub fn poll(&mut self, wait: Wait) -> impl Iterator<Item = Event> + '_ {
		// Backends report the preferences whenever the platform says a setting changed, which can be one they don't
		// read, so only a report that differs from the last goes out.
		let reported = &mut self.preferences;
		self.os_app
			.poll(wait)
			.filter(move |event| is_news(reported, event))
			.flat_map(|event| {
				let shortcut = shortcut_event(&event);
				std::iter::once(event).chain(shortcut)
			})
	}

	/// Opens an operating system dialog without waiting for the user. A `parent` window owns the dialog, which opens
	/// attached to it where the platform supports that.
	///
	/// Next, match [`crate::window::AppEvents::Dialog`] events from [`Self::poll`] against the returned id; their
	/// [`crate::window::DialogResult`] carries what the user chose.
	pub fn open_dialog(&mut self, dialog: &Dialog, parent: Option<&Window>) -> Result<DialogId, DialogError> {
		let id = DialogId::from_raw(self.last_dialog + 1);
		self.os_app.open_dialog(id, dialog, parent.map(Window::os_window))?;
		self.last_dialog += 1;
		Ok(id)
	}

	/// Puts `text` on the system clipboard as plain text, replacing what it held, so the user can paste it into another
	/// application.
	///
	/// Only macOS has a clipboard so far; elsewhere this returns an error that says so.
	pub fn set_clipboard_text(&mut self, text: &str) -> Result<(), String> {
		self.os_app.set_clipboard_text(text)
	}

	/// Makes `window` hold the cursor while it has focus, as [`Features::CAPTURE_CURSOR`] does when it's created, or
	/// gives the cursor back.
	///
	/// A window that stops capturing frees and shows the cursor at once. One that starts capturing holds the cursor
	/// from the next [`Self::poll`] in which it has focus and no mouse button is down.
	pub fn set_cursor_capture(&mut self, window: &Window, captures: bool) {
		self.os_app.set_cursor_capture(window.os_window(), captures);
	}

	/// Makes a waiting [`Self::poll`] return when a HID device connects or disconnects, so the caller can check
	/// [`crate::hid::Monitor::take_changed`] without polling on a timer.
	///
	/// Without it, changes are still seen on the next poll that another event ends.
	pub fn wake_on_hid_changes(&mut self, monitor: &crate::hid::Monitor) -> Result<(), String> {
		// Linux has no callback for device changes, so the event loop watches the monitor's descriptor; other
		// platforms call back on their own threads and wake the loop through its waker.
		#[cfg(target_os = "linux")]
		return self.os_app.wake_on_hid_changes(monitor);
		#[cfg(not(target_os = "linux"))]
		{
			monitor.os.signal.set_waker(self.waker());
			Ok(())
		}
	}

	/// Returns a handle other threads use to interrupt a waiting [`Self::poll`].
	pub fn waker(&self) -> AppWaker {
		AppWaker(self.os_app.waker())
	}
}

/// The `AppWaker` struct interrupts a waiting [`App::poll`] from any thread.
///
/// Waking an app that is not waiting makes its next waiting poll return at once. Repeated wakes before a poll
/// coalesce into one.
#[derive(Clone)]
pub struct AppWaker(os::AppWaker);

impl AppWaker {
	/// Makes a waiting [`App::poll`] return.
	pub fn wake(&self) {
		self.0.wake();
	}
}

/// Returns whether `event` should reach the caller: any event but a preferences report equal to the one in `reported`,
/// which becomes the new last report.
/// Returns the event of the shortcut modifier that `event`'s key stands for, which reports right after it; see
/// [`Keys::ShortcutLeft`](crate::window::input::Keys::ShortcutLeft).
fn shortcut_event(event: &Event) -> Option<Event> {
	match event {
		&Event::Window {
			window,
			event: Events::Key { seat, pressed, repeat, key },
		} => key.shortcut().map(|key| Event::Window {
			window,
			event: Events::Key { seat, pressed, repeat, key },
		}),
		_ => None,
	}
}

fn is_news(reported: &mut Option<Preferences>, event: &Event) -> bool {
	match event {
		Event::App(AppEvents::Preferences(preferences)) => reported.replace(*preferences) != Some(*preferences),
		_ => true,
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn a_preferences_report_goes_out_first_and_then_only_when_it_changes() {
		let report = |reduce_motion| {
			Event::App(AppEvents::Preferences(Preferences {
				reduce_motion: Some(reduce_motion),
				..Preferences::default()
			}))
		};
		let mut reported = None;

		assert!(is_news(&mut reported, &report(false)), "the first report");
		assert!(!is_news(&mut reported, &report(false)), "a repeat");
		assert!(is_news(&mut reported, &report(true)), "a change");
		assert!(is_news(&mut reported, &Event::App(AppEvents::Quit)), "another event");
	}

	/// Verifies the key the platform starts shortcuts with also reports as the shortcut modifier, and other keys don't.
	#[test]
	fn the_platforms_shortcut_key_also_reports_as_the_shortcut_modifier() {
		use crate::window::{Seat, WindowId, input::Keys};

		let key = |key, pressed| Event::Window {
			window: WindowId::from_raw(1),
			event: Events::Key {
				seat: Seat::stub(),
				pressed,
				repeat: false,
				key,
			},
		};
		// The platform's convention, which the reference specifies: Command on macOS, Control elsewhere.
		let (left, right) = if cfg!(target_os = "macos") {
			(Keys::MetaLeft, Keys::MetaRight)
		} else {
			(Keys::ControlLeft, Keys::ControlRight)
		};
		for (physical, shortcut, pressed) in [(left, "ShortcutLeft", true), (right, "ShortcutRight", false)] {
			let reported = shortcut_event(&key(physical, pressed));
			assert!(
				matches!(reported, Some(Event::Window { event: Events::Key { key, pressed: reported_pressed, .. }, .. })
					if format!("{key:?}") == shortcut && reported_pressed == pressed),
				"{reported:?}"
			);
		}
		assert!(shortcut_event(&key(Keys::A, true)).is_none());
		assert!(shortcut_event(&key(Keys::AltLeft, true)).is_none());
	}
}
