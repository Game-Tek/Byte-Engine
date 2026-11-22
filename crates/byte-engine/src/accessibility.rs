//! Accessibility: the player's preferences that change how the engine presents a game. See
//! [Reduced motion](/docs/reference/gui#reduced-motion) and [Text scale](/docs/reference/gui#text-scale).
//!
//! The application keeps each preference under the `accessibility.` settings, follows the operating system's
//! [`ghi::window::Preferences`] where the player didn't choose, and hands the result to the systems it changes:
//! [`crate::application::graphics::GraphicsApplication::motion`] gives a UI engine its [`MotionPreference`] through
//! [`crate::ui::layout::engine::Engine::set_motion`], and
//! [`crate::application::graphics::GraphicsApplication::text_scale`] its text scale through
//! [`crate::ui::layout::engine::Engine::set_text_scale`].

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

/// The `Choices` struct holds what the player set under `accessibility.`, where `None` follows the operating system.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub(crate) struct Choices {
	/// The `motion` setting.
	pub(crate) motion: Option<MotionPreference>,
	/// The `text-scale` setting.
	pub(crate) text_scale: Option<f32>,
}

/// The `accessibility.` settings: `motion`, the player's [`MotionPreference`], and `text-scale`, the factor every
/// text's font size is multiplied by. Under `system`, each follows the operating system's preference.
pub(crate) const SETTINGS: &[Setting<Choices>] = &[
	Setting {
		name: "motion",
		accepts: Accepts::Choice(&["system", "full", "reduced"]),
		value: |choices| {
			Read::Choice(match choices.motion {
				None => 0,
				Some(MotionPreference::Full) => 1,
				Some(MotionPreference::Reduced) => 2,
			})
		},
		apply: |choices, read| {
			let motion = match read.choice() {
				0 => None,
				1 => Some(MotionPreference::Full),
				_ => Some(MotionPreference::Reduced),
			};
			Ok(Choices { motion, ..choices })
		},
	},
	Setting {
		name: "text-scale",
		accepts: Accepts::NumberOr {
			min: 0.0,
			word: "system",
		},
		value: |choices| choices.text_scale.map_or(Read::Word, |scale| Read::Number(f64::from(scale))),
		apply: |choices, read| {
			let text_scale = match read {
				Read::Word => None,
				read => Some(read.number() as f32),
			};
			if text_scale == Some(0.0) {
				return Err(
					"The text scale 0 would draw no text. The most likely cause is a typo. Set `accessibility.text-scale` above 0, such as 1.5, or to `system`."
						.to_string(),
				);
			}
			Ok(Choices { text_scale, ..choices })
		},
	},
];

/// The motion preference in effect: the player's `chosen` one, or, when they follow the system, `Reduced` while the
/// operating system asks to reduce motion and `Full` otherwise, including when it doesn't say.
pub(crate) fn motion_in_effect(chosen: Option<MotionPreference>, system: &ghi::window::Preferences) -> MotionPreference {
	chosen.unwrap_or(match system.reduce_motion {
		Some(true) => MotionPreference::Reduced,
		Some(false) | None => MotionPreference::Full,
	})
}

/// The text scale in effect: the player's `chosen` one, or, when they follow the system, the operating system's, and 1
/// where it doesn't say.
pub(crate) fn text_scale_in_effect(chosen: Option<f32>, system: &ghi::window::Preferences) -> f32 {
	chosen.or(system.text_scale).unwrap_or(1.0)
}

#[cfg(test)]
mod tests {
	use super::*;
	use crate::configuration::{ConfigurationValue, apply_setting};

	#[test]
	fn motion_takes_system_full_or_reduced_in_any_case() {
		let text = |value: &str| ConfigurationValue::Text(value.to_string());
		let apply = |value| {
			apply_setting(Choices::default(), SETTINGS, "accessibility.", "motion", &value).map(|(choices, _)| choices.motion)
		};

		assert_eq!(apply(text("reduced")).unwrap(), Some(MotionPreference::Reduced));
		assert_eq!(apply(text("Full")).unwrap(), Some(MotionPreference::Full));
		assert_eq!(apply(text("SYSTEM")).unwrap(), None);
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

	#[test]
	fn text_scale_takes_a_number_above_zero_or_system() {
		let apply = |value| {
			apply_setting(Choices::default(), SETTINGS, "accessibility.", "text-scale", &value)
				.map(|(choices, _)| choices.text_scale)
		};

		assert_eq!(apply(ConfigurationValue::Float(1.5)).unwrap(), Some(1.5));
		assert_eq!(apply(ConfigurationValue::Text("2".to_string())).unwrap(), Some(2.0));
		assert_eq!(apply(ConfigurationValue::Text("System".to_string())).unwrap(), None);
		assert!(apply(ConfigurationValue::Float(0.0)).is_err());
		assert!(apply(ConfigurationValue::Float(-1.0)).is_err());
		assert!(apply(ConfigurationValue::Text("large".to_string())).is_err());
	}

	#[test]
	fn a_text_scale_choice_wins_over_the_system_and_the_system_defaults_to_one() {
		let system = |text_scale| ghi::window::Preferences {
			text_scale,
			..Default::default()
		};

		assert_eq!(text_scale_in_effect(Some(2.0), &system(Some(1.25))), 2.0);
		assert_eq!(text_scale_in_effect(None, &system(Some(1.25))), 1.25);
		assert_eq!(text_scale_in_effect(None, &system(None)), 1.0);
	}
}
