use crate::core::{entity::{EntityHandle, EntityBuilder}, listener::{EntitySubscriber, Listener}, Entity};

use ghi::{BoundComputePipelineMode, CommandBufferRecordable, GraphicsHardwareInterface};
use resource_management::{asset::material_asset_handler::ProgramGenerator, glsl_shader_generator::GLSLShaderGenerator, shader_generator::{ShaderGenerationSettings, ShaderGenerator}};
use utils::{json, Extent};

use crate::Vector3;

use super::{common_shader_generator::CommonShaderGenerator, directional_light::DirectionalLight, render_pass::RenderPass, world_render_domain::WorldRenderDomain};

pub struct BackgroundRenderingPass {
	pipeline: ghi::PipelineHandle,
	pipeline_layout: ghi::PipelineLayoutHandle,
	descriptor_set: ghi::DescriptorSetHandle,
	buffer: ghi::BaseBufferHandle,

	sun_direction: Vector3,
}

impl BackgroundRenderingPass {
	pub fn new<'c>(ghi: &mut ghi::GHI, views_buffer: ghi::BaseBufferHandle, depth_target: ghi::ImageHandle, out_target: ghi::ImageHandle) -> EntityBuilder<'c, Self> {
		let views_buffer_binding_template = ghi::DescriptorSetBindingTemplate::new(0, ghi::DescriptorType::StorageBuffer, ghi::Stages::COMPUTE);
		let depth_map_binding_template = ghi::DescriptorSetBindingTemplate::new(1, ghi::DescriptorType::CombinedImageSampler, ghi::Stages::COMPUTE);
		let out_map_binding_template = ghi::DescriptorSetBindingTemplate::new(2, ghi::DescriptorType::StorageImage, ghi::Stages::COMPUTE);
		let light_binding_template = ghi::DescriptorSetBindingTemplate::new(3, ghi::DescriptorType::StorageBuffer, ghi::Stages::COMPUTE);

		let descriptor_set_template = ghi.create_descriptor_set_template(Some("Sky Rendering Set Layout"), &[views_buffer_binding_template.clone(), depth_map_binding_template.clone(), out_map_binding_template.clone(), light_binding_template.clone(),]);

		let pipeline_layout = ghi.create_pipeline_layout(&[descriptor_set_template], &[]);

		let descriptor_set = ghi.create_descriptor_set(Some("Sky Rendering Descriptor Set"), &descriptor_set_template);

		let buffer = ghi.create_buffer(Some("Sky Rendering Buffer"), 3 * 4, ghi::Uses::Storage, ghi::DeviceAccesses::CpuWrite | ghi::DeviceAccesses::GpuRead, ghi::UseCases::DYNAMIC);

		let sampler = ghi.build_sampler(ghi::sampler::Builder::new().addressing_mode(ghi::SamplerAddressingModes::Border {}));
		let views_buffer_binding = ghi.create_descriptor_binding(descriptor_set, ghi::BindingConstructor::buffer(&views_buffer_binding_template, views_buffer));
		let depth_map_binding = ghi.create_descriptor_binding(descriptor_set, ghi::BindingConstructor::combined_image_sampler(&depth_map_binding_template, depth_target, sampler, ghi::Layouts::Read));
		let out_map_binding = ghi.create_descriptor_binding(descriptor_set, ghi::BindingConstructor::image(&out_map_binding_template, out_target, ghi::Layouts::General));
		let light_binding = ghi.create_descriptor_binding(descriptor_set, ghi::BindingConstructor::buffer(&light_binding_template, buffer));

		let shader = ghi.create_shader(Some("Sky Rendering Shader"), ghi::ShaderSource::GLSL(Self::make_shader()), ghi::ShaderTypes::Compute, &[views_buffer_binding_template.into_shader_binding_descriptor(0, ghi::AccessPolicies::READ,), depth_map_binding_template.into_shader_binding_descriptor(0, ghi::AccessPolicies::READ), out_map_binding_template.into_shader_binding_descriptor(0, ghi::AccessPolicies::WRITE), light_binding_template.into_shader_binding_descriptor(0, ghi::AccessPolicies::READ),]).unwrap();

		let pipeline = ghi.create_compute_pipeline(&pipeline_layout, ghi::ShaderParameter::new(&shader, ghi::ShaderTypes::Compute));

		EntityBuilder::new(Self {
			pipeline,
			pipeline_layout,
			descriptor_set,
			buffer,

			sun_direction: Vector3::new(0.0, -1.0, 0.0),
		}).listen_to::<DirectionalLight>()
	}

