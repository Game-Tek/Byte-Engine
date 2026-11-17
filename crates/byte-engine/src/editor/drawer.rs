//! How a drawer moves as it opens and closes: it springs in from the window's edge, settling with a slight overshoot,
//! and accelerates back out. The scene drawer moves so as the editor opens and closes, and a resource window's drawer
//! as what the window shows needs the window's width or gives it back.
//!
//! These are pure functions of the time since the drawer started moving; the shell places the drawer by them, and the
//! editor keeps its render pass drawing until the drawer has left.

/// How long the drawer takes to settle once the editor opens, in seconds.
pub(crate) const ENTER: f32 = 0.5;
/// How long the drawer takes to leave once the editor closes, in seconds.
pub(crate) const EXIT: f32 = 0.22;

/// The spring's damping ratio: under 1, so it overshoots a little, about 3%, before it settles.
const DAMPING: f32 = 0.75;
/// The spring's undamped angular frequency, in radians per second, which settles it in about 0.4 seconds.
const FREQUENCY: f32 = 14.0;

/// How much of the drawer shows `seconds` after the editor opened: 0 hidden, 1 in place, a little over 1 while it
/// overshoots. This is the step response of a damped spring.
pub(crate) fn entering(seconds: f32) -> f32 {
	if seconds >= ENTER {
		return 1.0;
	}
	let t = seconds.max(0.0);
	let decay = DAMPING * FREQUENCY;
	let damped = FREQUENCY * (1.0 - DAMPING * DAMPING).sqrt();
	1.0 - (-decay * t).exp() * ((damped * t).cos() + decay / damped * (damped * t).sin())
}

/// How much of the drawer shows `seconds` after the editor closed: it starts in place and leaves faster and faster, a
/// cubic ease-in, until it's hidden at [`EXIT`].
pub(crate) fn leaving(seconds: f32) -> f32 {
	let progress = (seconds / EXIT).clamp(0.0, 1.0);
	1.0 - progress * progress * progress
}

/// How much of a drawer shows `seconds` after it started toward `open` from showing `from`: it springs in or leaves as
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
	fn the_drawer_springs_in_from_hidden_and_settles_in_place() {
		let shown = samples(entering, ENTER);
		assert_eq!(shown[0], 0.0, "it starts hidden");
		assert_eq!(*shown.last().unwrap(), 1.0, "it ends in place");
		let peak = shown.iter().copied().fold(0.0, f32::max);
		assert!(peak > 1.0 && peak < 1.04, "it overshoots a little, by {peak}");
		let settled = entering(ENTER - 0.01);
		assert!((settled - 1.0).abs() < 0.01, "it has settled before it snaps in place: {settled}");
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
