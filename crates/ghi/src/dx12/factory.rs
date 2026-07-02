/// The `Factory` struct builds detached DX12 resources before they have public GHI handles.
pub struct Factory {
	pub(crate) shaders: Vec<Shader>,
}

/// The `Shader` struct stores detached DX12 shader metadata for later interning.
#[derive(Clone)]
pub(crate) struct Shader {
	pub(crate) name: Option<String>,
	pub(crate) source: ShaderSource,
	pub(crate) stage: ShaderTypes,
	pub(crate) bindings: Vec<BindingDescriptor>,
}

/// The `ShaderSource` enum stores owned shader bytecode for detached DX12 shader creation.
#[derive(Clone)]
pub(crate) enum ShaderSource {
	Spirv(Vec<u8>),
	Dxil(Vec<u8>),
	Hlsl { source: String, entry_point: String },
}

/// The `RasterPipeline` struct stores detached DX12 raster pipeline state until a frame interns it.
pub struct RasterPipeline {
	pub(crate) descriptor_set_templates: Vec<crate::DescriptorSetTemplateHandle>,
	pub(crate) push_constant_ranges: Vec<pipelines::PushConstantRange>,
	pub(crate) vertex_elements: Vec<VertexElement>,
	pub(crate) shaders: Vec<ShaderParameter>,
	pub(crate) render_targets: Vec<pipelines::raster::AttachmentDescriptor>,
	pub(crate) face_winding: pipelines::raster::FaceWinding,
	pub(crate) cull_mode: pipelines::raster::CullMode,
	pub(crate) factory_shaders: Vec<Shader>,
}

/// The `ComputePipeline` struct stores detached DX12 compute pipeline state until a frame interns it.
pub struct ComputePipeline {
	pub(crate) descriptor_set_templates: Vec<crate::DescriptorSetTemplateHandle>,
	pub(crate) push_constant_ranges: Vec<pipelines::PushConstantRange>,
	pub(crate) shader: ShaderParameter,
	pub(crate) factory_shaders: Vec<Shader>,
}

/// The `VertexElement` struct stores owned vertex element metadata for detached DX12 raster pipelines.
pub(crate) struct VertexElement {
	pub(crate) name: String,
	pub(crate) format: crate::DataTypes,
	pub(crate) binding: u32,
}

/// The `ShaderParameter` struct stores owned shader binding metadata for detached DX12 pipelines.
#[derive(Clone)]
pub(crate) struct ShaderParameter {
	pub(crate) handle: ShaderHandle,
	pub(crate) stage: ShaderTypes,
	pub(crate) specialization_map: Vec<pipelines::SpecializationMapEntry>,
}

/// The `FactoryImage` struct stores detached DX12 image creation parameters.
pub struct FactoryImage {
	pub(crate) name: Option<String>,
	pub(crate) extent: Extent,
	pub(crate) format: Formats,
	pub(crate) resource_uses: Uses,
	pub(crate) device_accesses: DeviceAccesses,
	pub(crate) use_case: UseCases,
	pub(crate) mip_levels: u32,
	pub(crate) array_layers: Option<std::num::NonZeroU32>,
}

/// The `FactorySampler` struct stores detached DX12 sampler creation parameters.
pub struct FactorySampler {
	pub(crate) filtering_mode: FilteringModes,
	pub(crate) reduction_mode: SamplingReductionModes,
	pub(crate) mip_map_mode: FilteringModes,
	pub(crate) addressing_mode: SamplerAddressingModes,
	pub(crate) anisotropy: Option<f32>,
	pub(crate) min_lod: f32,
	pub(crate) max_lod: f32,
}

/// The `Image` type alias preserves the detached image name used by backend-specific factory paths.
pub type Image = FactoryImage;

/// The `Sampler` type alias preserves the detached sampler name used by backend-specific factory paths.
pub type Sampler = FactorySampler;