	fn make_shader() -> String {
		let shader_generator = CommonShaderGenerator::new();

		let main_code = r#"
		ivec2 pixel_coordinates = ivec2(gl_GlobalInvocationID.xy);
		vec2 uv = make_uv(pixel_coordinates, uvec2(imageSize(out_target)));

		float depth = texture(depth_target, uv).r;

		if (depth != 0.0) { return; }

		vec3 sunPosition = -light.direction * 400000;
		float rayleigh = 2.295;
		vec3 primaries = vec3(6.8e-7, 5.5e-7, 4.5e-7);
		float refractive_index = 1.0003;
		float depolarization_factor = 0.095;
		float num_molecules = 2.547e25;
		vec3 mieKCoefficient = vec3(0.686, 0.678, 0.666);
		float turbidity = 2.5;
		float mieCoefficient = 0.011475;
		float mieV = 4.0;
		float rayleighZenithLength = 540;
		float mieZenithLength = 1.25e3;
		float mieDirectionalG = 0.814;
		float sunIntensityFactor = 1151.0;
		float sunIntensityFalloffSteepness = 1.22;
		float sunAngularDiameterDegrees = 0.00639;

		float sunfade = 1.0 - clamp(1.0 - exp((sunPosition.y / 450000.0)), 0.0, 1.0);
		vec3 betaR = total_rayleigh(primaries, refractive_index, depolarization_factor, num_molecules) * (rayleigh - (1.0 * (1.0 - sunfade)));
		
		vec3 betaM = total_mie(primaries, mieKCoefficient, turbidity, mieV) * mieCoefficient;
		
		View view = views.views[0];
		vec2 nc = uv * 2.0 - 1.0;
		nc.y = -nc.y;
		vec3 viewDir = normalize(view.inverse_view * vec4(nc, 1.0, 0.0)).xyz;
		
		float zenithAngle = acos(max(0.0, dot(vec3(0, 1, 0), viewDir)));
		float denom = cos(zenithAngle) + 0.15 * pow(93.885 - ((zenithAngle * 180.0) / PI), -1.253);
		float sR = rayleighZenithLength / denom;
		float sM = mieZenithLength / denom;
		
		vec3 Fex = exp(-(betaR * sR + betaM * sM));
		
		vec3 sunDirection = normalize(sunPosition);
		float cosTheta = dot(viewDir, sunDirection);
		vec3 betaRTheta = betaR * rayleigh_phase(cosTheta * 0.5 + 0.5);
		vec3 betaMTheta = betaM * henyey_greenstein_phase(cosTheta, mieDirectionalG);
		float sunE = sunIntensityFactor * max(0.0, 1.0 - exp(-(((PI / 1.95) - acos(dot(sunDirection, vec3(0, 1, 0)))) / sunIntensityFalloffSteepness)));
		vec3 Lin = pow(sunE * ((betaRTheta + betaMTheta) / (betaR + betaM)) * (1.0 - Fex), vec3(1.5));
		Lin *= mix(vec3(1.0), pow(sunE * ((betaRTheta + betaMTheta) / (betaR + betaM)) * Fex, vec3(0.5)), clamp(pow(1.0 - dot(vec3(0, 1, 0), sunDirection), 5.0), 0.0, 1.0));
		
		float sunAngularDiameterCos = cos(sunAngularDiameterDegrees);
		float sundisk = smoothstep(sunAngularDiameterCos, sunAngularDiameterCos + 0.00002, cosTheta);
		vec3 L0 = vec3(0.1) * Fex;
		L0 += sunE * 19000.0 * Fex * sundisk;
		vec3 texColor = Lin + L0;
		texColor *= 0.04;
		texColor += vec3(0.0, 0.001, 0.0025) * 0.3;

		imageStore(out_target, pixel_coordinates, vec4(texColor, 1.0));
		"#;

		let main = besl::parser::Node::function("main", Vec::new(), "void", vec![
			besl::parser::Node::glsl(main_code, &["make_uv", "light", "views", "total_rayleigh", "total_mie", "rayleigh_phase", "henyey_greenstein_phase", "out_target", "depth_target"], Vec::new())
		]);

		let root_node = besl::parser::Node::root();

		let mut root = shader_generator.transform(root_node, &json::object!{});

		let push_constant = besl::parser::Node::push_constant(vec![]);

		root.add(vec![
			besl::parser::Node::binding("depth_target", besl::parser::Node::combined_image_sampler(), 0, 1, true, false),
			besl::parser::Node::binding("out_target", besl::parser::Node::image("rgba16"), 0, 2, false, true),
			besl::parser::Node::binding("light", besl::parser::Node::buffer("LightData", vec![besl::parser::Node::member("direction", "vec3f")]), 0, 3, true, false),
			besl::parser::Node::function("total_rayleigh", vec![besl::parser::Node::parameter("lambda", "vec3f"), besl::parser::Node::parameter("refractive_index", "f32"), besl::parser::Node::parameter("depolarization_factor", "f32"), besl::parser::Node::parameter("num_molecules", "f32")], "vec3f", vec![
				besl::parser::Node::glsl("return (8.0 * pow(PI, 3.0) * pow(pow(refractive_index, 2.0) - 1.0, 2.0) * (6.0 + 3.0 * depolarization_factor)) / (3.0 * num_molecules * pow(lambda, vec3(4.0)) * (6.0 - 7.0 * depolarization_factor))", &[], vec![]),
			]),
			besl::parser::Node::function("total_mie", vec![besl::parser::Node::parameter("lambda", "vec3f"), besl::parser::Node::parameter("k", "vec3f"), besl::parser::Node::parameter("t", "f32"), besl::parser::Node::parameter("mie_v", "f32")], "vec3f", vec![
				besl::parser::Node::glsl("float c = 0.2 * t * 10e-18", &[], vec![]),
				besl::parser::Node::glsl("return 0.434 * c * PI * pow((2.0 * PI) / lambda, vec3(mie_v - 2.0)) * k;", &[], vec![]),
			]),
			besl::parser::Node::function("rayleigh_phase", vec![besl::parser::Node::parameter("cos_theta", "f32")], "f32", vec![
				besl::parser::Node::glsl("return (3.0 / (16.0 * PI)) * (1.0 + pow(cos_theta, 2.0))", &[], vec![]),
			]),
			besl::parser::Node::function("henyey_greenstein_phase", vec![besl::parser::Node::parameter("cos_theta", "f32"), besl::parser::Node::parameter("g", "f32")], "f32", vec![
				besl::parser::Node::glsl("return (1.0 / (4.0 * PI)) * ((1.0 - pow(g, 2.0)) / pow(1.0 - 2.0 * g * cos_theta + pow(g, 2.0), 1.5))", &[], vec![]),
			]),
			push_constant,
			main
		]);

		let root_node = besl::lex(root).unwrap();

		let main_node = root_node.borrow().get_main().unwrap();

		let glsl = GLSLShaderGenerator::new().generate(&ShaderGenerationSettings::compute(Extent::square(32)), &main_node).unwrap();

		glsl
	}
}

