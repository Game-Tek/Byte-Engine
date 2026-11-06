//! Step two: one [`InputSink`] per consumer defines its actions and pulls them from the collector.
//!
//! Give every distinct consumer its own sink: a UI, gameplay, a debug overlay.
//! Each owns its actions, resolves them from the shared
//! [`InputCollector`](super::InputCollector), and answers with a [`Capture`]
//! per action. Captured input stops at that sink.
//!
//! Pull sinks in priority order, once per tick each.

use std::alloc::{Allocator, Global};
use std::borrow::Cow;

use log::warn;

use super::collector::{InputCollector, SinkHandle};
use super::gesture::{self, Mode, TriggerMapping};
use super::{
	Action, ActionBindingDescription, ActionEvent, ActionHandle, ActionPhase, Binding, Bindings, DeviceHandle, SeatHandle,
	TickPolicy, Types, Value,
};
use crate::core::channel::{Channel as _, DefaultChannel};
use crate::core::factory::{CreateMessage, Handle};
use crate::core::listener::{DefaultListener, Listener};

/// The `ResolvedAction` struct lets a sink's consumer decide whether to capture an action.
///
/// The consumer receives it during [`InputSink::pull`] and answers with a [`Capture`].
#[derive(Copy, Clone, Debug, PartialEq)]
pub struct ResolvedAction {
	/// The action's index in its own sink.
	pub action: ActionHandle,
	/// The declared action's entity handle, absent for locally created actions.
	pub handle: Option<Handle>,
	/// The value the action resolved to.
	pub value: Value,
	/// The interaction stage, including drag start and release.
	pub phase: ActionPhase,
}

/// The `Capture` enum reports whether a sink's consumer took an action's input.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum Capture {
	/// The consumer handled the action, so its input does not reach later sinks.
	Captured,
	/// The consumer let the action pass, so later sinks still see its input.
	Passed,
}

impl Capture {
	/// Maps a consumer's decision to a capture. Use it where hit-testing yields a boolean.
	pub fn when(captured: bool) -> Self {
		if captured { Self::Captured } else { Self::Passed }
	}
}

/// The `InputSink` struct turns one consumer's share of the input into its actions.
///
/// Create it with the [`SinkHandle`] of its consumer, declare actions with
/// [`Self::create_action`] or [`Self::with_declarations`], then call
/// [`Self::pull`] once per tick before lower-priority sinks pull. Use
/// [`Self::cancel_action`], [`Self::cancel_seat`], or [`Self::cancel_device`]
/// when the consumer loses focus while holding a control. To let players
/// change controls, replace bindings with [`Self::apply`] or [`Self::rebind`],
/// and save [`Self::bindings`].
/// See [Respond to input](/docs/use/input) for the layered input workflow.
pub struct InputSink<A: Allocator + Clone = Global> {
	handle: SinkHandle,
	actions: Vec<SinkAction<A>, A>,
	declarations: Option<DefaultListener<CreateMessage<Action>>>,
	channel: DefaultChannel<ActionEvent>,
	/// Counts pulls, so a value driven by a record is not repeated again in the same tick.
	tick: u32,
	/// An action has drag bindings, so pulls run the drag state machine.
	drags: bool,
	/// Every (control, action) pair a record can drive, sorted, so a record only visits its actions.
	interest: Vec<(u32, u32), A>,
	/// What rebinding reads, kept apart from `actions` so the record loop doesn't stride over it.
	rebinding: Rebinding<A>,
}

/// The `Rebinding` struct keeps what a sink needs to replace and restore bindings by action name.
struct Rebinding<A: Allocator> {
	/// Each action's declaration, by action index.
	declared: Vec<Declared<A>, A>,
	/// The replaced bindings, by action name, kept for actions declared later and for saving.
	overrides: Vec<Override>,
}

/// The `Override` struct keeps one action's replacement bindings until it is reset.
struct Override {
	action: Box<str>,
	bindings: Box<[ActionBindingDescription]>,
}

impl InputSink {
	/// Creates a sink that publishes its actions through `channel`.
	///
	/// Next, add actions with [`Self::create_action`], or adopt declared ones with
	/// [`Self::with_declarations`].
	pub fn new(handle: SinkHandle, channel: DefaultChannel<ActionEvent>) -> Self {
		Self::new_in(handle, channel, Global)
	}
}

