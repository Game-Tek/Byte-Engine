//! How the editor's surfaces move: they glide in on one decelerating curve, [`arrive`], landing in place without
//! overshooting, and accelerate back out. The scene drawer moves so as the editor opens and closes, the toolbar as an
//! entity is selected and deselected, and a resource window's drawer as what the window shows needs the window's
//! width or gives it back; the palette and the detail level slider move on the same curve.
//!
//! These are pure functions of the time since the drawer started moving; the shell places the drawer by them, and the
//! editor keeps its render pass drawing until the drawer has left.

/// How long the drawer takes to arrive once it opens, in seconds.
pub(crate) const ENTER: f32 = 0.35;
/// How long the drawer takes to leave once it closes, in seconds.
pub(crate) const EXIT: f32 = 0.22;

/// How far along a move that's `progress` of the way through its time stands, from 0 to 1: fast at first and
/// slowing smoothly into place, a quartic ease-out, the curve every surface of the editor arrives on.
pub(crate) fn arrive(progress: f32) -> f32 {
	1.0 - (1.0 - progress.clamp(0.0, 1.0)).powi(4)
}

/// How much of the drawer shows `seconds` after it opened: 0 hidden, 1 in place, along [`arrive`].
pub(crate) fn entering(seconds: f32) -> f32 {
	arrive(seconds / ENTER)
}

/// How much of the drawer shows `seconds` after the editor closed: it starts in place and leaves faster and faster, a
/// cubic ease-in, until it's hidden at [`EXIT`].
pub(crate) fn leaving(seconds: f32) -> f32 {
	let progress = (seconds / EXIT).clamp(0.0, 1.0);
	1.0 - progress * progress * progress
}

/// How much of a drawer shows `seconds` after it started toward `open` from showing `from`: it arrives or leaves as
/// [`entering`] and [`leaving`] move a drawer from hidden or in place, scaled to start where it was, so a drawer turned
/// back partway doesn't jump.
pub(crate) fn toward(from: f32, open: bool, seconds: f32) -> f32 {
	if open {
		from + (1.0 - from) * entering(seconds)
	} else {
		from * leaving(seconds)
	}
}

/// Whether a drawer that started toward `open` `seconds` ago has stopped moving.
pub(crate) fn settled(open: bool, seconds: f32) -> bool {
	seconds >= if open { ENTER } else { EXIT }
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn a_drawer_turned_back_partway_starts_where_it_was() {
		let partway = toward(1.0, false, EXIT * 0.5);
		assert_eq!(toward(partway, true, 0.0), partway, "it opens again from where it was");
		assert_eq!(toward(partway, true, ENTER), 1.0, "and ends in place");
		assert_eq!(toward(partway, false, EXIT), 0.0, "or ends hidden");
		assert!(settled(true, ENTER) && !settled(false, EXIT * 0.5));
	}

	/// The fraction shown at each hundredth of a second over `seconds`.
	fn samples(motion: fn(f32) -> f32, seconds: f32) -> Vec<f32> {
		(0..=(seconds * 100.0) as usize).map(|step| motion(step as f32 / 100.0)).collect()
	}

	#[test]
	fn the_drawer_arrives_slower_and_slower_and_never_past_its_place() {
		let shown = samples(entering, ENTER);
		assert_eq!(shown[0], 0.0, "it starts hidden");
		assert_eq!(entering(ENTER), 1.0, "it ends in place");
		assert!(shown.iter().all(|shown| *shown <= 1.0), "it never overshoots");
		let steps: Vec<f32> = shown.windows(2).map(|pair| pair[1] - pair[0]).collect();
		assert!(
			steps.windows(2).all(|pair| pair[1] <= pair[0]),
			"each step is at most the last"
		);
	}

	#[test]
	fn the_drawer_leaves_faster_and_faster_until_hidden() {
		let shown = samples(leaving, EXIT);
		assert_eq!(shown[0], 1.0, "it starts in place");
		assert_eq!(leaving(EXIT), 0.0, "it ends hidden");
		let steps: Vec<f32> = shown.windows(2).map(|pair| pair[0] - pair[1]).collect();
		assert!(steps.windows(2).all(|pair| pair[1] >= pair[0]), "each step is at least the last");
	}
}
