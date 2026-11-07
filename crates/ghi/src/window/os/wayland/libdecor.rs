//! Draws client-side window decorations through libdecor on compositors that won't draw them, such as GNOME's.
//!
//! libdecor is loaded at run time, so building the engine needs no libdecor package and a system without it still
//! opens undecorated windows. A decorated frame owns its surface's `xdg_surface` and `xdg_toplevel`, and libdecor
//! creates its own protocol objects on the display's default queue. [`Libdecor::dispatch`] runs that queue, and the
//! frame callbacks only record [`FrameEvent`]s, which the app applies to its window state afterwards.

use std::{
	cell::{Cell, RefCell},
	collections::VecDeque,
	ffi::{CString, c_char, c_int, c_void},
	ptr::NonNull,
	rc::Rc,
};

use crate::window::WindowId;

/// The `FrameEvent` enum carries what a frame callback reported, until the app applies it to the window.
#[derive(Debug)]
pub(super) enum FrameEvent {
	/// The compositor configured the frame's content to this size, in surface units.
	Configure { width: i32, height: i32 },
	/// The user asked to close the window, such as with the title bar's close button.
	Close,
}

type Events = Rc<RefCell<VecDeque<(WindowId, FrameEvent)>>>;

/// Mirrors `struct libdecor_interface`.
#[repr(C)]
struct ContextInterface {
	error: unsafe extern "C" fn(*mut c_void, c_int, *const c_char),
	reserved: [Option<unsafe extern "C" fn()>; 10],
}

/// Mirrors `struct libdecor_frame_interface`.
#[repr(C)]
struct FrameInterface {
	configure: unsafe extern "C" fn(*mut c_void, *mut c_void, *mut c_void),
	close: unsafe extern "C" fn(*mut c_void, *mut c_void),
	commit: unsafe extern "C" fn(*mut c_void, *mut c_void),
	dismiss_popup: unsafe extern "C" fn(*mut c_void, *const c_char, *mut c_void),
	reserved: [Option<unsafe extern "C" fn()>; 10],
}

// libdecor reads the interfaces for as long as the context and its frames live, and never writes them.
static CONTEXT_INTERFACE: ContextInterface = ContextInterface {
	error: context_error,
	reserved: [None; 10],
};
static FRAME_INTERFACE: FrameInterface = FrameInterface {
	configure: frame_configure,
	close: frame_close,
	commit: frame_commit,
	dismiss_popup: frame_dismiss_popup,
	reserved: [None; 10],
};

/// The `Api` struct keeps the loaded library and the functions the backend calls, so they live as long as any
/// context or frame.
struct Api {
	new: unsafe extern "C" fn(*mut c_void, *const ContextInterface) -> *mut c_void,
	unref: unsafe extern "C" fn(*mut c_void),
	dispatch: unsafe extern "C" fn(*mut c_void, c_int) -> c_int,
	decorate: unsafe extern "C" fn(*mut c_void, *mut c_void, *const FrameInterface, *mut c_void) -> *mut c_void,
	frame_unref: unsafe extern "C" fn(*mut c_void),
	frame_set_title: unsafe extern "C" fn(*mut c_void, *const c_char),
	frame_set_app_id: unsafe extern "C" fn(*mut c_void, *const c_char),
	frame_map: unsafe extern "C" fn(*mut c_void),
	frame_commit: unsafe extern "C" fn(*mut c_void, *mut c_void, *mut c_void),
	state_new: unsafe extern "C" fn(c_int, c_int) -> *mut c_void,
	state_free: unsafe extern "C" fn(*mut c_void),
	configuration_get_content_size: unsafe extern "C" fn(*mut c_void, *mut c_void, *mut c_int, *mut c_int) -> bool,
	_library: libloading::Library,
}

impl Api {
	fn load() -> Result<Self, libloading::Error> {
		// SAFETY: Loading libdecor runs no initialization that depends on the caller, and every symbol is read with
		// the signature `libdecor.h` declares for it.
		unsafe {
			let library = libloading::Library::new("libdecor-0.so.0")?;
			Ok(Api {
				new: *library.get(b"libdecor_new\0")?,
				unref: *library.get(b"libdecor_unref\0")?,
				dispatch: *library.get(b"libdecor_dispatch\0")?,
				decorate: *library.get(b"libdecor_decorate\0")?,
				frame_unref: *library.get(b"libdecor_frame_unref\0")?,
				frame_set_title: *library.get(b"libdecor_frame_set_title\0")?,
				frame_set_app_id: *library.get(b"libdecor_frame_set_app_id\0")?,
				frame_map: *library.get(b"libdecor_frame_map\0")?,
				frame_commit: *library.get(b"libdecor_frame_commit\0")?,
				state_new: *library.get(b"libdecor_state_new\0")?,
				state_free: *library.get(b"libdecor_state_free\0")?,
				configuration_get_content_size: *library.get(b"libdecor_configuration_get_content_size\0")?,
				_library: library,
			})
		}
	}
}

