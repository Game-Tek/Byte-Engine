use super::*;

/// The `KeyboardState` struct provides XKB keycode translation for a Wayland window.
///
/// The XKB state holds references to its keymap and context, so it keeps them alive on its own.
pub(super) struct KeyboardState {
	pub(super) state: xkb::State,
	/// Joins dead keys and compose sequences, such as a dead acute then `e`, into the character they type. `None`
	/// when the locale has no compose table, which leaves every key typing its own character.
	compose: Option<xkb::compose::State>,
}

impl std::fmt::Debug for KeyboardState {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		f.debug_struct("KeyboardState").finish()
	}
}

/// Returns the user's locale for text, whose compose table joins dead keys: the one the C library would use.
pub(super) fn text_locale() -> std::ffi::OsString {
	["LC_ALL", "LC_CTYPE", "LANG"]
		.into_iter()
		.find_map(|name| std::env::var_os(name).filter(|value| !value.is_empty()))
		.unwrap_or_else(|| "C".into())
}

/// The `Typed` enum is what one key press types, for the text it sends as character events.
pub(super) enum Typed {
	/// Nothing: a key without text, a press inside a compose sequence, or a press while Control or Super is held.
	Nothing,
	/// One character, which a held key also repeats.
	Character(char),
	/// The text a compose sequence ends with, which can be more than one character.
	Text(String),
}

impl KeyboardState {
	/// Creates the keyboard state for `keymap`, with the compose table of `locale`; see [`text_locale`].
	pub(super) fn new(context: &xkb::Context, keymap: &xkb::Keymap, locale: &std::ffi::OsStr) -> Self {
		let compose = xkb::compose::Table::new_from_locale(context, locale, xkb::compose::COMPILE_NO_FLAGS)
			.ok()
			.map(|table| xkb::compose::State::new(&table, xkb::compose::STATE_NO_FLAGS));
		Self {
			state: xkb::State::new(keymap),
			compose,
		}
	}

	/// Returns what a press of `keycode` types with the modifiers held before it, which is why it runs before the
	/// press updates the state.
	///
	/// Like on macOS, a press types nothing while Control or Super is held, so shortcuts such as Super+F don't type.
	pub(super) fn typed(&mut self, keycode: xkb::Keycode) -> Typed {
		if self.holds_shortcut_modifier() {
			return Typed::Nothing;
		}
		let character = self.character(keycode).map_or(Typed::Nothing, Typed::Character);
		let Some(compose) = self.compose.as_mut() else {
			return character;
		};
		if compose.feed(self.state.key_get_one_sym(keycode)) == xkb::compose::FeedResult::Ignored {
			// Modifier keys don't take part in sequences.
			return character;
		}
		match compose.status() {
			xkb::compose::Status::Composing => Typed::Nothing,
			xkb::compose::Status::Composed => {
				let text = compose.utf8().unwrap_or_default();
				compose.reset();
				let mut characters = text.chars();
				match (characters.next(), characters.next()) {
					(None, _) => Typed::Nothing,
					(Some(character), None) => Typed::Character(character),
					_ => Typed::Text(text),
				}
			}
			// A key that breaks a sequence types nothing, as in other XKB clients.
			xkb::compose::Status::Cancelled => {
				compose.reset();
				Typed::Nothing
			}
			xkb::compose::Status::Nothing => character,
		}
	}

	/// Returns the character a repeat of the held `keycode` types, outside any compose sequence.
	pub(super) fn repeated(&self, keycode: xkb::Keycode) -> Option<char> {
		if self.holds_shortcut_modifier() {
			return None;
		}
		self.character(keycode)
	}

	/// Returns the printable character `keycode` types with the current modifiers, without composing.
	fn character(&self, keycode: xkb::Keycode) -> Option<char> {
		char::from_u32(self.state.key_get_utf32(keycode)).filter(|character| *character != '\0' && !character.is_control())
	}

	/// Returns whether Control or Super is held, which makes a key a shortcut that types nothing.
	fn holds_shortcut_modifier(&self) -> bool {
		[xkb::MOD_NAME_CTRL, xkb::MOD_NAME_LOGO]
			.into_iter()
			.any(|name| self.state.mod_name_is_active(name, xkb::STATE_MODS_EFFECTIVE))
	}
}

