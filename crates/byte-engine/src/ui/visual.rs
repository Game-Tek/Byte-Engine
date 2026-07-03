#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Visual {
	pub opacity: f32,
}

impl Visual {
	pub const DEFAULT: Self = Self { opacity: 1.0 };

	pub fn opacity(opacity: f32) -> Self {
		Self { opacity }
	}
}

impl Default for Visual {
	fn default() -> Self {
		Self::DEFAULT
	}
}
