use crate::{
	configuration::{Accepts, ConfigurationPort, ConfigurationUpdate, ConfigurationValue},
	rendering::{PendingUpdates, apply_pending_updates, render_pass::RenderPassState},
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
	pending: &mut PendingUpdates<(String, RenderPassState)>,
	mut set_state: impl FnMut(&str, RenderPassState) -> usize,
) {
	apply_pending_updates(configuration, pending, parse_render_pass_state, |(name, state)| {
		Ok((set_state(name, *state) > 0).then(|| ConfigurationValue::from(state.as_parameter_value())))
	});
}

/// Reads one `render.pass.<name>` update as the pass it names and the state it asks for.
fn parse_render_pass_state(update: &ConfigurationUpdate) -> Result<(String, RenderPassState), String> {
	let not_set = |cause: &str| format!("Render pass state was not set. The most likely cause is that {cause}.");
	if update.parameter().is_empty() {
		return Err(not_set("the parameter does not name a render pass"));
	}
	let state = match update.value().as_text() {
		Some("enabled") => RenderPassState::Enabled,
		Some("bypassed") => RenderPassState::Bypassed,
		Some(_) => return Err(not_set("the value is neither `enabled` nor `bypassed`")),
		None => return Err(not_set("the requested value is not text")),
	};
	Ok((update.parameter().to_string(), state))
}
