/// The `LutRenderPass` struct applies a baked 3D LUT to the current `main` render target.
pub struct LutRenderPass {
	pass: simple_compute::Pass,
	_parameters: ghi::BufferHandle<LutShaderParameters>,
	lut: Lut,
	lut_reference: Option<Reference<Lut>>,
	lut_image: ghi::ImageHandle,
	lut_uploaded: bool,
}

#[repr(C)]
#[derive(Clone, Copy)]
struct LutShaderParameters {
	domain_min: [f32; 4],
	domain_scale: [f32; 4],
	sampling: [f32; 4],
}

impl Entity for LutRenderPass {}

impl LutRenderPass {
	/// Creates a LUT grading pass from an injected LUT reference.
	pub fn new(render_pass_builder: &mut RenderPassBuilder, lut: Reference<Lut>) -> Self {
		Self::with_settings(render_pass_builder, LutRenderPassSettings { lut })
	}

	/// Creates a LUT grading pass with caller-supplied settings.
	pub fn with_settings(render_pass_builder: &mut RenderPassBuilder, settings: LutRenderPassSettings) -> Self {
		let LutRenderPassSettings { lut } = settings;
		let lut_metadata = lut.resource().clone();

		assert!(
			matches!(lut_metadata.kind, LutKind::ThreeDimensional),
			"Unsupported LUT kind for LUT render pass. The most likely cause is that the injected LUT resource is not a 3D LUT."
		);

		let source = render_pass_builder.read_from("main");
		let main_format = render_pass_builder.format_of("main");
		let output = render_pass_builder.create_render_target(
			ghi::image::Builder::new(main_format, ghi::Uses::Storage | ghi::Uses::Image).name("LUT Output"),
		);
		render_pass_builder.alias("LUT Output", "main");

		let pipeline = simple_compute::Pipeline::compile(
			render_pass_builder,
			simple_compute::Descriptor::new(
				"LUT",
				"byte-engine/rendering/lut/apply.besl",
				"LUT Render Pass Compute Shader",
			),
		)
		.expect("Failed to create LUT render shader. The most likely cause is an incompatible shader interface.");

		let context = render_pass_builder.context();

		let source_sampler = context.build_sampler(
			ghi::sampler::Builder::new()
				.filtering_mode(ghi::FilteringModes::Linear)
				.mip_map_mode(ghi::FilteringModes::Linear)
				.addressing_mode(ghi::SamplerAddressingModes::Clamp),
		);
		let lut_sampler = context.build_sampler(
			ghi::sampler::Builder::new()
				.filtering_mode(ghi::FilteringModes::Linear)
				.mip_map_mode(ghi::FilteringModes::Linear)
				.addressing_mode(ghi::SamplerAddressingModes::Clamp),
		);
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
		*context.get_mut_buffer_slice(parameters) = lut_shader_parameters(&lut_metadata);

		let pass = pipeline
			.bind(
				render_pass_builder,
				"LUT Render Pass Descriptor Set",
				&[
					simple_compute::Resource::combined_image_sampler(
						"source_texture",
						source,
						source_sampler,
						ghi::Layouts::Read,
					),
					simple_compute::Resource::combined_image_sampler("lut_texture", lut_image, lut_sampler, ghi::Layouts::Read),
					simple_compute::Resource::image("result_texture", output),
					simple_compute::Resource::buffer("parameters", parameters),
				],
			)
			.expect("Failed to bind LUT render resources. The most likely cause is that the BESL bindings changed.");

		Self {
			pass,
			_parameters: parameters,
			lut: lut_metadata,
			lut_reference: Some(lut),
			lut_image,
			lut_uploaded: false,
		}
	}

	/// Uploads the baked LUT payload into the cached GPU 3D texture the first time the pass is used.
	fn ensure_lut_uploaded(&mut self, frame: &mut ghi::implementation::Frame) {
		if self.lut_uploaded {
			return;
		}

		let lut_reference = self.lut_reference.as_mut().expect(
			"LUT reference missing during LUT upload. The most likely cause is that the LUT render pass lost its source resource before the first frame.",
		);
		let lut_bytes = load_lut_bytes(lut_reference);
		let target = frame.get_texture_slice_mut(self.lut_image.into());

		write_lut_bytes_to_rgba16f_upload_target(&self.lut, &lut_bytes, target);
		frame.sync_texture(self.lut_image.into());

		self.lut_uploaded = true;
		self.lut_reference = None;
	}
}

