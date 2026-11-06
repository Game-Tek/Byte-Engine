use crate::rendering::{
	Sink,
	render_pass::{MainRenderTarget, RenderPassBuilder, RenderPassReturn, simple_compute, simple_tile},
};

/// The `ImageBypassPass` struct copies one image into another so the `main` chain keeps flowing past a pass that
/// does not write it.
///
/// [`RenderPassBuilder::create_main_render_target`] builds one per replacement for bypassed passes, and the renderer
/// builds one to present scene color when no post-scene pass writes the swapchain.
pub(crate) struct ImageBypassPass {
	render_pass: Copy,
}

/// The `Copy` enum holds the pass that copies: a compute shader that writes a storage image, or a tile shader that
/// writes a render target, which keeps the destination free of shader-write usage.
enum Copy {
	Compute(simple_compute::Pass),
	Tile(simple_tile::Pass),
}

impl ImageBypassPass {
	const PIPELINE: &'static str = "byte_engine/rendering/blit/image.pipeline";
	const TILE_PIPELINE: &'static str = "byte_engine/rendering/blit/image-tile.pipeline";

	/// Starts the shared copy shader before a sink needs its forwarding pass.
	pub(crate) fn request_pipeline(manager: &crate::rendering::PipelineManagerClient) {
		manager.request_pipeline(Self::PIPELINE);
	}

	/// Creates a tile copy from the pass input to the replacement for `main` that downstream passes consume.
	///
	/// The replacement needs [`simple_tile::output_uses`], which [`RenderPassBuilder::create_main_render_target`]
	/// gives it.
	pub(crate) fn forwarding(
		render_pass_builder: &mut RenderPassBuilder<'_>,
		source: impl Into<ghi::BaseImageHandle>,
		destination: MainRenderTarget,
	) -> Self {
		let pipeline = simple_tile::Pipeline::compile(
			render_pass_builder,
			simple_tile::Descriptor::new("Render Pass Bypass", Self::TILE_PIPELINE, &[destination.format()]),
		);
		let render_pass = pipeline.bind(
			"Render Pass Bypass Descriptor Set",
			&[simple_tile::Resource::image("source", source.into())],
			&[simple_tile::Output::new("result", destination)],
		);

		Self {
			render_pass: Copy::Tile(render_pass),
		}
	}

	/// Creates a compute copy from the pass input to the output consumed by downstream passes.
	pub(crate) fn new(
		render_pass_builder: &mut RenderPassBuilder<'_>,
		source: impl Into<ghi::BaseImageHandle>,
		destination: impl Into<ghi::ImageOrSwapchain>,
	) -> Self {
		let source = source.into();
		let pipeline = simple_compute::Pipeline::compile(
			render_pass_builder,
			simple_compute::Descriptor::new("Render Pass Bypass", Self::PIPELINE),
		);
		let render_pass = pipeline.bind(
			"Render Pass Bypass Descriptor Set",
			&[
				simple_compute::Resource::image("source", source),
				simple_compute::Resource::image("result", destination),
			],
		);

		Self {
			render_pass: Copy::Compute(render_pass),
		}
	}

	/// Prepares the forwarding copy for the current sink.
	pub(crate) fn prepare<'a>(
		&mut self,
		frame: &mut ghi::implementation::Frame,
		sink: &Sink,
		frame_allocator: &'a bumpalo::Bump,
	) -> Option<RenderPassReturn<'a>> {
		match &mut self.render_pass {
			Copy::Compute(render_pass) => render_pass.prepare(frame, sink, frame_allocator),
			Copy::Tile(render_pass) => render_pass.prepare(frame, sink, frame_allocator),
		}
	}
}

#[cfg(test)]
mod tests {
	use besl::vm::{DescriptorBindings, output_slot};

	use super::simple_compute;
	use crate::rendering::shader_vm_test::{
		PIXEL_TILE, assert_rgba_close, empty_image, rgba, run_at, run_pixel, slot, texture_2d,
	};

	const IMAGE_BYPASS_SHADER: &str = include_str!("../../../assets/rendering/blit/image.besl");
	const IMAGE_TILE_BYPASS_SHADER: &str = include_str!("../../../assets/rendering/blit/image-tile.besl");

	#[test]
	fn image_bypass_besl_vm_copies_pixels_and_ignores_out_of_bounds_invocations() {
		assert_copy_shader_behavior(IMAGE_BYPASS_SHADER);
	}

	/// Verifies the tile copy forwards every pixel of a partial tile and skips the lanes past the outputs' edge.
	#[test]
	fn image_tile_bypass_besl_vm_copies_pixels_and_skips_lanes_past_the_edge() {
		let program = crate::rendering::shader_vm_test::compile(simple_compute::compile_test_program(IMAGE_TILE_BYPASS_SHADER));
		let expected = [
			[0.1, 0.2, 0.3, 0.4],
			[0.5, 0.6, 0.7, 0.8],
			[0.9, 0.8, 0.7, 0.6],
			[0.4, 0.3, 0.2, 0.1],
		];
		let mut source = texture_2d(2, 2, &expected);
		let mut result = empty_image(2, 2);

		// The one tile's lanes past the 2x2 outputs read nothing and write nothing.
		for y in 0..4 {
			for x in 0..4 {
				let mut descriptors = DescriptorBindings::new();
				descriptors.bind_image(slot(&program, "source"), &mut source);
				descriptors.bind_image(output_slot(0), &mut result);
				run_pixel(&program, &mut descriptors, [x, y], PIXEL_TILE, [2, 2]);
			}
		}

		for (index, expected) in expected.into_iter().enumerate() {
			assert_rgba_close(rgba(&result, [(index % 2) as u32, (index / 2) as u32]), expected, 0.0);
		}
	}

