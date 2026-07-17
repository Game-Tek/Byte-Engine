use ghi::{
	command_buffer::CommonCommandBufferMode as _,
	context::{Context as _, ContextCreate as _},
	frame::Frame as _,
};
use utils::{Box, Extent};

use crate::{
	core::Entity,
	rendering::{
		render_pass::{simple_compute, RenderPass, RenderPassBuilder, RenderPassReturn},
		Sink,
	},
};

const MAX_BLOOM_LEVELS: u32 = 6;

/// The `BloomPassSettings` struct defines the intent and shaping controls for a reusable HDR bloom stage.
#[derive(Clone, Copy, Debug)]
pub struct BloomPassSettings {
	pub enabled: bool,
	pub threshold: f32,
	pub soft_knee: f32,
	pub intensity: f32,
	pub radius: f32,
	pub levels: u32,
}

impl Default for BloomPassSettings {
	fn default() -> Self {
		Self {
			enabled: true,
			threshold: 1.0,
			soft_knee: 0.5,
			intensity: 0.08,
			radius: 1.0,
			levels: 5,
		}
	}
}

impl BloomPassSettings {
	fn resolved_level_count(self) -> usize {
		self.levels.clamp(1, MAX_BLOOM_LEVELS) as usize
	}
}

#[repr(C)]
#[derive(Clone, Copy)]
struct BloomShaderData {
	prefilter: [f32; 4],
	filter: [f32; 4],
}

/// The `BloomPass` struct creates a reusable pre-tonemap glow stage that can feed later post-processing.
pub struct BloomPass {
	settings: BloomPassSettings,
	parameters: ghi::DynamicBufferHandle<BloomShaderData>,
	extract_pass: simple_compute::Pass,
	downsample_passes: Vec<simple_compute::Pass>,
	upsample_passes: Vec<simple_compute::Pass>,
	composite_pass: simple_compute::Pass,
	downsample_images: Vec<ghi::DynamicImageHandle>,
	upsample_images: Vec<ghi::DynamicImageHandle>,
}

impl Entity for BloomPass {}

impl BloomPass {
	/// Creates a bloom pass with the default glow shaping parameters.
	pub fn new(render_pass_builder: &mut RenderPassBuilder) -> Self {
		Self::with_settings(render_pass_builder, BloomPassSettings::default())
	}

