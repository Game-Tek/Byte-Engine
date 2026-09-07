//! Step two: one input layer's actions, and what it consumes from the input.
//!
//! Give every distinct consumer its own processor: a UI layer, a gameplay layer,
//! a debug overlay. Each owns its actions, resolves them from the shared
//! [`InputEvents`](super::InputEvents) queue, and answers with a
//! [`Consumption`] per action. Consumed input stops at that layer.
//!
//! Process layers in priority order, then call
//! [`InputEvents::end_tick`](super::InputEvents::end_tick).

use std::alloc::{Allocator, Global};

use super::action::TriggerMapping;
use super::evaluator::resolve_action_value;
use super::events::{ConsumerHandle, InputEvents, Record};
use super::{
	Action, ActionBindingDescription, ActionHandle, DeviceHandle, InputActionError, SeatHandle, TickPolicy, Types, Value,
};
use crate::core::channel::{Channel as _, DefaultChannel};
use crate::core::factory::{CreateMessage, Handle};
use crate::core::listener::{DefaultListener, Listener};
use crate::input::ActionEvent;

/// The `ResolvedAction` struct lets a layer decide whether to consume an action.
///
/// An input layer receives it while processing
/// [`InputEvents`](super::InputEvents) and answers with a
/// [`Consumption`](super::Consumption).
#[derive(Copy, Clone, Debug, PartialEq)]
pub struct ResolvedAction {
	/// The action's index in its own processor.
	pub action: ActionHandle,
	/// The declared action's entity handle, absent for locally created actions.
	pub handle: Option<Handle>,
	/// The value the action resolved to.
	pub value: Value,
}

/// The `Binding` enum selects which of an action's bindings a record may drive.
///
/// Input layers use [`Self::Any`] because they publish each record at its own
/// place in the queue. Broadcast updates separate the two, because they resolve
/// snapshot bindings per record and direct bindings once per action.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
enum Binding {
	/// Only bindings that publish another control's value while a trigger is down.
	Snapshot,
	/// Only bindings driven by their own control.
	Direct,
	/// Whichever binding the record matches first.
	Any,
}

/// The synthetic device reserved for actions triggered without physical input.
pub(super) const MANUAL_ACTION_DEVICE: DeviceHandle = DeviceHandle(u32::MAX);

/// The `Consumption` enum reports whether an input layer handled an action.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum Consumption {
	/// The layer handled the action, so its input does not reach later layers.
	Consumed,
	/// The layer ignored the action, so later layers still see its input.
	Ignored,
}

/// The `ActionProcessor` struct turns one layer's share of the input into its actions.
///
/// Create it with the [`ConsumerHandle`] of its layer, declare its actions, then
/// call [`Self::process`] once per tick before the next layer processes. Use
/// [`Self::cancel_action`] or [`Self::cancel_device`] when the layer loses focus
/// while holding a control.
/// See [Input](/docs/reference/input) for the layered input workflow.
pub struct ActionProcessor<A: Allocator + Clone = Global> {
	consumer: ConsumerHandle,
	actions: Vec<InputAction<A>, A>,
	declarations: Option<DefaultListener<CreateMessage<Action>>>,
	channel: DefaultChannel<ActionEvent>,
	pending_manual_actions: Vec<(SeatHandle, ActionHandle, Value), A>,
}

impl ActionProcessor {
	/// Creates a processor that publishes its layer's actions through `channel`.
	///
	/// Next, add actions with [`Self::create_action`], or adopt declared ones with
	/// [`Self::with_declarations`].
	pub fn new(consumer: ConsumerHandle, channel: DefaultChannel<ActionEvent>) -> Self {
		Self::new_in(consumer, channel, Global)
	}
}

