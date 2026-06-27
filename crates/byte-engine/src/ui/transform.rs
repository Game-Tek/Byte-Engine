#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Transform {
	pub translate_x: f32,
	pub translate_y: f32,
	pub scale_x: f32,
	pub scale_y: f32,
}

impl Transform {
	pub const IDENTITY: Self = Self {
		translate_x: 0.0,
		translate_y: 0.0,
		scale_x: 1.0,
		scale_y: 1.0,
	};

	pub fn identity() -> Self {
		Self::default()
	}

	pub fn translate(mut self, x: f32, y: f32) -> Self {
		self.translate_x = x;
		self.translate_y = y;
		self
	}

	pub fn translate_x(mut self, x: f32) -> Self {
		self.translate_x = x;
		self
	}

	pub fn translate_y(mut self, y: f32) -> Self {
		self.translate_y = y;
		self
	}

	pub fn scale(mut self, scale: f32) -> Self {
		self.scale_x = scale;
		self.scale_y = scale;
		self
	}

	pub fn scale_xy(mut self, x: f32, y: f32) -> Self {
		self.scale_x = x;
		self.scale_y = y;
		self
	}
}

impl Default for Transform {
	fn default() -> Self {
		Self {
			translate_x: 0.0,
			translate_y: 0.0,
			scale_x: 1.0,
			scale_y: 1.0,
		}
	}
}
