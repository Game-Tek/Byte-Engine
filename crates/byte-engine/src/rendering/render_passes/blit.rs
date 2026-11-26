use crate::rendering::{
	Sink,
	render_pass::{MainRenderTarget, RenderPassBuilder, RenderPassReturn, simple_compute, simple_tile},
};

/// The `ImageBypassPass` struct copies one image into another so the `main` chain keeps flowing past a pass that
/// does not write it.
///
/// [`RenderPassBuilder::create_main_render_target`] builds one for a replacement that can't be the incoming `main`'s
/// image, which the renderer records in a frame the pass records nothing. The renderer also builds one to present
/// scene color when no post-scene pass replaces `main`.
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
			"Render Pass Bypass",
			Self::TILE_PIPELINE,
			&[destination.format()],
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
		let pipeline = simple_compute::Pipeline::compile(render_pass_builder, "Render Pass Bypass", Self::PIPELINE);
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

	use crate::rendering::shader_vm_test::{
		PIXEL_TILE, assert_rgba_close, compile_source, empty_image, rgba, run_at, run_pixel, slot, texture_2d,
	};

	const IMAGE_BYPASS_SHADER: &str = include_str!("../../../assets/rendering/blit/image.besl");
	const IMAGE_TILE_BYPASS_SHADER: &str = include_str!("../../../assets/rendering/blit/image-tile.besl");

	/// Verifies that the compute and the tile copy forward every pixel, and that the invocations past the image's edge,
	/// which dispatch rounding and a partial tile produce, read nothing and write nothing.
	#[test]
	fn image_bypass_besl_vm_copies_pixels_and_ignores_out_of_bounds_invocations() {
		let expected = [
			[0.1, 0.2, 0.3, 0.4],
			[0.5, 0.6, 0.7, 0.8],
			[0.9, 0.8, 0.7, 0.6],
			[0.4, 0.3, 0.2, 0.1],
		];
		for tile in [false, true] {
			let program = compile_source(if tile { IMAGE_TILE_BYPASS_SHADER } else { IMAGE_BYPASS_SHADER });
			let mut source = texture_2d(2, 2, &expected);
			let mut result = empty_image(2, 2);
			for y in 0..4 {
				for x in 0..4 {
					let mut descriptors = DescriptorBindings::new();
					descriptors.bind_image(slot(&program, "source"), &mut source);
					if tile {
						descriptors.bind_image(output_slot(0), &mut result);
						run_pixel(&program, &mut descriptors, [x, y], PIXEL_TILE, [2, 2]);
					} else {
						descriptors.bind_image(slot(&program, "result"), &mut result);
						run_at(&program, &mut descriptors, [x, y]);
					}
				}
			}
			for (index, expected) in expected.into_iter().enumerate() {
				assert_rgba_close(rgba(&result, [(index % 2) as u32, (index / 2) as u32]), expected, 0.0);
			}
		}
	}
}