impl Default for Factory {
	fn default() -> Self {
		Self { shaders: Vec::new() }
	}
}

impl crate::device::Device for Factory {
	type Context = crate::dx12::Device;
	type RasterPipeline = RasterPipeline;
	type ComputePipeline = ComputePipeline;
	type Image = FactoryImage;
	type Sampler = FactorySampler;

	#[cfg(debug_assertions)]
	fn has_errors(&self) -> bool {
		false
	}

	fn create_context(&self) -> Result<Self::Context, &'static str> {
		Err("Detached DX12 device cannot create a rendering context. The most likely cause is that asynchronous resource construction attempted to become the primary graphics device.")
	}

	fn create_shader(
		&mut self,
		name: Option<&str>,
		shader_source_type: Sources,
		stage: ShaderTypes,
		shader_binding_descriptors: impl IntoIterator<Item = BindingDescriptor>,
	) -> Result<ShaderHandle, ()> {
		let source = match shader_source_type {
			Sources::SPIRV(bytes) => ShaderSource::Spirv(bytes.to_vec()),
			Sources::DXIL(bytes) => ShaderSource::Dxil(bytes.to_vec()),
			Sources::HLSL { source, entry_point } => ShaderSource::Hlsl {
				source: source.to_string(),
				entry_point: entry_point.to_string(),
			},
			Sources::MTL { .. } | Sources::MTLB { .. } => return Err(()),
		};

		self.shaders.push(Shader {
			name: crate::debug_name(name),
			source,
			stage,
			bindings: shader_binding_descriptors.into_iter().collect(),
		});

		Ok(ShaderHandle((self.shaders.len() - 1) as u64))
	}

	fn create_raster_pipeline(&mut self, builder: pipelines::raster::Builder) -> Self::RasterPipeline {
		RasterPipeline {
			descriptor_set_templates: builder.descriptor_set_templates.to_vec(),
			push_constant_ranges: builder.push_constant_ranges.to_vec(),
			vertex_elements: builder
				.vertex_elements
				.iter()
				.map(|element| VertexElement {
					name: element.name.to_owned(),
					format: element.format,
					binding: element.binding,
				})
				.collect(),
			shaders: builder
				.shaders
				.iter()
				.map(|shader| ShaderParameter {
					handle: *shader.handle,
					stage: shader.stage,
					specialization_map: shader.specialization_map.to_vec(),
				})
				.collect(),
			render_targets: builder.render_targets.to_vec(),
			face_winding: builder.face_winding,
			cull_mode: builder.cull_mode,
			factory_shaders: self.shaders.clone(),
		}
	}

	fn create_compute_pipeline(&mut self, builder: pipelines::compute::Builder) -> Self::ComputePipeline {
		ComputePipeline {
			descriptor_set_templates: builder.descriptor_set_templates.to_vec(),
			push_constant_ranges: builder.push_constant_ranges.to_vec(),
			shader: ShaderParameter {
				handle: *builder.shader.handle,
				stage: builder.shader.stage,
				specialization_map: builder.shader.specialization_map.to_vec(),
			},
			factory_shaders: self.shaders.clone(),
		}
	}

	fn build_image(&mut self, builder: image::Builder) -> Self::Image {
		FactoryImage {
			name: crate::debug_name(builder.name),
			extent: builder.extent,
			format: builder.format,
			resource_uses: builder.resource_uses,
			device_accesses: builder.device_accesses,
			use_case: builder.use_case,
			mip_levels: builder.mip_levels,
			array_layers: builder.array_layers,
		}
	}

	fn build_sampler(&mut self, builder: sampler::Builder) -> Self::Sampler {
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

use utils::Extent;

use crate::{
	image, pipelines, sampler,
	shader::{BindingDescriptor, Sources},
	DeviceAccesses, FilteringModes, Formats, SamplerAddressingModes, SamplingReductionModes, ShaderHandle, ShaderTypes,
	UseCases, Uses,
};
