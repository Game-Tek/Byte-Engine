/// The `LutWorkflow` enum selects what a [`LutPass`] does around its 3D LUT: which shader runs and which pass and
/// target names it answers to.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LutWorkflow {
	/// Applies a creative LUT to scene-linear color and keeps the `main` format, before tone mapping.
	Creative,
	/// Converts scene-linear sRGB to ACEScg and ACEScct before applying the output LUT, then writes SDR display color.
	Aces,
	/// Converts scene-linear sRGB to ACEScg and ACEScct before applying the output LUT, then writes SDR display color
	/// through the ACES 2.0 output transform for a Rec.709, D65, 100-nit display.
	Aces2,
	/// Converts scene-linear sRGB to DaVinci Wide Gamut and DaVinci Intermediate before applying the output LUT, then
	/// writes SDR display color.
	DaVinciWideGamut,
}

impl LutWorkflow {
	fn pipeline_id(self) -> &'static str {
		match self {
			Self::Creative => "byte-engine/rendering/lut/apply.pipeline",
			Self::Aces => "byte-engine/rendering/color-grading/aces.pipeline",
			Self::Aces2 => "byte-engine/rendering/color-grading/aces2.pipeline",
			Self::DaVinciWideGamut => "byte-engine/rendering/color-grading/dwg.pipeline",
		}
	}

	/// Returns the stable name that `render.pass.<name>` enables or bypasses.
	fn pass_name(self) -> &'static str {
		match self {
			Self::Creative => "lut",
			Self::Aces => "aces-color-grading",
			Self::Aces2 => "aces2-color-grading",
			Self::DaVinciWideGamut => "dwg-color-grading",
		}
	}

	/// Returns the render-target name of the replacement `main`, which screenshots can capture.
	fn output_name(self) -> &'static str {
		match self {
			Self::Creative => "LUT Output",
			Self::Aces => "ACES Color Grading Output",
			Self::Aces2 => "ACES 2.0 Color Grading Output",
			Self::DaVinciWideGamut => "DWG Color Grading Output",
		}
	}

	/// Returns the label of the pass's GPU region and descriptor set.
	fn label(self) -> &'static str {
		match self {
			Self::Creative => "LUT",
			Self::Aces | Self::Aces2 | Self::DaVinciWideGamut => "Color Grading",
		}
	}
}

/// The `LutPassSettings` struct lets an application soften a LUT whose look is stronger than the scene needs.
///
/// Pass it to [`crate::application::graphics::setup_aces2_color_grading_render_pass`] or another LUT setup function.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct LutPassSettings {
	/// How much of the look to apply, from 0 (none) to 1 (all of it). The pass mixes the ungraded and graded color
	/// in the encoding the LUT reads, such as ACEScct, so a half-strength look stays in the LUT's color space.
	pub strength: f32,
}

impl Default for LutPassSettings {
	fn default() -> Self {
		Self { strength: 1.0 }
	}
}

/// The `LutPass` struct applies an asynchronously prepared 3D LUT through renderer-owned GPU storage.
///
/// Install it through [`crate::application::graphics::setup_lut_render_pass`] or a color-grading setup function.
pub struct LutPass {
	workflow: LutWorkflow,
	pass: simple_tile::Pass,
	_parameters: ghi::BufferHandle<LutShaderParameters>,
	lut: Lut,
	/// The prepared texels, dropped once the first frame uploads them into `lut_image`.
	lut_bytes: Option<StdBox<[u8]>>,
	lut_image: ghi::ImageHandle,
}

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct LutShaderParameters {
	domain_min: [f32; 4],
	domain_scale: [f32; 4],
	sampling: [f32; 4],
}

impl Entity for LutPass {}

