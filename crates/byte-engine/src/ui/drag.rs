//! Typed pointer drags for application-owned input routing.
//!
//! Resolve a source with [`super::intersection::HitTest`], then pass its identity
//! and payload to [`Drag::press`]. Keep forwarding the captured pointer's motion
//! and release even outside that source. The application decides whether a
//! [`DragDrop`] belongs to a valid target and how to apply its payload.

use super::{Id, UiPoint};

/// The `Drag` struct supports a captured pointer gesture with an owned payload.
///
/// Use one instance per concurrent drag. `Owner` identifies the pointer or input
/// seat; use `()` when the caller already isolates one pointer. Positions and the
/// movement threshold share the same layout units. No operation allocates or
/// clones the payload. Next, call [`Self::press`] after resolving a drag source.
pub struct Drag<T, Owner = ()> {
	threshold_squared: f32,
	capture: Option<DragCapture<T, Owner>>,
}

/// The `DragCapture` struct supports rendering feedback for the captured source.
///
/// Read it through [`Drag::capture`] to draw a preview or reserve the source's
/// place. Keep the source available until the application accepts a [`DragDrop`];
/// clearing capture then restores it after cancellation or an invalid drop.
#[derive(Debug)]
pub struct DragCapture<T, Owner = ()> {
	/// The pointer or input seat that owns this gesture.
	pub owner: Owner,
	/// The retained UI source identity.
	pub source: Id,
	/// The application data being dragged.
	pub payload: T,
	/// The press position in layout units.
	pub origin: UiPoint,
	/// The latest captured pointer position in layout units.
	pub position: UiPoint,
	/// Whether the pointer has crossed the movement threshold during this gesture.
	pub dragging: bool,
}

/// The `DragDrop` struct supports applying a released payload to a drop target.
///
/// Receive it from [`Drag::release`], validate `position` against the target, and
/// apply `payload` only when accepted. Keep or restore the source when the target
/// rejects the payload.
#[derive(Debug)]
pub struct DragDrop<T> {
	/// The retained UI source identity.
	pub source: Id,
	/// The application data transferred from the captured gesture.
	pub payload: T,
	/// The release position in layout units, including positions outside the UI.
	pub position: UiPoint,
}

impl<T, Owner: Copy + PartialEq> Drag<T, Owner> {
	/// Creates an idle gesture with a positive, finite movement threshold.
	///
	/// Next, call [`Self::press`] with a hit-tested source and its payload.
	pub fn new(threshold: f32) -> Self {
		assert!(
			threshold.is_finite() && threshold > 0.0,
			"Drag threshold is invalid. The most likely cause is a nonpositive or nonfinite layout distance."
		);
		Self {
			threshold_squared: threshold * threshold,
			capture: None,
		}
	}

	/// Captures a source until release or cancellation.
	///
	/// Returns the supplied payload unchanged if another gesture is already held.
	/// Next, forward pointer motion through [`Self::move_to`] without hit testing
	/// the source again.
	pub fn press(&mut self, owner: Owner, source: Id, payload: T, position: UiPoint) -> Result<(), T> {
		if self.capture.is_some() {
			return Err(payload);
		}
		self.capture = Some(DragCapture {
			owner,
			source,
			payload,
			origin: position,
			position,
			dragging: false,
		});
		Ok(())
	}

	/// Returns the captured gesture for source and preview feedback.
	pub fn capture(&self) -> Option<&DragCapture<T, Owner>> {
		self.capture.as_ref()
	}

	/// Updates the captured pointer and reports whether it owns the gesture.
	///
	/// Once activated, a drag stays active even if it returns to its press position.
	/// Next, call [`Self::release`] when the pointer is released.
	pub fn move_to(&mut self, owner: Owner, position: UiPoint) -> bool {
		let Some(capture) = self.capture.as_mut().filter(|capture| capture.owner == owner) else {
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
	/// separate motion event arrived. A click clears capture without yielding a
	/// drop; another owner's release leaves capture untouched. Next, validate the
	/// returned position before applying its payload.
	pub fn release(&mut self, owner: Owner, position: UiPoint) -> Option<DragDrop<T>> {
		if !self.move_to(owner, position) {
			return None;
		}
		let capture = self.capture.take()?;
		capture.dragging.then_some(DragDrop {
			source: capture.source,
			payload: capture.payload,
			position: capture.position,
		})
	}

	/// Clears capture and returns its payload without producing a drop.
	///
	/// Call this when focus is lost, the source is removed, or the user cancels.
	/// Next, render the source without capture feedback or call [`Self::press`]
	/// to begin another gesture.
	pub fn cancel(&mut self) -> Option<T> {
		self.capture.take().map(|capture| capture.payload)
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn click_restores_source_without_a_drop() {
		let mut drag = Drag::new(5.0);
		drag.press((), Id::new(1).unwrap(), "card", UiPoint::new(10.0, 20.0)).unwrap();
		assert!(!drag.capture().unwrap().dragging);
		assert!(drag.release((), UiPoint::new(12.0, 22.0)).is_none());
		assert!(drag.capture().is_none());
	}

	#[test]
	fn drag_keeps_its_source_outside_bounds_and_back_at_the_press_position() {
		let source = Id::new(2).unwrap();
		let origin = UiPoint::new(10.0, 20.0);
		let mut drag = Drag::new(5.0);
		drag.press((), source, 42, origin).unwrap();
		assert!(drag.move_to((), UiPoint::new(-200.0, -300.0)));
		assert_eq!(drag.capture().unwrap().source, source);
		assert_eq!(drag.capture().unwrap().position, UiPoint::new(-200.0, -300.0));
		assert!(drag.move_to((), origin));
		assert!(drag.capture().unwrap().dragging);

		let dropped = drag.release((), origin).unwrap();
		assert_eq!(dropped.source, source);
		assert_eq!(dropped.payload, 42);
		assert_eq!(dropped.position, origin);
		assert!(drag.capture().is_none());
		assert!(drag.release((), origin).is_none());
	}

	#[test]
	fn other_owners_cannot_replace_move_or_release_a_capture() {
		let origin = UiPoint::zero();
		let destination = UiPoint::new(3.0, 4.0);
		let mut drag = Drag::new(5.0);
		drag.press(1, Id::new(1).unwrap(), "first", origin).unwrap();
		assert_eq!(drag.press(2, Id::new(2).unwrap(), "second", origin), Err("second"));
		assert!(!drag.move_to(2, destination));
		assert!(drag.release(2, destination).is_none());
		assert_eq!(drag.capture().unwrap().position, origin);

		// The release itself can supply the motion that reaches the threshold.
		let dropped = drag.release(1, destination).unwrap();
		assert_eq!(dropped.payload, "first");
		assert_eq!(dropped.position, destination);
	}

	#[test]
	fn cancellation_returns_owned_payload_and_allows_another_gesture() {
		#[derive(Debug)]
		struct Payload;

		let mut drag = Drag::new(5.0);
		let source = Id::new(1).unwrap();
		drag.press((), source, Payload, UiPoint::zero()).unwrap();
		drag.move_to((), UiPoint::new(10.0, 0.0));
		let payload = drag.cancel().unwrap();
		assert!(drag.capture().is_none());
		assert!(drag.release((), UiPoint::new(10.0, 0.0)).is_none());
		drag.press((), source, payload, UiPoint::zero()).unwrap();
		assert!(!drag.capture().unwrap().dragging);
	}
}
