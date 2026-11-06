//! The pointer drag gesture owned by the UI engine.
//!
//! [`super::Engine::press`] hit-tests the pointer and holds the surface it finds.
//! Keep forwarding the captured pointer's motion and release even outside that
//! source. The application decides whether a [`DragDrop`] belongs to a valid
//! target and what its source means.

use super::{Id, UiPoint};

/// The `Drag` struct supports one captured pointer gesture.
///
/// Positions and the movement threshold share the same layout units. No
/// operation allocates.
pub(super) struct Drag {
	threshold_squared: f32,
	/// The held gesture; `None` while no source is held.
	held: Option<Held>,
}

/// The `Held` struct keeps one held gesture: the capture the source and the application read, and the motion the
/// source already heard about.
#[derive(Clone, Copy)]
struct Held {
	capture: DragCapture,
	/// The captured position the source was last told about through [`super::Events::Dragged`].
	reported: Option<UiPoint>,
}

/// The `DragCapture` struct supports rendering feedback for the captured source.
///
/// Read it through [`super::Engine::drag`] or a component's context to draw a
/// preview or reserve the source's place. Keep the source available until the
/// application accepts a [`DragDrop`]; clearing capture then restores it after
/// cancellation or an invalid drop.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct DragCapture {
	/// The retained UI source identity.
	pub source: Id,
	/// The press position in layout units.
	pub origin: UiPoint,
	/// The latest captured pointer position in layout units.
	pub position: UiPoint,
	/// Whether the pointer has crossed the movement threshold during this gesture.
	pub dragging: bool,
	/// The surface under the captured pointer other than the source, from the
	/// last evaluated frame. Read it to preview where a release would drop.
	pub over: Option<Id>,
}

/// The `DragDrop` struct supports applying a released source to a drop target.
///
/// Receive it from [`super::Engine::release`], validate `position` against the
/// target, and apply the source's meaning only when accepted. Keep or restore the
/// source when the target rejects it.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct DragDrop {
	/// The retained UI source identity.
	pub source: Id,
	/// The release position in layout units, including positions outside the UI.
	pub position: UiPoint,
}

impl Drag {
	/// Creates an idle gesture with a positive, finite movement threshold.
	pub(super) fn new(threshold: f32) -> Self {
		assert!(
			threshold.is_finite() && threshold > 0.0,
			"Drag threshold is invalid. The most likely cause is a nonpositive or nonfinite layout distance."
		);
		Self {
			threshold_squared: threshold * threshold,
			held: None,
		}
	}

	/// Captures a source until release or cancellation; a held gesture refuses another.
	pub(super) fn press(&mut self, source: Id, position: UiPoint) -> bool {
		if self.held.is_some() {
			return false;
		}
		self.held = Some(Held {
			capture: DragCapture {
				source,
				origin: position,
				position,
				dragging: false,
				over: None,
			},
			reported: None,
		});
		true
	}

	pub(super) fn capture(&self) -> Option<DragCapture> {
		self.held.map(|held| held.capture)
	}

	/// Records the surface under the captured pointer for the frame just evaluated.
	pub(super) fn set_over(&mut self, over: Option<Id>) {
		if let Some(held) = self.held.as_mut() {
			held.capture.over = over;
		}
	}

	/// Returns the active drag when its source has not heard about its latest position yet, and marks that position
	/// as heard.
	pub(super) fn report_motion(&mut self) -> Option<DragCapture> {
		let held = self.held.as_mut().filter(|held| held.capture.dragging)?;
		if held.reported == Some(held.capture.position) {
			return None;
		}
		held.reported = Some(held.capture.position);
		Some(held.capture)
	}

	/// Updates the captured pointer and reports whether a gesture is held.
	///
	/// Once activated, a drag stays active even if it returns to its press position.
	pub(super) fn move_to(&mut self, position: UiPoint) -> bool {
		let Some(capture) = self.held.as_mut().map(|held| &mut held.capture) else {
			return false;
		};
		capture.position = position;
		let x = position.x - capture.origin.x;
		let y = position.y - capture.origin.y;
		// Latch activation so returning over the source does not turn a drag into a click.
		capture.dragging |= x * x + y * y >= self.threshold_squared;
		true
	}

	/// Releases the captured pointer and returns a drop only after drag activation.
	///
	/// The release position participates in threshold detection, including when no
	/// separate motion event arrived. A click clears capture without yielding a drop.
	pub(super) fn release(&mut self, position: UiPoint) -> Option<DragDrop> {
		if !self.move_to(position) {
			return None;
		}
		let capture = self.held.take()?.capture;
		capture.dragging.then_some(DragDrop {
			source: capture.source,
			position: capture.position,
		})
	}

	/// Clears capture and returns its source without producing a drop.
	pub(super) fn cancel(&mut self) -> Option<Id> {
		self.held.take().map(|held| held.capture.source)
	}
}