impl LutPass {
	/// Creates one sink's LUT pass from asynchronously prepared metadata and bytes.
	///
	/// The ACES and ACES 2.0 workflows expect an ACEScct-to-ACEScct LUT. The DaVinci workflow expects a DaVinci Wide
	/// Gamut/Intermediate-to-Intermediate LUT. Prepare the LUT through [`PreparedLut::load`] on application-owned
	/// asynchronous work before constructing this render-thread pass.
	pub fn new(
		render_pass_builder: &mut RenderPassBuilder,
		workflow: LutWorkflow,
		lut: PreparedLut,
		settings: LutPassSettings,
	) -> Self {
		let PreparedLut {
			metadata: lut_metadata,
			bytes,
		} = lut;

		assert!(
			matches!(lut_metadata.kind, LutKind::ThreeDimensional),
			"Unsupported LUT kind for LUT render pass. The most likely cause is that the injected LUT resource is not a 3D LUT."
		);
		assert!(
			(0.0..=1.0).contains(&settings.strength),
			"Invalid LUT strength {}. The most likely cause is a strength outside 0 to 1. Set LutPassSettings::strength between 0 (no look) and 1 (the full look).",
			settings.strength
		);

		let source = render_pass_builder.read_from("main");
		// A creative LUT grades scene-linear color for later passes; the grading workflows end in display color.
		let output_format = match workflow {
			LutWorkflow::Creative => render_pass_builder.format_of("main"),
			LutWorkflow::Aces | LutWorkflow::Aces2 | LutWorkflow::DaVinciWideGamut => crate::rendering::DISPLAY_COLOR_FORMAT,
		};
		// The grade's tile output, which the builder gives the uses that needs.
		let output = render_pass_builder
			.create_main_render_target(ghi::image::Builder::new(output_format, ghi::Uses::Image).name(workflow.output_name()));

		let pipeline = simple_tile::Pipeline::compile(
			render_pass_builder,
			simple_tile::Descriptor::new(workflow.label(), workflow.pipeline_id(), &[output.format()]),
		);

		let context = render_pass_builder.context();

		// One linear, clamped sampler reads both the scene color and the LUT.
		let sampler = context.build_sampler(ghi::sampler::Builder::new());
		let lut_image = context.build_image(
			ghi::image::Builder::new(ghi::Formats::RGBA16F, ghi::Uses::Image | ghi::Uses::TransferDestination)
				.name("LUT Texture")
				.extent(Extent::cube(lut_metadata.size, lut_metadata.size, lut_metadata.size))
				.device_accesses(ghi::DeviceAccesses::HostToDevice)
				.use_case(ghi::UseCases::STATIC),
		);
		let parameters = context.build_buffer::<LutShaderParameters>(
			ghi::buffer::Builder::new(ghi::Uses::Storage)
				.name("LUT Parameters")
				.device_accesses(ghi::DeviceAccesses::HostToDevice),
		);
		*context.get_mut_buffer_slice(parameters) = lut_shader_parameters(&lut_metadata, settings.strength);

		let pass = pipeline.bind(
			workflow.label(),
			&[
				simple_tile::Resource::combined_image_sampler("source_texture", source, sampler, ghi::Layouts::Read),
				simple_tile::Resource::combined_image_sampler("lut_texture", lut_image, sampler, ghi::Layouts::Read),
				simple_tile::Resource::buffer("parameters", parameters),
			],
			&[simple_tile::Output::new("result_texture", output)],
		);

		Self {
			workflow,
			pass,
			_parameters: parameters,
			lut: lut_metadata,
			lut_bytes: Some(bytes),
			lut_image,
		}
	}

	/// Uploads the baked LUT payload into the cached GPU 3D texture the first time the pass is used.
	fn ensure_lut_uploaded(&mut self, frame: &mut ghi::implementation::Frame) {
		let Some(lut_bytes) = self.lut_bytes.take() else {
			return;
		};
		let target = frame.get_texture_slice_mut(self.lut_image.into());

		write_lut_bytes_to_rgba16f_upload_target(&self.lut, &lut_bytes, target);
		frame.sync_texture(self.lut_image.into());
	}
}

