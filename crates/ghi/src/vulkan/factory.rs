use std::{borrow::Cow, mem::align_of, num::NonZeroU32};

use ash::vk;
use utils::{hash::HashMap, Extent};

use crate::{graphics_hardware_interface, vulkan::DescriptorSetLayout};

/// The `Factory` struct builds Vulkan resources outside the context resource tables.
pub struct Factory {
	pub(crate) device: ash::Device,
	pub(crate) descriptor_set_layouts: Vec<DescriptorSetLayout>,
	pub(crate) shaders: Vec<crate::vulkan::Shader>,
}

unsafe impl Send for Factory {}

/// The `ComputePipeline` struct carries a Vulkan compute pipeline before it has a public GHI handle.
pub struct ComputePipeline {
	pub(crate) pipeline: vk::Pipeline,
	pub(crate) layout: crate::vulkan::PipelineLayout,
	pub(crate) shader_handles: HashMap<graphics_hardware_interface::ShaderHandle, [u8; 32]>,
	pub(crate) resource_access: Vec<((u32, u32), (crate::Stages, crate::AccessPolicies))>,
}

unsafe impl Send for ComputePipeline {}

/// The `RasterPipeline` struct marks detached Vulkan raster pipelines for future factory support.
pub struct RasterPipeline;

/// The `FactoryImage` struct stores Vulkan image creation parameters until a context interns them.
pub struct FactoryImage {
	pub(crate) name: Option<String>,
	pub(crate) extent: Extent,
	pub(crate) format: crate::Formats,
	pub(crate) resource_uses: crate::Uses,
	pub(crate) device_accesses: crate::DeviceAccesses,
	pub(crate) use_case: crate::UseCases,
	pub(crate) array_layers: Option<NonZeroU32>,
}

/// The `FactorySampler` struct stores Vulkan sampler creation parameters until a context interns them.
pub struct FactorySampler {
	pub(crate) filtering_mode: crate::FilteringModes,
	pub(crate) reduction_mode: crate::SamplingReductionModes,
	pub(crate) mip_map_mode: crate::FilteringModes,
	pub(crate) addressing_mode: crate::SamplerAddressingModes,
	pub(crate) anisotropy: Option<f32>,
	pub(crate) min_lod: f32,
	pub(crate) max_lod: f32,
}

impl crate::device::Device for Factory {
	type Context = crate::vulkan::context::Context;
	type RasterPipeline = RasterPipeline;
	type ComputePipeline = ComputePipeline;
	type Image = FactoryImage;
	type Sampler = FactorySampler;

	#[cfg(debug_assertions)]
	fn has_errors(&self) -> bool {
		false
	}

	fn create_context(&self) -> Result<Self::Context, &'static str> {
		Err("Detached Vulkan device cannot create a rendering context. The most likely cause is that asynchronous resource construction attempted to become the primary graphics device.")
	}

	fn create_shader(
		&mut self,
		_name: Option<&str>,
		shader_source_type: crate::shader::Sources,
		stage: crate::ShaderTypes,
		shader_binding_descriptors: impl IntoIterator<Item = crate::shader::BindingDescriptor>,
	) -> Result<graphics_hardware_interface::ShaderHandle, ()> {
		let shader = match shader_source_type {
			crate::shader::Sources::SPIRV(spirv) => {
				if spirv.as_ptr().is_aligned_to(align_of::<u32>()) {
					Cow::Borrowed(unsafe { std::slice::from_raw_parts(spirv.as_ptr() as *const u32, spirv.len() / 4) })
				} else {
					let mut words = Vec::with_capacity(spirv.len() / 4);
					for chunk in spirv.chunks_exact(4) {
						words.push(u32::from_ne_bytes([chunk[0], chunk[1], chunk[2], chunk[3]]));
					}
					Cow::Owned(words)
				}
			}
			crate::shader::Sources::DXIL(_)
			| crate::shader::Sources::HLSL { .. }
			| crate::shader::Sources::MTL { .. }
			| crate::shader::Sources::MTLB { .. } => return Err(()),
		};

		let shader_module_create_info = vk::ShaderModuleCreateInfo::default().code(&shader);
		let shader_module = unsafe {
			self.device
				.create_shader_module(&shader_module_create_info, None)
				.map_err(|_| ())?
		};
		let handle = graphics_hardware_interface::ShaderHandle(self.shaders.len() as u64);

		self.shaders.push(crate::vulkan::Shader {
			shader: shader_module,
			stage: stage.into(),
			shader_binding_descriptors: shader_binding_descriptors.into_iter().collect(),
		});

		Ok(handle)
	}

	fn create_raster_pipeline(&mut self, _builder: crate::pipelines::raster::Builder) -> Self::RasterPipeline {
		RasterPipeline
	}

	fn create_compute_pipeline(&mut self, builder: crate::pipelines::compute::Builder) -> Self::ComputePipeline {
		let layout = self.build_pipeline_layout(builder.descriptor_set_templates, builder.push_constant_ranges);
		let shader_parameter = builder.shader;
		let shader = &self.shaders[shader_parameter.handle.0 as usize];
		let (specialization_entries_buffer, specialization_map_entries) =
			build_specialization_entries(shader_parameter.specialization_map);
		let specialization_info = vk::SpecializationInfo::default()
			.data(&specialization_entries_buffer)
			.map_entries(&specialization_map_entries);
		let create_infos = [vk::ComputePipelineCreateInfo::default()
			.stage(
				vk::PipelineShaderStageCreateInfo::default()
					.stage(vk::ShaderStageFlags::COMPUTE)
					.module(shader.shader)
					.name(std::ffi::CStr::from_bytes_with_nul(b"main\0").unwrap())
					.specialization_info(&specialization_info),
			)
			.layout(layout.pipeline_layout)];
		let pipeline = unsafe {
			self.device
				.create_compute_pipelines(vk::PipelineCache::null(), &create_infos, None)
				.expect("Vulkan factory compute pipeline creation failed. The most likely cause is that shader specialization or pipeline layout creation failed.")[0]
		};
		let resource_access = shader
			.shader_binding_descriptors
			.iter()
			.map(|descriptor| {
				(
					(descriptor.set, descriptor.binding),
					(crate::Stages::COMPUTE, descriptor.access),
				)
			})
			.collect::<Vec<_>>();
		let mut shader_handles = HashMap::default();
		shader_handles.insert(*shader_parameter.handle, [0; 32]);

		ComputePipeline {
			pipeline,
			layout,
			shader_handles,
			resource_access,
		}
	}

	fn build_image(&mut self, builder: crate::image::Builder) -> Self::Image {
		FactoryImage {
			name: builder.name.map(str::to_owned),
			extent: builder.extent,
			format: builder.format,
			resource_uses: builder.resource_uses,
			device_accesses: builder.device_accesses,
			use_case: builder.use_case,
			array_layers: builder.array_layers,
		}
	}

	fn build_sampler(&mut self, builder: crate::sampler::Builder) -> Self::Sampler {
		FactorySampler {
			filtering_mode: builder.filtering_mode,
			reduction_mode: builder.reduction_mode,
			mip_map_mode: builder.mip_map_mode,
			addressing_mode: builder.addressing_mode,
			anisotropy: builder.anisotropy,
			min_lod: builder.min_lod,
			max_lod: builder.max_lod,
		}
	}
}

