//! Pure interaction rules: which binding a record drives, and the phase it reaches.
//!
//! An action's bindings fall into one [`Mode`]. Direct bindings follow their own
//! control. Snapshot bindings publish another control's value when a button is
//! pressed or released. Drag bindings follow a control from press through
//! movement to release, keeping per-interaction state in [`DragState`].
//! [`InputSink`](super::InputSink) calls [`resolve`] once per record and action.

use std::alloc::Allocator;

use super::queue::Record;
use super::resolve::resolve_value;
use super::{ActionPhase, TickPolicy, Types, Value, ValueMapping};

/// The `TriggerMapping` struct preserves a binding resolved to dense control indices, with its gate decided up front.
#[derive(Copy, Clone, Debug)]
pub(super) struct TriggerMapping {
	/// The control whose value the binding contributes.
	pub(super) source: u32,
	pub(super) gate: Gate,
	pub(super) mapping: ValueMapping,
}

/// The `Gate` enum is the button a binding waits on, classified once so the record loop only compares handles.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub(super) enum Gate {
	/// The binding follows its own control.
	None,
	/// The binding samples its source when the button is pressed.
	Press(u32),
	/// The binding samples its source when the button is released.
	Release(u32),
	/// The binding follows its source from press through movement to release.
	Drag(u32),
}

impl TriggerMapping {
	/// Reports whether a button, not the source itself, drives this binding.
	pub(super) fn gated(&self) -> bool {
		self.gate != Gate::None
	}

	/// Returns the controls whose records can drive this binding.
	pub(super) fn controls(&self) -> impl Iterator<Item = u32> {
		let (source, button) = match self.gate {
			Gate::None => (Some(self.source), None),
			Gate::Press(button) | Gate::Release(button) => (None, Some(button)),
			Gate::Drag(button) => (Some(self.source), Some(button)),
		};
		source.into_iter().chain(button)
	}
}

/// The `Mode` enum keeps only the policy or interaction state an action's bindings require.
pub(super) enum Mode<A: Allocator> {
	/// Bindings driven by their own control, repeated through the tick policy.
	Direct(TickPolicy),
	/// Bindings that publish a source value when a button reaches its phase.
	Snapshot,
	/// Bindings that follow a source from press to release, one slot per interaction.
	Drag(Vec<DragState, A>),
}

/// The `DragState` struct preserves one binding's interaction across ticks.
pub(super) struct DragState {
	pub(super) seat: u32,
	pub(super) device: u32,
	binding: u32,
	pub(super) value: Value,
	pub(super) active: bool,
	/// The sink captured the start, so movement and release stay captured.
	pub(super) captured: bool,
}

/// The `Resolution` struct is one action value a record produced.
pub(super) struct Resolution {
	pub(super) value: Value,
	pub(super) phase: ActionPhase,
	/// The drag slot this value advanced, so the sink can extend its capture.
	pub(super) drag: Option<u32>,
}

/// Chooses the mode an action's resolved bindings need.
pub(super) fn mode_for<A: Allocator>(mappings: &[TriggerMapping], policy: TickPolicy, allocator: A) -> Mode<A> {
	if mappings.iter().any(|mapping| matches!(mapping.gate, Gate::Drag(_))) {
		Mode::Drag(Vec::new_in(allocator))
	} else if mappings.iter().any(TriggerMapping::gated) {
		Mode::Snapshot
	} else {
		Mode::Direct(policy)
	}
}

/// Resolves the value `record` gives an action, through the control values `read` exposes.
///
/// `DRAGS` is false for a sink without drag bindings, which keeps the drag
/// state machine out of its record loop entirely. The loop measurably slows
/// down when the drag path is merely present beside the direct path.
#[inline]
pub(super) fn resolve<'records, const DRAGS: bool, A: Allocator>(
	kind: Types,
	mappings: &[TriggerMapping],
	mode: &mut Mode<A>,
	record: &Record,
	read: impl Fn(u32) -> Option<&'records Record>,
) -> Option<Resolution> {
	if DRAGS
		&& let Mode::Drag(drags) = mode
		&& let Some(resolution) = resolve_drag(kind, mappings, drags, record, &read)
	{
		return Some(resolution);
	}
	let mapping = mappings.iter().find(|mapping| match mapping.gate {
		Gate::None => mapping.source == record.trigger,
		Gate::Press(button) => button == record.trigger && record.value == Value::Bool(true),
		Gate::Release(button) => button == record.trigger && record.value == Value::Bool(false),
		Gate::Drag(_) => false,
	})?;
	resolve_value(kind, mappings, mapping, record, &read).map(|value| Resolution {
		value,
		phase: ActionPhase::Updated,
		drag: None,
	})
}

/// Advances a drag from a fresh press with a visible source sample, movement while held, or release.
// Keep the state machine separate so drag handling does not inflate the main record loop.
#[inline(never)]
fn resolve_drag<'records, A: Allocator>(
	kind: Types,
	mappings: &[TriggerMapping],
	drags: &mut Vec<DragState, A>,
	record: &Record,
	read: &impl Fn(u32) -> Option<&'records Record>,
) -> Option<Resolution> {
	for (binding, mapping, button) in mappings
		.iter()
		.enumerate()
		.filter_map(|(binding, mapping)| match mapping.gate {
			Gate::Drag(button) => Some((binding as u32, mapping, button)),
			_ => None,
		}) {
		let is_button = record.trigger == button;
		if !is_button && record.trigger != mapping.source {
			continue;
		}
		// Keep the matched state borrowed through resolution instead of indexing the vector again for each field.
		let existing = drags
			.iter()
			.enumerate()
			.find(|(_, drag)| drag.seat == record.seat && drag.device == record.device && drag.binding == binding);
		let active = existing.is_some_and(|(_, drag)| drag.active);
		let phase = match (is_button, record.value, active) {
			(true, Value::Bool(true), false) => ActionPhase::Started,
			(true, Value::Bool(false), true) => ActionPhase::Ended,
			(false, _, true) if read(button).is_some_and(|gate| gate.value == Value::Bool(true)) => ActionPhase::Updated,
			_ => continue,
		};
		let value = resolve_value(kind, mappings, mapping, record, read)
			.or_else(|| existing.filter(|_| active).map(|(_, drag)| drag.value));
		let Some(value) = value else {
			continue;
		};
		// Reuse the binding's slot across drags; only the first interaction grows storage.
		let slot = existing.map(|(slot, _)| slot).unwrap_or_else(|| {
			drags.push(DragState {
				seat: record.seat,
				device: record.device,
				binding,
				value,
				active: false,
				captured: false,
			});
			drags.len() - 1
		});
		let drag = &mut drags[slot];
		drag.value = value;
		drag.active = phase != ActionPhase::Ended;
		if phase == ActionPhase::Started {
			drag.captured = false;
		}
		return Some(Resolution {
			value,
			phase,
			drag: Some(slot as u32),
		});
	}
	None
}

/// Ends every active drag `interrupted` selects, yielding each one's seat and last value.
pub(super) fn cancel_drags<'a>(
	drags: &'a mut [DragState],
	interrupted: impl Fn(u32, u32) -> bool + 'a,
) -> impl Iterator<Item = (u32, Value)> + 'a {
	drags
		.iter_mut()
		.filter(move |drag| drag.active && interrupted(drag.seat, drag.device))
		.map(|drag| {
			drag.active = false;
			(drag.seat, drag.value)
		})
}