	/// Creates a bloom pass with caller-supplied settings and remaps `main` for downstream passes.
	pub fn with_settings(render_pass_builder: &mut RenderPassBuilder, settings: BloomPassSettings) -> Self {
		let source = render_pass_builder.read_from("main");
		let main_format = render_pass_builder.format_of("main");
		let output = render_pass_builder.create_render_target(
			ghi::image::Builder::new(main_format, ghi::Uses::Storage | ghi::Uses::Image).name("Bloom Output"),
		);
		render_pass_builder.alias("Bloom Output", "main");

		let context = render_pass_builder.context();
		let level_count = settings.resolved_level_count();
		let downsample_images = (0..level_count)
			.map(|index| {
				context.build_dynamic_image(
					ghi::image::Builder::new(main_format, ghi::Uses::Storage | ghi::Uses::Image)
						.name(match index {
							0 => "Bloom Downsample 0",
							1 => "Bloom Downsample 1",
							2 => "Bloom Downsample 2",
							3 => "Bloom Downsample 3",
							4 => "Bloom Downsample 4",
							_ => "Bloom Downsample 5",
						})
						.device_accesses(ghi::DeviceAccesses::DeviceOnly),
				)
			})
			.collect::<Vec<_>>();
		let upsample_images = (0..level_count.saturating_sub(1))
			.map(|index| {
				context.build_dynamic_image(
					ghi::image::Builder::new(main_format, ghi::Uses::Storage | ghi::Uses::Image)
						.name(match index {
							0 => "Bloom Upsample 0",
							1 => "Bloom Upsample 1",
							2 => "Bloom Upsample 2",
							3 => "Bloom Upsample 3",
							_ => "Bloom Upsample 4",
						})
						.device_accesses(ghi::DeviceAccesses::DeviceOnly),
				)
			})
			.collect::<Vec<_>>();

		let parameters = context.build_dynamic_buffer(
			ghi::buffer::Builder::new(ghi::Uses::Storage)
				.name("Bloom Parameters")
				.device_accesses(ghi::DeviceAccesses::HostToDevice),
		);
		let sampler = context.build_sampler(
			ghi::sampler::Builder::new()
				.filtering_mode(ghi::FilteringModes::Linear)
				.mip_map_mode(ghi::FilteringModes::Linear)
				.addressing_mode(ghi::SamplerAddressingModes::Clamp),
		);

		let extract_pipeline = simple_compute::Pipeline::compile(
			render_pass_builder,
			bloom_pipeline_descriptor(
				"Bloom Extract",
				"byte-engine/rendering/bloom/extract",
				"Bloom Extract Shader",
				"Bloom Extract Descriptor Set Layout",
				BLOOM_EXTRACT_BESL,
				2,
			),
		)
		.expect(
			"Failed to create bloom extract shader. The most likely cause is an incompatible bloom extract shader interface.",
		);
		let downsample_pipeline = extract_pipeline
			.compile_variant(
				render_pass_builder,
				bloom_pipeline_descriptor(
					"Bloom Downsample",
					"byte-engine/rendering/bloom/downsample",
					"Bloom Downsample Shader",
					"Bloom Extract Descriptor Set Layout",
					BLOOM_DOWNSAMPLE_BESL,
					2,
				),
			)
			.expect(
				"Failed to create bloom downsample shader. The most likely cause is an incompatible bloom downsample shader interface.",
			);
		let upsample_pipeline = simple_compute::Pipeline::compile(
			render_pass_builder,
			bloom_pipeline_descriptor(
				"Bloom Upsample",
				"byte-engine/rendering/bloom/upsample",
				"Bloom Upsample Shader",
				"Bloom Upsample Descriptor Set Layout",
				BLOOM_UPSAMPLE_BESL,
				3,
			),
		)
		.expect(
			"Failed to create bloom upsample shader. The most likely cause is an incompatible bloom upsample shader interface.",
		);
		let composite_pipeline = simple_compute::Pipeline::compile(
			render_pass_builder,
			bloom_pipeline_descriptor(
				"Bloom Composite",
				"byte-engine/rendering/bloom/composite",
				"Bloom Composite Shader",
				"Bloom Composite Descriptor Set Layout",
				BLOOM_COMPOSITE_BESL,
				3,
			),
		)
		.expect(
			"Failed to create bloom composite shader. The most likely cause is an incompatible bloom composite shader interface.",
		);

		let extract_pass = extract_pipeline
			.bind(
				render_pass_builder,
				"Bloom Extract Descriptor Set",
				&[
					simple_compute::Resource::combined_image_sampler("source_texture", source, sampler, ghi::Layouts::Read),
					simple_compute::Resource::image("result_texture", downsample_images[0]),
					simple_compute::Resource::buffer("bloom_parameters", parameters),
				],
			)
			.expect("Failed to bind bloom extract resources. The most likely cause is a changed BESL binding contract.");

		let downsample_passes = (1..level_count)
			.map(|index| {
				downsample_pipeline
					.bind(
						render_pass_builder,
						"Bloom Downsample Descriptor Set",
						&[
							simple_compute::Resource::combined_image_sampler(
								"source_texture",
								downsample_images[index - 1],
								sampler,
								ghi::Layouts::Read,
							),
							simple_compute::Resource::image("result_texture", downsample_images[index]),
							simple_compute::Resource::buffer("bloom_parameters", parameters),
						],
					)
					.expect(
						"Failed to bind bloom downsample resources. The most likely cause is a changed BESL binding contract.",
					)
			})
			.collect::<Vec<_>>();

		let upsample_passes =
			(0..level_count.saturating_sub(1))
				.rev()
				.map(|level| {
					let low_resolution_source: ghi::BaseImageHandle = if level == level_count - 2 {
						downsample_images[level + 1].into()
					} else {
						upsample_images[level + 1].into()
					};
					upsample_pipeline
					.bind(
						render_pass_builder,
						"Bloom Upsample Descriptor Set",
						&[
							simple_compute::Resource::combined_image_sampler(
								"low_resolution_texture",
								low_resolution_source,
								sampler,
								ghi::Layouts::Read,
							),
							simple_compute::Resource::combined_image_sampler(
								"high_resolution_texture",
								downsample_images[level],
								sampler,
								ghi::Layouts::Read,
							),
							simple_compute::Resource::image("result_texture", upsample_images[level]),
							// Keep the radius buffer ready; named extras become active as soon as BESL references the binding.
							simple_compute::Resource::planned_buffer("bloom_parameters", parameters),
						],
					)
					.expect("Failed to bind bloom upsample resources. The most likely cause is a changed BESL binding contract.")
				})
				.collect::<Vec<_>>();

		let bloom_source: ghi::BaseImageHandle = if level_count == 1 {
			downsample_images[0].into()
		} else {
			upsample_images[0].into()
		};
		let composite_pass = composite_pipeline
			.bind(
				render_pass_builder,
				"Bloom Composite Descriptor Set",
				&[
					simple_compute::Resource::combined_image_sampler(
						"scene_texture",
						bloom_source,
						sampler,
						ghi::Layouts::Read,
					),
					simple_compute::Resource::combined_image_sampler(
						"bloom_texture",
						bloom_source,
						sampler,
						ghi::Layouts::Read,
					),
					simple_compute::Resource::image("result_texture", output),
					simple_compute::Resource::buffer("bloom_parameters", parameters),
				],
			)
			.expect("Failed to bind bloom composite resources. The most likely cause is a changed BESL binding contract.");

		Self {
			settings,
			parameters,
			extract_pass,
			downsample_passes,
			upsample_passes,
			composite_pass,
			downsample_images,
			upsample_images,
		}
	}