fn lut_shader_parameters(lut: &Lut, strength: f32) -> LutShaderParameters {
	debug_assert!(
		lut.size > 0,
		"LUT size is zero. The most likely cause is accepting an empty LUT resource."
	);
	debug_assert!(
		(0..3).all(|index| {
			lut.domain_min[index].is_finite()
				&& lut.domain_max[index].is_finite()
				&& lut.domain_max[index] > lut.domain_min[index]
		}),
		"LUT domain is invalid. The most likely cause is a non-finite or non-increasing color range."
	);
	let domain_scale: [f32; 3] = std::array::from_fn(|index| 1.0 / (lut.domain_max[index] - lut.domain_min[index]));
	let lut_size = lut.size as f32;
	LutShaderParameters {
		domain_min: [lut.domain_min[0], lut.domain_min[1], lut.domain_min[2], 0.0],
		domain_scale: [domain_scale[0], domain_scale[1], domain_scale[2], 0.0],
		sampling: [(lut_size - 1.0) / lut_size, 0.5 / lut_size, strength, 0.0],
	}
}

impl RenderPass for LutPass {
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
}

/// Reads the baked LUT payload into owned worker-side bytes.
async fn load_lut_bytes(reference: &mut Reference<Lut>) -> Result<StdBox<[u8]>, String> {
	let read_target = ReadTargetsMut::Box {
		buffer: vec![0_u8; reference.size].into_boxed_slice(),
		offset: 0,
		size: None,
	};
	let read_result = reference.load(read_target).await.map_err(|_| {
		"LUT resource data could not be read. The most likely cause is that the cached payload is missing or unreadable."
			.to_string()
	})?;

	Ok(match read_result {
		resource_management::resource::ReadTargets::Box(bytes) => bytes,
		resource_management::resource::ReadTargets::Buffer(bytes) => bytes.into(),
		resource_management::resource::ReadTargets::Backing(backing) => backing.as_slice().into(),
		resource_management::resource::ReadTargets::Streams(_) => {
			return Err(
				"LUT resource has an unsupported stream layout. The most likely cause is that it was stored as streams instead of a flat payload."
					.to_string(),
			);
		}
	})
}

/// Converts the baked LUT RGB float payload directly into an RGBA16F 3D texture upload target.
fn write_lut_bytes_to_rgba16f_upload_target(lut: &Lut, lut_bytes: &[u8], upload_target: &mut [u8]) {
	assert!(
		matches!(lut.kind, LutKind::ThreeDimensional),
		"Unsupported LUT kind for upload. The most likely cause is that a non-3D LUT resource reached the LUT render pass."
	);

	let expected_size = expected_lut_payload_size(lut);

	assert_eq!(
		lut_bytes.len(),
		expected_size,
		"Invalid LUT payload size. The most likely cause is that the baked LUT binary does not match the LUT metadata."
	);

	let texel_count = lut
		.kind
		.expected_entry_count(lut.size)
		.expect("Invalid LUT dimensions. The most likely cause is that the LUT size overflowed during texture upload.");
	let expected_upload_size = texel_count * 4 * std::mem::size_of::<u16>();

	assert_eq!(
		upload_target.len(),
		expected_upload_size,
		"Unexpected LUT texture upload size. The most likely cause is that the GPU image extent or format does not match the LUT resource metadata."
	);

	// The resource stores tightly packed RGB f32 texels, while the GPU texture expects RGBA16F texels.
	for (rgb, rgba16f) in lut_bytes.as_chunks::<{ 3 * std::mem::size_of::<f32>() }>().0.iter().zip(
		upload_target
			.as_chunks_mut::<{ 4 * std::mem::size_of::<u16>() }>()
			.0
			.iter_mut(),
	) {
		let r = f32::from_le_bytes(rgb[0..4].try_into().unwrap());
		let g = f32::from_le_bytes(rgb[4..8].try_into().unwrap());
		let b = f32::from_le_bytes(rgb[8..12].try_into().unwrap());

		rgba16f[0..2].copy_from_slice(&f16::from_f32(r).to_bits().to_le_bytes());
		rgba16f[2..4].copy_from_slice(&f16::from_f32(g).to_bits().to_le_bytes());
		rgba16f[4..6].copy_from_slice(&f16::from_f32(b).to_bits().to_le_bytes());
		rgba16f[6..8].copy_from_slice(&f16::from_f32(1.0).to_bits().to_le_bytes());
	}
}

