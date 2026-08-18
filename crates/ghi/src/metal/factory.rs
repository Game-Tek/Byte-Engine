//! The `factory` module exposes detached Metal resource types for public API consumers.

/// The `Factory` struct provides detached Metal resource creation without owning render context state.
pub struct Factory {
	pub(crate) device: Retained<ProtocolObject<dyn mtl::MTLDevice>>,
	pub(crate) compiler: Retained<ProtocolObject<dyn mtl::MTL4Compiler>>,
	pub settings: crate::device::Features,
	pub(crate) shaders: Vec<Shader>,
}

impl Factory {
	/// Creates a detached Metal factory from a backend device snapshot.
	pub(crate) fn new(
		device: Retained<ProtocolObject<dyn mtl::MTLDevice>>,
		compiler: Retained<ProtocolObject<dyn mtl::MTL4Compiler>>,
		settings: crate::device::Features,
	) -> Self {
		Self {
			device,
			compiler,
			settings,
			shaders: Vec::new(),
		}
	}

	fn build_pipeline_layout(
		&self,
		shaders: &[crate::pipelines::ShaderParameter],
		push_constant_ranges: &[crate::pipelines::PushConstantRange],
	) -> PipelineLayout {
		let stage_resources = shaders
			.iter()
			.map(|shader_parameter| {
				let shader = &self.shaders[shader_parameter.handle.0 as usize];
				(shader.stage, shader.shader_resource_descriptors.clone())
			})
			.collect::<Vec<_>>();
		build_pipeline_layout(self.device.as_ref(), &stage_resources, push_constant_ranges)
	}

	fn build_library(&self, data: &[u8]) -> Retained<ProtocolObject<dyn MTLLibrary>> {
		let data = DispatchData::from_bytes(data);
		self.device.newLibraryWithData_error(&data).expect(
			"Metal library creation failed. The most likely cause is that the provided bytes were not a valid metallib binary.",
		)
	}
}

pub use crate::metal::device::{ComputePipeline, Image, Pipeline, Sampler};

/// The `RasterPipeline` type alias preserves the cross-platform raster pipeline name.
pub type RasterPipeline = Pipeline;

/// The `FactoryImage` type alias preserves the cross-platform detached image name.
pub type FactoryImage = Image;

/// The `FactorySampler` type alias preserves the cross-platform detached sampler name.
pub type FactorySampler = Sampler;

impl crate::device::Device for Factory {
	type Context = crate::metal::context::Context;
	type Allocator = std::alloc::Global;
	type RasterPipeline = Pipeline;
	type ComputePipeline = ComputePipeline;
	type Image = Image;
	type Sampler = Sampler;

	fn allocator(&self) -> &Self::Allocator {
		&std::alloc::Global
	}

	#[cfg(any(debug_assertions, test))]
	fn has_errors(&self) -> bool {
		false
	}

