//! Reads the desktop's accessibility settings through the settings portal and reports them as
//! [`AppEvents::Preferences`]. See [Presentation preferences](/docs/reference/ghi#presentation-preferences).
//!
//! The portal's `org.freedesktop.appearance` keys come first. Older portals lack `reduced-motion`, so GNOME's own
//! keys, which its portal also serves, stand in for any appearance key that's missing.

use std::{cell::Cell, collections::HashMap, os::fd::BorrowedFd, rc::Rc, time::Duration};

use dbus::{
	arg::PropMap,
	blocking::LocalConnection,
	channel::{BusType, Channel},
	message::MatchRule,
};

use crate::window::{AppEvents, Event, Preferences};

const PORTAL: &str = "org.freedesktop.portal.Desktop";
const PORTAL_PATH: &str = "/org/freedesktop/portal/desktop";
const SETTINGS: &str = "org.freedesktop.portal.Settings";
const APPEARANCE: &str = "org.freedesktop.appearance";
const GNOME_INTERFACE: &str = "org.gnome.desktop.interface";
const GNOME_A11Y: &str = "org.gnome.desktop.a11y.interface";
/// How long a read waits for the portal; a desktop that takes longer reports nothing.
const READ_TIMEOUT: Duration = Duration::from_millis(500);

/// The `PreferenceWatch` struct keeps the app told about the desktop's accessibility settings: the pump watches its
/// descriptor and drains it, and each change the portal signals becomes a new report.
pub(super) struct PreferenceWatch {
	connection: LocalConnection,
	/// Whether the portal signalled a change since the last drain.
	changed: Rc<Cell<bool>>,
}

impl PreferenceWatch {
	/// Connects to the session bus and listens for the portal's changes. Returns `None` without a session bus.
	pub(super) fn connect() -> Option<Self> {
		let mut channel = Channel::get_private(BusType::Session).ok()?;
		// The pump polls the connection's descriptor next to the display socket.
		channel.set_watch_enabled(true);
		let connection = LocalConnection::from(channel);
		let changed = Rc::new(Cell::new(false));
		let signalled = changed.clone();
		// Any key may change, including ones this doesn't read; the app passes on only reports that differ.
		connection
			.add_match(
				MatchRule::new_signal(SETTINGS, "SettingChanged"),
				move |(): (), _, _| {
					signalled.set(true);
					true
				},
			)
			.ok()?;
		Some(Self { connection, changed })
	}

	/// The descriptor a waiting poll watches for the portal's signals.
	pub(super) fn fd(&self) -> BorrowedFd<'_> {
		// SAFETY: The descriptor belongs to the connection, which outlives the borrow.
		unsafe { BorrowedFd::borrow_raw(self.connection.channel().watch().fd) }
	}

	/// Handles every message the bus delivered, without waiting, and returns a new report when the portal signalled
	/// a change.
	pub(super) fn drain(&self) -> Option<Event> {
		// A broken connection only stops the reports; there is no caller to tell.
		while self.connection.process(Duration::ZERO).unwrap_or(false) {}
		self.changed.replace(false).then(|| self.report())
	}

	/// The preferences as the desktop's settings stand now.
	pub(super) fn report(&self) -> Event {
		let proxy = self.connection.with_proxy(PORTAL, PORTAL_PATH, READ_TIMEOUT);
		let settings: HashMap<String, PropMap> = proxy
			.method_call(SETTINGS, "ReadAll", (vec![APPEARANCE, GNOME_INTERFACE, GNOME_A11Y],))
			.map(|(settings,)| settings)
			.unwrap_or_default();
		Event::App(AppEvents::Preferences(preferences(&settings)))
	}
}

/// Turns the portal's settings, by namespace and key, into preferences. The appearance keys use 0 for no preference
/// and 1 for the accessible choice.
fn preferences(settings: &HashMap<String, PropMap>) -> Preferences {
	let get = |namespace: &str, key: &str| settings.get(namespace)?.get(key);
	let flag = |namespace, key| get(namespace, key)?.0.as_u64().map(|value| value == 1);
	Preferences {
		reduce_motion: flag(APPEARANCE, "reduced-motion").or_else(|| flag(GNOME_INTERFACE, "enable-animations").map(|on| !on)),
		increase_contrast: flag(APPEARANCE, "contrast").or_else(|| flag(GNOME_A11Y, "high-contrast")),
		text_scale: get(GNOME_INTERFACE, "text-scaling-factor")
			.and_then(|value| value.0.as_f64())
			.map(|factor| factor as f32),
	}
}
