//! How the editor's surfaces move: they glide in on one decelerating curve, [`arriving`], landing in place without
//! overshooting, and speed up as they leave, [`leaving`]. The scene drawer moves so as the editor opens and closes,
//! the toolbar as an entity is selected and deselected, and a resource window's drawer as what the window shows needs
//! the window's width or gives it back; the palette and the detail level slider arrive on the same curve.
//!
//! What follows the selection chases it on one curve of its own, [`following`]: the scene list's highlight slides to
//! the selected entity's row, and the editor's camera glides to frame the entity, so both arrive together.
//!
//! A surface keeps how much of it shows as an [`Animation`] from 0, hidden, to 1, in place, which [`head`] turns
//! toward open or closed; the editor keeps its render pass drawing until the scene drawer has left, [`EXIT`].

use crate::ui::animation::Smoothing;
use crate::ui::{Animation, Easing, ease_in, ease_out_quart};

/// How long a drawer takes to arrive once it opens, in seconds.
pub(crate) const ENTER: f32 = 0.35;
/// How long a drawer takes to leave once it closes, in seconds.
pub(crate) const EXIT: f32 = 0.22;

/// The curve every surface of the editor arrives on, over `duration` seconds: fast at first, slowing smoothly into
/// place, a quartic ease-out.
pub(crate) fn arriving(duration: f32) -> Easing {
	ease_out_quart(duration)
}

/// The curve a surface leaves on, over `duration` seconds: it starts in place and leaves faster and faster.
pub(crate) fn leaving(duration: f32) -> Easing {
	ease_in(duration)
}

/// How long what follows the selection takes to cover half its way there, in seconds.
const FOLLOW_HALF_LIFE: f32 = 0.015;

/// The curve what follows the selection chases it on: the scene list's highlight sliding to the selected row and the
/// editor's camera gliding to frame the selected entity. Both close the same share of their way in the same time, so
/// they move and land together however far each has to go.
pub(crate) fn following() -> Smoothing {
	Smoothing::default().half_life(FOLLOW_HALF_LIFE)
}

/// How much of a surface shows, at rest at `shown`, from 0, hidden, to 1, in place.
pub(crate) fn shown(shown: f32) -> Animation<f32, Easing> {
	Animation::new(shown, arriving(ENTER))
}

/// Turns `shown` toward `open`, arriving over `enter` seconds or leaving over `exit`, from wherever it stands, so a
/// surface turned back partway doesn't jump. A surface already headed there keeps moving as it was.
pub(crate) fn head(shown: &mut Animation<f32, Easing>, open: bool, enter: f32, exit: f32) {
	let target = if open { 1.0 } else { 0.0 };
	if shown.target() != target {
		shown.set_motion(if open { arriving(enter) } else { leaving(exit) });
		shown.set_target(target);
	}
}

#[cfg(test)]
mod tests {
	use super::*;
	use crate::time::MediaTime;

	#[test]
	fn a_surface_turned_back_partway_starts_where_it_was_and_arrives_on_time() {
		let mut drawer = shown(1.0);
		head(&mut drawer, false, ENTER, EXIT);
		let partway = drawer.step(MediaTime::from_seconds_f32(EXIT * 0.5));
		head(&mut drawer, true, ENTER, EXIT);
		assert_eq!(drawer.current(), partway, "it opens again from where it was");
		assert_eq!(drawer.step(MediaTime::from_seconds_f32(ENTER)), 1.0, "and ends in place");
		assert!(drawer.is_settled());
	}

	#[test]
	fn a_surface_arrives_slower_and_slower_and_never_past_its_place() {
		let mut drawer = shown(0.0);
		head(&mut drawer, true, ENTER, EXIT);
		let values: Vec<f32> = (0..40).map(|_| drawer.step(MediaTime::from_seconds_f32(0.01))).collect();
		assert!(values.iter().all(|value| *value <= 1.0), "it never overshoots");
		let steps: Vec<f32> = values.windows(2).map(|pair| pair[1] - pair[0]).collect();
		assert!(
			steps.windows(2).all(|pair| pair[1] <= pair[0] + 1e-6),
			"each step is at most the last"
		);
	}
}