impl<A: Allocator + Clone> ActionProcessor<A> {
	/// Creates a layer whose actions, bindings, values, and manual queue use `allocator`.
	///
	/// Use storage that lasts as long as this processor: held values survive
	/// between ticks. The supplied channel keeps its own allocation policy.
	/// Next, declare actions with [`Self::create_action`] or [`Self::with_declarations`].
	pub fn new_in(consumer: ConsumerHandle, channel: DefaultChannel<ActionEvent>, allocator: A) -> Self {
		Self {
			consumer,
			actions: Vec::new_in(allocator.clone()),
			declarations: None,
			channel,
			pending_manual_actions: Vec::new_in(allocator),
		}
	}

	/// Adopts the actions an application declares through a factory.
	///
	/// Every action this listener reports belongs to this layer, so give each
	/// layer its own action factory.
	pub fn with_declarations(mut self, declarations: DefaultListener<CreateMessage<Action>>) -> Self {
		self.declarations = Some(declarations);
		self
	}

	/// Adds an action that only this layer resolves.
	///
	/// The returned handle identifies the action in every [`ResolvedAction`] this
	/// processor produces.
	pub fn create_action<E: Allocator + Clone>(
		&mut self,
		events: &InputEvents<E>,
		r#type: Types,
		bindings: &[ActionBindingDescription],
		tick_policy: TickPolicy,
	) -> ActionHandle {
		let allocator = self.actions.allocator();
		let mut trigger_mappings = Vec::with_capacity_in(bindings.len(), allocator.clone());
		trigger_mappings.extend(bindings.iter().filter_map(|binding| events.resolve_binding(binding)));
		let action = InputAction {
			r#type,
			trigger_mappings: trigger_mappings.into_boxed_slice(),
			handle: None,
			tick_policy,
			values: Vec::new_in(allocator.clone()),
		};

		let handle = ActionHandle(self.actions.len() as u32);
		self.actions.push(action);

		handle
	}

	/// Returns the channel this layer publishes its action events through.
	pub fn event_channel(&self) -> &DefaultChannel<ActionEvent> {
		&self.channel
	}

	/// Returns the latest resolved value of one action for a seat and device.
	pub fn action_state(&self, seat: SeatHandle, action: ActionHandle, device: DeviceHandle) -> Value {
		let action = &self.actions[action.0 as usize];
		action
			.values
			.iter()
			.find(|&&(owner, source, ..)| owner == seat && source == device)
			.map_or_else(|| action.r#type.default_value(), |&(_, _, value, _)| value)
	}

	/// Resolves this layer's actions from the pending input, in arrival order.
	///
	/// `handle` receives every action value this layer resolves, including the
	/// held ones its tick policies repeat, and reports whether the layer consumed
	/// it. Consuming an action consumes the record behind it, so no later layer
	/// turns the same input into another action; consuming a press keeps its
	/// control until release.
	pub fn process<E: Allocator + Clone>(
		&mut self,
		events: &mut InputEvents<E>,
		mut handle: impl FnMut(&ResolvedAction) -> Consumption,
	) {
		self.adopt_declarations(events);
		for index in 0..events.records.len() {
			let Some(record) = events.pending(self.consumer, index) else {
				continue;
			};
			let mut consumed = false;
			for (id, action) in self.actions.iter_mut().enumerate() {
				let Some(value) = action.resolve(&record, Binding::Any, |trigger| {
					events
						.visible_record(self.consumer, index + 1, &(record.seat_handle, record.device_handle, trigger))
						.copied()
				}) else {
					continue;
				};
				action.publish(
					&self.channel,
					record.seat_handle,
					record.device_handle,
					value,
					!action.has_snapshot_bindings() && !events.is_transient(record.trigger_handle),
				);
				consumed |= handle(&ResolvedAction {
					action: ActionHandle(id as u32),
					handle: action.handle,
					value,
				}) == Consumption::Consumed;
			}
			if consumed {
				events.consume(self.consumer, index);
			}
		}
		self.finish(handle);
	}

	/// Consumes every record this layer can still see, matched or not.
	///
	/// Use it for a context that must swallow input it has no action for, such as
	/// a modal dialog over a scene. Call it after [`Self::process`], so this
	/// layer's own actions still resolve.
	pub fn consume_pending<E: Allocator + Clone>(&self, events: &mut InputEvents<E>) {
		for index in 0..events.records.len() {
			if events.pending(self.consumer, index).is_some() {
				events.consume(self.consumer, index);
			}
		}
	}

	/// Queues an action value that no physical input produced.
	///
	/// Inspector and scripted requests address one action directly, so they reach
	/// their layer without competing for input. Next, call [`Self::process`].
	pub fn trigger_action(&mut self, seat: SeatHandle, action: ActionHandle, value: Value) -> Result<(), InputActionError> {
		let state = self
			.actions
			.get(action.0 as usize)
			.ok_or(InputActionError::UnknownAction(action))?;
		let actual_type = value.into();
		if state.r#type != actual_type {
			return Err(InputActionError::TypeMismatch {
				expected: state.r#type,
				actual: actual_type,
			});
		}

		self.pending_manual_actions.push((seat, action, value));

		Ok(())
	}

