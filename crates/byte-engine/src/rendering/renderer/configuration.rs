use std::{collections::VecDeque, rc::Rc};

use crate::{
	configuration::{ConfigurationEventId, ConfigurationPort, ConfigurationUpdate, ConfigurationValue},
	rendering::render_pass::{RenderPassState, RenderPassStates},
};

pub(crate) const RENDER_PASS_PARAMETER_PREFIX: &str = "render.pass.";

/// Sets the state every pass instance named `name` shares, including instances created later.
///
/// Returns how many existing instances share the state.
pub(super) fn set_render_pass_state(states: &mut RenderPassStates, name: &str, state: RenderPassState) -> usize {
	match states.get(name) {
		Some(shared) => {
			shared.set(state);
			instance_count(shared)
		}
		None => {
			states.insert(name.to_string(), Rc::new(state.into()));
			0
		}
	}
}

/// Counts the pass instances holding a shared state; the map holds the only other reference.
fn instance_count(shared: &Rc<std::cell::Cell<RenderPassState>>) -> usize {
	Rc::strong_count(shared) - 1
}

/// Applies valid queued states and retains updates whose named pass has not been installed yet.
pub(super) fn apply_render_pass_configuration(
	configuration: &ConfigurationPort,
	pending: &mut VecDeque<PendingRenderPassConfiguration>,
	states: &RenderPassStates,
) {
	while let Some(update) = configuration.read() {
		match PendingRenderPassConfiguration::from_update(update) {
			Ok(update) => pending.push_back(update),
			Err((id, reason)) => configuration.not_set(id, reason),
		}
	}

	// Try each retained update once per call. A pass that has not been installed yet keeps the event pending.
	let pending_count = pending.len();
	for _ in 0..pending_count {
		let update = pending.pop_front().expect("pending configuration count changed");
		let Some(shared) = states
			.get(&update.render_pass_name)
			.filter(|shared| instance_count(shared) > 0)
		else {
			pending.push_back(update);
			continue;
		};

		shared.set(update.state);
		configuration.set(update.event, ConfigurationValue::from(update.state.as_parameter_value()));
	}
}

pub(super) struct PendingRenderPassConfiguration {
	event: ConfigurationEventId,
	render_pass_name: String,
	state: RenderPassState,
}

impl PendingRenderPassConfiguration {
	/// Validates a generic configuration message once before retaining it for renderer application.
	fn from_update(update: ConfigurationUpdate) -> Result<Self, (ConfigurationEventId, String)> {
		let event = update.id();
		let Some(render_pass_name) = update.parameter().strip_prefix(RENDER_PASS_PARAMETER_PREFIX) else {
			return Err((
				event,
				"Render pass state was not set. The most likely cause is that the parameter is outside the `render.pass.` namespace."
					.to_string(),
			));
		};
		if render_pass_name.is_empty() {
			return Err((
				event,
				"Render pass state was not set. The most likely cause is that the parameter does not name a render pass."
					.to_string(),
			));
		}
		let Some(value) = update.value().as_text() else {
			return Err((
				event,
				"Render pass state was not set. The most likely cause is that the requested value is not text.".to_string(),
			));
		};
		let state = match value {
			"enabled" => RenderPassState::Enabled,
			"bypassed" => RenderPassState::Bypassed,
			_ => {
				return Err((
					event,
					"Render pass state was not set. The most likely cause is that the value is neither `enabled` nor `bypassed`."
						.to_string(),
				));
			}
		};

		Ok(Self {
			event,
			render_pass_name: render_pass_name.to_string(),
			state,
		})
	}
}