impl<A: Allocator + Clone> InputSink<A> {
	/// Creates a sink whose actions, bindings, and values use `allocator`.
	///
	/// Use storage that lasts as long as this sink: held values survive between
	/// ticks. The supplied channel keeps its own allocation policy.
	/// Next, declare actions with [`Self::create_action`] or [`Self::with_declarations`].
	pub fn new_in(handle: SinkHandle, channel: DefaultChannel<ActionEvent>, allocator: A) -> Self {
		Self {
			handle,
			actions: Vec::new_in(allocator.clone()),
			declarations: None,
			channel,
			tick: 0,
			drags: false,
			interest: Vec::new_in(allocator.clone()),
			rebinding: Rebinding {
				declared: Vec::new_in(allocator),
				overrides: Vec::new(),
			},
		}
	}

	/// Adopts the actions an application declares through a factory.
	///
	/// Every action this listener reports belongs to this sink, so give each sink
	/// its own action factory.
	pub fn with_declarations(mut self, declarations: DefaultListener<CreateMessage<Action>>) -> Self {
		self.declarations = Some(declarations);
		self
	}

	/// Adds an action that only this sink resolves.
	///
	/// `name` identifies the action to [`Self::rebind`] and in a [`Bindings`]
	/// document, and `bindings` are its defaults. When a replacement for `name`
	/// was already applied, the action starts with the replacement. A binding
	/// whose trigger is unknown or invalid is left out with a warning. The
	/// returned handle identifies the action in every [`ResolvedAction`] this
	/// sink produces.
	pub fn create_action<E: Allocator + Clone>(
		&mut self,
		collector: &InputCollector<E>,
		name: impl Into<Cow<'static, str>>,
		r#type: Types,
		bindings: &[ActionBindingDescription],
		tick_policy: TickPolicy,
	) -> ActionHandle {
		let allocator = self.actions.allocator().clone();
		let handle = ActionHandle::from_index(self.actions.len() as u32);
		let mut defaults = Vec::with_capacity_in(bindings.len(), allocator.clone());
		defaults.extend_from_slice(bindings);
		let name = name.into();
		// A replacement applied before the action existed wins over its defaults.
		let replacement = self.replacement(&name).map(<[_]>::to_vec);
		self.rebinding.declared.push(Declared {
			name,
			tick_policy,
			defaults: defaults.into_boxed_slice(),
		});
		self.actions.push(SinkAction {
			mode: Mode::Direct(tick_policy),
			r#type,
			mappings: Vec::new_in(allocator.clone()).into_boxed_slice(),
			handle: None,
			values: Vec::new_in(allocator),
		});
		self.install(collector, handle.index(), replacement.as_deref().unwrap_or(bindings));
		handle
	}

	/// Replaces the bindings of every action in `bindings`, as a player's saved controls.
	///
	/// Every binding is checked first, so an unknown trigger or an invalid button
	/// leaves this sink unchanged and returns an error that names the action and
	/// the binding. Actions this sink declares later start with their replacement,
	/// and actions the document leaves out keep their current bindings. Read a
	/// document with [`Bindings::read`] or [`Bindings::from_json`].
	pub fn apply<E: Allocator + Clone>(&mut self, collector: &InputCollector<E>, bindings: &Bindings) -> Result<(), String> {
		let mut resolved = Vec::with_capacity(bindings.actions.len());
		for (action, list) in &bindings.actions {
			let descriptions = list
				.iter()
				.enumerate()
				.map(|(position, binding)| {
					binding
						.description(collector)
						.map_err(|error| describe(action, position, &error))
				})
				.collect::<Result<Vec<_>, _>>()?;
			check(collector, action, &descriptions)?;
			resolved.push((action, descriptions));
		}
		for (action, descriptions) in resolved {
			self.replace(collector, action, &descriptions);
		}
		Ok(())
	}

	/// Replaces one action's bindings, for example with the control a player just pressed.
	///
	/// The replacement also applies to an action named `action` that this sink
	/// declares later. Any interaction the action holds ends as a cancellation.
	/// Next, save the replacements with [`Self::bindings`].
	///
	/// # Errors
	///
	/// Returns an error naming the binding when a trigger is unknown or a button is invalid, and changes nothing.
	pub fn rebind<E: Allocator + Clone>(
		&mut self,
		collector: &InputCollector<E>,
		action: &str,
		bindings: &[ActionBindingDescription],
	) -> Result<(), String> {
		check(collector, action, bindings)?;
		self.replace(collector, action, bindings);
		Ok(())
	}