	/// Ends the interaction one action holds, without waiting for a release.
	///
	/// Use it when this layer loses focus: the action publishes its neutral value
	/// with [`ActionEvent::is_cancelled`], stops repeating, and gives its controls
	/// back to later layers once they are released.
	pub fn cancel_action(&mut self, seat: SeatHandle, action: ActionHandle) {
		self.actions[action.0 as usize].cancel(&self.channel, |owner, _| owner == seat);
	}

	/// Ends this layer's held interactions for one seat. Claimed controls stay
	/// with the layer until release, so another layer requires a fresh press.
	pub fn cancel_seat(&mut self, seat: SeatHandle) {
		for action in &mut self.actions {
			action.cancel(&self.channel, |owner, _| owner == seat);
		}
	}

	/// Ends this layer's held interactions on a suspended or disconnected device.
	pub fn cancel_device(&mut self, seat: SeatHandle, device: DeviceHandle) {
		for action in &mut self.actions {
			action.cancel(&self.channel, |owner, source| owner == seat && source == device);
		}
	}

	/// Publishes every action a record drives, without offering it for consumption.
	///
	/// [`InputManager`](super::InputManager) uses this where no layer competes for
	/// the input: snapshot bindings publish at each press's place in the queue,
	/// and direct bindings publish once per action from each control's latest
	/// record.
	pub(super) fn broadcast<E: Allocator + Clone>(&mut self, events: &InputEvents<E>) {
		self.adopt_declarations(events);
		if self.actions.iter().any(InputAction::has_snapshot_bindings) {
			for (index, record) in events.records.iter().enumerate() {
				if record.value != Value::Bool(true) {
					continue;
				}
				for action in &mut self.actions {
					if let Some(value) = action.resolve(record, Binding::Snapshot, |trigger| {
						events
							.visible_record(self.consumer, index + 1, &(record.seat_handle, record.device_handle, trigger))
							.copied()
					}) {
						action.publish(&self.channel, record.seat_handle, record.device_handle, value, false);
					}
				}
			}
		}

		// The queue already has arrival order. The last matching record is the
		// direct action's final driver; no sorted or compacted copy is needed.
		for action in &mut self.actions {
			let Some(record) = events.records.iter().rev().find(|record| {
				action
					.trigger_mappings
					.iter()
					.any(|mapping| mapping.trigger.is_none() && mapping.trigger_handle == record.trigger_handle)
			}) else {
				continue;
			};
			if let Some(value) = action.resolve(record, Binding::Direct, |trigger| {
				events
					.visible_record(
						self.consumer,
						events.records.len(),
						&(record.seat_handle, record.device_handle, trigger),
					)
					.copied()
			}) {
				action.publish(
					&self.channel,
					record.seat_handle,
					record.device_handle,
					value,
					!action.has_snapshot_bindings() && !events.is_transient(record.trigger_handle),
				);
			}
		}
		self.finish(|_| Consumption::Ignored);
	}

