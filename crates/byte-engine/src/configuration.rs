//! Asynchronous configuration messages used by debugging adapters and engine systems.
//!
//! Create a [`Configuration`], register one [`ConfigurationPort`] for each owned
//! parameter prefix, then submit updates with [`Configuration::update`]. The
//! consuming system keeps each update pending until it can report either
//! [`ConfigurationUpdateState::Set`] or [`ConfigurationUpdateState::NotSet`].
//!
//! Each system also declares the names it takes with [`ConfigurationPort::declare`], so adapters can list them with
//! [`Configuration::parameters`] and describe their values with [`Accepts`].

use std::{
	collections::VecDeque,
	sync::{
		Arc,
		mpsc::{Receiver, SyncSender, TrySendError, sync_channel},
	},
};

use serde::{Deserialize, Serialize};
use utils::sync::Mutex;

const CONFIGURATION_PORT_CAPACITY: usize = 64;

/// The `ConfigurationEventId` struct identifies one configuration update from submission through application.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq, Serialize)]
#[serde(transparent)]
#[repr(transparent)]
pub struct ConfigurationEventId(std::num::NonZeroU64);

const _: () = assert!(size_of::<Option<ConfigurationEventId>>() == size_of::<u64>());

/// The `ConfigurationValue` enum carries the small set of values shared by configuration adapters and systems.
///
/// Untagged JSON reads in variant order, so a whole number becomes an [`Self::Integer`] and only a number with a
/// fraction or exponent becomes a [`Self::Float`].
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum ConfigurationValue {
	/// A Boolean configuration value.
	Bool(bool),
	/// A signed integer configuration value.
	Integer(i64),
	/// A floating-point configuration value.
	Float(f64),
	/// An owned text configuration value.
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

	/// Reads a Boolean, or the text `true` or `false` that startup parameters arrive as.
	pub(crate) fn to_bool(&self) -> Option<bool> {
		match self {
			Self::Bool(value) => Some(*value),
			Self::Text(value) => value.parse().ok(),
			Self::Integer(_) | Self::Float(_) => None,
		}
	}

	/// Reads a finite number, or the text of one that startup parameters arrive as.
	pub(crate) fn to_f64(&self) -> Option<f64> {
		match self {
			Self::Integer(value) => Some(*value as f64),
			Self::Float(value) if value.is_finite() => Some(*value),
			Self::Text(value) => value.parse().ok().filter(|value: &f64| value.is_finite()),
			Self::Bool(_) | Self::Float(_) => None,
		}
	}

	/// Reads a whole number that fits 32 unsigned bits, or the text of one that startup parameters arrive as.
	pub(crate) fn to_u32(&self) -> Option<u32> {
		match self {
			Self::Integer(value) => (*value).try_into().ok(),
			Self::Float(value) if value.is_finite() && value.fract() == 0.0 => (*value as i128).try_into().ok(),
			Self::Text(value) => value.parse().ok(),
			Self::Bool(_) | Self::Float(_) => None,
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

/// The `Accepts` enum describes the values one runtime setting takes, so its owning system reads them the same way
/// every setting does and adapters such as the console can complete and describe them.
///
/// Systems name it in a [`Setting`] table or pass it to [`ConfigurationPort::declare`]; adapters read it back from
/// [`Configuration::parameters`].
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Accepts {
	/// `true` or `false`.
	Bool,
	/// A finite number of `min` or more that fits in 32-bit floating point.
	Number {
		/// The smallest value accepted.
		min: f64,
	},
	/// A whole number from `min` through `max` that is a multiple of `step` away from `min`.
	Whole {
		/// The smallest value accepted.
		min: u32,
		/// The largest value accepted.
		max: u32,
		/// The distance between accepted values: 2 accepts every other number from `min`.
		step: u32,
	},
	/// One of these words, matched in any letter case.
	Choice(&'static [&'static str]),
	/// Any whole number, or this word.
	WholeOr(&'static str),
}

/// The `Read` enum is a value its [`Accepts`] took, typed, for a [`Setting::apply`] to store.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Read {
	/// What [`Accepts::Bool`] read.
	Bool(bool),
	/// What [`Accepts::Number`] read.
	Number(f64),
	/// What [`Accepts::Whole`] or [`Accepts::WholeOr`] read as a number.
	Whole(u32),
	/// The index, in [`Accepts::Choice`], of the word read.
	Choice(usize),
	/// The word of [`Accepts::WholeOr`].
	Word,
}

impl Read {
	/// Returns what [`Accepts::Bool`] read. A setting calls it only from its own `apply`, where its `accepts` decides
	/// the variant.
	pub fn bool(self) -> bool {
		match self {
			Self::Bool(value) => value,
			_ => panic!("{MISMATCH}"),
		}
	}

	/// Returns what [`Accepts::Number`] read.
	pub fn number(self) -> f64 {
		match self {
			Self::Number(value) => value,
			_ => panic!("{MISMATCH}"),
		}
	}

	/// Returns what [`Accepts::Whole`] read.
	pub fn whole(self) -> u32 {
		match self {
			Self::Whole(value) => value,
			_ => panic!("{MISMATCH}"),
		}
	}

	/// Returns the index in [`Accepts::Choice`] of the word read.
	pub fn choice(self) -> usize {
		match self {
			Self::Choice(index) => index,
			_ => panic!("{MISMATCH}"),
		}
	}
}

/// The panic message of a [`Read`] taken as a kind its setting doesn't accept.
const MISMATCH: &str = "A setting read a value of another kind than it accepts. The most likely cause is an `apply` that doesn't match its `accepts`. Read the kind the setting's `Accepts` gives.";

impl Accepts {
	/// The words this setting completes to, such as `true` and `false`. Numbers have none.
	pub fn words(&self) -> &[&'static str] {
		match self {
			Self::Bool => &["true", "false"],
			Self::Choice(words) => words,
			Self::WholeOr(word) => std::slice::from_ref(word),
			Self::Number { .. } | Self::Whole { .. } => &[],
		}
	}

	/// Reads `value`, or returns `None` when it isn't one this setting accepts.
	pub fn read(self, value: &ConfigurationValue) -> Option<Read> {
		match self {
			Self::Bool => value.to_bool().map(Read::Bool),
			Self::Number { min } => value
				.to_f64()
				.filter(|number| *number >= min && *number <= f32::MAX as f64)
				.map(Read::Number),
			Self::Whole { min, max, step } => value
				.to_u32()
				.filter(|number| (min..=max).contains(number) && (number - min) % step.max(1) == 0)
				.map(Read::Whole),
			Self::Choice(words) => {
				let text = value.as_text()?;
				words
					.iter()
					.position(|word| text.eq_ignore_ascii_case(word))
					.map(Read::Choice)
			}
			Self::WholeOr(word) if value.as_text().is_some_and(|text| text.eq_ignore_ascii_case(word)) => Some(Read::Word),
			Self::WholeOr(_) => value.to_u32().map(Read::Whole),
		}
	}

	/// Returns `read` as the value a system reports as in effect: a choice by its word as listed.
	pub fn value(self, read: Read) -> ConfigurationValue {
		match (read, self) {
			(Read::Bool(value), _) => ConfigurationValue::Bool(value),
			(Read::Number(value), _) => ConfigurationValue::Float(value),
			(Read::Whole(value), _) => ConfigurationValue::Integer(i64::from(value)),
			(Read::Choice(index), Self::Choice(words)) => ConfigurationValue::from(words[index]),
			(Read::Word, Self::WholeOr(word)) => ConfigurationValue::from(word),
			(Read::Choice(_) | Read::Word, _) => panic!("{MISMATCH}"),
		}
	}

	/// Describes the accepted values in a phrase that completes "Set it to …", such as "a whole number from 1
	/// through 32".
	pub fn describe(self) -> String {
		match self {
			Self::Bool => "`true` or `false`".to_string(),
			Self::Number { min } => format!("a number of {min} or more"),
			Self::Whole {
				min,
				max: u32::MAX,
				step: 1,
			} => format!("a whole number of {min} or more"),
			Self::Whole { min, max, step: 1 } => format!("a whole number from {min} through {max}"),
			Self::Whole { min, max, step: 2 } if min % 2 == 0 => format!("an even whole number from {min} through {max}"),
			Self::Whole { min, max, step } => format!("a whole number from {min} through {max}, in steps of {step}"),
			Self::Choice(words) => list_words(words, "or"),
			Self::WholeOr(word) => format!("a whole number or `{word}`"),
		}
	}
}

/// Lists `words` in backticks, joined by commas and `conjunction` before the last.
fn list_words(words: &[&str], conjunction: &str) -> String {
	match words {
		[] => String::new(),
		[only] => format!("`{only}`"),
		[first, second] => format!("`{first}` {conjunction} `{second}`"),
		[rest @ .., last] => {
			let mut text = String::new();
			for word in rest {
				text.push_str(&format!("`{word}`, "));
			}
			text.push_str(&format!("{conjunction} `{last}`"));
			text
		}
	}
}

/// The `Setting` struct is one runtime setting of a system whose settings are a copyable value `S`, such as the
/// visibility pipeline's detail level controls.
///
/// A system lists its settings in a `&'static [Setting<S>]` table, declares it with [`ConfigurationPort::declare`]
/// when it gets its port, and applies queued updates with [`ConfigurationPort::drain`] each frame. The table is the one
/// place a setting's name and accepted values live, so reading, error messages, and completion agree.
#[cfg(feature = "headed")]
pub(crate) struct Setting<S> {
	/// The name inside the port's prefix, such as `bias` under `render.lod.`.
	pub(crate) name: &'static str,
	pub(crate) accepts: Accepts,
	/// Stores a read value in the settings, or returns why the system can't take it, such as a startup-only size
	/// asked to change. Values outside [`Self::accepts`] never reach it.
	pub(crate) apply: fn(S, Read) -> Result<S, String>,
}

#[cfg(feature = "headed")]
impl<S> Setting<S> {
	/// Returns the name and accepted values to pass to [`ConfigurationPort::declare`].
	pub(crate) fn parameter(&self) -> (&'static str, Accepts) {
		(self.name, self.accepts)
	}
}

/// The `ConfigurationUpdateState` enum reports the current application result for one configuration event.
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(tag = "status", rename_all = "kebab-case")]
pub enum ConfigurationUpdateState {
	/// The owning system has not processed the update yet.
	Pending,
	/// The owning system applied the update.
	Set {
		/// The value stored by the owning system.
		value: ConfigurationValue,
	},
	/// The owning system rejected or could not apply the update.
	NotSet {
		/// The cause reported by the owning system.
		reason: String,
	},
}

/// The `ConfigurationEvent` struct provides the requested value and latest application state for debugging tools.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct ConfigurationEvent {
	id: ConfigurationEventId,
	parameter: String,
	requested: ConfigurationValue,
	state: ConfigurationUpdateState,
	/// The [`ConfigurationState::revision`] of the event's last change, which [`Configuration::changes_since`] compares.
	#[serde(skip)]
	revision: u64,
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

	/// Returns the parameter name inside the prefix the receiving port registered, so a system reads its
	/// parameters by their own names wherever its caller placed them.
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
	/// The names inside `prefix` the owning system declared, with the values each accepts.
	parameters: Vec<(String, Accepts)>,
}

