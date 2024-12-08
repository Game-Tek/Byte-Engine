use core::Entity;

/// A sound object you can spawn to play a sound.
pub struct Sound {
	pub(crate) asset: String,
}

impl Sound {
	/// Create a new sound object.
	pub fn new(asset: String) -> Self {
		Sound {
			asset,
		}
	}
}

impl Entity for Sound {}