/// The `Libdecor` struct is the app's libdecor context, through which it decorates windows when the compositor
/// draws no decorations itself.
pub(super) struct Libdecor {
	context: NonNull<c_void>,
	api: Rc<Api>,
	/// What frame callbacks reported since the app last took it.
	events: Events,
}

impl Libdecor {
	/// Loads libdecor and connects it to `display`, a `wl_display` pointer.
	pub(super) fn new(display: *mut c_void) -> Result<Self, String> {
		let api = Api::load().map_err(|error| error.to_string())?;
		// SAFETY: `display` is the app's live connection, and the interface is static.
		let context = unsafe { (api.new)(display, &CONTEXT_INTERFACE) };
		let context = NonNull::new(context).ok_or("libdecor_new returned no context")?;
		if prefers_dark() {
			prefer_dark_gtk_theme();
		}
		Ok(Libdecor {
			context,
			api: Rc::new(api),
			events: Events::default(),
		})
	}

	/// Runs libdecor's queued work without waiting, or waiting up to `timeout_ms` when it's positive, or with no
	/// limit when it's -1, and returns the frame events its callbacks reported.
	///
	/// Call it only while the app holds no read guard of its own queue: libdecor prepares a read of the default
	/// queue, and a thread can't hold two.
	pub(super) fn dispatch(&self, timeout_ms: c_int) -> impl Iterator<Item = (WindowId, FrameEvent)> + '_ {
		// SAFETY: The context is live; a failed dispatch means the connection broke, which the app's own read reports.
		unsafe { (self.api.dispatch)(self.context.as_ptr(), timeout_ms) };
		std::iter::from_fn(|| self.events.borrow_mut().pop_front())
	}

	/// Decorates `surface`, a `wl_surface` pointer, as a toplevel of content `size` in surface units, and maps it.
	///
	/// The frame reports its first configuration through [`Self::dispatch`].
	pub(super) fn decorate(
		&self,
		surface: &wayland_client::protocol::wl_surface::WlSurface,
		id: WindowId,
		title: &str,
		app_id: &str,
		size: (i32, i32),
	) -> Result<Frame, String> {
		use wayland_client::Proxy as _;

		let data = Box::new(FrameData {
			id,
			size: Cell::new(size),
			surface: surface.clone(),
			api: Rc::clone(&self.api),
			events: Rc::clone(&self.events),
		});
		// SAFETY: The surface is live and has no role yet, the interface is static, and `data` outlives the frame,
		// since `Frame` unrefs the frame before it drops the data.
		let frame = unsafe {
			(self.api.decorate)(
				self.context.as_ptr(),
				surface.id().as_ptr().cast(),
				&FRAME_INTERFACE,
				std::ptr::from_ref(&*data).cast_mut().cast(),
			)
		};
		let frame = NonNull::new(frame).ok_or("libdecor_decorate returned no frame")?;
		// Interior NUL bytes would end the strings early, so they are dropped.
		let text = |text: &str| CString::new(text.replace('\0', "")).unwrap_or_default();
		let (title, app_id) = (text(title), text(app_id));
		// SAFETY: The frame is live and libdecor copies both strings.
		unsafe {
			(self.api.frame_set_title)(frame.as_ptr(), title.as_ptr());
			(self.api.frame_set_app_id)(frame.as_ptr(), app_id.as_ptr());
			(self.api.frame_map)(frame.as_ptr());
		}
		Ok(Frame {
			frame,
			_data: data,
			api: Rc::clone(&self.api),
		})
	}
}

impl Drop for Libdecor {
	fn drop(&mut self) {
		// SAFETY: Every frame keeps the library loaded, and the context is dropped once.
		unsafe { (self.api.unref)(self.context.as_ptr()) };
	}
}

/// The `Frame` struct owns one decorated toplevel; dropping it destroys the decorations and the surface's role.
pub(super) struct Frame {
	frame: NonNull<c_void>,
	/// The data the callbacks read, freed only after the frame is.
	_data: Box<FrameData>,
	api: Rc<Api>,
}

impl Drop for Frame {
	fn drop(&mut self) {
		// SAFETY: The frame is live and unreffed once, before its callback data is freed.
		unsafe { (self.api.frame_unref)(self.frame.as_ptr()) };
	}
}

impl std::fmt::Debug for Frame {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		f.debug_struct("Frame").field("frame", &self.frame).finish()
	}
}

/// The `FrameData` struct is what one frame's callbacks read.
struct FrameData {
	id: WindowId,
	/// The content size the frame last committed, kept for configurations that leave the size to the client.
	size: Cell<(i32, i32)>,
	surface: wayland_client::protocol::wl_surface::WlSurface,
	api: Rc<Api>,
	events: Events,
}

