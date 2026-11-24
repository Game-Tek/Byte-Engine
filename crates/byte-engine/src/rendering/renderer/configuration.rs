use std::collections::VecDeque;

use crate::{
	configuration::{Accepts, ConfigurationEventId, ConfigurationPort, ConfigurationUpdate, ConfigurationValue},
	rendering::render_pass::RenderPassState,
};

/// Sets `state` on every registration of `registrations`, each a name and its state, named `name`.
///
/// Returns how many registrations are named `name`.
pub(super) fn set_render_pass_state<'r>(
	registrations: impl Iterator<Item = (&'static str, &'r mut RenderPassState)>,
	name: &str,
	state: RenderPassState,
) -> usize {
	registrations
		.filter(|(registered, _)| *registered == name)
		.map(|(_, registered)| *registered = state)
		.count()
}

/// Declares a `render.pass.<name>` parameter on `configuration` for every registration of `registrations`, each a name
/// and its state, so adapters such as the console list the passes this application has. Call it again whenever passes
/// are registered.
pub(super) fn declare_render_passes(
	configuration: &ConfigurationPort,
	registrations: impl Iterator<Item = (&'static str, RenderPassState)>,
) {
	configuration.declare(registrations.map(|(name, state)| {
		(
			name,
			Accepts::Choice(&["enabled", "bypassed"]),
			state.as_parameter_value().into(),
		)
	}));
}

/// Applies valid queued states through `set_state`, which sets a state by name and returns how many registrations it
/// reached, and retains updates whose named pass has not been registered yet.
pub(super) fn apply_render_pass_configuration(
	configuration: &ConfigurationPort,
	pending: &mut VecDeque<PendingRenderPassConfiguration>,
	mut set_state: impl FnMut(&str, RenderPassState) -> usize,
) {
	while let Some(update) = configuration.read() {
		match PendingRenderPassConfiguration::from_update(update) {
			Ok(update) => pending.push_back(update),
			Err((id, reason)) => configuration.not_set(id, reason),
		}
	}

	// Try each retained update once per call. A pass that has not been registered yet keeps the event pending.
	let pending_count = pending.len();
	for _ in 0..pending_count {
		let update = pending.pop_front().expect("pending configuration count changed");
		if set_state(&update.render_pass_name, update.state) == 0 {
			pending.push_back(update);
			continue;
		}
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
		let render_pass_name = update.parameter();
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
