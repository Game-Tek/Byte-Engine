use crate::core::Entity;

/// Selects what happens when an audio sample reaches its final frame.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PlaybackMode {
	/// Stops the player after one pass through the sample.
	Once,
	/// Returns to the first frame and keeps playing until the entity is deleted.
	Loop,
}

/// The `AudioSamplePlayer` struct describes a resource-backed audio entity that
/// the default audio worker will load and play.
///
/// Create it through
/// [`crate::gameplay::world::DefaultWorld::audio_sample_player_factory_mut`].
/// Install the worker with
/// [`crate::application::graphics::setup_default_audio`] before you create the
/// first player.
/// To stop the player, send its factory handle through the world's
/// [`crate::gameplay::world::DefaultWorld::delete_channel_mut`].
/// Deletion is terminal for that handle; create a new player instead of deriving
/// another player from a deleted handle.
#[derive(Debug, Clone, PartialEq)]
pub struct AudioSamplePlayer {
	resource_id: String,
	playback_mode: PlaybackMode,
}

impl AudioSamplePlayer {
	/// Creates a player that repeats the whole resource until the entity is
	/// deleted.
	pub fn looping(resource_id: impl Into<String>) -> Self {
		Self {
			resource_id: resource_id.into(),
			playback_mode: PlaybackMode::Loop,
		}
	}

	/// Creates a player that stops after one pass through the whole resource.
	pub fn once(resource_id: impl Into<String>) -> Self {
		Self {
			resource_id: resource_id.into(),
			playback_mode: PlaybackMode::Once,
		}
	}

	/// Returns the resource ID requested by the async audio loader.
	pub fn resource_id(&self) -> &str {
		&self.resource_id
	}

	/// Returns what happens after the final sample frame.
	pub fn playback_mode(&self) -> PlaybackMode {
		self.playback_mode
	}

	pub(crate) fn into_parts(self) -> (String, PlaybackMode) {
		(self.resource_id, self.playback_mode)
	}
}

impl Entity for AudioSamplePlayer {}

#[cfg(test)]
mod tests {
	use super::{AudioSamplePlayer, PlaybackMode};

	#[test]
	fn player_constructors_preserve_resource_gain_and_playback_mode() {
		let looping = AudioSamplePlayer::looping("audio/engine.ogg");
		assert_eq!(looping.resource_id(), "audio/engine.ogg");
		assert_eq!(looping.playback_mode(), PlaybackMode::Loop);

		let once = AudioSamplePlayer::once("audio/impact.wav");
		assert_eq!(once.resource_id(), "audio/impact.wav");
		assert_eq!(once.playback_mode(), PlaybackMode::Once);
	}
}
