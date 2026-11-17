//! The editor's undo and redo, in three layers: the [`Delta`] algebra of transforms, the [`Operation`]s that change
//! one entity by a delta, and the [`History`] of steps on two stacks. The editor applies an operation through one path
//! whether it's an edit, an undo, or a redo, so undo applies the very operation the edit applied, negated.
//!
//! The rules are specified in [Editor history](/docs/reference/editor-history).

use std::ops::{Add, Neg};

use crate::core::factory::Handle;
use crate::gameplay::Transform;
use crate::math::{Orientation, Scale, Vector};

/// The `Delta` struct is the difference between two transforms, which edits record and undo negates.
///
/// Deltas form a group: `+` applies one, then the other, and `-` undoes one. A
/// transform plus a delta, [`Delta::applied_to`], is a transform, and a transform less another, [`Delta::between`],
/// is the delta between them; a transform less itself is the zero delta, which changes nothing.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Delta {
	/// What the position moves by.
	offset: Vector,
	/// What the orientation turns by, after its own rotation. The position doesn't move with it.
	turn: Orientation,
	/// What the scale grows by, per axis. It adds rather than multiplies, so a scale of zero still has an inverse.
	stretch: Scale,
}

impl Add for Delta {
	type Output = Self;

	/// `self`, then `other`.
	fn add(self, other: Self) -> Self {
		Self {
			offset: self.offset + other.offset,
			turn: other.turn.compose(self.turn),
			stretch: self.stretch + other.stretch,
		}
	}
}

impl Neg for Delta {
	type Output = Self;

	fn neg(self) -> Self {
		Self {
			offset: -self.offset,
			turn: self.turn.inverse(),
			stretch: Scale::new(0.0, 0.0, 0.0) - self.stretch,
		}
	}
}

impl Delta {
	/// The delta that takes `from` to `to`: `to − from`.
	pub(crate) fn between(from: &Transform, to: &Transform) -> Self {
		Self {
			offset: to.get_position() - from.get_position(),
			turn: to.get_orientation().compose(from.get_orientation().inverse()),
			stretch: to.scale() - from.scale(),
		}
	}

	/// `transform + self`: the transform moved, turned, and stretched by this delta.
	pub(crate) fn applied_to(self, transform: &Transform) -> Transform {
		Transform::new(
			transform.get_position() + self.offset,
			transform.scale() + self.stretch,
			self.turn.compose(transform.get_orientation()),
		)
	}
}

/// The `Operation` enum is one change to one entity, which the editor applies to its copy and posts to the game. Its
/// negation undoes it, and two on the same entity add into one. Next, record one in a [`History`].
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum Operation {
	/// Adds `delta` to the entity's transform, posted as a `TransformationUpdate`.
	Transform { entity: Handle, delta: Delta },
}

impl Operation {
	/// `self`, then `other`, as one operation, or `None` when they change different entities or values.
	fn then(self, other: Self) -> Option<Self> {
		match (self, other) {
			(Self::Transform { entity, delta }, Self::Transform { entity: next, delta: more }) if entity == next => {
				Some(Self::Transform {
					entity,
					delta: delta + more,
				})
			}
			_ => None,
		}
	}
}

impl Neg for Operation {
	type Output = Self;

	fn neg(self) -> Self {
		match self {
			Self::Transform { entity, delta } => Self::Transform { entity, delta: -delta },
		}
	}
}

/// The `History` struct keeps what the editor did as steps on two stacks, so Cmd+Z and Cmd+Shift+Z can take them back
/// and apply them again. A step is a sum of operations, one per entity it changed; a pointer gesture records one step.
#[derive(Default)]
pub(crate) struct History {
	/// The steps undo takes back, oldest first.
	undo: Vec<Vec<Operation>>,
	/// The steps redo applies again, most recently undone last.
	redo: Vec<Vec<Operation>>,
	/// Whether a pointer is pressed, so what's recorded joins one step.
	gesture: bool,
	/// Whether the top undo step is the open gesture's, which what's recorded adds to.
	open: bool,
}

impl History {
	/// Starts a gesture: what's recorded until [`Self::end_gesture`] is one step.
	pub(crate) fn begin_gesture(&mut self) {
		self.gesture = true;
		self.open = false;
	}

	/// Ends the gesture, so what's recorded next starts a step of its own.
	pub(crate) fn end_gesture(&mut self) {
		self.gesture = false;
		self.open = false;
	}

	/// Records `operation`, which the editor applied: it adds to the open gesture's step, or starts a step. Redo has
	/// nothing left to apply once something new is done.
	pub(crate) fn record(&mut self, operation: Operation) {
		self.redo.clear();
		match self.undo.last_mut() {
			Some(step) if self.open => {
				// An operation on an entity the step already changed adds to that entity's.
				let combined = step.iter_mut().find_map(|done| done.then(operation).map(|sum| (done, sum)));
				match combined {
					Some((done, sum)) => *done = sum,
					None => step.push(operation),
				}
			}
			_ => {
				self.undo.push(vec![operation]);
				self.open = self.gesture;
			}
		}
	}

	/// Moves the last step to the redo stack and returns what undoes it: its operations, negated, in reverse order.
	/// Returns nothing when there's nothing to undo.
	pub(crate) fn undo(&mut self) -> impl Iterator<Item = Operation> + '_ {
		// What's recorded after an undo starts a step, even within a gesture.
		self.open = false;
		let step = self.undo.pop();
		let undone = step.map(|step| {
			self.redo.push(step);
			&self.redo[self.redo.len() - 1]
		});
		undone.into_iter().flat_map(|step| step.iter().rev().map(|operation| -*operation))
	}

	/// Moves the last undone step back to the undo stack and returns its operations, to apply again. Returns nothing
	/// when nothing was undone.
	pub(crate) fn redo(&mut self) -> impl Iterator<Item = Operation> + '_ {
		self.open = false;
		let step = self.redo.pop();
		let redone = step.map(|step| {
			self.undo.push(step);
			&self.undo[self.undo.len() - 1]
		});
		redone.into_iter().flat_map(|step| step.iter().copied())
	}
}

