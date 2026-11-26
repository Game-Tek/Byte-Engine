//! One pixel step from the incoming `main` into display color, shared by tone mappers and display encoders.

use crate::rendering::render_pass::{RenderFeature, RenderPassBuilder, pixel_steps::PixelStep, simple_tile};

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
/// Its [`ImageTransform`] feature builds it for every sink. Its run writes display color, or the swapchain when it is the final pass.
pub struct ImageTransformStep {
	configuration: &'static Configuration,
}

/// The `ImageTransform` struct registers one image transform, such as a tone mapper, which every sink applies as its
/// own [`ImageTransformStep`].
///
/// Register it with [`crate::rendering::Renderer::add_pixel_step`] and its configuration's pipeline.
pub struct ImageTransform {
	pub configuration: &'static Configuration,
}

impl RenderFeature for ImageTransform {
	type Pass = ImageTransformStep;

	fn name(&self) -> &'static str {
		self.configuration.name
	}

	fn create_pass(&mut self, builder: &mut RenderPassBuilder) -> ImageTransformStep {
		ImageTransformStep::new(builder, self.configuration)
	}
}

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

impl PixelStep<ImageTransform> for ImageTransformStep {
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
