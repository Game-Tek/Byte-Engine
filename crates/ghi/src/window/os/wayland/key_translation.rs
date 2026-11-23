use super::*;

/// The `KeyboardState` struct provides XKB keycode translation for a Wayland window.
///
/// The XKB state holds references to its keymap and context, so it keeps them alive on its own.
pub(super) struct KeyboardState {
	pub(super) state: xkb::State,
	/// Joins dead keys and compose sequences, such as a dead acute then `e`, into the character they type. `None`
	/// when the locale has no compose table, which leaves every key typing its own character.
	compose: Option<xkb::compose::State>,
	/// The keymap's Control and Super modifiers, which make a key a shortcut that types nothing.
	shortcut_modifiers: [xkb::ModIndex; 2],
}

impl std::fmt::Debug for KeyboardState {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		f.debug_struct("KeyboardState").finish()
	}
}

/// Reads the compose table of the user's locale for text, the one the C library would use, which joins dead keys.
/// `None` when the locale has none.
///
/// Parsing a compose file takes milliseconds, so the first keymap reads it and later keymaps take it over; see
/// [`KeyboardState::compose_table`].
pub(super) fn compose_table(context: &xkb::Context) -> Option<xkb::compose::Table> {
	let locale = ["LC_ALL", "LC_CTYPE", "LANG"]
		.into_iter()
		.find_map(|name| std::env::var_os(name).filter(|value| !value.is_empty()))
		.unwrap_or_else(|| "C".into());
	xkb::compose::Table::new_from_locale(context, &locale, xkb::compose::COMPILE_NO_FLAGS).ok()
}

/// The `Typed` enum is what one key press types, for the text it sends as character events.
pub(super) enum Typed {
	/// Nothing: a key without text, a press inside a compose sequence, or a press while Control or Super is held.
	Nothing,
	/// One character, which a held key also repeats.
	Character(char),
	/// The text a compose sequence ends with when it's more than one character.
	Text(String),
}

impl KeyboardState {
	/// Creates the keyboard state for `keymap`, joining dead keys through `compose`; see [`compose_table`].
	pub(super) fn new(keymap: &xkb::Keymap, compose: Option<&xkb::compose::Table>) -> Self {
		Self {
			state: xkb::State::new(keymap),
			compose: compose.map(|table| xkb::compose::State::new(table, xkb::compose::STATE_NO_FLAGS)),
			shortcut_modifiers: [xkb::MOD_NAME_CTRL, xkb::MOD_NAME_LOGO].map(|name| keymap.mod_get_index(name)),
		}
	}

	/// Returns the compose table this state joins sequences with, for the state of a keymap that replaces it.
	pub(super) fn compose_table(&self) -> Option<xkb::compose::Table> {
		self.compose.as_ref().map(xkb::compose::State::compose_table)
	}

	/// Returns what a press of `keycode` types with the modifiers held before it, which is why it runs before the
	/// press updates the state.
	///
	/// Like on macOS, a press types nothing while Control or Super is held, so shortcuts such as Super+F don't type.
	pub(super) fn typed(&mut self, keycode: xkb::Keycode) -> Typed {
		// A shortcut doesn't take part in compose sequences either.
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
				// A sequence that types one character names it as a keysym, which needs no text allocated.
				let typed = match compose.keysym().and_then(|keysym| char::from_u32(xkb::keysym_to_utf32(keysym))) {
					Some(character) if character != '\0' => Typed::Character(character),
					_ => compose.utf8().map_or(Typed::Nothing, Typed::Text),
				};
				compose.reset();
				typed
			}
			// A key that breaks a sequence types nothing, as in other XKB clients.
			xkb::compose::Status::Cancelled => {
				compose.reset();
				Typed::Nothing
			}
			xkb::compose::Status::Nothing => character,
		}
	}

	/// Returns the printable character `keycode` types with the current modifiers, without composing, or `None` while
	/// Control or Super is held. A repeat of a held key types this.
	pub(super) fn character(&self, keycode: xkb::Keycode) -> Option<char> {
		if self.holds_shortcut_modifier() {
			return None;
		}
		char::from_u32(self.state.key_get_utf32(keycode)).filter(|character| *character != '\0' && !character.is_control())
	}

	/// Returns whether Control or Super is held, which makes a key a shortcut that types nothing.
	fn holds_shortcut_modifier(&self) -> bool {
		self.shortcut_modifiers
			.iter()
			.any(|&modifier| self.state.mod_index_is_active(modifier, xkb::STATE_MODS_EFFECTIVE))
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
		let compose = xkb::compose::Table::new_from_locale(&context, "en_US.UTF-8".as_ref(), xkb::compose::COMPILE_NO_FLAGS).ok();
		KeyboardState::new(&keymap, compose.as_ref())
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
