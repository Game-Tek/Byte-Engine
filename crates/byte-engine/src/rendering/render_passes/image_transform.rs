//! One pixel step from the incoming `main` into display color, shared by tone mappers and display encoders.

use crate::{
	core::Entity,
	rendering::render_pass::{RenderPassBuilder, pixel_steps::PixelStep, simple_tile},
};

/// The `Configuration` struct names what one image transform step reads, runs, and writes.
///
/// Each transform is a `const` next to its shader tests, such as [`super::aces::TONE_MAPPING`]. Pass it to
/// [`ImageTransformStep::new`] from a pixel step factory.
pub struct Configuration {
	/// The stable pass name that `render.pass.<name>` enables or bypasses.
	pub name: &'static str,
	/// The step's `tile` pipeline, whose shader is a pixel shader without a layer.
	pub pipeline_id: &'static str,
	/// The render-target name of the replacement `main` when the step ends its run, which screenshots can capture.
	pub output_name: &'static str,
	/// Whether the transform needs scene-linear floating-point input, so a preceding display-referred `main` is a
	/// setup mistake.
	pub requires_float_input: bool,
}

/// The `ImageTransformStep` struct runs one sink's image transform, such as a tone mapper, as a pixel step.
///
/// Register it with [`crate::rendering::renderer::Renderer::add_pixel_step_for_all_sinks`] and its configuration's
/// pipeline. Its run writes display color, or the swapchain when it is the final pass.
pub struct ImageTransformStep {
	configuration: &'static Configuration,
}

impl Entity for ImageTransformStep {}

impl ImageTransformStep {
	/// Checks the run's incoming `main` against the transform and creates the step.
	pub fn new(render_pass_builder: &mut RenderPassBuilder<'_>, configuration: &'static Configuration) -> Self {
		if configuration.requires_float_input {
			assert_eq!(
				render_pass_builder.format_of("main").encoding(),
				Some(ghi::Encodings::FloatingPoint),
				"The `{}` step requires scene-linear floating-point input. The most likely cause is a preceding pass that replaced `main` with another format.",
				configuration.name
			);
		}
		Self { configuration }
	}
}

impl PixelStep for ImageTransformStep {
	fn name(&self) -> &'static str {
		self.configuration.name
	}

	fn output(&self) -> &'static str {
		self.configuration.output_name
	}

	fn output_format(&self, _input: ghi::Formats) -> ghi::Formats {
		crate::rendering::DISPLAY_COLOR_FORMAT
	}

	fn resources(&self) -> &[simple_tile::Resource] {
		&[]
	}
}
