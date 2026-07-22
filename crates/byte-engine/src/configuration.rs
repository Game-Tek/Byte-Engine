//! Asynchronous configuration messages used by debugging adapters and engine systems.
//!
//! Create a [`Configuration`], register one [`ConfigurationPort`] for each owned
//! parameter prefix, then submit updates with [`Configuration::update`]. The
//! consuming system keeps each update pending until it can report either
//! [`ConfigurationUpdateState::Set`] or [`ConfigurationUpdateState::NotSet`].

use std::{
	collections::VecDeque,
	sync::{
		mpsc::{sync_channel, Receiver, SyncSender, TryRecvError, TrySendError},
		Arc,
	},
};

use serde::Serialize;
use utils::sync::Mutex;

const CONFIGURATION_PORT_CAPACITY: usize = 64;

/// The `ConfigurationEventId` struct identifies one configuration update from submission through application.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq, Serialize)]
#[serde(transparent)]
pub struct ConfigurationEventId(u64);

/// The `ConfigurationValue` enum carries the small set of values shared by configuration adapters and systems.
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(untagged)]
pub enum ConfigurationValue {
	Bool(bool),
	Integer(i64),
	Float(f64),
	Text(String),
}

impl ConfigurationValue {
	/// Returns the text value expected by string-backed configuration consumers.
	pub fn as_text(&self) -> Option<&str> {
		match self {
			Self::Text(value) => Some(value),
			_ => None,
		}
	}
}

impl From<&str> for ConfigurationValue {
	fn from(value: &str) -> Self {
		Self::Text(value.to_string())
	}
}

impl From<String> for ConfigurationValue {
	fn from(value: String) -> Self {
		Self::Text(value)
	}
}

/// The `ConfigurationUpdateState` enum reports the current application result for one configuration event.
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(tag = "status", rename_all = "kebab-case")]
pub enum ConfigurationUpdateState {
	Pending,
	Set { value: ConfigurationValue },
	NotSet { reason: String },
}

/// The `ConfigurationEvent` struct provides the requested value and latest application state for debugging tools.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct ConfigurationEvent {
	id: ConfigurationEventId,
	parameter: String,
	requested: ConfigurationValue,
	state: ConfigurationUpdateState,
}

impl ConfigurationEvent {
	/// Returns the identifier assigned when this update was submitted.
	pub fn id(&self) -> ConfigurationEventId {
		self.id
	}

	/// Returns the discoverable parameter name.
	pub fn parameter(&self) -> &str {
		&self.parameter
	}

	/// Returns the value requested by the producer.
	pub fn requested(&self) -> &ConfigurationValue {
		&self.requested
	}

	/// Returns the latest state reported by the owning system.
	pub fn state(&self) -> &ConfigurationUpdateState {
		&self.state
	}
}

/// The `ConfigurationUpdate` struct transfers one requested value to the system that owns its parameter prefix.
#[derive(Clone, Debug, PartialEq)]
pub struct ConfigurationUpdate {
	id: ConfigurationEventId,
	parameter: String,
	value: ConfigurationValue,
}

impl ConfigurationUpdate {
	/// Returns the event identifier used to report the application result.
	pub fn id(&self) -> ConfigurationEventId {
		self.id
	}

	/// Returns the parameter name routed to this system.
	pub fn parameter(&self) -> &str {
		&self.parameter
	}

	/// Returns the requested parameter value.
	pub fn value(&self) -> &ConfigurationValue {
		&self.value
	}
}

struct ConfigurationRoute {
	prefix: String,
	sender: SyncSender<ConfigurationUpdate>,
}

struct ConfigurationState {
	next_event_id: u64,
	routes: Vec<ConfigurationRoute>,
	events: VecDeque<ConfigurationEvent>,
}

/// The `Configuration` struct routes configuration messages while retaining their latest state for debugging tools.
#[derive(Clone)]
pub struct Configuration {
	state: Arc<Mutex<ConfigurationState>>,
}

impl Default for Configuration {
	fn default() -> Self {
		Self::new()
	}
}

impl Configuration {
	/// Creates an empty configuration exchange.
	///
	/// Next, call [`Self::register`] before submitting matching updates through
	/// [`Self::update`].
	pub fn new() -> Self {
		Self {
			state: Arc::new(Mutex::new(ConfigurationState {
				next_event_id: 1,
				routes: Vec::new(),
				events: VecDeque::new(),
			})),
		}
	}

	/// Registers the system that owns parameters beginning with `prefix`.
	///
	/// Next, keep the returned port with that system and call
	/// [`ConfigurationPort::read`] when the system is ready to queue updates.
	pub fn register(&self, prefix: &str) -> ConfigurationPort {
		let (sender, receiver) = sync_channel(CONFIGURATION_PORT_CAPACITY);
		self.state.lock().routes.push(ConfigurationRoute {
			prefix: prefix.to_string(),
			sender,
		});

		ConfigurationPort {
			receiver,
			state: self.state.clone(),
		}
	}