	/// Writes the static bloom controls into the per-frame parameter buffer before dispatch.
	fn write_parameters(&self, frame: &mut ghi::implementation::Frame, intensity_multiplier: f32) {
		let parameters = frame.get_mut_dynamic_buffer_slice(self.parameters);

		parameters.prefilter = [
			self.settings.threshold.max(0.0),
			self.settings.soft_knee.clamp(0.0, 1.0),
			self.settings.intensity.max(0.0) * intensity_multiplier,
			0.0,
		];
		parameters.filter = [self.settings.radius.max(0.5), 0.0, 0.0, 0.0];
	}

	/// Resizes every bloom pyramid image to match the current sink-dependent chain resolution.
	fn resize_images(&self, frame: &mut ghi::implementation::Frame, extent: Extent) {
		for (level, image) in self.downsample_images.iter().enumerate() {
			frame.resize_image((*image).into(), bloom_extent(extent, level));
		}

		for (level, image) in self.upsample_images.iter().enumerate() {
			frame.resize_image((*image).into(), bloom_extent(extent, level));
		}
	}
}

impl RenderPass for BloomPass {
	fn prepare<'a>(
		&mut self,
		frame: &mut ghi::implementation::Frame,
		sink: &Sink,
		frame_allocator: &'a bumpalo::Bump,
	) -> Option<RenderPassReturn<'a>> {
		let extent = sink.extent();
		let bloom_enabled = self.settings.enabled;

		self.resize_images(frame, extent);
		self.write_parameters(frame, if bloom_enabled { 1.0 } else { 0.0 });

		let extract_pass = self.extract_pass;
		let downsample_passes = frame_allocator.alloc_slice_copy(&self.downsample_passes);
		let upsample_passes = frame_allocator.alloc_slice_copy(&self.upsample_passes);
		let composite_pass = self.composite_pass;
		let level_count = self.downsample_images.len();

		Some(crate::rendering::render_pass::allocate_render_command(
			frame_allocator,
			move |command_buffer, _| {
				command_buffer.region(
					|label| label.write_str("Bloom"),
					|command_buffer| {
						if bloom_enabled {
							extract_pass.record(command_buffer, bloom_extent(extent, 0));

							for (index, pass) in downsample_passes.iter().enumerate() {
								pass.record(command_buffer, bloom_extent(extent, index + 1));
							}

							if level_count > 1 {
								for (level, pass) in (0..level_count - 1).rev().zip(upsample_passes.iter()) {
									pass.record(command_buffer, bloom_extent(extent, level));
								}
							}
						}

						composite_pass.record(command_buffer, extent);
					},
				);
			},
		))
	}
}

