use crate::{os::{self, WindowLike as _}, input::{Keys, MouseKeys}, Events};
use utils::Extent;

pub struct Window {
	name: String,
	extent: Extent,
	id_name: String,
	os_window: os::Window,
}

impl Window {
	pub fn new(name: &str, extent: Extent) -> Option<Window> {
		Self::new_with_params(name, extent, name)
	}

	pub fn new_with_params(name: &str, extent: Extent, id_name: &str) -> Option<Window> {
		let os_window = os::Window::try_new(name, extent, id_name).ok()?;

		Some(Window {
			name: name.to_owned(),
			extent,
			id_name: id_name.to_owned(),
			os_window,
		})
	}

	pub fn poll<'a>(&'a mut self) -> impl Iterator<Item = Events> + 'a {
		self.os_window.poll()
	}

	pub fn os_handles(&self) -> os::Handles {
		self.os_window.handles()
	}
}

impl TryFrom<u8> for Keys {
	type Error = ();

	fn try_from(value: u8) -> Result<Self, Self::Error> {
		match value {
			0x26 => Ok(Keys::A), 0x38 => Ok(Keys::B), 0x36 => Ok(Keys::C), 0x28 => Ok(Keys::D), 0x1a => Ok(Keys::E), 0x29 => Ok(Keys::F),
			0x2a => Ok(Keys::G), 0x2b => Ok(Keys::H), 0x1f => Ok(Keys::I), 0x2c => Ok(Keys::J), 0x2d => Ok(Keys::K), 0x2e => Ok(Keys::L),
			0x3a => Ok(Keys::M), 0x39 => Ok(Keys::N), 0x20 => Ok(Keys::O), 0x21 => Ok(Keys::P), 24 => Ok(Keys::Q), 0x1b => Ok(Keys::R),
			0x27 => Ok(Keys::S), 28 => Ok(Keys::T),	30 => Ok(Keys::U), 0x37 => Ok(Keys::V), 25 => Ok(Keys::W), 0x35 => Ok(Keys::X),
			0x1d => Ok(Keys::Y), 0x34 => Ok(Keys::Z),

			90 => Ok(Keys::NumPad0), 87 => Ok(Keys::NumPad1), 88 => Ok(Keys::NumPad2), 89 => Ok(Keys::NumPad3), 0x53 => Ok(Keys::NumPad4),
			0x54 => Ok(Keys::NumPad5), 0x55 => Ok(Keys::NumPad6), 79 => Ok(Keys::NumPad7), 80 => Ok(Keys::NumPad8), 81 => Ok(Keys::NumPad9),

			113 => Ok(Keys::ArrowLeft), 116 => Ok(Keys::ArrowDown), 114 => Ok(Keys::ArrowRight), 111 => Ok(Keys::ArrowUp),

			9 => Ok(Keys::Escape), 23 => Ok(Keys::Tab), 50 => Ok(Keys::ShiftLeft), 37 => Ok(Keys::ControlLeft), 64 => Ok(Keys::AltLeft),
			65 => Ok(Keys::Space), 108 => Ok(Keys::AltRight), 105 => Ok(Keys::ControlRight), 62 => Ok(Keys::ShiftRight), 36 => Ok(Keys::Enter),
			22 => Ok(Keys::Backspace),

			_ => Err(()),
		}
	}
}

impl TryFrom<u8> for MouseKeys {
	type Error = ();

	fn try_from(value: u8) -> Result<Self, Self::Error> {
		match value {
			1 => Ok(MouseKeys::Left),
			2 => Ok(MouseKeys::Middle),
			3 => Ok(MouseKeys::Right),
			4 => Ok(MouseKeys::ScrollUp),
			5 => Ok(MouseKeys::ScrollDown),
			_ => Err(()),
		}
	}
}
