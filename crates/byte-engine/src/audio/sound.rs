use crate::{audio::source::Source, core::Entity};

/// The `Sound` struct identifies an audio asset that can be used as a
/// [`crate::audio::Source`].
pub struct Sound {
	pub(crate) asset: String,
}

impl Sound {
	/// Creates a sound for the specified audio asset.
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