/// Reports a libdecor failure, such as a compositor it can't decorate for, in the log.
unsafe extern "C" fn context_error(_: *mut c_void, error: c_int, message: *const c_char) {
	// SAFETY: libdecor passes a NUL-terminated message that lives for the call.
	let message = unsafe { std::ffi::CStr::from_ptr(message) }.to_string_lossy();
	log::error!(
		"libdecor reported error {error}: {message}. The most likely cause is a compositor libdecor can't decorate windows for. Run the application on a compositor that supports xdg-shell, or without window decorations."
	);
}

/// Commits the configuration libdecor received and records the content size it settled on.
unsafe extern "C" fn frame_configure(frame: *mut c_void, configuration: *mut c_void, user_data: *mut c_void) {
	// SAFETY: `user_data` is the frame's `FrameData`, which outlives the frame.
	let data = unsafe { &*user_data.cast::<FrameData>() };
	let api = &data.api;
	let (mut width, mut height) = (0, 0);
	// SAFETY: The configuration is valid for the duration of this callback.
	let sized = unsafe { (api.configuration_get_content_size)(configuration, frame, &mut width, &mut height) };
	// A floating window chooses its own size, so it keeps the one it had.
	let (width, height) = if sized && width > 0 && height > 0 {
		data.size.set((width, height));
		(width, height)
	} else {
		data.size.get()
	};
	// SAFETY: The state is freed after the commit copies it, and the configuration is still valid.
	unsafe {
		let state = (api.state_new)(width, height);
		(api.frame_commit)(frame, state, configuration);
		(api.state_free)(state);
	}
	data.events
		.borrow_mut()
		.push_back((data.id, FrameEvent::Configure { width, height }));
}

unsafe extern "C" fn frame_close(_: *mut c_void, user_data: *mut c_void) {
	// SAFETY: `user_data` is the frame's `FrameData`, which outlives the frame.
	let data = unsafe { &*user_data.cast::<FrameData>() };
	data.events.borrow_mut().push_back((data.id, FrameEvent::Close));
}

/// Commits the surface when libdecor changed the decorations around it, so the change shows without a new frame.
unsafe extern "C" fn frame_commit(_: *mut c_void, user_data: *mut c_void) {
	// SAFETY: `user_data` is the frame's `FrameData`, which outlives the frame.
	let data = unsafe { &*user_data.cast::<FrameData>() };
	data.surface.commit();
}

/// The backend opens no popups of its own, so there is none to dismiss.
unsafe extern "C" fn frame_dismiss_popup(_: *mut c_void, _: *const c_char, _: *mut c_void) {}

/// Returns whether the desktop asks applications for a dark appearance, through the settings portal's
/// `color-scheme`, where 1 means dark. A desktop without the portal counts as light.
fn prefers_dark() -> bool {
	use dbus::arg::{RefArg as _, Variant};

	let Ok(connection) = dbus::blocking::Connection::new_session() else {
		return false;
	};
	let proxy = connection.with_proxy(
		"org.freedesktop.portal.Desktop",
		"/org/freedesktop/portal/desktop",
		std::time::Duration::from_millis(500),
	);
	let reply: Result<(Variant<Box<dyn dbus::arg::RefArg>>,), _> = proxy.method_call(
		"org.freedesktop.portal.Settings",
		"ReadOne",
		("org.freedesktop.appearance", "color-scheme"),
	);
	reply.is_ok_and(|(Variant(value),)| value.as_u64() == Some(1))
}

/// Makes libdecor's GTK plugin draw with the dark variant of the GTK theme.
///
/// GTK 3 doesn't follow the desktop's color scheme on its own; applications ask for the dark variant through the
/// `gtk-application-prefer-dark-theme` setting. The plugin loaded GTK into the process, so this only looks it up, and
/// does nothing when libdecor picked a plugin without GTK.
fn prefer_dark_gtk_theme() {
	// glibc and musl both define `RTLD_NOLOAD` as 4: open the library only if it's already loaded.
	const RTLD_NOLOAD: c_int = 4;
	// SAFETY: Both symbols are read with the signatures GTK and GLib declare, and the settings object belongs to GTK,
	// which the plugin already initialized on this thread.
	unsafe {
		let Ok(gtk) = libloading::os::unix::Library::open(Some("libgtk-3.so.0"), RTLD_NOLOAD | libloading::os::unix::RTLD_LAZY)
		else {
			return;
		};
		let (Ok(settings_get_default), Ok(object_set)) = (
			gtk.get::<unsafe extern "C" fn() -> *mut c_void>(b"gtk_settings_get_default\0"),
			gtk.get::<unsafe extern "C" fn(*mut c_void, *const c_char, ...)>(b"g_object_set\0"),
		) else {
			return;
		};
		let settings = settings_get_default();
		if !settings.is_null() {
			object_set(
				settings,
				c"gtk-application-prefer-dark-theme".as_ptr(),
				1 as c_int,
				std::ptr::null::<c_char>(),
			);
		}
	}
}
