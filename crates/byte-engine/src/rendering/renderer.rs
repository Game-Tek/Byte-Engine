mod configuration;
mod core;
mod targets;

pub use core::Renderer;
pub(crate) use core::RendererScreenshotError;
#[cfg(test)]
use std::collections::VecDeque;

#[cfg(test)]
use configuration::{apply_render_pass_configuration, set_render_pass_state};
pub(crate) use targets::RenderNode;
pub use targets::{Placement, RenderTargets, TargetId, TargetSet};

#[cfg(test)]
use crate::{
	configuration::{Configuration, ConfigurationValue},
	rendering::{
		Sink,
		render_pass::{
			Installed, InstalledPasses, RenderFeature, RenderPass, RenderPassBuilder, RenderPassReturn, RenderPassState,
		},
	},
};

#[cfg(test)]
#[allow(
	unsafe_code,
	clippy::undocumented_unsafe_blocks,
	reason = "Renderer tests manufacture opaque GHI handles without exposing a production constructor."
)]
pub(crate) mod tests {
	use super::*;
	use crate::configuration::ConfigurationUpdateState;

	/// Creates an opaque nonzero image handle for render-target bookkeeping tests.
	pub(crate) fn image_handle(value: u64) -> ghi::BaseImageHandle {
		assert_ne!(value, 0);
		// SAFETY: Test values are nonzero and `BaseImageHandle` is the transparent opaque handle representation used by GHI.
		unsafe { std::mem::transmute(value) }
	}

	/// The `LensSampling` struct is a feature whose passes ask for lens samples while their setting says so.
	struct LensSampling {
		samples: bool,
	}

	/// The `LensSamplingPass` struct is one sink's pass of [`LensSampling`].
	struct LensSamplingPass;

	impl RenderFeature for LensSampling {
		type Pass = LensSamplingPass;

		fn name(&self) -> &'static str {
			"lens-sampling"
		}

		fn create_pass(&mut self, _builder: &mut RenderPassBuilder) -> LensSamplingPass {
			LensSamplingPass
		}
	}

	impl RenderPass<LensSampling> for LensSamplingPass {
		fn prepare<'a>(
			&mut self,
			_feature: &mut LensSampling,
			_frame: &mut ghi::implementation::Frame,
			_sink: &Sink,
			_frame_allocator: &'a bumpalo::Bump,
		) -> Option<RenderPassReturn<'a>> {
			None
		}

		fn hints(&self, feature: &LensSampling) -> crate::rendering::render_pass::PassHints {
			crate::rendering::render_pass::PassHints {
				samples_lens: feature.samples,
				..Default::default()
			}
		}
	}

	#[test]
	fn a_pass_reads_its_feature_and_only_its_sink_has_one() {
		let mut installed = Installed::new(LensSampling { samples: true });
		installed.passes.push((0, LensSamplingPass));
		let samples_lens = |installed: &Installed<LensSampling>, sink| installed.hints(sink).samples_lens;
		assert!(samples_lens(&installed, 0) && !samples_lens(&installed, 1));

		installed.feature.samples = false;
		assert!(!samples_lens(&installed, 0));

		installed.remove(0);
		installed.feature.samples = true;
		assert!(!samples_lens(&installed, 0));
	}

	/// Returns registrations named `names`, each enabled, as the renderer keeps their names and states.
	fn registrations<const N: usize>(names: [&'static str; N]) -> [(&'static str, RenderPassState); N] {
		names.map(|name| (name, RenderPassState::Enabled))
	}

	/// Sets `state` on every registration of `registrations` named `name`, as the renderer does.
	fn set(registrations: &mut [(&'static str, RenderPassState)], name: &str, state: RenderPassState) -> usize {
		set_render_pass_state(registrations.iter_mut().map(|(name, state)| (*name, state)), name, state)
	}

	#[test]
	fn render_pass_state_updates_every_registration_with_the_requested_name() {
		let mut registrations = registrations(["bloom", "ui", "bloom"]);

		let updated = set(&mut registrations, "bloom", RenderPassState::Bypassed);

		assert_eq!(updated, 2);
		assert_eq!(registrations[0].1, RenderPassState::Bypassed);
		assert_eq!(registrations[1].1, RenderPassState::Enabled);
		assert_eq!(registrations[2].1, RenderPassState::Bypassed);
		assert_eq!(set(&mut registrations, "missing", RenderPassState::Enabled), 0);
	}

	#[test]
	fn render_configuration_sets_every_registration_with_the_name() {
		let configuration = Configuration::new();
		let port = configuration.register("render.pass.");
		let event = configuration.update("render.pass.bloom", "bypassed");
		let mut pending = VecDeque::new();
		let mut registrations = registrations(["bloom", "bloom"]);

		apply_render_pass_configuration(&port, &mut pending, |name, state| set(&mut registrations, name, state));

		assert_eq!(registrations[0].1, RenderPassState::Bypassed);
		assert_eq!(registrations[1].1, RenderPassState::Bypassed);
		assert!(matches!(
			configuration.event(event).unwrap().state(),
			ConfigurationUpdateState::Set { value }
				if value == &ConfigurationValue::from("bypassed")
		));
	}

	#[test]
	fn render_configuration_stays_pending_until_the_pass_is_registered() {
		let configuration = Configuration::new();
		let port = configuration.register("render.pass.");
		let event = configuration.update("render.pass.bloom", "bypassed");
		let mut pending = VecDeque::new();
		let mut none: [(&'static str, RenderPassState); 0] = [];

		apply_render_pass_configuration(&port, &mut pending, |name, state| set(&mut none, name, state));

		assert_eq!(pending.len(), 1);
		assert_eq!(
			configuration.event(event).unwrap().state(),
			&ConfigurationUpdateState::Pending
		);

		let mut registrations = registrations(["bloom"]);
		apply_render_pass_configuration(&port, &mut pending, |name, state| set(&mut registrations, name, state));

		assert_eq!(pending.len(), 0);
		assert_eq!(registrations[0].1, RenderPassState::Bypassed);
	}

	#[test]
	fn render_targets_keep_names_and_aliases_isolated_by_sink() {
		let mut rt = RenderTargets::new();
		let first_image = image_handle(1);
		let second_image = image_handle(2);
		let other_sink_image = image_handle(3);

		rt.insert(
			"first".to_string(),
			0,
			first_image,
			ghi::Formats::RGBA16UNORM,
			ghi::Uses::empty(),
			1,
			Placement::Persistent,
		);
		rt.insert(
			"second".to_string(),
			0,
			second_image,
			ghi::Formats::RGBA16UNORM,
			ghi::Uses::empty(),
			1,
			Placement::Persistent,
		);
		rt.insert(
			"main".to_string(),
			1,
			other_sink_image,
			ghi::Formats::Depth32,
			ghi::Uses::empty(),
			1,
			Placement::Persistent,
		);
		rt.alias(0, "first", "main");
		rt.alias(0, "second", "main");

		let (sink0_image, _) = rt.get("main", 0).expect("sink 0 main should resolve");
		let (sink1_image, _) = rt.get("main", 1).expect("sink 1 main should resolve");

		assert_eq!(sink0_image, second_image);
		assert_eq!(sink1_image, other_sink_image);
		assert_eq!(rt.get("missing", 0), None);
	}
}