/// Returns the key at the physical position of the Linux evdev key `code`, the `key` of a `wl_keyboard.key` event.
///
/// Keys name positions, so a key reads the same on every layout and with any modifier held, like the macOS key code
/// table: a key always releases as the key it pressed, and WASD stays where it is on a non-Latin layout. The text a key
/// types comes from XKB instead. The codes are those of `linux/input-event-codes.h`.
pub(super) fn evdev_to_key(code: u32) -> Option<Keys> {
	let key = match code {
		1 => Keys::Escape,
		2 => Keys::Num1,
		3 => Keys::Num2,
		4 => Keys::Num3,
		5 => Keys::Num4,
		6 => Keys::Num5,
		7 => Keys::Num6,
		8 => Keys::Num7,
		9 => Keys::Num8,
		10 => Keys::Num9,
		11 => Keys::Num0,
		14 => Keys::Backspace,
		15 => Keys::Tab,
		16 => Keys::Q,
		17 => Keys::W,
		18 => Keys::E,
		19 => Keys::R,
		20 => Keys::T,
		21 => Keys::Y,
		22 => Keys::U,
		23 => Keys::I,
		24 => Keys::O,
		25 => Keys::P,
		28 => Keys::Enter,
		29 => Keys::ControlLeft,
		30 => Keys::A,
		31 => Keys::S,
		32 => Keys::D,
		33 => Keys::F,
		34 => Keys::G,
		35 => Keys::H,
		36 => Keys::J,
		37 => Keys::K,
		38 => Keys::L,
		42 => Keys::ShiftLeft,
		44 => Keys::Z,
		45 => Keys::X,
		46 => Keys::C,
		47 => Keys::V,
		48 => Keys::B,
		49 => Keys::N,
		50 => Keys::M,
		52 => Keys::Period,
		54 => Keys::ShiftRight,
		55 => Keys::NumPadMultiply,
		56 => Keys::AltLeft,
		57 => Keys::Space,
		58 => Keys::CapsLock,
		59 => Keys::F1,
		60 => Keys::F2,
		61 => Keys::F3,
		62 => Keys::F4,
		63 => Keys::F5,
		64 => Keys::F6,
		65 => Keys::F7,
		66 => Keys::F8,
		67 => Keys::F9,
		68 => Keys::F10,
		69 => Keys::NumLock,
		70 => Keys::ScrollLock,
		71 => Keys::NumPad7,
		72 => Keys::NumPad8,
		73 => Keys::NumPad9,
		74 => Keys::NumPadSubtract,
		75 => Keys::NumPad4,
		76 => Keys::NumPad5,
		77 => Keys::NumPad6,
		78 => Keys::NumPadAdd,
		79 => Keys::NumPad1,
		80 => Keys::NumPad2,
		81 => Keys::NumPad3,
		82 => Keys::NumPad0,
		83 => Keys::NumPadDecimal,
		87 => Keys::F11,
		88 => Keys::F12,
		96 => Keys::NumPadEnter,
		97 => Keys::ControlRight,
		98 => Keys::NumPadDivide,
		99 => Keys::PrintScreen,
		100 => Keys::AltRight,
		102 => Keys::Home,
		103 => Keys::ArrowUp,
		104 => Keys::PageUp,
		105 => Keys::ArrowLeft,
		106 => Keys::ArrowRight,
		107 => Keys::End,
		108 => Keys::ArrowDown,
		109 => Keys::PageDown,
		110 => Keys::Insert,
		111 => Keys::Delete,
		125 => Keys::MetaLeft,
		126 => Keys::MetaRight,
		127 => Keys::Menu,
		_ => return None,
	};

	Some(key)
}

#[cfg(test)]
mod tests {
	use super::*;

	/// Builds the keyboard state of an XKB layout, as a compositor's keymap would describe it.
	fn keyboard(layout: &str, variant: &str) -> KeyboardState {
		let context = xkb::Context::new(xkb::CONTEXT_NO_FLAGS);
		let keymap = xkb::Keymap::new_from_names(&context, "", "", layout, variant, None, xkb::COMPILE_NO_FLAGS)
			.expect("The XKB layout must compile. Install xkeyboard-config.");
		KeyboardState::new(&context, &keymap, std::ffi::OsStr::new("en_US.UTF-8"))
	}

	/// Presses or releases the evdev key `code`, the way the keyboard handler updates the state after reading text.
	fn press(keyboard: &mut KeyboardState, code: u32) -> Typed {
		let keycode = xkb::Keycode::new(code + 8);
		let typed = keyboard.typed(keycode);
		keyboard.state.update_key(keycode, xkb::KeyDirection::Down);
		typed
	}

	fn release(keyboard: &mut KeyboardState, code: u32) {
		keyboard.state.update_key(xkb::Keycode::new(code + 8), xkb::KeyDirection::Up);
	}

	const KEY_E: u32 = 18;
	const KEY_F: u32 = 33;
	const KEY_APOSTROPHE: u32 = 40;
	const KEY_LEFTSHIFT: u32 = 42;
	const KEY_LEFTCTRL: u32 = 29;
	const KEY_LEFTMETA: u32 = 125;

	fn character(typed: Typed) -> Option<char> {
		match typed {
			Typed::Character(character) => Some(character),
			_ => None,
		}
	}

	/// Verifies a key types the character its layout gives it under the held modifiers.
	#[test]
	fn keys_type_their_layout_characters() {
		let mut keyboard = keyboard("us", "");
		assert_eq!(character(press(&mut keyboard, KEY_F)), Some('f'));
		release(&mut keyboard, KEY_F);
		press(&mut keyboard, KEY_LEFTSHIFT);
		assert_eq!(character(press(&mut keyboard, KEY_F)), Some('F'));
	}

	/// Verifies Control and Super make a key a shortcut that types nothing, as on macOS.
	#[test]
	fn shortcut_modifiers_type_nothing() {
		for modifier in [KEY_LEFTCTRL, KEY_LEFTMETA] {
			let mut keyboard = keyboard("us", "");
			press(&mut keyboard, modifier);
			assert!(matches!(press(&mut keyboard, KEY_F), Typed::Nothing));
		}
	}

	/// Verifies a dead key types nothing, and joins the next key into one character.
	#[test]
	fn dead_keys_compose_with_the_next_key() {
		let mut keyboard = keyboard("us", "intl");
		assert!(matches!(press(&mut keyboard, KEY_APOSTROPHE), Typed::Nothing));
		release(&mut keyboard, KEY_APOSTROPHE);
		assert_eq!(character(press(&mut keyboard, KEY_E)), Some('é'));
	}

	/// Verifies a key keeps its position on a layout whose letters aren't Latin, so WASD still moves.
	#[test]
	fn keys_name_positions_on_every_layout() {
		let mut keyboard = keyboard("ru", "");
		assert_eq!(character(press(&mut keyboard, KEY_F)), Some('а'));
		assert!(matches!(evdev_to_key(KEY_F), Some(Keys::F)));
	}
}