impl Factory {
	fn build_pipeline_layout(
		&self,
		descriptor_set_template_handles: &[graphics_hardware_interface::DescriptorSetTemplateHandle],
		push_constant_ranges: &[crate::pipelines::PushConstantRange],
	) -> crate::vulkan::PipelineLayout {
		let descriptor_set_layouts = descriptor_set_template_handles
			.iter()
			.map(|handle| self.descriptor_set_layouts[handle.0 as usize].descriptor_set_layout)
			.collect::<Vec<_>>();
		let default_push_constant_range;
		let push_constant_ranges = if push_constant_ranges.is_empty() {
			default_push_constant_range = [crate::pipelines::PushConstantRange::new(0, 128)];
			default_push_constant_range.as_slice()
		} else {
			push_constant_ranges
		};
		let push_constant_stages = vk::ShaderStageFlags::VERTEX
			| vk::ShaderStageFlags::FRAGMENT
			| vk::ShaderStageFlags::COMPUTE
			| vk::ShaderStageFlags::MESH_EXT;
		let push_constant_ranges_vk = push_constant_ranges
			.iter()
			.map(|range| {
				vk::PushConstantRange::default()
					.stage_flags(push_constant_stages)
					.offset(range.offset)
					.size(range.size)
			})
			.collect::<Vec<_>>();
		let pipeline_layout_create_info = vk::PipelineLayoutCreateInfo::default()
			.set_layouts(&descriptor_set_layouts)
			.push_constant_ranges(&push_constant_ranges_vk);
		let pipeline_layout = unsafe {
			self.device
				.create_pipeline_layout(&pipeline_layout_create_info, None)
				.expect("Vulkan factory pipeline layout creation failed. The most likely cause is that a descriptor set template handle was invalid.")
		};
		let descriptor_set_template_indices = descriptor_set_template_handles
			.iter()
			.enumerate()
			.map(|(index, handle)| (*handle, index as u32))
			.collect();

		crate::vulkan::PipelineLayout {
			pipeline_layout,
			descriptor_set_template_indices,
		}
	}
}

fn build_specialization_entries(
	specialization_map: &[crate::pipelines::SpecializationMapEntry],
) -> (Vec<u8>, Vec<vk::SpecializationMapEntry>) {
	let mut data = Vec::<u8>::with_capacity(256);
	let mut entries = Vec::with_capacity(48);

	for specialization_map_entry in specialization_map {
		let scalar_count = match specialization_map_entry.get_type().as_str() {
			"bool" | "u32" | "f32" => 1,
			"vec2f" => 2,
			"vec3f" => 3,
			"vec4f" => 4,
			_ => panic!("Unsupported Vulkan specialization constant type. The most likely cause is that the Vulkan backend was not updated for a new specialization entry type."),
		};
		let offset = data.len() as u32;
		for i in 0..scalar_count {
			entries.push(
				vk::SpecializationMapEntry::default()
					.constant_id(specialization_map_entry.get_constant_id() + i)
					.offset(offset + i * 4)
					.size(4),
			);
		}
		data.extend_from_slice(specialization_map_entry.get_data());
	}

	(data, entries)
}