#[cfg(test)]
mod tests {
	use super::*;
	use crate::math::{Point, Radians};

	fn transform(position: [f32; 3], turn: f32, scale: [f32; 3]) -> Transform {
		let axis: crate::math::UnitVector = Vector::new(0.0, 1.0, 0.0).normalized().unwrap();
		Transform::new(
			Point::new(position[0], position[1], position[2]),
			Scale::from_array(scale),
			Orientation::try_from_axis_angle(axis, Radians::new(turn)).unwrap(),
		)
	}

	/// Whether two transforms are the same within rounding; a rotation and its negated quaternion are the same.
	fn same(a: &Transform, b: &Transform) -> bool {
		let close = |x: [f32; 3], y: [f32; 3]| x.iter().zip(y).all(|(x, y)| (x - y).abs() < 1e-4);
		close(a.get_position().to_array(), b.get_position().to_array())
			&& close(a.scale().to_array(), b.scale().to_array())
			&& a.get_orientation().dot(b.get_orientation()).abs() > 1.0 - 1e-5
	}

	#[test]
	fn deltas_follow_the_group_laws() {
		let a = transform([1.0, 2.0, 3.0], 0.4, [1.0, 2.0, 0.0]);
		let b = transform([-4.0, 0.5, 7.0], -1.3, [0.0, 3.0, 2.0]);
		let c = transform([0.0, 9.0, -2.0], 2.1, [5.0, 1.0, 1.0]);
		let (d, e, f) = (Delta::between(&a, &b), Delta::between(&b, &c), Delta::between(&c, &a));

		assert!(same(&d.applied_to(&a), &b), "difference");
		assert!(same(&(-d).applied_to(&d.applied_to(&a)), &a), "inverse, a scale of zero included");
		assert!(same(&(d + -d).applied_to(&a), &a), "a delta plus its negation is zero");
		let zero = Delta::between(&a, &a);
		assert!(same(&zero.applied_to(&b), &b) && same(&(d + zero).applied_to(&a), &b), "identity");
		assert!(same(&((d + e) + f).applied_to(&a), &(d + (e + f)).applied_to(&a)), "associativity");
		assert!(same(&(d + e).applied_to(&a), &c), "adding applies one, then the other");
	}

	/// What applying `operations` to one entity leaves `start` as.
	fn apply(start: &Transform, operations: impl Iterator<Item = Operation>) -> Transform {
		operations.fold(start.clone(), |transform, Operation::Transform { delta, .. }| delta.applied_to(&transform))
	}

	#[test]
	fn a_gesture_is_one_step_that_undo_and_redo_take_back_and_apply_again() {
		let entity = Handle::new();
		let start = transform([0.0, 0.0, 0.0], 0.0, [1.0, 1.0, 1.0]);
		let moves = [
			transform([1.0, 0.0, 0.0], 0.2, [1.0, 1.0, 1.0]),
			transform([2.0, 1.0, 0.0], 0.5, [2.0, 1.0, 1.0]),
		];
		let mut history = History::default();
		history.begin_gesture();
		let mut at = start.clone();
		for to in &moves {
			history.record(Operation::Transform { entity, delta: Delta::between(&at, to) });
			at = to.clone();
		}
		history.end_gesture();

		let undone = apply(&at, history.undo());
		assert!(same(&undone, &start), "one undo takes the whole gesture back");
		assert_eq!(history.undo().count(), 0, "nothing is left to undo");
		let redone = apply(&undone, history.redo());
		assert!(same(&redone, &moves[1]));
		assert_eq!(history.redo().count(), 0, "nothing is left to redo");
	}

	#[test]
	fn steps_outside_a_gesture_stand_alone_and_a_new_edit_clears_redo() {
		let (entity, other) = (Handle::new(), Handle::new());
		let step = |x: f32| Delta::between(&transform([0.0; 3], 0.0, [1.0; 3]), &transform([x, 0.0, 0.0], 0.0, [1.0; 3]));
		let mut history = History::default();
		history.record(Operation::Transform { entity, delta: step(1.0) });
		history.record(Operation::Transform { entity, delta: step(2.0) });
		assert_eq!(
			history.undo().collect::<Vec<_>>(),
			[Operation::Transform { entity, delta: -step(2.0) }],
			"a typed edit is a step of its own"
		);

		history.record(Operation::Transform { entity: other, delta: step(3.0) });
		assert_eq!(history.redo().count(), 0, "a new edit clears redo");

		// A gesture over two entities keeps one operation for each, undone in reverse order.
		history.begin_gesture();
		history.record(Operation::Transform { entity, delta: step(1.0) });
		history.record(Operation::Transform { entity: other, delta: step(1.0) });
		history.record(Operation::Transform { entity, delta: step(1.0) });
		history.end_gesture();
		let undone: Vec<_> = history.undo().collect();
		assert_eq!(undone.len(), 2);
		assert!(matches!(undone[0], Operation::Transform { entity: e, .. } if e == other));
		assert!(matches!(undone[1], Operation::Transform { entity: e, delta } if e == entity
			&& same(&delta.applied_to(&transform([0.0; 3], 0.0, [1.0; 3])), &transform([-2.0, 0.0, 0.0], 0.0, [1.0; 3]))));
	}
}
