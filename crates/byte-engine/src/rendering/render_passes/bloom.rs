use ghi::{
	command_buffer::{BoundComputePipelineMode as _, BoundPipelineLayoutMode as _, CommonCommandBufferMode as _},
	context::{Context as _, ContextCreate as _},
	frame::Frame as _,
};
use resource_management::{
	resources::material, shader::generator::ShaderGenerationSettings, types::ShaderTypes as ResourceShaderTypes,
};
use utils::{Box, Extent};

use crate::{
	core::Entity,
	rendering::{
		render_pass::{RenderPass, RenderPassBuilder, RenderPassReturn},
		Sink,
	},
};

const EXTRACT_SOURCE_BINDING: ghi::DescriptorSetBindingTemplate = ghi::DescriptorSetBindingTemplate::new(
	0,
	ghi::descriptors::DescriptorType::CombinedImageSampler,
	ghi::Stages::COMPUTE,
);
const EXTRACT_OUTPUT_BINDING: ghi::DescriptorSetBindingTemplate =
	ghi::DescriptorSetBindingTemplate::new(1, ghi::descriptors::DescriptorType::StorageImage, ghi::Stages::COMPUTE);
const EXTRACT_PARAMETERS_BINDING: ghi::DescriptorSetBindingTemplate =
	ghi::DescriptorSetBindingTemplate::new(2, ghi::descriptors::DescriptorType::StorageBuffer, ghi::Stages::COMPUTE);

const UPSAMPLE_LOW_BINDING: ghi::DescriptorSetBindingTemplate = ghi::DescriptorSetBindingTemplate::new(
	0,
	ghi::descriptors::DescriptorType::CombinedImageSampler,
	ghi::Stages::COMPUTE,
);
const UPSAMPLE_HIGH_BINDING: ghi::DescriptorSetBindingTemplate = ghi::DescriptorSetBindingTemplate::new(
	1,
	ghi::descriptors::DescriptorType::CombinedImageSampler,
	ghi::Stages::COMPUTE,
);
const UPSAMPLE_OUTPUT_BINDING: ghi::DescriptorSetBindingTemplate =
	ghi::DescriptorSetBindingTemplate::new(2, ghi::descriptors::DescriptorType::StorageImage, ghi::Stages::COMPUTE);
const UPSAMPLE_PARAMETERS_BINDING: ghi::DescriptorSetBindingTemplate =
	ghi::DescriptorSetBindingTemplate::new(3, ghi::descriptors::DescriptorType::StorageBuffer, ghi::Stages::COMPUTE);

const COMPOSITE_SCENE_BINDING: ghi::DescriptorSetBindingTemplate = ghi::DescriptorSetBindingTemplate::new(
	0,
	ghi::descriptors::DescriptorType::CombinedImageSampler,
	ghi::Stages::COMPUTE,
);
const COMPOSITE_BLOOM_BINDING: ghi::DescriptorSetBindingTemplate = ghi::DescriptorSetBindingTemplate::new(
	1,
	ghi::descriptors::DescriptorType::CombinedImageSampler,
	ghi::Stages::COMPUTE,
);
const COMPOSITE_OUTPUT_BINDING: ghi::DescriptorSetBindingTemplate =
	ghi::DescriptorSetBindingTemplate::new(2, ghi::descriptors::DescriptorType::StorageImage, ghi::Stages::COMPUTE);