	/// Publishes queued manual actions, then repeats the values held by each action.
	fn finish(&mut self, mut handle: impl FnMut(&ResolvedAction) -> Consumption) {
		for (seat, action, value) in self.pending_manual_actions.drain(..) {
			// Queuing validates handles; actions are never removed.
			let action = &mut self.actions[action.0 as usize];
			action.publish(
				&self.channel,
				seat,
				MANUAL_ACTION_DEVICE,
				value,
				!action.has_snapshot_bindings(),
			);
		}
		for (id, action) in self.actions.iter().enumerate() {
			if action.tick_policy == TickPolicy::OnChange {
				continue;
			}
			for &(seat, _, value, holding) in &action.values {
				if !holding || (action.tick_policy == TickPolicy::WhileActive && value.is_default()) {
					continue;
				}
				// Repetition has no new record to consume.
				handle(&ResolvedAction {
					action: ActionHandle(id as u32),
					handle: action.handle,
					value,
				});
				if let Some(handle) = action.handle {
					self.channel.send(ActionEvent::new(seat, handle, value));
				}
			}
		}
	}

	/// Adds the actions declared since the last update.
	fn adopt_declarations<E: Allocator + Clone>(&mut self, events: &InputEvents<E>) {
		while let Some(message) = self.declarations.as_mut().and_then(Listener::read) {
			let handle = message.handle();
			let action = message.into_data();

			let index = self.create_action(events, action.r#type, &action.bindings, action.tick_policy);
			self.actions[index.0 as usize].handle = Some(handle);
		}
	}
}

/// The `InputAction` struct keeps a layer's binding policy and values together.
struct InputAction<A: Allocator> {
	r#type: Types,
	trigger_mappings: Box<[TriggerMapping], A>,
	handle: Option<Handle>,
	tick_policy: TickPolicy,
	/// Values grow in the same allocator as the action and retain capacity between ticks.
	values: Vec<(SeatHandle, DeviceHandle, Value, bool), A>,
}

impl<A: Allocator> InputAction<A> {
	fn has_snapshot_bindings(&self) -> bool {
		self.trigger_mappings.iter().any(|mapping| mapping.trigger.is_some())
	}

	/// Resolves a matching binding through the caller's view of physical input.
	fn resolve(
		&self,
		record: &Record,
		binding: Binding,
		read: impl Fn(super::TriggerHandle) -> Option<Record>,
	) -> Option<Value> {
		let mapping = self.trigger_mappings.iter().find(|mapping| match mapping.trigger {
			Some(trigger) => {
				binding != Binding::Direct && trigger == record.trigger_handle && record.value == Value::Bool(true)
			}
			None => binding != Binding::Snapshot && mapping.trigger_handle == record.trigger_handle,
		})?;
		resolve_action_value(self.r#type, &self.trigger_mappings, mapping, record, &read)
	}

	/// Stores a device's value beside its action and publishes declared actions.
	fn publish(
		&mut self,
		channel: &DefaultChannel<ActionEvent>,
		seat: SeatHandle,
		device: DeviceHandle,
		value: Value,
		holds: bool,
	) {
		if let Some((_, _, previous, holding)) = self
			.values
			.iter_mut()
			.find(|(owner, source, ..)| *owner == seat && *source == device)
		{
			*previous = value;
			*holding |= holds;
		} else {
			self.values.push((seat, device, value, holds));
		}
		if let Some(handle) = self.handle {
			log::debug!(target: "byte_engine::input::actions", "Publishing input action: handle={handle:?}, seat={seat:?}, device={device:?}, value={value:?}");
			channel.send(ActionEvent::new(seat, handle, value));
		}
	}

	/// Neutralizes matching values and reports interrupted holds once.
	fn cancel(&mut self, channel: &DefaultChannel<ActionEvent>, interrupted: impl Fn(SeatHandle, DeviceHandle) -> bool) {
		for (seat, device, value, holding) in &mut self.values {
			if !interrupted(*seat, *device) {
				continue;
			}
			let active = *holding && !value.is_default();
			*holding = false;
			*value = self.r#type.default_value();
			if let Some(handle) = self.handle
				&& active
			{
				channel.send(ActionEvent::cancelled(*seat, handle, *value));
			}
		}
	}
}

#[cfg(test)]
mod tests;
