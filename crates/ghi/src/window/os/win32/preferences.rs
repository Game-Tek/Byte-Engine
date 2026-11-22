//! Reads the accessibility settings Windows keeps and reports them as [`AppEvents::Preferences`]. See
//! [Presentation preferences](/docs/reference/ghi#presentation-preferences).
//!
//! The app reports them once when it starts, and each window reports them again when it receives `WM_SETTINGCHANGE`.
//! The settings change rarely, so they're read afresh each time instead of watched.

use windows::{
	Win32::{
		System::Registry::{HKEY_CURRENT_USER, RRF_RT_REG_DWORD, RegGetValueW},
		UI::{
			Accessibility::{HCF_HIGHCONTRASTON, HIGHCONTRASTW},
			WindowsAndMessaging::{
				SPI_GETCLIENTAREAANIMATION, SPI_GETHIGHCONTRAST, SYSTEM_PARAMETERS_INFO_UPDATE_FLAGS, SystemParametersInfoW,
			},
		},
	},
	core::{BOOL, w},
};

use crate::window::{AppEvents, Event, Preferences};

/// The preferences as the settings stand now. A setting Windows can't read is left out.
pub(super) fn report() -> Event {
	Event::App(AppEvents::Preferences(Preferences {
		reduce_motion: animations().map(|on| !on),
		increase_contrast: high_contrast(),
		text_scale: Some(text_scale()),
	}))
}

/// Whether "Animation effects" is on.
fn animations() -> Option<bool> {
	let mut on = BOOL::default();
	// SAFETY: The action writes one `BOOL` through the pointer, which outlives the call.
	unsafe {
		SystemParametersInfoW(
			SPI_GETCLIENTAREAANIMATION,
			0,
			Some((&raw mut on).cast()),
			SYSTEM_PARAMETERS_INFO_UPDATE_FLAGS(0),
		)
	}
	.ok()
	.map(|()| on.as_bool())
}

/// Whether a high contrast theme is on.
fn high_contrast() -> Option<bool> {
	let mut contrast = HIGHCONTRASTW {
		cbSize: size_of::<HIGHCONTRASTW>() as u32,
		..Default::default()
	};
	// SAFETY: The action fills the structure, whose size field names its length, through the pointer.
	unsafe {
		SystemParametersInfoW(
			SPI_GETHIGHCONTRAST,
			contrast.cbSize,
			Some((&raw mut contrast).cast()),
			SYSTEM_PARAMETERS_INFO_UPDATE_FLAGS(0),
		)
	}
	.ok()
	.map(|()| contrast.dwFlags.contains(HCF_HIGHCONTRASTON))
}

/// The "Text size" setting as a factor. Windows writes the value only once the user changes it, so a missing value is
/// the default size, 1.
fn text_scale() -> f32 {
	let mut percent = 0u32;
	let mut size = size_of::<u32>() as u32;
	// SAFETY: The call writes at most `size` bytes, one `u32`, through the pointer, which outlives it.
	let read = unsafe {
		RegGetValueW(
			HKEY_CURRENT_USER,
			w!("Software\\Microsoft\\Accessibility"),
			w!("TextScaleFactor"),
			RRF_RT_REG_DWORD,
			None,
			Some((&raw mut percent).cast()),
			Some(&mut size),
		)
	};
	if read.is_ok() { percent as f32 / 100.0 } else { 1.0 }
}
