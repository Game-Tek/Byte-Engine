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
	gain: f32,
	playback_mode: PlaybackMode,
}

impl AudioSamplePlayer {
	/// Creates a player that repeats the whole resource until the entity is
	/// deleted.
	pub fn looping(resource_id: impl Into<String>) -> Self {
		Self {
			resource_id: resource_id.into(),
			gain: 1.0,
			playback_mode: PlaybackMode::Loop,
		}
	}

	/// Creates a player that stops after one pass through the whole resource.
	pub fn once(resource_id: impl Into<String>) -> Self {
		Self {
			resource_id: resource_id.into(),
			gain: 1.0,
			playback_mode: PlaybackMode::Once,
		}
	}

	/// Sets the linear gain applied while this player is mixed.
	///
	/// # Panics
	///
	/// Panics if `gain` is negative, infinite, or not a number.
	pub fn with_gain(mut self, gain: f32) -> Self {
		assert!(
			gain.is_finite() && gain >= 0.0,
			"Invalid audio sample gain. The gain must be a finite, non-negative value."
		);
		self.gain = gain;
		self
	}

	/// Returns the resource ID requested by the async audio loader.
	pub fn resource_id(&self) -> &str {
		&self.resource_id
	}

	/// Returns the linear gain applied during mixing.
	pub fn gain(&self) -> f32 {
		self.gain
	}

	/// Returns what happens after the final sample frame.
	pub fn playback_mode(&self) -> PlaybackMode {
		self.playback_mode
	}

	pub(crate) fn into_parts(self) -> (String, f32, PlaybackMode) {
		(self.resource_id, self.gain, self.playback_mode)
	}
}

impl Entity for AudioSamplePlayer {}

#[cfg(test)]
mod tests {
	use super::{AudioSamplePlayer, PlaybackMode};

	#[test]
	fn player_constructors_preserve_resource_gain_and_playback_mode() {
		let looping = AudioSamplePlayer::looping("audio/engine.ogg").with_gain(0.25);
		assert_eq!(looping.resource_id(), "audio/engine.ogg");
		assert_eq!(looping.gain(), 0.25);
		assert_eq!(looping.playback_mode(), PlaybackMode::Loop);

		let once = AudioSamplePlayer::once("audio/impact.wav");
		assert_eq!(once.resource_id(), "audio/impact.wav");
		assert_eq!(once.gain(), 1.0);
		assert_eq!(once.playback_mode(), PlaybackMode::Once);
	}

	#[test]
	#[should_panic(expected = "Invalid audio sample gain")]
	fn player_rejects_non_finite_gain() {
		let _ = AudioSamplePlayer::looping("audio/engine.ogg").with_gain(f32::NAN);
	}
}