fn expected_lut_payload_size(lut: &Lut) -> usize {
	lut.kind
		.expected_entry_count(lut.size)
		.and_then(|entry_count| entry_count.checked_mul(3 * std::mem::size_of::<f32>()))
		.expect("Invalid LUT payload size calculation. The most likely cause is that the LUT dimensions overflowed.")
}

/// The `PreparedLut` struct keeps asynchronously loaded LUT metadata and bytes independent from GPU placement.
#[derive(Clone)]
pub struct PreparedLut {
	metadata: Lut,
	bytes: StdBox<[u8]>,
}

impl PreparedLut {
	/// Loads one LUT completely on an application-owned asynchronous task.
	///
	/// Pass the result to [`LutPass::new`] or a LUT setup function on the render
	/// thread. GPU image creation remains owned by the selected render pass.
	pub async fn load(resource_manager: &resource_management::ResourceManager, id: &str) -> Result<Self, String> {
		let mut reference: Reference<Lut> = resource_manager
			.request(id)
			.await
			.map_err(|error| format!("Could not load LUT '{id}'. {error}"))?;
		let metadata = reference.resource().clone();
		let bytes = load_lut_bytes(&mut reference).await?;
		Ok(Self { metadata, bytes })
	}
}

#[cfg(test)]
mod tests {
	use besl::vm::{DescriptorBindings, ResourceSlot, Texture, Value, output_slot};
	use resource_management::resources::lut::{Lut, LutKind};

	use super::lut_shader_parameters;
	use crate::rendering::render_pass::simple_compute;
	use crate::rendering::shader_vm_test::{PIXEL_TILE, assert_rgba_close, buffer, empty_image, rgba, run_pixel, texture_2d};

	static LUT_SHADER: std::sync::LazyLock<&str> =
		std::sync::LazyLock::new(|| crate::rendering::shader_vm_test::asset_source("lut/apply.besl"));

