//! Accessibility: the player's preferences that change how the engine presents a game, such as reduced motion. See
//! [Reduced motion](/docs/reference/gui#reduced-motion).
//!
//! The application keeps each preference under the `accessibility.` settings, follows the operating system's
//! [`ghi::window::Preferences`] where the player didn't choose, and hands the result to the systems it changes:
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

/// The `accessibility.` settings: `motion`, the player's [`MotionPreference`], or `None` under `system` to follow the
/// operating system's.
pub(crate) const SETTINGS: &[Setting<Option<MotionPreference>>] = &[Setting {
	name: "motion",
	accepts: Accepts::Choice(&["system", "full", "reduced"]),
	value: |motion| {
		Read::Choice(match motion {
			None => 0,
			Some(MotionPreference::Full) => 1,
			Some(MotionPreference::Reduced) => 2,
		})
	},
	apply: |_, read| {
		Ok(match read.choice() {
			0 => None,
			1 => Some(MotionPreference::Full),
			_ => Some(MotionPreference::Reduced),
		})
	},
}];

/// The motion preference in effect: the player's `chosen` one, or, when they follow the system, `Reduced` while the
/// operating system asks to reduce motion and `Full` otherwise, including when it doesn't say.
pub(crate) fn motion_in_effect(chosen: Option<MotionPreference>, system: &ghi::window::Preferences) -> MotionPreference {
	chosen.unwrap_or(match system.reduce_motion {
		Some(true) => MotionPreference::Reduced,
		Some(false) | None => MotionPreference::Full,
	})
}

#[cfg(test)]
mod tests {
	use super::*;
	use crate::configuration::{ConfigurationValue, apply_setting};

	#[test]
	fn motion_takes_system_full_or_reduced_in_any_case() {
		let text = |value: &str| ConfigurationValue::Text(value.to_string());
		let apply = |value| apply_setting(None, SETTINGS, "accessibility.", "motion", &value);

		assert_eq!(apply(text("reduced")).unwrap().0, Some(MotionPreference::Reduced));
		assert_eq!(apply(text("Full")).unwrap().0, Some(MotionPreference::Full));
		assert_eq!(apply(text("SYSTEM")).unwrap().0, None);
		assert!(apply(text("none")).is_err());
		assert!(apply(ConfigurationValue::Integer(1)).is_err());
	}

	#[test]
	fn a_choice_wins_over_the_system_and_the_system_counts_only_when_it_asks_to_reduce() {
		let system = |reduce_motion| ghi::window::Preferences {
			reduce_motion,
			..Default::default()
		};
		use MotionPreference::{Full, Reduced};

		assert_eq!(motion_in_effect(Some(Full), &system(Some(true))), Full);
		assert_eq!(motion_in_effect(Some(Reduced), &system(Some(false))), Reduced);
		assert_eq!(motion_in_effect(None, &system(Some(true))), Reduced);
		assert_eq!(motion_in_effect(None, &system(Some(false))), Full);
		assert_eq!(motion_in_effect(None, &system(None)), Full);
	}
}