fn bloom_dispatch_extent() -> Extent {
	Extent::new(8, 8, 1)
}

fn bloom_extent(extent: Extent, level: usize) -> Extent {
	let divisor = 1u32 << (level as u32 + 1);
	Extent::rectangle(
		extent.width().div_ceil(divisor).max(1),
		extent.height().div_ceil(divisor).max(1),
	)
}

fn bloom_pipeline_descriptor<'a>(
	label: &'static str,
	id: &'a str,
	name: &'a str,
	layout_name: &'a str,
	source: &str,
	parameters_binding: u32,
) -> simple_compute::Descriptor<'a> {
	simple_compute::Descriptor::new(
		label,
		id,
		name,
		build_bloom_program(source, parameters_binding),
		bloom_dispatch_extent(),
	)
	.layout_name(layout_name)
}

fn build_bloom_program(source: &str, parameters_binding: u32) -> besl::NodeReference {
	let mut program = simple_compute::Program::new();
	let vec4f = program.type_node("vec4f").expect("vec4f type not found in BESL root");

	// Bloom shaders share one test/program builder, so expose the superset of
	// texture bindings used by extract, downsample, upsample, and composite.
	for (name, binding) in [
		("source_texture", 0),
		("low_resolution_texture", 0),
		("scene_texture", 0),
		("high_resolution_texture", 1),
		("bloom_texture", 1),
	] {
		program.binding(
			name,
			besl::BindingTypes::CombinedImageSampler { format: String::new() },
			binding,
			true,
			false,
		);
	}
	program.binding(
		"result_texture",
		besl::BindingTypes::Image {
			format: "rgba16".to_string(),
		},
		parameters_binding - 1,
		false,
		true,
	);
	program.binding(
		"bloom_parameters",
		besl::BindingTypes::Buffer {
			members: vec![
				besl::Node::array("prefilter", vec4f.clone(), 1),
				besl::Node::array("blur_data", vec4f, 1),
			],
		},
		parameters_binding,
		true,
		false,
	);
	program
		.compile(source)
		.expect("Failed to compile bloom BESL shader. The most likely cause is invalid BESL syntax.")
}

const BLOOM_EXTRACT_BESL: &str = r#"
main: fn() -> void {
	let coord: vec2u = thread_id();
	guard_image_bounds(result_texture, coord);
	let source_size: vec2u = texture_size(source_texture);
	let uv: vec2f = (vec2f(f32(coord.x), f32(coord.y)) + vec2f(0.5, 0.5)) / vec2f(f32(source_size.x), f32(source_size.y));
	let sampled: vec4f = texture_lod(source_texture, uv);
	let brightness: f32 = max(max(sampled.x, sampled.y), sampled.z);
	let threshold: f32 = bloom_parameters.prefilter[0].x;
	let soft_knee: f32 = bloom_parameters.prefilter[0].y;
	let knee: f32 = max(threshold * soft_knee, 0.00001);
	let soft: f32 = clamp(brightness - threshold + knee, 0.0, 2.0 * knee);
	soft = (soft * soft) / (4.0 * knee + 0.00001);
	let contribution: f32 = max(soft, brightness - threshold);
	contribution = contribution / max(brightness, 0.00001);
	let bloom_color: vec3f = vec3f(sampled.x * contribution, sampled.y * contribution, sampled.z * contribution);
	write(result_texture, coord, vec4f(bloom_color.x, bloom_color.y, bloom_color.z, 1.0));
}
"#;

