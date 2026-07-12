use crate::{audio::source::Source, core::Entity};

/// A sound object you can spawn to play a sound.
pub struct Sound {
	pub(crate) asset: String,
}

impl Sound {
	/// Create a new sound object.
	pub fn new(asset: String) -> Self {
		Sound { asset }
	}
}

impl Entity for Sound {}

impl Source for Sound {}

#[cfg(test)]
mod tests {
	use super::Sound;

	#[test]
	fn sound_retains_the_exact_asset_identifier() {
		let sound = Sound::new("audio/ambience.ogg#loop".into());
		assert_eq!(sound.asset, "audio/ambience.ogg#loop");
	}
}