	/// Restores one action's default bindings and forgets its replacement.
	pub fn reset<E: Allocator + Clone>(&mut self, collector: &InputCollector<E>, action: &str) {
		self.rebinding.overrides.retain(|replacement| &*replacement.action != action);
		for id in 0..self.actions.len() {
			if self.rebinding.declared[id].name != action {
				continue;
			}
			let defaults = self.rebinding.declared[id].defaults.to_vec();
			self.install(collector, id as u32, &defaults);
		}
	}

	/// Returns every replacement this sink holds, as a document to save.
	///
	/// Actions that use their defaults are left out, so the document only holds
	/// what the player changed. Write it with [`Bindings::write`] or
	/// [`Bindings::to_json`].
	pub fn bindings<E: Allocator + Clone>(&self, collector: &InputCollector<E>) -> Bindings {
		let mut document = Bindings::default();
		for replacement in &self.rebinding.overrides {
			let list = replacement
				.bindings
				.iter()
				.filter_map(|binding| Binding::from_description(binding, collector))
				.collect();
			document.actions.insert(replacement.action.to_string(), list);
		}
		document
	}

	/// Stores a checked replacement and installs it on every action with that name.
	fn replace<E: Allocator + Clone>(
		&mut self,
		collector: &InputCollector<E>,
		action: &str,
		bindings: &[ActionBindingDescription],
	) {
		match self
			.rebinding
			.overrides
			.iter_mut()
			.find(|replacement| &*replacement.action == action)
		{
			Some(replacement) => replacement.bindings = bindings.into(),
			None => self.rebinding.overrides.push(Override {
				action: action.into(),
				bindings: bindings.into(),
			}),
		}
		for id in 0..self.actions.len() {
			if self.rebinding.declared[id].name == action {
				self.install(collector, id as u32, bindings);
			}
		}
	}

	/// Returns the replacement bindings set for an action name, if any.
	fn replacement(&self, action: &str) -> Option<&[ActionBindingDescription]> {
		self.rebinding
			.overrides
			.iter()
			.find(|replacement| &*replacement.action == action)
			.map(|replacement| &*replacement.bindings)
	}

	/// Resolves `bindings` into one action's mappings and indexes the controls that drive it.
	///
	/// Ends the action's held interactions first, so a control that no longer
	/// drives it can't leave a press or a drag active.
	// Declaring and rebinding are cold; keeping them out of line keeps `pull`'s record loop as compact as before.
	#[cold]
	#[inline(never)]
	fn install<E: Allocator + Clone>(&mut self, collector: &InputCollector<E>, id: u32, bindings: &[ActionBindingDescription]) {
		let allocator = self.actions.allocator().clone();
		let declared = &self.rebinding.declared[id as usize];
		let action = &mut self.actions[id as usize];
		action.cancel(&self.channel, |_, _| true);
		let mut mappings = Vec::with_capacity_in(bindings.len(), allocator.clone());
		for binding in bindings {
			match collector.resolve_binding(binding) {
				Ok(mapping) => mappings.push(mapping),
				Err(error) => warn!("{error} Action: '{}'.", declared.name),
			}
		}
		action.mode = gesture::mode_for(&mappings, declared.tick_policy, allocator);
		self.interest.retain(|&(_, owner)| owner != id);
		for mapping in &mappings {
			for trigger in mapping.controls() {
				let entry = (trigger, id);
				if let Err(position) = self.interest.binary_search(&entry) {
					self.interest.insert(position, entry);
				}
			}
		}
		action.mappings = mappings.into_boxed_slice();
		self.drags = self.actions.iter().any(|action| matches!(action.mode, Mode::Drag(_)));
	}

	/// Returns the channel this sink publishes its action events through.
	pub fn event_channel(&self) -> &DefaultChannel<ActionEvent> {
		&self.channel
	}