struct ConfigurationState {
	next_event_id: u64,
	routes: Vec<ConfigurationRoute>,
	events: VecDeque<ConfigurationEvent>,
	/// Counts every request and state change, so readers can ask for what changed after the count they saw.
	revision: u64,
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
				revision: 0,
			})),
		}
	}

	/// Registers the system that owns parameters beginning with `prefix`, which the caller chooses: the port
	/// delivers each parameter by the rest of its name.
	///
	/// Next, keep the returned port with that system and call
	/// [`ConfigurationPort::read`] when the system is ready to queue updates.
	pub fn register(&self, prefix: &str) -> ConfigurationPort {
		let (sender, receiver) = sync_channel(CONFIGURATION_PORT_CAPACITY);
		let mut state = self.state.lock();
		state.routes.push(ConfigurationRoute {
			prefix: prefix.to_string(),
			sender,
			parameters: Vec::new(),
		});

		ConfigurationPort {
			receiver,
			route: state.routes.len() - 1,
			prefix: prefix.to_string(),
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
		let (id, route) = {
			let mut state = self.state.lock();
			let next = state.next_event_id.checked_add(1).expect(
				"Configuration event identities are exhausted. The process issued u64::MAX updates. Restart the application.",
			);
			let id = ConfigurationEventId(
				std::num::NonZeroU64::new(state.next_event_id)
					.expect("Configuration event identity is invalid. The counter returned zero. Restart the application."),
			);
			state.next_event_id = next;

			let route = state
				.routes
				.iter()
				.filter(|route| parameter.starts_with(&route.prefix))
				.max_by_key(|route| route.prefix.len())
				.map(|route| (route.sender.clone(), route.prefix.len()));

			state.revision += 1;
			let revision = state.revision;
			state.events.push_back(ConfigurationEvent {
				id,
				parameter: parameter.clone(),
				requested: value.clone(),
				state: ConfigurationUpdateState::Pending,
				revision,
			});

			(id, route)
		};

		match route {
			Some((sender, prefix_len)) => match sender.try_send(ConfigurationUpdate {
				id,
				parameter: parameter[prefix_len..].to_string(),
				value,
			}) {
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

	/// Returns every parameter the registered systems declared, by full name in name order, with the values each
	/// accepts.
	///
	/// Adapters such as the console use it to complete and describe names; a system that declared nothing still
	/// takes updates under its prefix.
	pub fn parameters(&self) -> Vec<(String, Accepts)> {
		let state = self.state.lock();
		let mut parameters: Vec<(String, Accepts)> = state
			.routes
			.iter()
			.flat_map(|route| {
				route
					.parameters
					.iter()
					.map(|(name, accepts)| (format!("{}{name}", route.prefix), *accepts))
			})
			.collect();
		parameters.sort_by(|a, b| a.0.cmp(&b.0));
		parameters
	}

	/// Returns the latest state for one configuration event.
	pub fn event(&self, id: ConfigurationEventId) -> Option<ConfigurationEvent> {
		self.state.lock().events.iter().find(|event| event.id == id).cloned()
	}

	/// Returns a snapshot of all retained configuration events.
	pub fn events(&self) -> Vec<ConfigurationEvent> {
		self.state.lock().events.iter().cloned().collect()
	}

	/// Returns the retained events requested or changed after `revision`, in request order, and the revision to pass
	/// next. Pass `0` to read every retained event.
	pub fn changes_since(&self, revision: u64) -> (Vec<ConfigurationEvent>, u64) {
		let state = self.state.lock();
		let changed = state
			.events
			.iter()
			.filter(|event| event.revision > revision)
			.cloned()
			.collect();
		(changed, state.revision)
	}

	fn finish_not_set(&self, id: ConfigurationEventId, reason: impl Into<String>) {
		finish_event(&self.state, id, ConfigurationUpdateState::NotSet { reason: reason.into() });
	}
}

/// The `ConfigurationPort` struct gives one consuming system an update queue and result-reporting handle.
pub struct ConfigurationPort {
	receiver: Receiver<ConfigurationUpdate>,
	/// This port's index in [`ConfigurationState::routes`], which only grows.
	route: usize,
	prefix: String,
	state: Arc<Mutex<ConfigurationState>>,
}

impl ConfigurationPort {
	/// Returns the next queued update without waiting for a producer.
	pub fn read(&self) -> Option<ConfigurationUpdate> {
		self.receiver.try_recv().ok()
	}

	/// Reports the effective value applied by the consuming system.
	pub fn set(&self, id: ConfigurationEventId, value: ConfigurationValue) {
		finish_event(&self.state, id, ConfigurationUpdateState::Set { value });
	}

	/// Reports why the consuming system did not apply the requested value.
	pub fn not_set(&self, id: ConfigurationEventId, reason: impl Into<String>) {
		finish_event(&self.state, id, ConfigurationUpdateState::NotSet { reason: reason.into() });
	}

	/// Declares the parameters this port's system takes, by their names inside the prefix, replacing those declared
	/// before.
	///
	/// A system with a [`Setting`] table declares it once, as `port.declare(SETTINGS.iter().map(Setting::parameter))`;
	/// a system whose names change while it runs, such as the installed render passes, declares them again on each
	/// change. Next, [`Configuration::parameters`] lists them.
	pub fn declare<N: Into<String>>(&self, parameters: impl IntoIterator<Item = (N, Accepts)>) {
		let parameters = parameters.into_iter().map(|(name, accepts)| (name.into(), accepts)).collect();
		self.state.lock().routes[self.route].parameters = parameters;
	}

	/// Applies every queued update to `settings` with the setting `table` names, and reports each result.
	///
	/// Systems with runtime controls call this once per frame before they use their settings. An update whose name
	/// isn't in `table`, or whose value its setting doesn't accept, is reported as not set with what to use instead.
	// Only render settings use setting tables today, and they exist only in headed builds.
	#[cfg(feature = "headed")]
	pub(crate) fn drain<S: Copy>(&self, settings: &mut S, table: &[Setting<S>]) {
		while let Some(update) = self.read() {
			match apply_setting(*settings, table, &self.prefix, update.parameter(), update.value()) {
				Ok((updated, effective_value)) => {
					*settings = updated;
					self.set(update.id(), effective_value);
				}
				Err(reason) => self.not_set(update.id(), reason),
			}
		}
	}
}

/// Applies `value` to the setting of `table` called `name`, returning the updated settings and the value in effect, or
/// why it wasn't set. `prefix` is where the port placed the table's names, so errors give full names.
///
/// [`ConfigurationPort::drain`] calls it for each queued update.
#[cfg(feature = "headed")]
pub(crate) fn apply_setting<S: Copy>(
	settings: S,
	table: &[Setting<S>],
	prefix: &str,
	name: &str,
	value: &ConfigurationValue,
) -> Result<(S, ConfigurationValue), String> {
	let Some(setting) = table.iter().find(|setting| setting.name == name) else {
		let names: Vec<String> = table.iter().map(|setting| format!("{prefix}{}", setting.name)).collect();
		let names: Vec<&str> = names.iter().map(String::as_str).collect();
		return Err(format!(
			"`{prefix}{name}` was not set. The most likely cause is that no setting has that name. Use {}.",
			list_words(&names, "or")
		));
	};
	let Some(read) = setting.accepts.read(value) else {
		let accepted = setting.accepts.describe();
		return Err(format!(
			"`{prefix}{name}` was not set. The most likely cause is that the value isn't {accepted}. Set it to {accepted}."
		));
	};
	Ok(((setting.apply)(settings, read)?, setting.accepts.value(read)))
}

/// Completes a pending event once so late or duplicate reports cannot rewrite debugging history.
fn finish_event(state: &Arc<Mutex<ConfigurationState>>, id: ConfigurationEventId, result: ConfigurationUpdateState) {
	let mut state = state.lock();
	let revision = state.revision + 1;
	let Some(event) = state.events.iter_mut().find(|event| event.id == id) else {
		return;
	};
	if matches!(event.state, ConfigurationUpdateState::Pending) {
		event.state = result;
		event.revision = revision;
		state.revision = revision;
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

		assert_eq!(
			configuration.event(id).expect("expected test value").state(),
			&ConfigurationUpdateState::Pending
		);

		let update = port.read().expect("render configuration update");

		assert_eq!(update.id(), id);
		assert_eq!(update.parameter(), "bloom");
		assert_eq!(update.value(), &ConfigurationValue::from("bypassed"));

		port.set(id, ConfigurationValue::from("bypassed"));

		assert_eq!(
			configuration.event(id).expect("expected test value").state(),
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
			configuration.event(id).expect("expected test value").state(),
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

	#[test]
	fn parameters_list_every_declared_name_in_full_and_a_declaration_replaces_the_last() {
		let configuration = Configuration::new();
		let pass = configuration.register("render.pass.");
		let lod = configuration.register("render.lod.");
		lod.declare([("force", Accepts::WholeOr("off")), ("bias", Accepts::Number { min: 0.0 })]);
		pass.declare([("ui", Accepts::Choice(&["enabled", "bypassed"]))]);
		pass.declare([("bloom", Accepts::Choice(&["enabled", "bypassed"]))]);

		let names: Vec<String> = configuration.parameters().into_iter().map(|(name, _)| name).collect();
		assert_eq!(names, ["render.lod.bias", "render.lod.force", "render.pass.bloom"]);
	}

	#[test]
	fn values_read_as_their_setting_accepts() {
		let whole = Accepts::Whole {
			min: 2,
			max: 32,
			step: 2,
		};
		assert_eq!(whole.read(&ConfigurationValue::Integer(4)), Some(Read::Whole(4)));
		assert_eq!(whole.read(&ConfigurationValue::from("6")), Some(Read::Whole(6)));
		assert_eq!(whole.read(&ConfigurationValue::Integer(5)), None);
		assert_eq!(whole.read(&ConfigurationValue::Integer(34)), None);

		let choice = Accepts::Choice(&["ssgi", "surfels"]);
		let read = choice.read(&ConfigurationValue::from("SURFELS")).unwrap();
		assert_eq!(choice.value(read), ConfigurationValue::from("surfels"));

		let level = Accepts::WholeOr("off");
		assert_eq!(level.read(&ConfigurationValue::from("off")), Some(Read::Word));
		assert_eq!(level.read(&ConfigurationValue::Integer(3)), Some(Read::Whole(3)));
		assert_eq!(Accepts::Number { min: 1.0 }.read(&ConfigurationValue::Float(0.5)), None);
	}

	#[cfg(feature = "headed")]
	#[test]
	fn drain_applies_a_table_and_says_what_to_use_instead_of_a_bad_name_or_value() {
		const TAPS: &[Setting<u32>] = &[Setting {
			name: "taps",
			accepts: Accepts::Whole {
				min: 1,
				max: 32,
				step: 1,
			},
			apply: |_, read| Ok(read.whole()),
		}];
		let configuration = Configuration::new();
		let port = configuration.register("render.motion-blur.");
		let set = configuration.update("render.motion-blur.taps", ConfigurationValue::Integer(12));
		let too_many = configuration.update("render.motion-blur.taps", ConfigurationValue::Integer(64));
		let unknown = configuration.update("render.motion-blur.shutter", ConfigurationValue::Float(0.01));

		let mut taps = 8;
		port.drain(&mut taps, TAPS);

		assert_eq!(taps, 12);
		let state = |id| configuration.event(id).unwrap().state().clone();
		assert_eq!(
			state(set),
			ConfigurationUpdateState::Set {
				value: ConfigurationValue::Integer(12)
			}
		);
		assert!(matches!(state(too_many), ConfigurationUpdateState::NotSet { reason }
			if reason.ends_with("Set it to a whole number from 1 through 32.")));
		assert!(matches!(state(unknown), ConfigurationUpdateState::NotSet { reason }
			if reason.starts_with("`render.motion-blur.shutter` was not set.") && reason.ends_with("Use `render.motion-blur.taps`.")));
	}
}