	static ACES_SHADER: std::sync::LazyLock<&str> =
		std::sync::LazyLock::new(|| crate::rendering::shader_vm_test::asset_source("color-grading/aces.besl"));
	static ACES2_SHADER: std::sync::LazyLock<&str> =
		std::sync::LazyLock::new(|| crate::rendering::shader_vm_test::asset_source("color-grading/aces2.besl"));
	static DWG_SHADER: std::sync::LazyLock<&str> =
		std::sync::LazyLock::new(|| crate::rendering::shader_vm_test::asset_source("color-grading/dwg.besl"));

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
			("sampling", [0.5, 0.25, 1.0, 0.0]),
		] {
			parameters
				.write(name, Value::Vec4F(value))
				.expect("Expected color-grading parameters to match the shader");
		}
		let mut descriptors = DescriptorBindings::new();
		descriptors.bind_texture(ResourceSlot::new(0), &mut source);
		descriptors.bind_texture(ResourceSlot::new(1), &mut lut);
		descriptors.bind_image(output_slot(0), &mut result);
		descriptors.bind_buffer(parameter_slot, &mut parameters);
		run_pixel(&program, &mut descriptors, [0, 0], PIXEL_TILE, [1, 1]);
		drop(descriptors);
		rgba(&result, [0, 0])
	}

	#[test]
	fn grading_workflows_preserve_neutral_middle_gray_through_their_sdr_transforms() {
		assert_rgba_close(
			run_workflow(*ACES_SHADER, [0.18, 0.18, 0.18, 0.4]),
			[0.3584574, 0.3584574, 0.3584574, 0.4],
			4e-4,
		);
		assert_rgba_close(
			run_workflow(*DWG_SHADER, [0.18, 0.18, 0.18, 0.4]),
			[0.45925015, 0.45925015, 0.45925015, 0.4],
			4e-4,
		);
	}

	#[test]
	fn grading_workflows_bound_black_and_hdr_values_for_sdr_output() {
		for shader in [*ACES_SHADER, *ACES2_SHADER, *DWG_SHADER] {
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

	/// Checks the ACES 2.0 output transform against the Academy reference, through an identity look.
	///
	/// The expected colors come from `scripts/generate-aces2-tables.py --vectors`, a port of the reference
	/// implementation, which agrees with OpenColorIO 2.6's ACES 2.0 view within 0.001. They cover mid grey at 10 nits,
	/// black, highlights past white, and saturated colors that the chroma and gamut compression reshape.
	#[test]
	fn aces2_grading_matches_the_reference_output_transform() {
		for (input, expected) in [
			([0.18, 0.18, 0.18], [0.3491891, 0.3491891, 0.3491891]),
			([0.0, 0.0, 0.0], [0.0, 0.0, 0.0]),
			([1.0, 1.0, 1.0], [0.7066839, 0.7066839, 0.7066839]),
			([16.0, 16.0, 16.0], [0.9747303, 0.9747303, 0.9747303]),
			([0.5, 0.1, 0.05], [0.5848386, 0.2504861, 0.1614661]),
			([0.05, 0.4, 0.1], [0.1915109, 0.5202122, 0.2732095]),
			([0.02, 0.05, 0.6], [0.0505988, 0.1406792, 0.6100106]),
			([4.0, 0.5, 0.1], [1.0, 0.619354, 0.4811281]),
			([0.3, 0.25, 0.9], [0.4576994, 0.4243927, 0.7209163]),
			([0.9, 0.9, 0.05], [0.6972113, 0.7018813, 0.3009407]),
		] {
			let output = run_workflow(*ACES2_SHADER, [input[0], input[1], input[2], 0.5]);
			assert_rgba_close(output, [expected[0], expected[1], expected[2], 0.5], 2e-4);
		}
	}

	/// Verifies identity trilinear interpolation, domain clamping, and alpha preservation through the VM.
	#[test]
	fn lut_besl_vm_trilinearly_applies_identity_lut_and_domain_clamping() {
		let lut = Lut {
			kind: LutKind::ThreeDimensional,
			size: 2,
			domain_min: [0.0, 0.0, 0.0],
			domain_max: [1.0, 1.0, 1.0],
		};
		let program = crate::rendering::shader_vm_test::compile(simple_compute::compile_test_program(*LUT_SHADER));
		let parameter_slot = ResourceSlot::new(3);
		let shader_parameters = lut_shader_parameters(&lut, 1.0);
		let mut parameters = buffer(&program, parameter_slot);
		for (name, value) in [
			("domain_min", shader_parameters.domain_min),
			("domain_scale", shader_parameters.domain_scale),
			("sampling", shader_parameters.sampling),
		] {
			parameters
				.write(name, Value::Vec4F(value))
				.expect("Failed to initialize LUT parameters. The most likely cause is a changed canonical buffer layout.");
		}
		let mut source = texture_2d(3, 1, &[[0.5, 0.5, 0.5, 0.4], [-1.0, 0.25, 0.75, 0.2], [2.0, 0.75, -2.0, 0.8]]);
		let mut identity_lut = Texture::new_3d(2, 2, 2)
			.expect("Failed to create a VM 3D texture. The most likely cause is an invalid LUT fixture extent.");
		// Each corner stores its normalized coordinate, so interpolation must reproduce any in-domain input color.
		for z in 0..2 {
			for y in 0..2 {
				for x in 0..2 {
					identity_lut
						.write_3d([x, y, z], [x as f32, y as f32, z as f32, 1.0])
						.expect("Failed to initialize the VM LUT. The most likely cause is an invalid fixture coordinate.");
				}
			}
		}
		let mut result = empty_image(3, 1);

		for x in 0..3 {
			let mut descriptors = DescriptorBindings::new();
			descriptors.bind_texture(ResourceSlot::new(0), &mut source);
			descriptors.bind_texture(ResourceSlot::new(1), &mut identity_lut);
			descriptors.bind_image(output_slot(0), &mut result);
			descriptors.bind_buffer(parameter_slot, &mut parameters);
			run_pixel(&program, &mut descriptors, [x, 0], PIXEL_TILE, [3, 1]);
		}

		assert_rgba_close(rgba(&result, [0, 0]), [0.5, 0.5, 0.5, 0.4], 1e-6);
		assert_rgba_close(rgba(&result, [1, 0]), [0.0, 0.25, 0.75, 0.2], 1e-6);
		assert_rgba_close(rgba(&result, [2, 0]), [1.0, 0.75, 0.0, 0.8], 1e-6);
	}

	/// Verifies that the look strength mixes the input and the looked-up color, through an inverting LUT.
	#[test]
	fn lut_besl_vm_mixes_the_look_by_its_strength() {
		let lut = Lut {
			kind: LutKind::ThreeDimensional,
			size: 2,
			domain_min: [0.0, 0.0, 0.0],
			domain_max: [1.0, 1.0, 1.0],
		};
		let program = crate::rendering::shader_vm_test::compile(simple_compute::compile_test_program(*LUT_SHADER));
		let parameter_slot = ResourceSlot::new(3);
		// Each corner stores one minus its coordinate, so the full look maps 0.25 to 0.75.
		let mut inverting_lut = Texture::new_3d(2, 2, 2).expect("Expected a valid test LUT extent");
		for z in 0..2 {
			for y in 0..2 {
				for x in 0..2 {
					inverting_lut
						.write_3d([x, y, z], [1.0 - x as f32, 1.0 - y as f32, 1.0 - z as f32, 1.0])
						.expect("Expected a valid test LUT coordinate");
				}
			}
		}

		for (strength, expected) in [(0.0, 0.25), (0.25, 0.375), (1.0, 0.75)] {
			let shader_parameters = lut_shader_parameters(&lut, strength);
			let mut parameters = buffer(&program, parameter_slot);
			for (name, value) in [
				("domain_min", shader_parameters.domain_min),
				("domain_scale", shader_parameters.domain_scale),
				("sampling", shader_parameters.sampling),
			] {
				parameters
					.write(name, Value::Vec4F(value))
					.expect("Expected LUT parameters to match the shader");
			}
			let mut source = texture_2d(1, 1, &[[0.25, 0.25, 0.25, 0.5]]);
			let mut result = empty_image(1, 1);
			let mut descriptors = DescriptorBindings::new();
			descriptors.bind_texture(ResourceSlot::new(0), &mut source);
			descriptors.bind_texture(ResourceSlot::new(1), &mut inverting_lut);
			descriptors.bind_image(output_slot(0), &mut result);
			descriptors.bind_buffer(parameter_slot, &mut parameters);
			run_pixel(&program, &mut descriptors, [0, 0], PIXEL_TILE, [1, 1]);
			drop(descriptors);

			assert_rgba_close(rgba(&result, [0, 0]), [expected, expected, expected, 0.5], 1e-6);
		}
	}

	#[test]
	fn lut_besl_reflects_3d_texture_and_parameter_bindings() {
		let program = simple_compute::compile_test_program(*LUT_SHADER);
		let main_node = program.get_main().expect("Canonical LUT shader should define main");
		let bindings = resource_management::shader::besl::evaluation::ProgramEvaluation::from_main(&main_node)
			.expect("Failed to evaluate the LUT descriptor schema")
			.into_bindings();
		let lut_texture = bindings
			.iter()
			.find(|binding| binding.name == "lut_texture")
			.expect("Canonical LUT shader should retain its 3D texture binding");

		assert!(matches!(
			lut_texture.kind,
			resource_management::shader::besl::evaluation::BindingKind::CombinedImageSampler {
				view: resource_management::shader::besl::evaluation::TextureView::Texture3D,
				sampler: None,
			}
		));
		let parameters = bindings
			.iter()
			.find(|binding| binding.name == "parameters")
			.unwrap_or_else(|| panic!("Canonical LUT shader should retain its parameter buffer: {bindings:?}"));

		assert!(parameters.read && !parameters.write);
		assert_eq!(
			parameters.kind,
			resource_management::shader::besl::evaluation::BindingKind::StorageBuffer
		);
	}
}

use std::boxed::Box as StdBox;

use ghi::{
	context::{Context as _, ContextCreate as _},
	frame::Frame as _,
};
use half::f16;
use resource_management::{
	Reference,
	resource::ReadTargetsMut,
	resources::lut::{Lut, LutKind},
};
use utils::Extent;

use crate::{
	core::Entity,
	rendering::{
		Sink,
		render_pass::{RenderPass, RenderPassBuilder, RenderPassReturn, simple_tile},
	},
};
