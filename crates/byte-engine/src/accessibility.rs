//! Accessibility: the player's preferences that change how the engine presents a game, such as reduced motion. See
//! [Reduced motion](/docs/reference/gui#reduced-motion).
//!
//! The application keeps each preference under the `accessibility.` settings and hands it to the systems it changes:
//! [`crate::application::graphics::GraphicsApplication::motion`] gives a UI engine its [`MotionPreference`] through
//! [`crate::ui::layout::engine::Engine::set_motion`].

use crate::configuration::{Accepts, Read, Setting};

/// The `MotionPreference` enum lets a player who gets sick from movement on screen ask for less of it.
///
/// Under [`Self::Reduced`], UI animations jump to where they're headed and motion blur draws nothing. Read it from
/// [`crate::application::graphics::GraphicsApplication::motion`] and pass it to
/// [`crate::ui::layout::engine::Engine::set_motion`].
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum MotionPreference {
	/// Animations play and motion blur draws as the application set them up.
	#[default]
	Full,
	/// Animations jump to their targets and motion blur draws nothing.
	Reduced,
}

/// The `accessibility.` settings: `motion`, the player's [`MotionPreference`].
pub(crate) const SETTINGS: &[Setting<MotionPreference>] = &[Setting {
	name: "motion",
	// The words follow the variants' order, so a variant's index is its word's.
	accepts: Accepts::Choice(&["full", "reduced"]),
	value: |motion| Read::Choice(motion as usize),
	apply: |_, read| {
		Ok(if read.choice() == 0 {
			MotionPreference::Full
		} else {
			MotionPreference::Reduced
		})
	},
}];

#[cfg(test)]
mod tests {
	use super::*;
	use crate::configuration::{ConfigurationValue, apply_setting};

	#[test]
	fn motion_takes_full_or_reduced_in_any_case() {
		let text = |value: &str| ConfigurationValue::Text(value.to_string());
		let apply = |value| apply_setting(MotionPreference::Full, SETTINGS, "accessibility.", "motion", &value);

		assert_eq!(apply(text("reduced")).unwrap().0, MotionPreference::Reduced);
		assert_eq!(apply(text("Reduced")).unwrap().0, MotionPreference::Reduced);
		assert_eq!(apply(text("full")).unwrap().0, MotionPreference::Full);
		assert!(apply(text("none")).is_err());
		assert!(apply(ConfigurationValue::Integer(1)).is_err());
	}
}