impl RenderPass for BackgroundRenderingPass {
	fn add_render_pass(&mut self, render_pass: EntityHandle<dyn RenderPass>) {
		unimplemented!()
	}

	fn record(&self, command_buffer_recording: &mut ghi::CommandBufferRecording, extent: Extent) {
		command_buffer_recording.start_region("Sky Rendering");

		command_buffer_recording.bind_descriptor_sets(&self.pipeline_layout, &[self.descriptor_set]);

		let pipeline_bind = command_buffer_recording.bind_compute_pipeline(&self.pipeline);

		pipeline_bind.dispatch(ghi::DispatchExtent::new(extent, Extent::square(32)));

		command_buffer_recording.end_region();
	}

	fn prepare(&self, ghi: &mut ghi::GHI, extent: Extent) {
		let buffer = ghi.get_mut_buffer_slice(self.buffer);

		let sun_direction = self.sun_direction;

		unsafe {
			let buffer = buffer.as_mut_ptr() as *mut Vector3;

			std::ptr::copy_nonoverlapping(&sun_direction, buffer, 1);
		}
	}

	fn resize(&self, ghi: &mut ghi::GHI, extent: Extent) {}
}

impl EntitySubscriber<DirectionalLight> for BackgroundRenderingPass {
	fn on_create<'a>(&'a mut self, handle: EntityHandle<DirectionalLight>, params: &'a DirectionalLight) -> () {
		self.sun_direction = params.direction;
	}
}

impl Entity for BackgroundRenderingPass {}