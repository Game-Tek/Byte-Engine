mod configuration;
mod core;
mod targets;

pub(crate) use core::RendererScreenshotError;
pub use core::Renderer;
#[cfg(test)]
use std::collections::VecDeque;

pub(crate) use configuration::RENDER_PASS_PARAMETER_PREFIX;
#[cfg(test)]
use configuration::{apply_render_pass_configuration, set_render_pass_state};
pub(crate) use targets::RenderNode;
pub use targets::RenderTargets;

#[cfg(test)]
use crate::{
	configuration::{Configuration, ConfigurationValue},
	rendering::{
		Sink,
		render_pass::{RenderPass, RenderPassHarness, RenderPassReturn, RenderPassState, RenderPassStates},
	},
};

#[cfg(test)]
#[allow(
	unsafe_code,
	clippy::undocumented_unsafe_blocks,
	reason = "Renderer tests manufacture opaque GHI handles without exposing a production constructor."
)]
mod tests {
	use utils::Box;

	use super::*;
	use crate::configuration::ConfigurationUpdateState;

	/// Creates an opaque nonzero image handle for render-target bookkeeping tests.
	pub(super) fn image_handle(value: u64) -> ghi::BaseImageHandle {
		assert_ne!(value, 0);
		// SAFETY: Test values are nonzero and `BaseImageHandle` is the transparent opaque handle representation used by GHI.
		unsafe { std::mem::transmute(value) }
	}

	struct NamedRenderPass(&'static str);

	impl RenderPass for NamedRenderPass {
		fn name(&self) -> &'static str {
			self.0
		}

		fn prepare<'a>(
			&mut self,
			_frame: &mut ghi::implementation::Frame,
			_sink: &Sink,
			_frame_allocator: &'a bumpalo::Bump,
		) -> Option<RenderPassReturn<'a>> {
			None
		}
	}

	/// Creates one harness per name, sharing state by name the way the renderer does for every sink.
	fn harnesses<const N: usize>(states: &mut RenderPassStates, names: [&'static str; N]) -> [RenderPassHarness; N] {
		names.map(|name| RenderPassHarness::new(Box::new(NamedRenderPass(name)), states))
	}

	#[test]
	fn render_pass_state_updates_every_sink_instance_with_the_requested_name() {
		let mut states = RenderPassStates::default();
		let render_passes = harnesses(&mut states, ["bloom", "ui", "bloom"]);

		let updated = set_render_pass_state(&mut states, "bloom", RenderPassState::Bypassed);

		assert_eq!(updated, 2);
		assert_eq!(render_passes[0].state(), RenderPassState::Bypassed);
		assert_eq!(render_passes[1].state(), RenderPassState::Enabled);
		assert_eq!(render_passes[2].state(), RenderPassState::Bypassed);
		assert_eq!(set_render_pass_state(&mut states, "missing", RenderPassState::Enabled), 0);
	}

	#[test]
	fn render_configuration_sets_existing_and_future_pass_instances() {
		let configuration = Configuration::new();
		let port = configuration.register(RENDER_PASS_PARAMETER_PREFIX);
		let event = configuration.update("render.pass.bloom", "bypassed");
		let mut pending = VecDeque::new();
		let mut states = RenderPassStates::default();
		let passes = harnesses(&mut states, ["bloom", "bloom"]);

		apply_render_pass_configuration(&port, &mut pending, &states);

		assert_eq!(passes[0].state(), RenderPassState::Bypassed);
		assert_eq!(passes[1].state(), RenderPassState::Bypassed);
		assert!(matches!(
			configuration.event(event).unwrap().state(),
			ConfigurationUpdateState::Set { value }
				if value == &ConfigurationValue::from("bypassed")
		));

		let [future] = harnesses(&mut states, ["bloom"]);

		assert_eq!(future.state(), RenderPassState::Bypassed);
	}

	#[test]
	fn render_configuration_stays_pending_until_the_pass_exists() {
		let configuration = Configuration::new();
		let port = configuration.register(RENDER_PASS_PARAMETER_PREFIX);
		let event = configuration.update("render.pass.bloom", "bypassed");
		let mut pending = VecDeque::new();
		let mut states = RenderPassStates::default();

		apply_render_pass_configuration(&port, &mut pending, &states);

		assert_eq!(pending.len(), 1);
		assert_eq!(
			configuration.event(event).unwrap().state(),
			&ConfigurationUpdateState::Pending
		);

		let passes = harnesses(&mut states, ["bloom"]);
		apply_render_pass_configuration(&port, &mut pending, &states);

		assert_eq!(pending.len(), 0);
		assert_eq!(passes[0].state(), RenderPassState::Bypassed);
	}

	#[test]
	fn render_targets_keep_names_and_aliases_isolated_by_sink() {
		let mut rt = RenderTargets::new();
		let first_image = image_handle(1);
		let second_image = image_handle(2);
		let other_sink_image = image_handle(3);

		rt.insert("first".to_string(), 0, first_image, ghi::Formats::RGBA16UNORM, 1, false);
		rt.insert("second".to_string(), 0, second_image, ghi::Formats::RGBA16UNORM, 1, false);
		rt.insert("main".to_string(), 1, other_sink_image, ghi::Formats::Depth32, 1, false);
		rt.alias(0, "first", "main");
		rt.alias(0, "second", "main");

		let (sink0_image, _) = rt.get("main", 0).expect("sink 0 main should resolve");
		let (sink1_image, _) = rt.get("main", 1).expect("sink 1 main should resolve");

		assert_eq!(sink0_image, second_image);
		assert_eq!(sink1_image, other_sink_image);
		assert_eq!(rt.get("missing", 0), None);
	}
}
