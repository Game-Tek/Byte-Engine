mod configuration;
mod core;
mod targets;

pub(crate) use core::RendererScreenshotError;
pub use core::{Renderer, Settings};
#[cfg(test)]
use std::collections::VecDeque;

#[cfg(test)]
use configuration::{
	RENDER_PASS_PARAMETER_PREFIX, apply_render_pass_configuration, render_pass_harness_with_state,
	set_render_pass_state_by_name,
};
pub use targets::RenderTargets;
#[cfg(test)]
use utils::hash::HashMap;

#[cfg(test)]
use crate::{
	configuration::{Configuration, ConfigurationValue},
	rendering::{
		Sink,
		render_pass::{RenderPass, RenderPassHarness, RenderPassReturn, RenderPassState},
	},
};

#[cfg(test)]
#[allow(
	unsafe_code,
	clippy::undocumented_unsafe_blocks,
	reason = "Renderer tests manufacture opaque GHI handles without exposing a production constructor."
)]
mod tests {
	use utils::{Box, hash::HashMapExt as _};

	use super::core::{ResolvedScreenshotCapture, captures_after_pass};
	use super::*;
	use crate::configuration::ConfigurationUpdateState;

	/// Creates an opaque nonzero image handle for render-target bookkeeping tests.
	fn image_handle(value: u64) -> ghi::BaseImageHandle {
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

		fn bypass<'a>(
			&mut self,
			_frame: &mut ghi::implementation::Frame,
			_sink: &Sink,
			_frame_allocator: &'a bumpalo::Bump,
		) -> Option<RenderPassReturn<'a>> {
			None
		}
	}

	#[test]
	fn captures_keep_request_order_at_a_prepared_pass_entry() {
		let image: ghi::BaseImageHandle = image_handle(7);
		let target = ghi::ImageOrSwapchain::Image(image);
		let captures = [
			Ok(ResolvedScreenshotCapture::AfterPass { pass: 2, target }),
			Ok(ResolvedScreenshotCapture::AfterPass { pass: 3, target }),
			Ok(ResolvedScreenshotCapture::AfterPass { pass: 2, target }),
		];

		// Scheduling depends on the retained pass entry, not on whether its prepared command is Some or None.
		assert_eq!(captures_after_pass(&captures, 2).collect::<Vec<_>>(), [0, 2]);
		assert!(captures_after_pass(&captures, 1).next().is_none());
	}

	#[test]
	fn render_pass_state_updates_every_sink_instance_with_the_requested_name() {
		let mut render_passes = [
			RenderPassHarness::new(Box::new(NamedRenderPass("bloom"))),
			RenderPassHarness::new(Box::new(NamedRenderPass("ui"))),
			RenderPassHarness::new(Box::new(NamedRenderPass("bloom"))),
		];

		let updated = set_render_pass_state_by_name(&mut render_passes, "bloom", RenderPassState::Bypassed);

		assert_eq!(updated, 2);
		assert_eq!(render_passes[0].state(), RenderPassState::Bypassed);
		assert_eq!(render_passes[1].state(), RenderPassState::Enabled);
		assert_eq!(render_passes[2].state(), RenderPassState::Bypassed);
		assert_eq!(
			set_render_pass_state_by_name(&mut render_passes, "missing", RenderPassState::Enabled),
			0
		);
	}

	#[test]
	fn render_configuration_sets_existing_and_future_pass_instances() {
		let configuration = Configuration::new();
		let port = configuration.register(RENDER_PASS_PARAMETER_PREFIX);
		let event = configuration.update("render.pass.bloom", "bypassed");
		let mut pending = VecDeque::new();
		let mut states = HashMap::new();
		let mut passes = [
			RenderPassHarness::new(Box::new(NamedRenderPass("bloom"))),
			RenderPassHarness::new(Box::new(NamedRenderPass("bloom"))),
		];

		apply_render_pass_configuration(&port, &mut pending, &mut states, &mut passes);

		assert_eq!(passes[0].state(), RenderPassState::Bypassed);
		assert_eq!(passes[1].state(), RenderPassState::Bypassed);
		assert!(matches!(
			configuration.event(event).unwrap().state(),
			ConfigurationUpdateState::Set { value }
				if value == &ConfigurationValue::from("bypassed")
		));

		let future = render_pass_harness_with_state(Box::new(NamedRenderPass("bloom")), &states);

		assert_eq!(future.state(), RenderPassState::Bypassed);
	}

	#[test]
	fn render_configuration_stays_pending_until_the_pass_exists() {
		let configuration = Configuration::new();
		let port = configuration.register(RENDER_PASS_PARAMETER_PREFIX);
		let event = configuration.update("render.pass.bloom", "bypassed");
		let mut pending = VecDeque::new();
		let mut states = HashMap::new();
		let mut passes = [];

		apply_render_pass_configuration(&port, &mut pending, &mut states, &mut passes);

		assert_eq!(pending.len(), 1);
		assert_eq!(
			configuration.event(event).unwrap().state(),
			&ConfigurationUpdateState::Pending
		);

		let mut passes = [RenderPassHarness::new(Box::new(NamedRenderPass("bloom")))];
		apply_render_pass_configuration(&port, &mut pending, &mut states, &mut passes);

		assert_eq!(pending.len(), 0);
		assert_eq!(passes[0].state(), RenderPassState::Bypassed);
	}

	#[test]
	fn render_configuration_reports_an_unsupported_state() {
		let configuration = Configuration::new();
		let port = configuration.register(RENDER_PASS_PARAMETER_PREFIX);
		let event = configuration.update("render.pass.bloom", "disabled");
		let mut pending = VecDeque::new();
		let mut states = HashMap::new();
		let mut passes = [RenderPassHarness::new(Box::new(NamedRenderPass("bloom")))];

		apply_render_pass_configuration(&port, &mut pending, &mut states, &mut passes);

		assert!(matches!(
			configuration.event(event).unwrap().state(),
			ConfigurationUpdateState::NotSet { reason } if reason.contains("neither `enabled` nor `bypassed`")
		));
		assert_eq!(passes[0].state(), RenderPassState::Enabled);
	}

	#[test]
	fn render_targets_keep_names_and_aliases_isolated_by_sink() {
		let mut rt = RenderTargets::new();
		let first_image = image_handle(1);
		let second_image = image_handle(2);
		let other_sink_image = image_handle(3);

		rt.insert("first".to_string(), 0, first_image, ghi::Formats::RGBA16UNORM);
		rt.insert("second".to_string(), 0, second_image, ghi::Formats::RGBA16UNORM);
		rt.insert("main".to_string(), 1, other_sink_image, ghi::Formats::Depth32);
		rt.alias(0, "first", "main");
		rt.alias(0, "second", "main");

		let (sink0_image, _) = rt.get("main", 0).expect("sink 0 main should resolve");
		let (sink1_image, _) = rt.get("main", 1).expect("sink 1 main should resolve");

		assert_eq!(*sink0_image, second_image);
		assert_eq!(*sink1_image, other_sink_image);
		assert_eq!(rt.get("missing", 0), None);
		assert_eq!(rt.get_attachment_infos(0).len(), 2);
		assert_eq!(rt.get_attachment_infos(1).len(), 1);
		assert!(RenderTargets::new().get_attachment_infos(0).is_empty());
	}
}
