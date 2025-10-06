#[derive(Debug, Clone, Copy)]
/// The keys that can be pressed on a keyboard.
pub enum Keys {
	/// The A key.
	A,
	/// The B key.
	B,
	/// The C key.
	C,
	/// The D key.
	D,
	/// The E key.
	E,
	/// The F key.
	F,
	/// The G key.
	G,
	/// The H key.
	H,
	/// The I key.
	I,
	/// The J key.
	J,
	/// The K key.
	K,
	/// The L key.
	L,
	/// The M key.
	M,
	/// The N key.
	N,
	/// The O key.
	O,
	/// The P key.
	P,
	/// The Q key.
	Q,
	/// The R key.
	R,
	/// The S key.
	S,
	/// The T key.
	T,
	/// The U key.
	U,
	/// The V key.
	V,
	/// The W key.
	W,
	/// The X key.
	X,
	/// The Y key.
	Y,
	/// The Z key.
	Z,

	/// The number 0 key.
	Num0,
	/// The number 1 key.
	Num1,
	/// The number 2 key.
	Num2,
	/// The number 3 key.
	Num3,
	/// The number 4 key.
	Num4,
	/// The number 5 key.
	Num5,
	/// The number 6 key.
	Num6,
	/// The number 7 key.
	Num7,
	/// The number 8 key.
	Num8,
	/// The number 9 key.
	Num9,

	/// The numpad 0 key.
	NumPad0,
	/// The numpad 1 key.
	NumPad1,
	/// The numpad 2 key.
	NumPad2,
	/// The numpad 3 key.
	NumPad3,
	/// The numpad 4 key.
	NumPad4,
	/// The numpad 5 key.
	NumPad5,
	/// The numpad 6 key.
	NumPad6,
	/// The numpad 7 key.
	NumPad7,
	/// The numpad 8 key.
	NumPad8,
	/// The numpad 9 key.
	NumPad9,

	/// The numpad add key.
	NumPadAdd,
	/// The numpad subtract key.
	NumPadSubtract,
	/// The numpad multiply key.
	NumPadMultiply,
	/// The numpad divide key.
	NumPadDivide,
	/// The numpad decimal key.
	NumPadDecimal,
	/// The numpad enter key.
	NumPadEnter,

	/// The backspace key.
	Backspace,
	/// The tab key.
	Tab,
	/// The enter key.
	Enter,
	/// The shift left key.
	ShiftLeft,
	/// The shift right key.
	ShiftRight,
	/// The control left key.
	ControlLeft,
	/// The control right key.
	ControlRight,
	/// The alt left key.
	AltLeft,
	/// The alt right key.
	AltRight,
	/// The menu key.
	Menu,
	/// The spacebar key.
	Space,
	/// The insert key.
	Insert,
	/// The delete key.
	Delete,
	/// The home key.
	Home,
	/// The end key.
	End,
	/// The page up key.
	PageUp,
	/// The page down key.
	PageDown,
	/// The arrow up key.
	ArrowUp,
	/// The arrow down key.
	ArrowDown,
	/// The arrow left key.
	ArrowLeft,
	/// The arrow right key.
	ArrowRight,

	/// The escape key.
	Escape,
	/// The F1 key.
	F1,
	/// The F2 key.
	F2,
	/// The F3 key.
	F3,
	/// The F4 key.
	F4,
	/// The F5 key.
	F5,
	/// The F6 key.
	F6,
	/// The F7 key.
	F7,
	/// The F8 key.
	F8,
	/// The F9 key.
	F9,
	/// The F10 key.
	F10,
	/// The F11 key.
	F11,
	/// The F12 key.
	F12,

	/// The num lock key.
	NumLock,
	/// The scroll lock key.
	ScrollLock,
	/// The caps lock key.
	CapsLock,
	/// The print screen key.
	PrintScreen,
}

#[derive(Debug, Clone, Copy)]
pub enum MouseKeys {
	Left,
	Middle,
	Right,
	ScrollUp,
	ScrollDown,
}