	/// Returns the latest resolved value of one action for a seat and device.
	pub fn action_state(&self, seat: SeatHandle, action: ActionHandle, device: DeviceHandle) -> Value {
		let action = &self.actions[action.index() as usize];
		action
			.values
			.iter()
			.find(|held| held.seat == seat.index() && held.device == device.index())
			.map_or_else(|| action.r#type.default_value(), |held| held.value)
	}

	/// Resolves this sink's actions from the pending input, in arrival order.
	///
	/// `decide` receives every action value this sink resolves, including the
	/// held ones its tick policies repeat, and reports whether the consumer
	/// captured it. Capturing an action captures the record behind it, so no
	/// later sink turns the same input into another action; capturing a press
	/// keeps its control until release.
	pub fn pull<E: Allocator + Clone>(
		&mut self,
		collector: &mut InputCollector<E>,
		mut decide: impl FnMut(&ResolvedAction) -> Capture,
	) {
		self.adopt_declarations(collector);
		self.tick = self.tick.wrapping_add(1);
		// A sink without drag bindings compiles the drag state machine out of its loop.
		if self.drags {
			self.pull_records::<true, E>(collector, &mut decide);
		} else {
			self.pull_records::<false, E>(collector, &mut decide);
		}
		self.repeat(&mut decide);
	}

	/// Offers every pending record to every action, capturing the records the consumer takes.
	fn pull_records<const DRAGS: bool, E: Allocator + Clone>(
		&mut self,
		collector: &mut InputCollector<E>,
		decide: &mut impl FnMut(&ResolvedAction) -> Capture,
	) {
		let sink = self.handle;
		let tick = self.tick;
		let queue = collector.queue_mut();
		for index in queue.unseen(sink)..queue.len() {
			let Some(record) = queue.pending(sink, index) else {
				continue;
			};
			let mut captured = false;
			let first = self.interest.partition_point(|(trigger, _)| *trigger < record.trigger);
			for &(_, id) in self.interest[first..]
				.iter()
				.take_while(|(trigger, _)| *trigger == record.trigger)
			{
				let action = &mut self.actions[id as usize];
				let read = |trigger| queue.visible(sink, index + 1, &(record.seat, record.device, trigger));
				let Some(resolution) =
					gesture::resolve::<DRAGS, A>(action.r#type, &action.mappings, &mut action.mode, record, read)
				else {
					continue;
				};
				let holds = matches!(action.mode, Mode::Direct(_)) && !record.transient;
				action.store(record.seat, record.device, resolution.value, holds, tick);
				action.publish(&self.channel, record.seat, resolution.value, resolution.phase);
				let taken = decide(&ResolvedAction {
					action: ActionHandle(super::HANDLE_TAG | id),
					handle: action.handle,
					value: resolution.value,
					phase: resolution.phase,
				}) == Capture::Captured;
				captured |= taken;
				if DRAGS && let (Some(slot), Mode::Drag(drags)) = (resolution.drag, &mut action.mode) {
					// Capturing a drag keeps movement outside the original hit area captured.
					let drag = &mut drags[slot as usize];
					drag.captured |= taken;
					captured |= drag.captured;
				}
			}
			if captured {
				queue.claim(sink, index);
			}
		}
		queue.advance(sink);
	}

	/// Captures every record this sink can still see, matched or not.
	///
	/// Use it for a context that must swallow input it has no action for, such as
	/// a modal dialog over a scene. Call it after [`Self::pull`], so this sink's
	/// own actions still resolve.
	pub fn capture_pending<E: Allocator + Clone>(&self, collector: &mut InputCollector<E>) {
		collector.queue_mut().claim_pending(self.handle);
	}

	/// Ends the interaction one action holds, without waiting for a release.
	///
	/// Use it when this sink's consumer loses focus: a drag publishes its last
	/// position; other actions publish their neutral value. Both set
	/// [`ActionEvent::is_cancelled`], stop repeating, and give their controls
	/// back to later sinks once they are released.
	pub fn cancel_action(&mut self, seat: SeatHandle, action: ActionHandle) {
		self.actions[action.index() as usize].cancel(&self.channel, |owner, _| owner == seat.index());
	}

	/// Ends this sink's held interactions for one seat. Captured controls stay
	/// with the sink until release, so another sink requires a fresh press.
	pub fn cancel_seat(&mut self, seat: SeatHandle) {
		for action in &mut self.actions {
			action.cancel(&self.channel, |owner, _| owner == seat.index());
		}
	}

	/// Ends this sink's held interactions on a suspended or disconnected device.
	pub fn cancel_device(&mut self, seat: SeatHandle, device: DeviceHandle) {
		for action in &mut self.actions {
			action.cancel(&self.channel, |owner, source| {
				owner == seat.index() && source == device.index()
			});
		}
	}

	/// Repeats held values the tick policy asks for, once per tick, when no record drove them.
	fn repeat(&mut self, decide: &mut impl FnMut(&ResolvedAction) -> Capture) {
		for (id, action) in self.actions.iter().enumerate() {
			let Mode::Direct(policy) = action.mode else {
				continue;
			};
			if policy == TickPolicy::OnChange {
				continue;
			}
			for held in action.values.iter().filter(|held| held.repeats(policy, self.tick)) {
				// Repetition has no new record to capture.
				decide(&ResolvedAction {
					action: ActionHandle(super::HANDLE_TAG | id as u32),
					handle: action.handle,
					value: held.value,
					phase: ActionPhase::Updated,
				});
				if let Some(handle) = action.handle {
					self.channel.send(ActionEvent::new(
						SeatHandle(super::HANDLE_TAG | held.seat),
						handle,
						held.value,
					));
				}
			}
		}
	}

	/// Adds the actions declared since the last pull.
	#[inline(never)]
	fn adopt_declarations<E: Allocator + Clone>(&mut self, collector: &InputCollector<E>) {
		while let Some(message) = self.declarations.as_mut().and_then(Listener::read) {
			let handle = message.handle();
			let action = message.into_data();

			let index = self.create_action(collector, action.name, action.r#type, &action.bindings, action.tick_policy);
			self.actions[index.index() as usize].handle = Some(handle);
		}
	}
}

/// The `SinkAction` struct keeps one action's binding policy and per-device values together.
struct SinkAction<A: Allocator> {
	mode: Mode<A>,
	r#type: Types,
	mappings: Box<[TriggerMapping], A>,
	handle: Option<Handle>,
	/// Values retain capacity between ticks in the action's allocator.
	values: Vec<Held, A>,
}

/// Checks that every binding of `action` resolves against `collector`'s triggers.
fn check<E: Allocator + Clone>(
	collector: &InputCollector<E>,
	action: &str,
	bindings: &[ActionBindingDescription],
) -> Result<(), String> {
	for (position, binding) in bindings.iter().enumerate() {
		collector
			.resolve_binding(binding)
			.map_err(|error| describe(action, position, &error))?;
	}
	Ok(())
}

/// Places a binding error in the action and the one-based binding position it came from.
fn describe(action: &str, position: usize, error: &str) -> String {
	format!("Action '{action}', binding {}: {error}", position + 1)
}

/// The `Declared` struct keeps what an action was declared with, for rebinding it by name and resetting it.
struct Declared<A: Allocator> {
	name: Cow<'static, str>,
	tick_policy: TickPolicy,
	/// The declared bindings, restored by [`InputSink::reset`].
	defaults: Box<[ActionBindingDescription], A>,
}

/// The `Held` struct is one device's last value of an action.
struct Held {
	seat: u32,
	device: u32,
	value: Value,
	/// A retained control drove the value, so a tick policy may repeat it.
	holding: bool,
	/// The pull that last stored the value, so the same pull does not repeat it.
	tick: u32,
}

impl Held {
	/// Applies a tick policy to a value no record drove this pull.
	fn repeats(&self, policy: TickPolicy, tick: u32) -> bool {
		self.holding && self.tick != tick && !(policy == TickPolicy::WhileActive && self.value.is_default())
	}
}

impl<A: Allocator> SinkAction<A> {
	/// Stores a device's value beside its action.
	fn store(&mut self, seat: u32, device: u32, value: Value, holds: bool, tick: u32) {
		if let Some(held) = self.values.iter_mut().find(|held| held.seat == seat && held.device == device) {
			held.value = value;
			held.holding |= holds;
			held.tick = tick;
		} else {
			self.values.push(Held {
				seat,
				device,
				value,
				holding: holds,
				tick,
			});
		}
	}

	/// Publishes a declared action's value through the sink's channel.
	fn publish(&self, channel: &DefaultChannel<ActionEvent>, seat: u32, value: Value, phase: ActionPhase) {
		if let Some(handle) = self.handle {
			log::debug!(target: "byte_engine::input::actions", "Publishing input action: handle={handle:?}, seat={seat:?}, value={value:?}, phase={phase:?}");
			channel.send(ActionEvent::with_phase(
				SeatHandle(super::HANDLE_TAG | seat),
				handle,
				value,
				phase,
			));
		}
	}

	/// Neutralizes matching values and reports interrupted holds once.
	fn cancel(&mut self, channel: &DefaultChannel<ActionEvent>, interrupted: impl Fn(u32, u32) -> bool) {
		if let Mode::Drag(drags) = &mut self.mode {
			for (seat, value) in gesture::cancel_drags(drags, &interrupted) {
				if let Some(handle) = self.handle {
					channel.send(ActionEvent::cancelled(SeatHandle(super::HANDLE_TAG | seat), handle, value));
				}
			}
		}
		for held in self.values.iter_mut().filter(|held| interrupted(held.seat, held.device)) {
			let active = held.holding && !held.value.is_default();
			held.holding = false;
			held.value = self.r#type.default_value();
			if let Some(handle) = self.handle
				&& active
			{
				channel.send(ActionEvent::cancelled(
					SeatHandle(super::HANDLE_TAG | held.seat),
					handle,
					held.value,
				));
			}
		}
	}
}