	/// Queues a value for the system that owns the longest matching parameter prefix.
	///
	/// The returned event starts as pending. Use [`Self::event`] to inspect the
	/// system-reported result later.
	pub fn update(&self, parameter: impl Into<String>, value: impl Into<ConfigurationValue>) -> ConfigurationEventId {
		let parameter = parameter.into();
		let value = value.into();
		let (id, sender) = {
			let mut state = self.state.lock();
			let id = ConfigurationEventId(state.next_event_id);
			state.next_event_id += 1;

			let sender = state
				.routes
				.iter()
				.filter(|route| parameter.starts_with(&route.prefix))
				.max_by_key(|route| route.prefix.len())
				.map(|route| route.sender.clone());

			state.events.push_back(ConfigurationEvent {
				id,
				parameter: parameter.clone(),
				requested: value.clone(),
				state: ConfigurationUpdateState::Pending,
			});

			(id, sender)
		};

		let update = ConfigurationUpdate { id, parameter, value };
		match sender {
			Some(sender) => match sender.try_send(update) {
				Ok(()) => {}
				Err(TrySendError::Full(_)) => self.finish_not_set(
					id,
					"Configuration update was not queued. The most likely cause is that the owning system has not drained its pending updates.",
				),
				Err(TrySendError::Disconnected(_)) => self.finish_not_set(
					id,
					"Configuration update was not queued. The most likely cause is that the owning system stopped receiving updates.",
				),
			},
			None => self.finish_not_set(
				id,
				"Configuration parameter was not routed. The most likely cause is that no system registered its parameter prefix.",
			),
		}

		id
	}

	/// Returns the latest state for one configuration event.
	pub fn event(&self, id: ConfigurationEventId) -> Option<ConfigurationEvent> {
		self.state.lock().events.iter().find(|event| event.id == id).cloned()
	}

	/// Returns a snapshot of all retained configuration events.
	pub fn events(&self) -> Vec<ConfigurationEvent> {
		self.state.lock().events.iter().cloned().collect()
	}

	fn finish_not_set(&self, id: ConfigurationEventId, reason: impl Into<String>) {
		finish_event(&self.state, id, ConfigurationUpdateState::NotSet { reason: reason.into() });
	}
}

/// The `ConfigurationPort` struct gives one consuming system an update queue and result-reporting handle.
pub struct ConfigurationPort {
	receiver: Receiver<ConfigurationUpdate>,
	state: Arc<Mutex<ConfigurationState>>,
}

impl ConfigurationPort {
	/// Returns the next queued update without waiting for a producer.
	pub fn read(&self) -> Option<ConfigurationUpdate> {
		match self.receiver.try_recv() {
			Ok(update) => Some(update),
			Err(TryRecvError::Empty | TryRecvError::Disconnected) => None,
		}
	}

	/// Reports the effective value applied by the consuming system.
	pub fn set(&self, id: ConfigurationEventId, value: ConfigurationValue) {
		finish_event(&self.state, id, ConfigurationUpdateState::Set { value });
	}

	/// Reports why the consuming system did not apply the requested value.
	pub fn not_set(&self, id: ConfigurationEventId, reason: impl Into<String>) {
		finish_event(&self.state, id, ConfigurationUpdateState::NotSet { reason: reason.into() });
	}
}

/// Completes a pending event once so late or duplicate reports cannot rewrite debugging history.
fn finish_event(state: &Arc<Mutex<ConfigurationState>>, id: ConfigurationEventId, result: ConfigurationUpdateState) {
	let mut state = state.lock();
	let Some(event) = state.events.iter_mut().find(|event| event.id == id) else {
		return;
	};
	if matches!(event.state, ConfigurationUpdateState::Pending) {
		event.state = result;
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn owner_reports_the_effective_value_for_a_pending_update() {
		let configuration = Configuration::new();
		let port = configuration.register("render.pass.");

		let id = configuration.update("render.pass.bloom", "bypassed");
		assert_eq!(configuration.event(id).unwrap().state(), &ConfigurationUpdateState::Pending);

		let update = port.read().expect("render configuration update");
		assert_eq!(update.id(), id);
		assert_eq!(update.parameter(), "render.pass.bloom");
		assert_eq!(update.value(), &ConfigurationValue::from("bypassed"));

		port.set(id, ConfigurationValue::from("bypassed"));
		assert_eq!(
			configuration.event(id).unwrap().state(),
			&ConfigurationUpdateState::Set {
				value: ConfigurationValue::from("bypassed"),
			}
		);
	}

	#[test]
	fn unrouted_update_gets_an_event_and_a_not_set_result() {
		let configuration = Configuration::new();

		let id = configuration.update("audio.master.gain", ConfigurationValue::Float(0.5));

		assert!(matches!(
			configuration.event(id).unwrap().state(),
			ConfigurationUpdateState::NotSet { reason } if reason.contains("no system registered")
		));
	}

	#[test]
	fn the_longest_registered_prefix_owns_the_update() {
		let configuration = Configuration::new();
		let broad = configuration.register("render.");
		let pass = configuration.register("render.pass.");

		configuration.update("render.pass.bloom", "enabled");

		assert!(broad.read().is_none());
		assert!(pass.read().is_some());
	}
}
