//! The toasts the editor shows for the game's warnings and errors, stacked as Sonner stacks them.
//!
//! This module holds what each toast says, how long it has shown, and where the stack places it. The editor posts the
//! log's warnings and errors here, and the shell draws each toast as a card that eases toward its placement.

/// How long a toast shows before it leaves on its own, in seconds, as Sonner's default.
pub(crate) const LIFETIME: f32 = 4.0;
/// How many toasts the stack shows; posting another sends the oldest away.
pub(crate) const VISIBLE: usize = 3;
/// How long a leaving toast takes to fade before it's dropped, in seconds.
pub(crate) const EXIT: f32 = 0.2;
/// How long the stack stays spread out after the pointer leaves it, so crossing the gap between two cards doesn't
/// collapse it.
const LINGER: f32 = 0.15;

/// A toast card's size, in pixels: Sonner's width, and room for a title and a detail line.
pub(crate) const WIDTH: f32 = 356.0;
pub(crate) const HEIGHT: f32 = 58.0;
/// How far each toast behind the front one peeks out above it while the stack is collapsed.
const PEEK: f32 = 14.0;
/// How much smaller each toast behind the front one is while the stack is collapsed.
const SHRINK: f32 = 0.05;
/// The gap between toasts while the stack is spread out.
const GAP: f32 = 14.0;

/// The `Toast` struct is one warning or error the stack shows.
#[derive(Clone, Debug)]
pub(crate) struct Toast {
	/// Names the toast's card, which stays the same while it moves through the stack.
	pub(crate) id: u64,
	pub(crate) level: log::Level,
	/// The message's first sentence: what happened.
	pub(crate) title: String,
	/// The rest of the message's first line, such as the likely cause.
	pub(crate) detail: String,
	/// How many times the message was posted while the toast showed.
	pub(crate) count: u32,
	/// Whether the pointer is over the toast's card.
	pub(crate) hovered: bool,
	/// How long the toast has shown since it was last posted, not counting time the stack was spread out.
	shown: f32,
	/// How long the toast has been leaving, once it is.
	leaving: Option<f32>,
}

/// The `Placement` struct is where the stack puts a toast's card, relative to the front card's resting place.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Placement {
	/// How far down the card sits, in pixels; negative is up.
	pub(crate) y: f32,
	/// The card's scale about its top edge.
	pub(crate) scale: f32,
	pub(crate) opacity: f32,
	/// Orders the cards, so newer ones draw over older ones.
	pub(crate) depth: i32,
}

/// The `Toasts` struct is the stack: every toast showing or leaving, oldest first.
#[derive(Default)]
pub(crate) struct Toasts {
	list: Vec<Toast>,
	next_id: u64,
	/// How much longer the stack stays spread out after the pointer left it.
	linger: f32,
}

impl Toasts {
	/// Shows `message` at `level`, or, when a showing toast already says it, counts it again and brings that toast
	/// back to the front with its full time.
	pub(crate) fn post(&mut self, level: log::Level, message: &str) {
		let (title, detail) = split(message);
		let repeated = self.list.iter().position(|toast| {
			toast.leaving.is_none() && toast.level == level && toast.title == title && toast.detail == detail
		});
		let toast = match repeated {
			Some(index) => {
				let mut toast = self.list.remove(index);
				toast.count += 1;
				toast.shown = 0.0;
				toast
			}
			None => {
				self.next_id += 1;
				Toast {
					id: self.next_id,
					level,
					title: title.to_string(),
					detail: detail.to_string(),
					count: 1,
					hovered: false,
					shown: 0.0,
					leaving: None,
				}
			}
		};
		self.list.push(toast);
		// Past the visible count, the oldest showing toast leaves.
		if self.showing().count() > VISIBLE
			&& let Some(oldest) = self.list.iter_mut().find(|toast| toast.leaving.is_none())
		{
			oldest.leaving = Some(0.0);
		}
	}

	/// Sends the toast `id` away.
	pub(crate) fn dismiss(&mut self, id: u64) {
		if let Some(toast) = self.list.iter_mut().find(|toast| toast.id == id) {
			toast.leaving.get_or_insert(0.0);
		}
	}

	/// Ages the toasts by `dt` seconds: those past their lifetime start leaving, and those done leaving are dropped.
	/// While the stack is spread out, showing toasts don't age, so one being read doesn't leave.
	pub(crate) fn advance(&mut self, dt: f32) {
		self.linger = if self.list.iter().any(|toast| toast.hovered) {
			LINGER
		} else {
			(self.linger - dt).max(0.0)
		};
		let expanded = self.expanded();
		for toast in &mut self.list {
			match &mut toast.leaving {
				Some(leaving) => *leaving += dt,
				None if !expanded => {
					toast.shown += dt;
					if toast.shown >= LIFETIME {
						toast.leaving = Some(0.0);
					}
				}
				None => {}
			}
		}
		self.list.retain(|toast| toast.leaving.is_none_or(|leaving| leaving < EXIT));
	}

	/// Whether the stack is spread out, which it is while the pointer is over it and shortly after.
	pub(crate) fn expanded(&self) -> bool {
		self.linger > 0.0
	}

	/// Every toast showing or leaving, oldest first.
	pub(crate) fn list(&self) -> &[Toast] {
		&self.list
	}