fn lut_shader_parameters(lut: &Lut) -> LutShaderParameters {
	let domain_scale: [f32; 3] = std::array::from_fn(|index| 1.0 / (lut.domain_max[index] - lut.domain_min[index]));
	let lut_size = lut.size as f32;
	LutShaderParameters {
		domain_min: [lut.domain_min[0], lut.domain_min[1], lut.domain_min[2], 0.0],
		domain_scale: [domain_scale[0], domain_scale[1], domain_scale[2], 0.0],
		sampling: [(lut_size - 1.0) / lut_size, 0.5 / lut_size, 0.0, 0.0],
	}
}

impl RenderPass for LutRenderPass {
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

/// Reads the baked LUT payload from the resource reference into owned bytes.
fn load_lut_bytes(reference: &mut Reference<Lut>) -> StdBox<[u8]> {
	let read_target = ReadTargetsMut::Box {
		buffer: vec![0_u8; reference.size].into_boxed_slice(),
		offset: 0,
		size: None,
	};
	let read_result = reference.load(read_target).expect(
		"Failed to read LUT resource data. The most likely cause is that the cached LUT payload is missing or unreadable.",
	);

	match read_result {
		resource_management::resource::ReadTargets::Box(bytes) => bytes,
		resource_management::resource::ReadTargets::Buffer(bytes) => bytes.into(),
		resource_management::resource::ReadTargets::Backing(backing) => backing.as_slice().into(),
		resource_management::resource::ReadTargets::Streams(_) => {
			panic!(
				"Unexpected LUT stream layout. The most likely cause is that the LUT resource was stored as streams instead of a flat payload."
			);
		}
	}
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
	for (rgb, rgba16f) in lut_bytes
		.chunks_exact(3 * std::mem::size_of::<f32>())
		.zip(upload_target.chunks_exact_mut(4 * std::mem::size_of::<u16>()))
	{
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

/// The `LutRenderPassSettings` struct carries the LUT resource used by the LUT grading pass.
pub struct LutRenderPassSettings {
	pub lut: Reference<Lut>,
}

#[cfg(test)]
mod tests {
	use besl::vm::{DescriptorBindings, ResourceSlot, Texture, Value};
	use half::f16;
	use resource_management::resources::lut::{Lut, LutKind};

	use super::{expected_lut_payload_size, lut_shader_parameters, write_lut_bytes_to_rgba16f_upload_target};
	use crate::rendering::render_pass::simple_compute;
	use crate::rendering::shader_vm_test::{assert_rgba_close, buffer, empty_image, rgba, run_at, texture_2d};

	const LUT_SHADER: &str = include_str!("../../../assets/rendering/lut/apply.besl");

	/// Verifies identity trilinear interpolation, domain clamping, and alpha preservation through the VM.
	#[test]
	fn lut_besl_vm_trilinearly_applies_identity_lut_and_domain_clamping() {
		let lut = Lut {
			kind: LutKind::ThreeDimensional,
			size: 2,
			domain_min: [0.0, 0.0, 0.0],
			domain_max: [1.0, 1.0, 1.0],
		};
		let program = crate::rendering::shader_vm_test::compile(simple_compute::compile_test_program(LUT_SHADER));
		let parameter_slot = ResourceSlot::new(3);
		let shader_parameters = lut_shader_parameters(&lut);
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
			descriptors.bind_image(ResourceSlot::new(2), &mut result);
			descriptors.bind_buffer(parameter_slot, &mut parameters);
			run_at(&program, &mut descriptors, [x, 0]);
		}

		assert_rgba_close(rgba(&result, [0, 0]), [0.5, 0.5, 0.5, 0.4], 1e-6);
		assert_rgba_close(rgba(&result, [1, 0]), [0.0, 0.25, 0.75, 0.2], 1e-6);
		assert_rgba_close(rgba(&result, [2, 0]), [1.0, 0.75, 0.0, 0.8], 1e-6);
	}

	#[test]
	fn lut_besl_reflects_3d_texture_and_parameter_bindings() {
		let main_node = simple_compute::compile_test_program(LUT_SHADER);
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
				view: resource_management::shader::besl::evaluation::TextureView::Texture3D
			}
		));
		let parameters = bindings
			.iter()
			.find(|binding| binding.name == "parameters")
			.unwrap_or_else(|| panic!("Canonical LUT shader should retain its parameter buffer: {bindings:?}"));
		assert_eq!(parameters.slot, 3);
		assert!(parameters.read && !parameters.write);
		assert_eq!(
			parameters.kind,
			resource_management::shader::besl::evaluation::BindingKind::StorageBuffer
		);
	}

	#[test]
	fn converts_rgb_float_payload_into_rgba16f_upload() {
		let lut = Lut {
			kind: LutKind::ThreeDimensional,
			size: 2,
			domain_min: [0.0, 0.0, 0.0],
			domain_max: [1.0, 1.0, 1.0],
		};
		let mut lut_bytes = Vec::new();

		for [r, g, b] in [
			[0.0_f32, 0.0, 0.0],
			[1.0, 0.0, 0.0],
			[0.0, 1.0, 0.0],
			[1.0, 1.0, 0.0],
			[0.0, 0.0, 1.0],
			[1.0, 0.0, 1.0],
			[0.0, 1.0, 1.0],
			[1.0, 1.0, 1.0],
		] {
			lut_bytes.extend_from_slice(&r.to_le_bytes());
			lut_bytes.extend_from_slice(&g.to_le_bytes());
			lut_bytes.extend_from_slice(&b.to_le_bytes());
		}

		let mut upload = [0_u8; 8 * 4 * std::mem::size_of::<u16>()];

		write_lut_bytes_to_rgba16f_upload_target(&lut, &lut_bytes, &mut upload);

		assert_eq!(
			u16::from_le_bytes(upload[0..2].try_into().unwrap()),
			f16::from_f32(0.0).to_bits()
		);
		assert_eq!(
			u16::from_le_bytes(upload[6..8].try_into().unwrap()),
			f16::from_f32(1.0).to_bits()
		);
		assert_eq!(
			u16::from_le_bytes(upload[upload.len() - 8..upload.len() - 6].try_into().unwrap()),
			f16::from_f32(1.0).to_bits()
		);
		assert_eq!(
			u16::from_le_bytes(upload[upload.len() - 2..].try_into().unwrap()),
			f16::from_f32(1.0).to_bits()
		);
	}

	#[test]
	#[should_panic(expected = "Invalid LUT payload size")]
	fn rejects_invalid_lut_payload_size() {
		let lut = Lut {
			kind: LutKind::ThreeDimensional,
			size: 2,
			domain_min: [0.0, 0.0, 0.0],
			domain_max: [1.0, 1.0, 1.0],
		};

		let mut upload = [0_u8; 8 * 4 * std::mem::size_of::<u16>()];

		write_lut_bytes_to_rgba16f_upload_target(&lut, &[0_u8; 8], &mut upload);
	}

	#[test]
	fn computes_expected_lut_payload_size() {
		let lut = Lut {
			kind: LutKind::ThreeDimensional,
			size: 4,
			domain_min: [0.0, 0.0, 0.0],
			domain_max: [1.0, 1.0, 1.0],
		};

		assert_eq!(
			expected_lut_payload_size(&lut),
			4_usize.pow(3) * 3 * std::mem::size_of::<f32>()
		);
	}
}

use std::boxed::Box as StdBox;

use ghi::{context::Context as _, frame::Frame as _, types::Size as _};
use half::f16;
use resource_management::{
	resource::ReadTargetsMut,
	resources::lut::{Lut, LutKind},
	Reference,
};
use utils::{Box, Extent};

use crate::{
	core::Entity,
	rendering::{
		render_pass::{simple_compute, RenderPass, RenderPassBuilder, RenderPassReturn},
		Sink,
	},
};
