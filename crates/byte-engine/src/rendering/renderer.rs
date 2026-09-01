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
		let image: ghi::BaseImageHandle = unsafe { std::mem::transmute(7_u64) };
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
	fn test_insert_and_get() {
		let mut rt = RenderTargets::new();
		let image = unsafe { std::mem::transmute::<u64, ghi::BaseImageHandle>(1) };
		let format = ghi::Formats::RGBA8UNORM;
		let index = rt.insert("test".to_string(), 0, image, format);

		assert_eq!(index, 0);
		let retrieved = rt.get("test", 0);

		assert!(retrieved.is_some());
		assert_eq!(rt.get("nonexistent", 0), None);
	}

	#[test]
	fn test_insert_multiple() {
		let mut rt = RenderTargets::new();
		let image1 = unsafe { std::mem::transmute::<u64, ghi::BaseImageHandle>(1) };
		let format1 = ghi::Formats::RGBA8UNORM;
		let image2 = unsafe { std::mem::transmute::<u64, ghi::BaseImageHandle>(2) };
		let format2 = ghi::Formats::Depth32;

		rt.insert("color".to_string(), 0, image1, format1);
		rt.insert("depth".to_string(), 0, image2, format2);

		assert!(rt.get("color", 0).is_some());
		assert!(rt.get("depth", 0).is_some());
	}

	#[test]
	fn test_get_attachment_infos() {
		let mut rt = RenderTargets::new();
		let image1 = unsafe { std::mem::transmute::<u64, ghi::BaseImageHandle>(1) };
		let format1 = ghi::Formats::RGBA8UNORM;
		let image2 = unsafe { std::mem::transmute::<u64, ghi::BaseImageHandle>(2) };
		let format2 = ghi::Formats::Depth32;

		rt.insert("color".to_string(), 0, image1, format1);
		rt.insert("depth".to_string(), 0, image2, format2);
		rt.insert(
			"other".to_string(),
			1,
			unsafe { std::mem::transmute::<u64, ghi::BaseImageHandle>(3) },
			ghi::Formats::RGBA16UNORM,
		);

		let attachments = rt.get_attachment_infos(0);

		assert_eq!(attachments.len(), 2);

		let attachments_view1 = rt.get_attachment_infos(1);

		assert_eq!(attachments_view1.len(), 1);
	}

	#[test]
	fn test_get_attachment_infos_empty_view() {
		let rt = RenderTargets::new();
		let attachments = rt.get_attachment_infos(0);

		assert!(attachments.is_empty());
	}

	#[test]
	fn test_alias_overrides_previous_mapping() {
		let mut rt = RenderTargets::new();
		let first_image = unsafe { std::mem::transmute::<u64, ghi::BaseImageHandle>(1) };
		let second_image = unsafe { std::mem::transmute::<u64, ghi::BaseImageHandle>(2) };

		rt.insert("first".to_string(), 0, first_image, ghi::Formats::RGBA16UNORM);
		rt.insert("second".to_string(), 0, second_image, ghi::Formats::RGBA16UNORM);
		rt.alias(0, "first", "main");
		rt.alias(0, "second", "main");

		let (image, _) = rt.get("main", 0).expect("main alias should resolve");

		assert_eq!(*image, second_image);
	}

	#[test]
	fn test_insert_same_name_for_different_sinks() {
		let mut rt = RenderTargets::new();
		let image1 = unsafe { std::mem::transmute::<u64, ghi::BaseImageHandle>(1) };
		let image2 = unsafe { std::mem::transmute::<u64, ghi::BaseImageHandle>(2) };

		rt.insert("main".to_string(), 0, image1, ghi::Formats::RGBA16UNORM);
		rt.insert("main".to_string(), 1, image2, ghi::Formats::RGBA16UNORM);

		let (sink0_image, _) = rt.get("main", 0).expect("sink 0 main should resolve");
		let (sink1_image, _) = rt.get("main", 1).expect("sink 1 main should resolve");

		assert_eq!(*sink0_image, image1);
		assert_eq!(*sink1_image, image2);
	}
}
