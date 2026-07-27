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