const COMPOSITE_PARAMETERS_BINDING: ghi::DescriptorSetBindingTemplate =
	ghi::DescriptorSetBindingTemplate::new(3, ghi::descriptors::DescriptorType::StorageBuffer, ghi::Stages::COMPUTE);

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
	extract_pipeline: ghi::PipelineHandle,
	downsample_pipeline: ghi::PipelineHandle,
	upsample_pipeline: ghi::PipelineHandle,
	composite_pipeline: ghi::PipelineHandle,
	extract_descriptor_set: ghi::DescriptorSetHandle,
	downsample_descriptor_sets: Vec<ghi::DescriptorSetHandle>,
	upsample_descriptor_sets: Vec<ghi::DescriptorSetHandle>,
	composite_descriptor_set: ghi::DescriptorSetHandle,
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

		let shader_storage = render_pass_builder.shader_storage();
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

		let extract_descriptor_set_layout = context.create_descriptor_set_template(
			Some("Bloom Extract Descriptor Set Layout"),
			&[EXTRACT_SOURCE_BINDING, EXTRACT_OUTPUT_BINDING, EXTRACT_PARAMETERS_BINDING],
		);
		let upsample_descriptor_set_layout = context.create_descriptor_set_template(
			Some("Bloom Upsample Descriptor Set Layout"),
			&[
				UPSAMPLE_LOW_BINDING,
				UPSAMPLE_HIGH_BINDING,
				UPSAMPLE_OUTPUT_BINDING,
				UPSAMPLE_PARAMETERS_BINDING,
			],
		);
		let composite_descriptor_set_layout = context.create_descriptor_set_template(
			Some("Bloom Composite Descriptor Set Layout"),
			&[
				COMPOSITE_SCENE_BINDING,
				COMPOSITE_BLOOM_BINDING,
				COMPOSITE_OUTPUT_BINDING,
				COMPOSITE_PARAMETERS_BINDING,
			],
		);

		let extract_pipeline = create_extract_pipeline(context, shader_storage, extract_descriptor_set_layout);
		let downsample_pipeline = create_downsample_pipeline(context, shader_storage, extract_descriptor_set_layout);
		let upsample_pipeline = create_upsample_pipeline(context, shader_storage, upsample_descriptor_set_layout);
		let composite_pipeline = create_composite_pipeline(context, shader_storage, composite_descriptor_set_layout);

		let extract_descriptor_set =
			context.create_descriptor_set(Some("Bloom Extract Descriptor Set"), &extract_descriptor_set_layout);
		let _ = context.create_descriptor_binding(
			extract_descriptor_set,
			ghi::BindingConstructor::combined_image_sampler(&EXTRACT_SOURCE_BINDING, source, sampler, ghi::Layouts::Read),
		);
		let _ = context.create_descriptor_binding(
			extract_descriptor_set,
			ghi::BindingConstructor::image(&EXTRACT_OUTPUT_BINDING, downsample_images[0]),
		);
		let _ = context.create_descriptor_binding(
			extract_descriptor_set,
			ghi::BindingConstructor::buffer(&EXTRACT_PARAMETERS_BINDING, parameters.into()),
		);

		let downsample_descriptor_sets = (1..level_count)
			.map(|index| {
				let descriptor_set =
					context.create_descriptor_set(Some("Bloom Downsample Descriptor Set"), &extract_descriptor_set_layout);
				let _ = context.create_descriptor_binding(
					descriptor_set,
					ghi::BindingConstructor::combined_image_sampler(
						&EXTRACT_SOURCE_BINDING,
						downsample_images[index - 1],
						sampler,
						ghi::Layouts::Read,
					),
				);
				let _ = context.create_descriptor_binding(
					descriptor_set,
					ghi::BindingConstructor::image(&EXTRACT_OUTPUT_BINDING, downsample_images[index]),
				);
				let _ = context.create_descriptor_binding(
					descriptor_set,
					ghi::BindingConstructor::buffer(&EXTRACT_PARAMETERS_BINDING, parameters.into()),
				);
				descriptor_set
			})
			.collect::<Vec<_>>();

		let upsample_descriptor_sets = (0..level_count.saturating_sub(1))
			.rev()
			.map(|level| {
				let descriptor_set =
					context.create_descriptor_set(Some("Bloom Upsample Descriptor Set"), &upsample_descriptor_set_layout);
				let low_resolution_source: ghi::BaseImageHandle = if level == level_count - 2 {
					downsample_images[level + 1].into()
				} else {
					upsample_images[level + 1].into()
				};
				let _ = context.create_descriptor_binding(
					descriptor_set,
					ghi::BindingConstructor::combined_image_sampler(
						&UPSAMPLE_LOW_BINDING,
						low_resolution_source,
						sampler,
						ghi::Layouts::Read,
					),
				);
				let _ = context.create_descriptor_binding(
					descriptor_set,
					ghi::BindingConstructor::combined_image_sampler(
						&UPSAMPLE_HIGH_BINDING,
						downsample_images[level],
						sampler,
						ghi::Layouts::Read,
					),
				);
				let _ = context.create_descriptor_binding(
					descriptor_set,
					ghi::BindingConstructor::image(&UPSAMPLE_OUTPUT_BINDING, upsample_images[level]),
				);
				let _ = context.create_descriptor_binding(
					descriptor_set,
					ghi::BindingConstructor::buffer(&UPSAMPLE_PARAMETERS_BINDING, parameters.into()),
				);
				descriptor_set
			})
			.collect::<Vec<_>>();

		let bloom_source: ghi::BaseImageHandle = if level_count == 1 {
			downsample_images[0].into()
		} else {
			upsample_images[0].into()
		};
		let composite_descriptor_set =
			context.create_descriptor_set(Some("Bloom Composite Descriptor Set"), &composite_descriptor_set_layout);
		let _ = context.create_descriptor_binding(
			composite_descriptor_set,
			ghi::BindingConstructor::combined_image_sampler(
				&COMPOSITE_SCENE_BINDING,
				bloom_source,
				sampler,
				ghi::Layouts::Read,
			),
		);
		let _ = context.create_descriptor_binding(
			composite_descriptor_set,
			ghi::BindingConstructor::combined_image_sampler(
				&COMPOSITE_BLOOM_BINDING,
				bloom_source,
				sampler,
				ghi::Layouts::Read,
			),
		);
		let _ = context.create_descriptor_binding(
			composite_descriptor_set,
			ghi::BindingConstructor::image(&COMPOSITE_OUTPUT_BINDING, output),
		);
		let _ = context.create_descriptor_binding(
			composite_descriptor_set,
			ghi::BindingConstructor::buffer(&COMPOSITE_PARAMETERS_BINDING, parameters.into()),
		);

		Self {
			settings,
			parameters,
			extract_pipeline,
			downsample_pipeline,
			upsample_pipeline,
			composite_pipeline,
			extract_descriptor_set,
			downsample_descriptor_sets,
			upsample_descriptor_sets,
			composite_descriptor_set,
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

		let extract_pipeline = self.extract_pipeline;
		let downsample_pipeline = self.downsample_pipeline;
		let upsample_pipeline = self.upsample_pipeline;
		let composite_pipeline = self.composite_pipeline;
		let extract_descriptor_set = self.extract_descriptor_set;
		let downsample_descriptor_sets = self.downsample_descriptor_sets.clone();
		let upsample_descriptor_sets = self.upsample_descriptor_sets.clone();
		let composite_descriptor_set = self.composite_descriptor_set;
		let level_count = self.downsample_images.len();

		Some(crate::rendering::render_pass::allocate_render_command(
			frame_allocator,
			move |command_buffer, _| {
				command_buffer.region(
					|label| label.write_str("Bloom"),
					|command_buffer| {
						if bloom_enabled {
							let extract = command_buffer.bind_compute_pipeline(extract_pipeline);
							extract.bind_descriptor_sets(&[extract_descriptor_set]);
							extract.dispatch(ghi::DispatchExtent::new(bloom_extent(extent, 0), bloom_dispatch_extent()));

							for (index, descriptor_set) in downsample_descriptor_sets.iter().enumerate() {
								let downsample = command_buffer.bind_compute_pipeline(downsample_pipeline);
								downsample.bind_descriptor_sets(&[*descriptor_set]);
								downsample.dispatch(ghi::DispatchExtent::new(
									bloom_extent(extent, index + 1),
									bloom_dispatch_extent(),
								));
							}

							if level_count > 1 {
								for (level, descriptor_set) in (0..level_count - 1).rev().zip(upsample_descriptor_sets.iter()) {
									let upsample = command_buffer.bind_compute_pipeline(upsample_pipeline);
									upsample.bind_descriptor_sets(&[*descriptor_set]);
									upsample.dispatch(ghi::DispatchExtent::new(
										bloom_extent(extent, level),
										bloom_dispatch_extent(),
									));
								}
							}
						}

						let composite = command_buffer.bind_compute_pipeline(composite_pipeline);
						composite.bind_descriptor_sets(&[composite_descriptor_set]);
						composite.dispatch(ghi::DispatchExtent::new(extent, bloom_dispatch_extent()));
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

fn create_extract_pipeline(
	context: &mut ghi::implementation::Context,
	shader_storage: Option<&dyn resource_management::resource::StorageBackend>,
	descriptor_set_layout: ghi::DescriptorSetTemplateHandle,
) -> ghi::PipelineHandle {
	let shader = crate::rendering::shader_store::create_shader(
		context,
		shader_storage,
		&besl_bloom_shader_descriptor(
			"byte-engine/rendering/bloom/extract",
			"Bloom Extract Shader",
			BLOOM_EXTRACT_BESL,
			2,
			vec![
				material::Binding::new(0, 0, true, false),
				material::Binding::new(0, 1, false, true),
				material::Binding::new(0, 2, true, false),
			],
		),
	)
	.expect("Failed to create bloom extract shader. The most likely cause is an incompatible bloom extract shader interface.");

	context.create_compute_pipeline(ghi::pipelines::compute::Builder::new(
		&[descriptor_set_layout],
		&[],
		ghi::ShaderParameter::new(&shader, ghi::ShaderTypes::Compute),
	))
}

fn create_downsample_pipeline(
	context: &mut ghi::implementation::Context,
	shader_storage: Option<&dyn resource_management::resource::StorageBackend>,
	descriptor_set_layout: ghi::DescriptorSetTemplateHandle,
) -> ghi::PipelineHandle {
	let shader = crate::rendering::shader_store::create_shader(
		context,
		shader_storage,
		&besl_bloom_shader_descriptor(
			"byte-engine/rendering/bloom/downsample",
			"Bloom Downsample Shader",
			BLOOM_DOWNSAMPLE_BESL,
			2,
			vec![
				material::Binding::new(0, 0, true, false),
				material::Binding::new(0, 1, false, true),
				material::Binding::new(0, 2, true, false),
			],
		),
	)
	.expect(
		"Failed to create bloom downsample shader. The most likely cause is an incompatible bloom downsample shader interface.",
	);

	context.create_compute_pipeline(ghi::pipelines::compute::Builder::new(
		&[descriptor_set_layout],
		&[],
		ghi::ShaderParameter::new(&shader, ghi::ShaderTypes::Compute),
	))
}

fn create_upsample_pipeline(
	context: &mut ghi::implementation::Context,
	shader_storage: Option<&dyn resource_management::resource::StorageBackend>,
	descriptor_set_layout: ghi::DescriptorSetTemplateHandle,
) -> ghi::PipelineHandle {
	let shader = crate::rendering::shader_store::create_shader(
		context,
		shader_storage,
		&besl_bloom_shader_descriptor(
			"byte-engine/rendering/bloom/upsample",
			"Bloom Upsample Shader",
			BLOOM_UPSAMPLE_BESL,
			3,
			vec![
				material::Binding::new(0, 0, true, false),
				material::Binding::new(0, 1, true, false),
				material::Binding::new(0, 2, false, true),
				material::Binding::new(0, 3, true, false),
			],
		),
	)
	.expect(
		"Failed to create bloom upsample shader. The most likely cause is an incompatible bloom upsample shader interface.",
	);

	context.create_compute_pipeline(ghi::pipelines::compute::Builder::new(
		&[descriptor_set_layout],
		&[],
		ghi::ShaderParameter::new(&shader, ghi::ShaderTypes::Compute),
	))
}

fn create_composite_pipeline(
	context: &mut ghi::implementation::Context,
	shader_storage: Option<&dyn resource_management::resource::StorageBackend>,
	descriptor_set_layout: ghi::DescriptorSetTemplateHandle,
) -> ghi::PipelineHandle {
	let shader = crate::rendering::shader_store::create_shader(
		context,
		shader_storage,
		&besl_bloom_shader_descriptor(
			"byte-engine/rendering/bloom/composite",
			"Bloom Composite Shader",
			BLOOM_COMPOSITE_BESL,
			3,
			vec![
				material::Binding::new(0, 0, true, false),
				material::Binding::new(0, 1, true, false),
				material::Binding::new(0, 2, false, true),
				material::Binding::new(0, 3, true, false),
			],
		),
	)
	.expect(
		"Failed to create bloom composite shader. The most likely cause is an incompatible bloom composite shader interface.",
	);

	context.create_compute_pipeline(ghi::pipelines::compute::Builder::new(
		&[descriptor_set_layout],
		&[],
		ghi::ShaderParameter::new(&shader, ghi::ShaderTypes::Compute),
	))
}

fn besl_bloom_shader_descriptor<'a>(
	id: &'a str,
	name: &'a str,
	source: &'a str,
	parameters_binding: u32,
	bindings: Vec<material::Binding>,
) -> crate::rendering::shader_store::ShaderSourceDescriptor<'a> {
	let main_node = build_bloom_program(source, parameters_binding);

	crate::rendering::shader_store::ShaderSourceDescriptor {
		id,
		name,
		stage: ResourceShaderTypes::Compute,
		source: crate::rendering::shader_store::ShaderSourceDefinition::Besl {
			settings: ShaderGenerationSettings::compute(Extent::new(8, 8, 1)).name(name.to_string()),
			main_node,
		},
		interface: material::ShaderInterface {
			workgroup_size: Some((8, 8, 1)),
			bindings,
		},
	}
}

fn build_bloom_program(source: &str, parameters_binding: u32) -> besl::NodeReference {
	let mut root = besl::Node::root();

	let vec4f = root.get_child("vec4f").expect("vec4f type not found in BESL root");

	// Bloom shaders share one test/program builder, so expose the superset of
	// texture bindings used by extract, downsample, upsample, and composite.
	for (name, binding) in [
		("source_texture", 0),
		("low_resolution_texture", 0),
		("scene_texture", 0),
		("high_resolution_texture", 1),
		("bloom_texture", 1),
	] {
		root.add_child(
			besl::Node::binding(
				name,
				besl::BindingTypes::CombinedImageSampler { format: String::new() },
				0,
				binding,
				true,
				false,
			)
			.into(),
		);
	}
	root.add_child(
		besl::Node::binding(
			"result_texture",
			besl::BindingTypes::Image {
				format: "rgba16".to_string(),
			},
			0,
			parameters_binding - 1,
			false,
			true,
		)
		.into(),
	);

	root.add_child(
		besl::Node::binding(
			"bloom_parameters",
			besl::BindingTypes::Buffer {
				members: vec![
					besl::Node::array("prefilter", vec4f.clone(), 1),
					besl::Node::array("blur_data", vec4f, 1),
				],
			},
			0,
			parameters_binding,
			true,
			false,
		)
		.into(),
	);

	let program = besl::compile_to_besl(source, Some(root))
		.expect("Failed to compile bloom BESL shader. The most likely cause is invalid BESL syntax.");
	program.get_main().expect(
		"Failed to find the bloom BESL entry point. The most likely cause is that the BESL program did not define main.",
	)
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
	use super::*;

	#[cfg(target_os = "linux")]
	#[test]
	fn bloom_extract_besl_compiles_to_spirv() {
		let main_node = build_bloom_program(BLOOM_EXTRACT_BESL, 2);
		resource_management::shader::besl::backends::spirv::SPIRVShaderGenerator::new()
			.generate(
				&ShaderGenerationSettings::compute(Extent::new(8, 8, 1)).name("Bloom Extract Test".to_string()),
				&main_node,
			)
			.expect("Failed to compile bloom extract BESL to SPIR-V.");
	}

	#[cfg(target_os = "linux")]
	#[test]
	fn bloom_downsample_besl_compiles_to_spirv() {
		let main_node = build_bloom_program(BLOOM_DOWNSAMPLE_BESL, 2);
		resource_management::shader::besl::backends::spirv::SPIRVShaderGenerator::new()
			.generate(
				&ShaderGenerationSettings::compute(Extent::new(8, 8, 1)).name("Bloom Downsample Test".to_string()),
				&main_node,
			)
			.expect("Failed to compile bloom downsample BESL to SPIR-V.");
	}

	#[cfg(target_os = "linux")]
	#[test]
	fn bloom_upsample_besl_compiles_to_spirv() {
		let main_node = build_bloom_program(BLOOM_UPSAMPLE_BESL, 3);
		resource_management::shader::besl::backends::spirv::SPIRVShaderGenerator::new()
			.generate(
				&ShaderGenerationSettings::compute(Extent::new(8, 8, 1)).name("Bloom Upsample Test".to_string()),
				&main_node,
			)
			.expect("Failed to compile bloom upsample BESL to SPIR-V.");
	}

	#[cfg(target_os = "linux")]
	#[test]
	fn bloom_composite_besl_compiles_to_spirv() {
		let main_node = build_bloom_program(BLOOM_COMPOSITE_BESL, 3);
		resource_management::shader::besl::backends::spirv::SPIRVShaderGenerator::new()
			.generate(
				&ShaderGenerationSettings::compute(Extent::new(8, 8, 1)).name("Bloom Composite Test".to_string()),
				&main_node,
			)
			.expect("Failed to compile bloom composite BESL to SPIR-V.");
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
