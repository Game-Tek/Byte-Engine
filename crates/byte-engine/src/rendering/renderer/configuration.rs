use std::collections::VecDeque;

use utils::{Box, hash::HashMap};

use crate::{
	configuration::{ConfigurationEventId, ConfigurationPort, ConfigurationUpdate, ConfigurationValue},
	rendering::render_pass::{RenderPass, RenderPassHarness, RenderPassState},
};

/// Updates every sink-local instance because one render-pass factory may create the same named pass for many sinks.
pub(super) fn set_render_pass_state_by_name(
	render_passes: &mut [RenderPassHarness],
	name: &str,
	state: RenderPassState,
) -> usize {
	let mut updated = 0;
	for render_pass in render_passes {
		if render_pass.name() == name {
			render_pass.set_state(state);
			updated += 1;
		}
	}
	updated
}

pub(super) const RENDER_PASS_PARAMETER_PREFIX: &str = "render.pass.";

/// Builds a render-pass harness with the state previously selected for its stable name.
pub(super) fn render_pass_harness_with_state(
	render_pass: Box<dyn RenderPass>,
	render_pass_states: &HashMap<String, RenderPassState>,
) -> RenderPassHarness {
	let mut harness = RenderPassHarness::new(render_pass);
	if let Some(state) = render_pass_states.get(harness.name()) {
		harness.set_state(*state);
	}
	harness
}

/// Applies valid queued states and retains updates whose named pass has not been installed yet.
pub(super) fn apply_render_pass_configuration(
	configuration: &ConfigurationPort,
	pending: &mut VecDeque<PendingRenderPassConfiguration>,
	render_pass_states: &mut HashMap<String, RenderPassState>,
	render_passes: &mut [RenderPassHarness],
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
		let updated = set_render_pass_state_by_name(render_passes, &update.render_pass_name, update.state);
		if updated == 0 {
			pending.push_back(update);
			continue;
		}

		render_pass_states.insert(update.render_pass_name.clone(), update.state);
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
