use ghi::{
	context::{Context as _, ContextCreate as _},
	frame::Frame as _,
	types::Size as _,
};
use resource_management::{
	Reference,
	resources::lut::{Lut, LutKind},
};
use utils::Extent;

use super::lut::{LutShaderParameters, load_lut_bytes, lut_shader_parameters, write_lut_bytes_to_rgba16f_upload_target};
use crate::{
	core::Entity,
	rendering::{
		Sink,
		render_pass::{RenderPass, RenderPassBuilder, RenderPassReturn, simple_compute},
		render_passes::blit::SwapchainBlitPass,
	},
};

/// The `ColorGradingWorkflow` enum selects the fixed grading gamut and transfer function used by a color-grading pass.
#[derive(Clone, Copy)]
pub enum ColorGradingWorkflow {
	/// Converts scene-linear sRGB to ACEScg and ACEScct before applying the output LUT.
	Aces,
	/// Converts scene-linear sRGB to DaVinci Wide Gamut and DaVinci Intermediate before applying the output LUT.
	DaVinciWideGamut,
}

impl ColorGradingWorkflow {
	fn pipeline_id(self) -> &'static str {
		match self {
			Self::Aces => "byte-engine/rendering/color-grading/aces.pipeline",
			Self::DaVinciWideGamut => "byte-engine/rendering/color-grading/dwg.pipeline",
		}
	}

	fn pass_name(self) -> &'static str {
		match self {
			Self::Aces => "aces-color-grading",
			Self::DaVinciWideGamut => "dwg-color-grading",
		}
	}
}

/// The `ColorGradingPass` struct provides a contained scene-linear-to-SDR grading workflow for one rendered view.
pub struct ColorGradingPass {
	pass: simple_compute::Pass,
	bypass_pass: SwapchainBlitPass,
	_parameters: ghi::BufferHandle<LutShaderParameters>,
	lut: Lut,
	lut_reference: Option<Reference<Lut>>,
	lut_image: ghi::ImageHandle,
	lut_uploaded: bool,
	workflow: ColorGradingWorkflow,
}

impl Entity for ColorGradingPass {}

impl ColorGradingPass {
	/// Creates one fused grading and output pass from a workflow-specific creative LUT.
	///
	/// The ACES workflow expects an ACEScct-to-ACEScct LUT. The DaVinci workflow
	/// expects a DaVinci Wide Gamut/Intermediate-to-Intermediate LUT.
	pub fn new(render_pass_builder: &mut RenderPassBuilder<'_>, workflow: ColorGradingWorkflow, lut: Reference<Lut>) -> Self {
		let lut_metadata = lut.resource().clone();
		assert!(
			matches!(lut_metadata.kind, LutKind::ThreeDimensional),
			"Unsupported color-grading LUT. The most likely cause is that the injected resource is not a 3D LUT."
		);

		let source = render_pass_builder.read_from("main");
		let destination = render_pass_builder.render_to_swapchain();
		let pipeline = simple_compute::Pipeline::compile(
			render_pass_builder,
			simple_compute::Descriptor::new("Color Grading", workflow.pipeline_id()),
		)
		.expect("Failed to create color-grading shader. The most likely cause is an incompatible shader interface.");
		let context = render_pass_builder.context();
		let sampler = context.build_sampler(
			ghi::sampler::Builder::new()
				.filtering_mode(ghi::FilteringModes::Linear)
				.mip_map_mode(ghi::FilteringModes::Linear)
				.addressing_mode(ghi::SamplerAddressingModes::Clamp),
		);
		let lut_image = context.build_image(
			ghi::image::Builder::new(ghi::Formats::RGBA16F, ghi::Uses::Image | ghi::Uses::TransferDestination)
				.name("Color Grading LUT")
				.extent(Extent::cube(lut_metadata.size, lut_metadata.size, lut_metadata.size))
				.device_accesses(ghi::DeviceAccesses::HostToDevice)
				.use_case(ghi::UseCases::STATIC),
		);
		let parameters = context.build_buffer::<LutShaderParameters>(
			ghi::buffer::Builder::new(ghi::Uses::Storage)
				.name("Color Grading LUT Parameters")
				.device_accesses(ghi::DeviceAccesses::HostToDevice),
		);
		*context.get_mut_buffer_slice(parameters) = lut_shader_parameters(&lut_metadata);
		let pass = pipeline
			.bind(
				render_pass_builder,
				"Color Grading Descriptor Set",
				&[
					simple_compute::Resource::combined_image_sampler("source_texture", source, sampler, ghi::Layouts::Read),
					simple_compute::Resource::combined_image_sampler("lut_texture", lut_image, sampler, ghi::Layouts::Read),
					simple_compute::Resource::swapchain("result_texture", destination),
					simple_compute::Resource::buffer("parameters", parameters),
				],
			)
			.expect("Failed to bind color-grading resources. The most likely cause is that the BESL interface changed.");
		let bypass_pass = SwapchainBlitPass::from_source(render_pass_builder, source);

		Self {
			pass,
			bypass_pass,
			_parameters: parameters,
			lut: lut_metadata,
			lut_reference: Some(lut),
			lut_image,
			lut_uploaded: false,
			workflow,
		}
	}