const BLOOM_DOWNSAMPLE_BESL: &str = r#"
main: fn() -> void {
	let coord: vec2u = thread_id();
	guard_image_bounds(result_texture, coord);
	let result_size: vec2u = image_size(result_texture);
	let source_size: vec2u = texture_size(source_texture);
	let uv: vec2f = (vec2f(f32(coord.x), f32(coord.y)) + vec2f(0.5, 0.5)) / vec2f(f32(result_size.x), f32(result_size.y));
	let center: vec4f = texture_lod(source_texture, uv);
	write(result_texture, coord, vec4f(center.x, center.y, center.z, 1.0));
}
"#;

const BLOOM_UPSAMPLE_BESL: &str = r#"
main: fn() -> void {
	let coord: vec2u = thread_id();
	guard_image_bounds(result_texture, coord);
	let result_size: vec2u = image_size(result_texture);
	let low_size: vec2u = texture_size(low_resolution_texture);
	let uv: vec2f = (vec2f(f32(coord.x), f32(coord.y)) + vec2f(0.5, 0.5)) / vec2f(f32(result_size.x), f32(result_size.y));
	let low_res: vec4f = texture_lod(low_resolution_texture, uv);
	let high_res: vec4f = texture_lod(high_resolution_texture, uv);
	let combined: vec3f = vec3f(high_res.x, high_res.y, high_res.z) + vec3f(low_res.x, low_res.y, low_res.z);
	write(result_texture, coord, vec4f(combined.x, combined.y, combined.z, 1.0));
}
"#;

const BLOOM_COMPOSITE_BESL: &str = r#"
main: fn() -> void {
	let coord: vec2u = thread_id();
	guard_image_bounds(result_texture, coord);
	let result_size: vec2u = image_size(result_texture);
	let uv: vec2f = (vec2f(f32(coord.x), f32(coord.y)) + vec2f(0.5, 0.5)) / vec2f(f32(result_size.x), f32(result_size.y));
	let scene: vec4f = texture_lod(scene_texture, uv);
	let intensity: f32 = bloom_parameters.prefilter[0].z;
	if (intensity <= 0.0) {
		write(result_texture, coord, scene);
		return;
	}
	let bloom: vec4f = texture_lod(bloom_texture, uv);
	let final_color: vec3f = vec3f(scene.x, scene.y, scene.z) + vec3f(bloom.x, bloom.y, bloom.z) * intensity;
	write(result_texture, coord, vec4f(final_color.x, final_color.y, final_color.z, 1.0));
}
"#;

#[cfg(test)]
mod tests {
	use besl::vm::{DescriptorBindings, ResourceSlot, Value};

	use super::*;
	use crate::rendering::shader_vm_test::{assert_rgba_close, buffer, empty_image, rgba, run_at, texture_2d};

	/// Verifies threshold rejection and soft-knee extraction through the production bloom program.
	#[test]
	fn bloom_extract_besl_vm_applies_threshold_and_soft_knee() {
		let program = crate::rendering::shader_vm_test::compile(build_bloom_program(BLOOM_EXTRACT_BESL, 2));
		let parameter_slot = ResourceSlot::new(2);
		let mut parameters = buffer(&program, parameter_slot);
		parameters
			.write_indexed("prefilter", 0, Value::Vec4F([1.0, 0.5, 0.0, 0.0]))
			.expect("Failed to initialize bloom parameters. The most likely cause is a changed production buffer layout.");

		for (source_color, expected) in [
			([0.25, 0.2, 0.1, 0.25], [0.0, 0.0, 0.0, 1.0]),
			([2.0, 1.0, 0.5, 0.25], [1.0, 0.5, 0.25, 1.0]),
		] {
			let mut source = texture_2d(1, 1, &[source_color]);
			let mut result = empty_image(1, 1);
			let mut descriptors = DescriptorBindings::new();
			descriptors.bind_texture(ResourceSlot::new(0), &mut source);
			descriptors.bind_image(ResourceSlot::new(1), &mut result);
			descriptors.bind_buffer(parameter_slot, &mut parameters);
			run_at(&program, &mut descriptors, [0, 0]);
			drop(descriptors);

			assert_rgba_close(rgba(&result, [0, 0]), expected, 1e-5);
		}
	}