	/// Executes one production copy shader and verifies forwarding and dispatch-boundary behavior.
	fn assert_copy_shader_behavior(source_code: &str) {
		let program = crate::rendering::shader_vm_test::compile(simple_compute::compile_test_program(source_code));
		let expected = [
			[0.1, 0.2, 0.3, 0.4],
			[0.5, 0.6, 0.7, 0.8],
			[0.9, 0.8, 0.7, 0.6],
			[0.4, 0.3, 0.2, 0.1],
		];
		let mut source = texture_2d(2, 2, &expected);
		let mut result = empty_image(2, 2);

		for y in 0..2 {
			for x in 0..2 {
				let mut descriptors = DescriptorBindings::new();
				descriptors.bind_image(slot(&program, "source"), &mut source);
				descriptors.bind_image(slot(&program, "result"), &mut result);
				run_at(&program, &mut descriptors, [x, y]);
			}
		}

		for (index, expected) in expected.into_iter().enumerate() {
			assert_rgba_close(rgba(&result, [(index % 2) as u32, (index / 2) as u32]), expected, 0.0);
		}

		// Dispatch rounding may produce excess invocations, so the production guard must make those invocations true no-ops.
		for coordinate in [[2, 0], [0, 2]] {
			let mut descriptors = DescriptorBindings::new();
			descriptors.bind_image(slot(&program, "source"), &mut source);
			descriptors.bind_image(slot(&program, "result"), &mut result);
			run_at(&program, &mut descriptors, coordinate);
		}
		for (index, expected) in expected.into_iter().enumerate() {
			assert_rgba_close(rgba(&result, [(index % 2) as u32, (index / 2) as u32]), expected, 0.0);
		}
	}

	/// Lowers every program that reads or writes an R11G11B10F scene target through the platform shader compiler.
	/// `main` is RGBA16F before the sky pass and R11G11B10F after it, so these programs declare their `main` storage
	/// reads without a format, and the sky LUT writes name the packed format. The copy, the image transforms, the fog,
	/// the LUT passes, and SMAA are tile shaders, lowered as tile functions.
	#[cfg(target_os = "macos")]
	#[compio::test]
	async fn packed_hdr_programs_lower_to_the_platform_shader_language() {
		use resource_management::shader::ShaderGenerationSettings;
		use resource_management::shader::besl::backends::platform::PlatformShaderCompiler;
		use utils::Extent;

		for (name, source) in [
			("blit_image", IMAGE_BYPASS_SHADER),
			(
				"ui_composite",
				crate::rendering::shader_vm_test::asset_source("ui/composite.besl"),
			),
			("sky", crate::rendering::shader_vm_test::asset_source("sky.besl")),
			(
				"sky_transmittance",
				crate::rendering::shader_vm_test::asset_source("sky-transmittance.besl"),
			),
			(
				"sky_multiple_scattering",
				crate::rendering::shader_vm_test::asset_source("sky-multiple-scattering.besl"),
			),
			("sky_view", crate::rendering::shader_vm_test::asset_source("sky-view.besl")),
		] {
			let mut root = crate::rendering::shader_vm_test::with_engine_modules(source);
			root.add(crate::rendering::common_shader_generator::common_shader_module().to_vec());
			let root = besl::lex(root).unwrap_or_else(|error| panic!("{name} should link: {error:?}"));
			let settings = ShaderGenerationSettings::compute(Extent::rectangle(8, 8)).name(name.to_string());

			PlatformShaderCompiler::new()
				.generate(&settings, &root)
				.await
				.unwrap_or_else(|error| panic!("{name} should compile for the platform shader language: {error}"));
		}

		// Every one of these declares a 16-pixel tile with one lane per pixel.
		for (name, source) in [
			("blit_image_tile", IMAGE_TILE_BYPASS_SHADER),
			(
				"smaa_edge_detection",
				include_str!("../../../assets/rendering/smaa/edge-detection.besl"),
			),
			(
				"smaa_blend_weights",
				include_str!("../../../assets/rendering/smaa/blend-weights.besl"),
			),
		] {
			let mut root = crate::rendering::shader_vm_test::with_engine_modules(source);
			root.add(crate::rendering::common_shader_generator::common_shader_module().to_vec());
			let root = besl::lex(root).unwrap_or_else(|error| panic!("{name} should link: {error:?}"));
			let settings = ShaderGenerationSettings::tile(PIXEL_TILE, Extent::square(PIXEL_TILE)).name(name.to_string());
			PlatformShaderCompiler::new()
				.generate(&settings, &root)
				.await
				.unwrap_or_else(|error| panic!("{name} should compile for the platform shader language: {error}"));
		}

		// Pixel steps bake through the pixel chain that wraps them, on the same tile.
		for step in [
			"srgb-display/encode.besl",
			"agx/tone-mapping.besl",
			"aces/tone-mapping.besl",
			"hable/tone-mapping.besl",
			"height-fog.besl",
			"color-correction/correct.besl",
			"lut/apply.besl",
			"color-grading/aces.besl",
			"color-grading/aces2.besl",
			"color-grading/dwg.besl",
		] {
			let root = crate::rendering::shader_vm_test::link_pixel_chain(&[step]);
			let settings = ShaderGenerationSettings::tile(PIXEL_TILE, Extent::square(PIXEL_TILE)).name(step.to_string());
			PlatformShaderCompiler::new()
				.generate(&settings, &root)
				.await
				.unwrap_or_else(|error| panic!("{step} should compile for the platform shader language: {error}"));
		}
	}
}