	/// Uploads the immutable output LUT on the first frame that uses this pass.
	fn ensure_lut_uploaded(&mut self, frame: &mut ghi::implementation::Frame) {
		if self.lut_uploaded {
			return;
		}

		let reference = self.lut_reference.as_mut().expect(
			"Color-grading LUT reference is missing. The most likely cause is that the pass lost its resource before the first frame.",
		);
		let bytes = load_lut_bytes(reference);
		let target = frame.get_texture_slice_mut(self.lut_image.into());
		write_lut_bytes_to_rgba16f_upload_target(&self.lut, &bytes, target);
		frame.sync_texture(self.lut_image.into());
		self.lut_reference = None;
		self.lut_uploaded = true;
	}
}

impl RenderPass for ColorGradingPass {
	fn name(&self) -> &'static str {
		self.workflow.pass_name()
	}

	fn prepare<'a>(
		&mut self,
		frame: &mut ghi::implementation::Frame,
		sink: &Sink,
		frame_allocator: &'a bumpalo::Bump,
	) -> Option<RenderPassReturn<'a>> {
		self.ensure_lut_uploaded(frame);
		self.pass.prepare(frame, sink, frame_allocator)
	}

	fn bypass<'a>(
		&mut self,
		frame: &mut ghi::implementation::Frame,
		sink: &Sink,
		frame_allocator: &'a bumpalo::Bump,
	) -> Option<RenderPassReturn<'a>> {
		self.bypass_pass.prepare(frame, sink, frame_allocator)
	}
}

#[cfg(test)]
mod tests {
	use besl::vm::{DescriptorBindings, ResourceSlot, Texture, Value};

	use super::ColorGradingWorkflow;
	use crate::rendering::{
		render_pass::simple_compute,
		shader_vm_test::{assert_rgba_close, buffer, empty_image, rgba, run_at, texture_2d},
	};

	const ACES_SHADER: &str = include_str!("../../../assets/rendering/color-grading/aces.besl");
	const DWG_SHADER: &str = include_str!("../../../assets/rendering/color-grading/dwg.besl");

	/// Executes one complete workflow with a two-point identity LUT in its grading encoding.
	fn run_workflow(shader: &str, source_color: [f32; 4]) -> [f32; 4] {
		let program = crate::rendering::shader_vm_test::compile(simple_compute::compile_test_program(shader));
		let mut source = texture_2d(1, 1, &[source_color]);
		let mut lut = Texture::new_3d(2, 2, 2).expect("Expected a valid test LUT extent");
		for z in 0..2 {
			for y in 0..2 {
				for x in 0..2 {
					lut.write_3d([x, y, z], [x as f32, y as f32, z as f32, 1.0])
						.expect("Expected a valid test LUT coordinate");
				}
			}
		}
		let mut result = empty_image(1, 1);
		let parameter_slot = ResourceSlot::new(3);
		let mut parameters = buffer(&program, parameter_slot);
		for (name, value) in [
			("domain_min", [0.0, 0.0, 0.0, 0.0]),
			("domain_scale", [1.0, 1.0, 1.0, 0.0]),
			("sampling", [0.5, 0.25, 0.0, 0.0]),
		] {
			parameters
				.write(name, Value::Vec4F(value))
				.expect("Expected color-grading parameters to match the shader");
		}
		let mut descriptors = DescriptorBindings::new();
		descriptors.bind_texture(ResourceSlot::new(0), &mut source);
		descriptors.bind_texture(ResourceSlot::new(1), &mut lut);
		descriptors.bind_image(ResourceSlot::new(2), &mut result);
		descriptors.bind_buffer(parameter_slot, &mut parameters);
		run_at(&program, &mut descriptors, [0, 0]);
		drop(descriptors);
		rgba(&result, [0, 0])
	}

	#[test]
	fn grading_workflows_preserve_neutral_middle_gray_through_their_sdr_transforms() {
		assert_rgba_close(
			run_workflow(ACES_SHADER, [0.18, 0.18, 0.18, 0.4]),
			[0.3584574, 0.3584574, 0.3584574, 0.4],
			4e-4,
		);
		assert_rgba_close(
			run_workflow(DWG_SHADER, [0.18, 0.18, 0.18, 0.4]),
			[0.45925015, 0.45925015, 0.45925015, 0.4],
			4e-4,
		);
	}

	#[test]
	fn grading_workflows_bound_black_and_hdr_values_for_sdr_output() {
		for shader in [ACES_SHADER, DWG_SHADER] {
			for input in [0.0, 1.0, 16.0] {
				let output = run_workflow(shader, [input, input, input, 0.25]);
				assert!(
					output[..3]
						.iter()
						.all(|channel| channel.is_finite() && (0.0..=1.0).contains(channel)),
					"Invalid fitted SDR output. The most likely cause is unstable grading or display-transform arithmetic: {output:?}"
				);
				assert!((output[0] - output[1]).abs() <= 4e-4 && (output[1] - output[2]).abs() <= 4e-4);
			}
		}
	}

	#[test]
	fn workflow_names_are_stable_for_render_pass_controls() {
		assert_eq!(ColorGradingWorkflow::Aces.pass_name(), "aces-color-grading");
		assert_eq!(ColorGradingWorkflow::DaVinciWideGamut.pass_name(), "dwg-color-grading");
	}
}