	/// Returns the toast `id`, while it's showing or leaving.
	pub(crate) fn get_mut(&mut self, id: u64) -> Option<&mut Toast> {
		self.list.iter_mut().find(|toast| toast.id == id)
	}

	/// Returns where the stack puts the toast `id`'s card, or `None` once it's dropped.
	///
	/// The newest showing toast is at the front. Collapsed, each older one peeks out a little higher and smaller
	/// behind it; spread out, each sits a card's height and a gap above the next. A leaving toast keeps its place and
	/// fades out.
	pub(crate) fn placement(&self, id: u64) -> Option<Placement> {
		let at = self.list.iter().position(|toast| toast.id == id)?;
		let toast = &self.list[at];
		// Count the showing toasts in front of this one; a leaving one no longer holds a place.
		let behind = self.list[at + 1..].iter().filter(|newer| newer.leaving.is_none()).count();
		let (y, scale) = if self.expanded() {
			(-(behind as f32) * (HEIGHT + GAP), 1.0)
		} else {
			(-(behind as f32) * PEEK, 1.0 - behind as f32 * SHRINK)
		};
		let opacity = if toast.leaving.is_some() || behind >= VISIBLE {
			0.0
		} else {
			1.0
		};
		Some(Placement {
			y,
			scale,
			opacity,
			// Sibling cards draw in rising depth, so the front one, with none in front of it, draws last.
			depth: -(behind as i32),
		})
	}

	/// The toasts that aren't leaving.
	fn showing(&self) -> impl Iterator<Item = &Toast> {
		self.list.iter().filter(|toast| toast.leaving.is_none())
	}
}

/// Splits a log message into a toast's title, its first sentence, and its detail, the rest of its first line.
///
/// Engine messages say what happened, then the likely cause, then the fix, so the title is what happened.
fn split(message: &str) -> (&str, &str) {
	let line = message.lines().next().unwrap_or_default().trim();
	match line.find(". ") {
		Some(end) => (&line[..=end], line[end + 1..].trim()),
		None => (line, ""),
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	const FAILURE: &str = "The level could not be spawned. The most likely cause is a typo. Check the ID.\nmore";

	#[test]
	fn a_message_shows_what_happened_over_its_cause() {
		let mut toasts = Toasts::default();
		toasts.post(log::Level::Warn, FAILURE);
		let toast = &toasts.list()[0];
		assert_eq!(toast.title, "The level could not be spawned.");
		assert_eq!(toast.detail, "The most likely cause is a typo. Check the ID.");
	}

	#[test]
	fn a_repeated_message_counts_on_one_toast_brought_to_the_front() {
		let mut toasts = Toasts::default();
		toasts.post(log::Level::Error, FAILURE);
		toasts.post(log::Level::Warn, "Another thing.");
		toasts.post(log::Level::Error, FAILURE);

		let list = toasts.list();
		assert_eq!(list.len(), 2);
		assert_eq!(list[1].title, "The level could not be spawned.");
		assert_eq!(list[1].count, 2);
		assert_eq!(
			toasts.placement(list[1].id).unwrap().y,
			0.0,
			"the repeated toast is at the front"
		);
	}

	#[test]
	fn the_stack_shows_three_and_older_ones_leave() {
		let mut toasts = Toasts::default();
		for message in ["One.", "Two.", "Three.", "Four."] {
			toasts.post(log::Level::Warn, message);
		}
		let first = toasts.list()[0].id;
		assert_eq!(toasts.placement(first).unwrap().opacity, 0.0, "the oldest fades out");

		toasts.advance(EXIT);
		assert_eq!(toasts.list().len(), VISIBLE);
		assert!(toasts.placement(first).is_none());
	}

	#[test]
	fn a_toast_leaves_after_its_lifetime_unless_the_stack_is_spread_out() {
		let mut toasts = Toasts::default();
		toasts.post(log::Level::Warn, "Once.");
		let id = toasts.list()[0].id;

		toasts.get_mut(id).unwrap().hovered = true;
		toasts.advance(LIFETIME * 2.0);
		assert_eq!(toasts.placement(id).unwrap().opacity, 1.0, "a hovered stack keeps its toasts");

		toasts.get_mut(id).unwrap().hovered = false;
		toasts.advance(LIFETIME + LINGER);
		toasts.advance(EXIT);
		assert!(toasts.list().is_empty());
	}

	#[test]
	fn collapsed_toasts_peek_above_the_front_and_spread_out_under_the_pointer() {
		let mut toasts = Toasts::default();
		toasts.post(log::Level::Warn, "Old.");
		toasts.post(log::Level::Warn, "New.");
		let [old, new] = [toasts.list()[0].id, toasts.list()[1].id];

		let (behind, front) = (toasts.placement(old).unwrap(), toasts.placement(new).unwrap());
		assert!(behind.y < front.y && behind.scale < front.scale && behind.depth < front.depth);

		toasts.get_mut(new).unwrap().hovered = true;
		toasts.advance(0.0);
		assert_eq!(toasts.placement(old).unwrap().y, -(HEIGHT + GAP));
		assert_eq!(toasts.placement(old).unwrap().scale, 1.0);
	}
}