	/// Verifies that downsampling reads the bilinear center of the source texture.
	#[test]
	fn bloom_downsample_besl_vm_samples_the_source_center() {
		let program = crate::rendering::shader_vm_test::compile(build_bloom_program(BLOOM_DOWNSAMPLE_BESL, 2));
		let mut source = texture_2d(
			2,
			2,
			&[
				[0.0, 0.0, 0.0, 0.0],
				[1.0, 0.0, 0.0, 0.0],
				[0.0, 1.0, 0.0, 0.0],
				[0.0, 0.0, 1.0, 0.0],
			],
		);
		let mut result = empty_image(1, 1);
		let mut descriptors = DescriptorBindings::new();
		descriptors.bind_texture(ResourceSlot::new(0), &mut source);
		descriptors.bind_image(ResourceSlot::new(1), &mut result);
		run_at(&program, &mut descriptors, [0, 0]);
		drop(descriptors);

		assert_rgba_close(rgba(&result, [0, 0]), [0.25, 0.25, 0.25, 1.0], 1e-6);
	}

	/// Verifies that upsampling combines both production pyramid inputs.
	#[test]
	fn bloom_upsample_besl_vm_combines_both_levels() {
		let program = crate::rendering::shader_vm_test::compile(build_bloom_program(BLOOM_UPSAMPLE_BESL, 3));
		let mut low = texture_2d(1, 1, &[[0.1, 0.2, 0.3, 0.0]]);
		let mut high = texture_2d(1, 1, &[[0.4, 0.5, 0.6, 0.0]]);
		let mut result = empty_image(1, 1);
		let mut descriptors = DescriptorBindings::new();
		descriptors.bind_texture(ResourceSlot::new(0), &mut low);
		descriptors.bind_texture(ResourceSlot::new(1), &mut high);
		descriptors.bind_image(ResourceSlot::new(2), &mut result);
		run_at(&program, &mut descriptors, [0, 0]);
		drop(descriptors);

		assert_rgba_close(rgba(&result, [0, 0]), [0.5, 0.7, 0.9, 1.0], 1e-6);
	}

	/// Verifies additive bloom and the zero-intensity passthrough branch.
	#[test]
	fn bloom_composite_besl_vm_preserves_zero_intensity_and_adds_positive_bloom() {
		let program = crate::rendering::shader_vm_test::compile(build_bloom_program(BLOOM_COMPOSITE_BESL, 3));
		let parameter_slot = ResourceSlot::new(3);
		let scene_color = [0.2, 0.3, 0.4, 0.6];
		let bloom_color = [0.5, 0.25, 0.125, 0.0];

		for (intensity, expected) in [(0.0, scene_color), (2.0, [1.2, 0.8, 0.65, 1.0])] {
			let mut scene = texture_2d(1, 1, &[scene_color]);
			let mut bloom = texture_2d(1, 1, &[bloom_color]);
			let mut result = empty_image(1, 1);
			let mut parameters = buffer(&program, parameter_slot);
			parameters
				.write_indexed("prefilter", 0, Value::Vec4F([0.0, 0.0, intensity, 0.0]))
				.expect("Failed to initialize bloom parameters. The most likely cause is a changed production buffer layout.");
			let mut descriptors = DescriptorBindings::new();
			descriptors.bind_texture(ResourceSlot::new(0), &mut scene);
			descriptors.bind_texture(ResourceSlot::new(1), &mut bloom);
			descriptors.bind_image(ResourceSlot::new(2), &mut result);
			descriptors.bind_buffer(parameter_slot, &mut parameters);
			run_at(&program, &mut descriptors, [0, 0]);
			drop(descriptors);

			assert_rgba_close(rgba(&result, [0, 0]), expected, 1e-6);
		}
	}

	#[test]
	fn bloom_level_count_is_clamped() {
		let settings = BloomPassSettings {
			levels: MAX_BLOOM_LEVELS + 4,
			..Default::default()
		};

		assert_eq!(settings.resolved_level_count(), MAX_BLOOM_LEVELS as usize);
	}

	#[test]
	fn bloom_extent_stays_non_zero() {
		let extent = bloom_extent(Extent::rectangle(1, 1), 4);

		assert_eq!(extent.width(), 1);
		assert_eq!(extent.height(), 1);
	}
}