	fn create_context(&self) -> Result<Self::Context, &'static str> {
		Err(
			"Detached Metal factory cannot create a rendering context. The most likely cause is that asynchronous resource construction attempted to become the primary graphics device.",
		)
	}

	fn create_shader(
		&mut self,
		name: Option<&str>,
		shader_source_type: crate::shader::Sources,
		stage: crate::ShaderTypes,
		shader_resource_descriptors: impl IntoIterator<Item = crate::shader::ShaderResourceDescriptor>,
	) -> Result<graphics_hardware_interface::ShaderHandle, ()> {
		let (metal_library, metal_entry_point, threadgroup_size) = match shader_source_type {
			crate::shader::Sources::SPIRV(_) => {
				eprintln!(
					"Metal shader creation failed for {:?} shader {:?}. The most likely cause is that SPIR-V was supplied to the Metal backend without translation to MSL or MTLB.",
					stage,
					name.unwrap_or("<unnamed>"),
				);
				return Err(());
			}
			crate::shader::Sources::DXIL(_) | crate::shader::Sources::HLSL { .. } => return Err(()),
			crate::shader::Sources::MTLB {
				binary,
				entry_point,
				threadgroup_size,
			} => {
				let library = self.build_library(binary);

				(Some(library), Some(entry_point.to_owned()), threadgroup_size)
			}
			crate::shader::Sources::MTL { source, entry_point } => {
				let threadgroup_size = match stage {
					crate::ShaderTypes::Task | crate::ShaderTypes::Mesh | crate::ShaderTypes::Compute => {
						parse_threadgroup_size_metadata(source)
					}
					_ => None,
				};
				let compile_options = MTLCompileOptions::new();
				let source = NSString::from_str(source);
				let library = self
					.device
					.newLibraryWithSource_options_error(&source, Some(&compile_options))
					.map_err(|error| {
						eprintln!("Metal shader compilation failed: {}", error.localizedDescription());
					})?;

				(Some(library), Some(entry_point.to_owned()), threadgroup_size)
			}
		};

		let stages = stage.into();

		self.shaders.push(Shader {
			name: crate::debug_name(name),
			stage: stages,
			shader_resource_descriptors: shader_resource_descriptors.into_iter().collect(),
			metal_library,
			metal_entry_point,
			threadgroup_size,
		});

		Ok(graphics_hardware_interface::ShaderHandle((self.shaders.len() - 1) as u64))
	}

	fn create_raster_pipeline(&mut self, builder: crate::pipelines::raster::Builder) -> Self::RasterPipeline {
		let layout = self.build_pipeline_layout(builder.shaders.as_ref(), builder.push_constant_ranges.as_ref());
		let has_depth_attachment = builder
			.render_targets
			.iter()
			.any(|attachment| attachment.format.channel_layout() == crate::ChannelLayout::Depth);
		let vertex_layout =
			(!builder.vertex_elements.is_empty()).then(|| build_vertex_layout(builder.vertex_elements.as_ref()));
		let mut shader_handles = HashMap::default();
		let mut object_function = None;
		let mut vertex_function = None;
		let mut mesh_function = None;
		let mut fragment_function = None;
		let mut object_threadgroup_size = None;
		let mut mesh_threadgroup_size = None;
		for shader_parameter in builder.shaders.iter() {
			let shader = &self.shaders[shader_parameter.handle.0 as usize];
			shader_handles.insert(*shader_parameter.handle, [0; 32]);
			match shader_parameter.stage {
				crate::ShaderTypes::Task => {
					object_function = build_metal4_function_descriptor(shader, shader_parameter.specialization_map);
					object_threadgroup_size = Some(shader.threadgroup_size.unwrap_or(Extent::new(1, 1, 1)));
				}
				crate::ShaderTypes::Vertex => {
					vertex_function = build_metal4_function_descriptor(shader, shader_parameter.specialization_map)
				}
				crate::ShaderTypes::Mesh => {
					mesh_function = build_metal4_function_descriptor(shader, shader_parameter.specialization_map);
					mesh_threadgroup_size = shader.threadgroup_size;
				}
				crate::ShaderTypes::Fragment => {
					fragment_function = build_metal4_function_descriptor(shader, shader_parameter.specialization_map)
				}
				_ => {}
			}
		}

		let depth_stencil_state = if has_depth_attachment {
			let descriptor = MTLDepthStencilDescriptor::new();
			descriptor.setDepthCompareFunction(MTLCompareFunction::GreaterEqual);
			descriptor.setDepthWriteEnabled(builder.depth_write);
			self.device.newDepthStencilStateWithDescriptor(&descriptor)
		} else {
			None
		};

		let pipeline_name = if cfg!(debug_assertions) && self.settings.debug_labels {
			builder.name
		} else {
			None
		};
		let raster_pipeline_state = if let Some(mesh_function) = mesh_function.as_ref() {
			compile_metal4_mesh_pipeline(
				self.compiler.as_ref(),
				pipeline_name,
				object_function.as_deref(),
				mesh_function,
				fragment_function.as_deref(),
				builder.render_targets.as_ref(),
			)
		} else if let Some(vertex_function) = vertex_function.as_ref() {
			compile_metal4_render_pipeline(
				self.compiler.as_ref(),
				pipeline_name,
				vertex_function,
				fragment_function.as_deref(),
				vertex_layout.as_ref().map(|layout| layout.vertex_descriptor.as_ref()),
				builder.render_targets.as_ref(),
			)
		} else {
			let shader_names = builder
				.shaders
				.iter()
				.map(|shader_parameter| {
					let shader = &self.shaders[shader_parameter.handle.0 as usize];
					format!(
						"{:?} {:?}",
						shader_parameter.stage,
						shader.name.as_deref().unwrap_or("<unnamed>")
					)
				})
				.collect::<Vec<_>>()
				.join(", ");
			panic!(
				"Metal raster pipeline creation failed because no vertex or mesh shader function was available. The most likely cause is shader creation failed or SPIR-V was supplied to the Metal backend without translation to MSL or MTLB. Shaders: {shader_names}",
			);
		};

		Pipeline {
			pipeline: PipelineState::Raster(raster_pipeline_state),
			depth_stencil_state,
			layout,
			vertex_layout,
			shader_handles,
			compute_threadgroup_size: None,
			object_threadgroup_size,
			mesh_threadgroup_size,
			face_winding: builder.face_winding,
			cull_mode: builder.cull_mode,
		}
	}

	fn create_compute_pipeline(&mut self, builder: crate::pipelines::compute::Builder) -> Self::ComputePipeline {
		let layout = self.build_pipeline_layout(std::slice::from_ref(&builder.shader), builder.push_constant_ranges);
		let shader_handle = *builder.shader.handle;
		let compute_pipeline_state = {
			let shader_parameter = &builder.shader;
			let shader = &self.shaders[shader_handle.0 as usize];
			assert!(
				shader.stage == crate::Stages::COMPUTE,
				"Metal compute pipeline creation requires a compute shader. The most likely cause is that a non-compute shader was passed to compute::Builder.",
			);
			let function = build_metal4_function_descriptor(shader, shader_parameter.specialization_map).expect(
				"Metal 4 compute pipeline creation requires a Metal function descriptor. The most likely cause is that the shader has no Metal library or entry point.",
			);

			let pipeline_name = if cfg!(debug_assertions) && self.settings.debug_labels {
				builder.name
			} else {
				None
			};
			compile_metal4_compute_pipeline(self.compiler.as_ref(), pipeline_name, &function)
		};

		let mut shader_handles = HashMap::default();
		shader_handles.insert(shader_handle, [0; 32]);
		let compute_threadgroup_size = self.shaders[shader_handle.0 as usize].threadgroup_size;

		ComputePipeline {
			pipeline: PipelineState::Compute(compute_pipeline_state),
			depth_stencil_state: None,
			layout,
			shader_handles,
			compute_threadgroup_size,
			object_threadgroup_size: None,
			mesh_threadgroup_size: None,
			face_winding: crate::pipelines::raster::FaceWinding::Clockwise,
			cull_mode: crate::pipelines::raster::CullMode::Back,
		}
	}

	/// Builds a Metal image that can be interned by a device later.
	fn build_image(&mut self, builder: crate::image::Builder) -> Self::Image {
		if builder.use_case == crate::UseCases::DYNAMIC {
			panic!(
				"Metal factory image creation does not support dynamic images. The most likely cause is that the image requires per-frame resource instances."
			);
		}

		if builder.device_accesses.intersects(crate::DeviceAccesses::HostOnly) {
			panic!(
				"Metal factory image creation does not support CPU-visible images. The most likely cause is that the image requires an associated staging buffer."
			);
		}

		let layers = builder.array_layers.map(|layers| layers.get()).unwrap_or(1);
		let descriptor = build_texture_descriptor(
			builder.format,
			builder.extent,
			builder.resource_uses,
			builder.device_accesses,
			layers,
			builder.cube_compatible,
			builder.cube_array_compatible,
			builder.mip_levels,
		);

		let texture = self
			.device
			.newTextureWithDescriptor(&descriptor)
			.expect("Metal texture creation failed. The most likely cause is that the device is out of memory.");

		#[cfg(debug_assertions)]
		if self.settings.debug_labels {
			if let Some(name) = builder.name {
				texture.setLabel(Some(&NSString::from_str(name)));
			}
		}

		Image {
			image: crate::metal::image::Image {
				name: crate::debug_name(builder.name),
				texture,
				extent: builder.extent,
				format: builder.format,
				uses: builder.resource_uses,
				access: builder.device_accesses,
				array_layers: layers,
				cube_compatible: builder.cube_compatible,
				cube_array_compatible: builder.cube_array_compatible,
				mip_levels: builder.mip_levels,
				staging: None,
			},
		}
	}

	/// Builds a Metal sampler that can be interned by a device later.
	fn build_sampler(&mut self, builder: crate::sampler::Builder) -> Self::Sampler {
		let descriptor = build_sampler_descriptor(&builder);
		apply_sampler_reduction_fallback(self.device.as_ref(), &descriptor);

		let sampler_state = self
			.device
			.newSamplerStateWithDescriptor(&descriptor)
			.expect("Metal sampler creation failed. The most likely cause is that the device is out of sampler resources.");

		Sampler {
			sampler: crate::metal::sampler::Sampler { sampler: sampler_state },
		}
	}
}

use dispatch2::DispatchData;
use objc2_foundation::NSString;
use objc2_metal::{MTLCompareFunction, MTLCompileOptions, MTLDepthStencilDescriptor, MTLDevice, MTLLibrary, MTLResource as _};

use super::*;
use crate::metal::utils::parse_threadgroup_size_metadata;
