use std::{ffi::c_void, ptr::NonNull};

use dispatch2::DispatchData;
use objc2::{rc::Retained, runtime::ProtocolObject};
use objc2_foundation::{NSRange, NSString};
use objc2_metal::{
	MTLBlendFactor, MTLBlendOperation, MTLCompareFunction, MTLCompileOptions, MTLDataType, MTLDepthStencilDescriptor,
	MTLDepthStencilState, MTLDevice, MTLFunction, MTLFunctionConstantValues, MTLLibrary, MTLMeshRenderPipelineDescriptor,
	MTLPipelineOption, MTLRenderPipelineDescriptor, MTLVertexDescriptor, MTLVertexStepFunction,
};
use utils::{hash::HashMap, Extent};

use crate::{
	graphics_hardware_interface,
	metal::{
		utils::{data_type_size, parse_threadgroup_size_metadata, to_pixel_format, vertex_format},
		PipelineLayout, PipelineState, Shader, VertexElementDescriptor, VertexLayout,
	},
};

pub struct Factory {
	pub(crate) device: Retained<ProtocolObject<dyn MTLDevice>>,

	pub(crate) shaders: Vec<Shader>,
}

impl crate::pipelines::factory::Factory for Factory {
	type RasterPipeline = Pipeline;
	type ComputePipeline = ComputePipeline;

	fn create_shader(
		&mut self,
		_name: Option<&str>,
		shader_source_type: crate::shader::Sources,
		stage: crate::ShaderTypes,
		shader_binding_descriptors: impl IntoIterator<Item = crate::shader::BindingDescriptor>,
	) -> Result<graphics_hardware_interface::ShaderHandle, ()> {
		let (spirv, metal_library, metal_entry_point, threadgroup_size) = match shader_source_type {
			crate::shader::Sources::MTLB { binary, entry_point } => {
				let library = self.build_library(binary);

				(None, Some(library), Some(entry_point.to_owned()), None)
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
						eprintln!(
							"Metal shader compilation failed: {}",
							error.localizedDescription().to_string()
						);
						()
					})?;

				(None, Some(library), Some(entry_point.to_owned()), threadgroup_size)
			}
			_ => {
				eprintln!("Unsupported shader source type");
				return Err(());
			}
		};

		let stages = match stage {
			crate::ShaderTypes::Vertex => crate::Stages::VERTEX,
			crate::ShaderTypes::Fragment => crate::Stages::FRAGMENT,
			crate::ShaderTypes::Compute => crate::Stages::COMPUTE,
			crate::ShaderTypes::RayGen => crate::Stages::RAYGEN,
			crate::ShaderTypes::Intersection => crate::Stages::INTERSECTION,
			crate::ShaderTypes::AnyHit => crate::Stages::ANY_HIT,
			crate::ShaderTypes::ClosestHit => crate::Stages::CLOSEST_HIT,
			crate::ShaderTypes::Miss => crate::Stages::MISS,
			crate::ShaderTypes::Callable => crate::Stages::CALLABLE,
			crate::ShaderTypes::Task => crate::Stages::TASK,
			crate::ShaderTypes::Mesh => crate::Stages::MESH,
		};

		self.shaders.push(Shader {
			stage: stages,
			shader_binding_descriptors: shader_binding_descriptors.into_iter().collect(),
			metal_library,
			metal_entry_point,
			spirv,
			threadgroup_size,
		});

		Ok(graphics_hardware_interface::ShaderHandle((self.shaders.len() - 1) as u64))
	}

	fn create_raster_pipeline(&mut self, builder: crate::pipelines::raster::Builder) -> Self::RasterPipeline {
		let layout = self.build_pipeline_layout(
			builder.descriptor_set_templates.as_ref(),
			builder.push_constant_ranges.as_ref(),
		);
		let has_depth_attachment = builder
			.render_targets
			.iter()
			.any(|attachment| attachment.format.channel_layout() == crate::ChannelLayout::Depth);
		let vertex_layout =
			(!builder.vertex_elements.is_empty()).then(|| self.build_vertex_layout(builder.vertex_elements.as_ref()));
		let mut shader_handles = HashMap::default();
		let mut object_function = None;
		let mut vertex_function = None;
		let mut mesh_function = None;
		let mut fragment_function = None;
		let mut object_threadgroup_size = None;
		let mut mesh_threadgroup_size = None;
		let resource_access = builder
			.shaders
			.iter()
			.flat_map(|shader_parameter| {
				let shader = &self.shaders[shader_parameter.handle.0 as usize];
				shader_handles.insert(*shader_parameter.handle, [0; 32]);
				match shader_parameter.stage {
					crate::ShaderTypes::Task => {
						object_function = self.create_metal_function(shader_parameter);
						object_threadgroup_size = shader.threadgroup_size;
					}
					crate::ShaderTypes::Vertex => vertex_function = self.create_metal_function(shader_parameter),
					crate::ShaderTypes::Mesh => {
						mesh_function = self.create_metal_function(shader_parameter);
						mesh_threadgroup_size = shader.threadgroup_size;
					}
					crate::ShaderTypes::Fragment => fragment_function = self.create_metal_function(shader_parameter),
					_ => {}
				}
				shader
					.shader_binding_descriptors
					.iter()
					.map(|descriptor| {
						(
							(descriptor.set, descriptor.binding),
							(shader_parameter.stage.into(), descriptor.access),
						)
					})
					.collect::<Vec<_>>()
			})
			.collect::<Vec<_>>();

		let depth_stencil_state = if has_depth_attachment {
			let descriptor = MTLDepthStencilDescriptor::new();
			descriptor.setDepthCompareFunction(MTLCompareFunction::GreaterEqual);
			descriptor.setDepthWriteEnabled(true);
			self.device.newDepthStencilStateWithDescriptor(&descriptor)
		} else {
			None
		};

		let raster_pipeline_state = if let Some(mesh_function) = mesh_function.as_ref() {
			let descriptor = MTLMeshRenderPipelineDescriptor::new();
			descriptor.setLabel(Some(&NSString::from_str("mesh_pipeline")));
			unsafe {
				descriptor.setObjectFunction(object_function.as_ref().map(|function| function.as_ref()));
				descriptor.setMeshFunction(Some(mesh_function.as_ref()));
				descriptor.setFragmentFunction(fragment_function.as_ref().map(|function| function.as_ref()));
			}

			for (index, attachment) in builder.render_targets.iter().enumerate() {
				if attachment.format.channel_layout() == crate::ChannelLayout::Depth {
					descriptor.setDepthAttachmentPixelFormat(to_pixel_format(attachment.format));
				} else {
					let color_attachment = unsafe { descriptor.colorAttachments().objectAtIndexedSubscript(index as _) };
					color_attachment.setPixelFormat(to_pixel_format(attachment.format));
					match attachment.blend {
						crate::pipelines::raster::BlendMode::None => color_attachment.setBlendingEnabled(false),
						crate::pipelines::raster::BlendMode::Alpha => {
							color_attachment.setBlendingEnabled(true);
							color_attachment.setRgbBlendOperation(MTLBlendOperation::Add);
							color_attachment.setAlphaBlendOperation(MTLBlendOperation::Add);
							color_attachment.setSourceRGBBlendFactor(MTLBlendFactor::SourceAlpha);
							color_attachment.setDestinationRGBBlendFactor(MTLBlendFactor::OneMinusSourceAlpha);
							color_attachment.setSourceAlphaBlendFactor(MTLBlendFactor::One);
							color_attachment.setDestinationAlphaBlendFactor(MTLBlendFactor::OneMinusSourceAlpha);
						}
					}
				}
			}

			self.device
				.newRenderPipelineStateWithMeshDescriptor_options_reflection_error(&descriptor, MTLPipelineOption::None, None)
				.ok()
		} else if let Some(vertex_function) = vertex_function.as_ref() {
			let descriptor = MTLRenderPipelineDescriptor::new();
			descriptor.setLabel(Some(&NSString::from_str("raster_pipeline")));
			descriptor.setVertexFunction(Some(vertex_function.as_ref()));
			descriptor.setFragmentFunction(fragment_function.as_ref().map(|function| function.as_ref()));
			descriptor.setVertexDescriptor(vertex_layout.as_ref().map(|layout| layout.vertex_descriptor.as_ref()));

			for (index, attachment) in builder.render_targets.iter().enumerate() {
				if attachment.format.channel_layout() == crate::ChannelLayout::Depth {
					descriptor.setDepthAttachmentPixelFormat(to_pixel_format(attachment.format));
				} else {
					let color_attachment = unsafe { descriptor.colorAttachments().objectAtIndexedSubscript(index as _) };
					color_attachment.setPixelFormat(to_pixel_format(attachment.format));
					match attachment.blend {
						crate::pipelines::raster::BlendMode::None => color_attachment.setBlendingEnabled(false),
						crate::pipelines::raster::BlendMode::Alpha => {
							color_attachment.setBlendingEnabled(true);
							color_attachment.setRgbBlendOperation(MTLBlendOperation::Add);
							color_attachment.setAlphaBlendOperation(MTLBlendOperation::Add);
							color_attachment.setSourceRGBBlendFactor(MTLBlendFactor::SourceAlpha);
							color_attachment.setDestinationRGBBlendFactor(MTLBlendFactor::OneMinusSourceAlpha);
							color_attachment.setSourceAlphaBlendFactor(MTLBlendFactor::One);
							color_attachment.setDestinationAlphaBlendFactor(MTLBlendFactor::OneMinusSourceAlpha);
						}
					}
				}
			}

			self.device.newRenderPipelineStateWithDescriptor_error(&descriptor).ok()
		} else {
			None
		};

		Pipeline {
			pipeline: PipelineState::Raster(raster_pipeline_state),
			depth_stencil_state,
			layout,
			vertex_layout,
			shader_handles,
			resource_access,
			compute_threadgroup_size: None,
			object_threadgroup_size,
			mesh_threadgroup_size,
			face_winding: builder.face_winding,
			cull_mode: builder.cull_mode,
		}
	}

	fn create_compute_pipeline(&mut self, builder: crate::pipelines::compute::Builder) -> Self::ComputePipeline {
		let layout = self.build_pipeline_layout(builder.descriptor_set_templates, builder.push_constant_ranges);
		let shader_handle = *builder.shader.handle;
		let compute_pipeline_state = {
			let shader_parameter = &builder.shader;
			let shader = &self.shaders[shader_handle.0 as usize];
			assert!(
				shader.stage == crate::Stages::COMPUTE,
				"Metal compute pipeline creation requires a compute shader. The most likely cause is that a non-compute shader was passed to compute::Builder.",
			);
			let function = self.create_metal_function(shader_parameter).expect(
				"Metal compute pipeline creation requires a Metal shader function. The most likely cause is that this compute shader was created from SPIR-V, which this backend does not translate to MSL.",
			);

			Some(
				self.device
					.newComputePipelineStateWithFunction_error(&function)
					.expect("Metal compute pipeline creation failed. The most likely cause is that the shader function was invalid for compute pipeline creation."),
			)
		};

		let mut shader_handles = HashMap::default();
		shader_handles.insert(shader_handle, [0; 32]);
		let resource_access = self.shaders[shader_handle.0 as usize]
			.shader_binding_descriptors
			.iter()
			.map(|descriptor| {
				(
					(descriptor.set, descriptor.binding),
					(crate::Stages::COMPUTE, descriptor.access),
				)
			})
			.collect::<Vec<_>>();
		let compute_threadgroup_size = self.shaders[shader_handle.0 as usize].threadgroup_size;

		ComputePipeline {
			pipeline: PipelineState::Compute(compute_pipeline_state),
			depth_stencil_state: None,
			layout,
			shader_handles,
			resource_access,
			compute_threadgroup_size,
			object_threadgroup_size: None,
			mesh_threadgroup_size: None,
			face_winding: crate::pipelines::raster::FaceWinding::Clockwise,
			cull_mode: crate::pipelines::raster::CullMode::Back,
		}
	}
}

impl Factory {
	fn create_metal_function(
		&self,
		shader_parameter: &crate::pipelines::ShaderParameter,
	) -> Option<Retained<ProtocolObject<dyn MTLFunction>>> {
		let shader = &self.shaders[shader_parameter.handle.0 as usize];
		let library = shader.metal_library.as_ref()?;
		let entry_point = shader.metal_entry_point.as_ref()?;
		let entry_point = NSString::from_str(entry_point);

		let constant_values = MTLFunctionConstantValues::new();

		for specialization_map_entry in shader_parameter.specialization_map {
			self.apply_specialization_map_entry(&constant_values, specialization_map_entry);
		}

		library
			.newFunctionWithName_constantValues_error(&entry_point, &constant_values)
			.map_err(|error| {
				eprintln!(
					"Metal shader specialization failed: {}",
					error.localizedDescription().to_string()
				);
			})
			.ok()
	}

	fn apply_specialization_map_entry(
		&self,
		constant_values: &MTLFunctionConstantValues,
		specialization_map_entry: &crate::pipelines::SpecializationMapEntry,
	) {
		match specialization_map_entry.get_type().as_str() {
			"bool" => unsafe {
				let value = specialization_map_entry.get_data().as_ptr() as *const c_void as *mut c_void;
				constant_values.setConstantValue_type_atIndex(
					NonNull::new(value).expect(
						"Metal specialization constant value pointer was null. The most likely cause is an empty specialization entry.",
					),
					MTLDataType::Bool,
					specialization_map_entry.get_constant_id() as usize,
				);
			},
			"u32" => unsafe {
				let value = specialization_map_entry.get_data().as_ptr() as *const c_void as *mut c_void;
				constant_values.setConstantValue_type_atIndex(
					NonNull::new(value).expect(
						"Metal specialization constant value pointer was null. The most likely cause is an empty specialization entry.",
					),
					MTLDataType::UInt,
					specialization_map_entry.get_constant_id() as usize,
				);
			},
			"f32" => unsafe {
				let value = specialization_map_entry.get_data().as_ptr() as *const c_void as *mut c_void;
				constant_values.setConstantValue_type_atIndex(
					NonNull::new(value).expect(
						"Metal specialization constant value pointer was null. The most likely cause is an empty specialization entry.",
					),
					MTLDataType::Float,
					specialization_map_entry.get_constant_id() as usize,
				);
			},
			"vec2f" => unsafe {
				let value = specialization_map_entry.get_data().as_ptr() as *const c_void as *mut c_void;
				constant_values.setConstantValues_type_withRange(
					NonNull::new(value).expect(
						"Metal specialization constant value pointer was null. The most likely cause is an empty specialization entry.",
					),
					MTLDataType::Float,
					NSRange::new(specialization_map_entry.get_constant_id() as usize, 2),
				);
			},
			"vec3f" => unsafe {
				let value = specialization_map_entry.get_data().as_ptr() as *const c_void as *mut c_void;
				constant_values.setConstantValues_type_withRange(
					NonNull::new(value).expect(
						"Metal specialization constant value pointer was null. The most likely cause is an empty specialization entry.",
					),
					MTLDataType::Float,
					NSRange::new(specialization_map_entry.get_constant_id() as usize, 3),
				);
			},
			"vec4f" => unsafe {
				let value = specialization_map_entry.get_data().as_ptr() as *const c_void as *mut c_void;
				constant_values.setConstantValues_type_withRange(
					NonNull::new(value).expect(
						"Metal specialization constant value pointer was null. The most likely cause is an empty specialization entry.",
					),
					MTLDataType::Float,
					NSRange::new(specialization_map_entry.get_constant_id() as usize, 4),
				);
			},
			_ => panic!(
				"Unsupported Metal specialization constant type. The most likely cause is that the Metal backend was not updated for a new specialization entry type."
			),
		}
	}

	fn build_pipeline_layout(
		&self,
		descriptor_set_template_handles: &[graphics_hardware_interface::DescriptorSetTemplateHandle],
		push_constant_ranges: &[crate::pipelines::PushConstantRange],
	) -> PipelineLayout {
		let descriptor_set_template_indices = descriptor_set_template_handles
			.iter()
			.enumerate()
			.map(|(index, handle)| (*handle, index as u32))
			.collect();
		let push_constant_size = push_constant_ranges
			.iter()
			.map(|range| range.offset as usize + range.size as usize)
			.max()
			.unwrap_or(0);

		PipelineLayout {
			descriptor_set_template_indices,
			push_constant_ranges: push_constant_ranges.to_vec(),
			push_constant_size,
		}
	}

	fn build_vertex_layout(&mut self, vertex_elements: &[crate::pipelines::VertexElement]) -> VertexLayout {
		let elements = vertex_elements
			.iter()
			.map(|element| VertexElementDescriptor {
				name: element.name.to_owned(),
				format: element.format,
				binding: element.binding,
			})
			.collect::<Vec<_>>();

		let max_binding = elements
			.iter()
			.map(|element| element.binding)
			.max()
			.map(|binding| binding as usize + 1)
			.unwrap_or(0);

		let mut strides = vec![0; max_binding];

		let vertex_descriptor = MTLVertexDescriptor::vertexDescriptor();

		let mut binding_offsets = vec![0usize; max_binding];

		for (attribute_index, element) in elements.iter().enumerate() {
			strides[element.binding as usize] += element.format.size() as u32;

			let offset = binding_offsets[element.binding as usize];
			let attribute = unsafe { vertex_descriptor.attributes().objectAtIndexedSubscript(attribute_index as _) };
			attribute.setFormat(vertex_format(element.format));
			unsafe {
				attribute.setOffset(offset as _);
				attribute.setBufferIndex(element.binding as _);
			}

			binding_offsets[element.binding as usize] += data_type_size(element.format);
		}

		for (binding, stride) in strides.iter().copied().enumerate() {
			let layout = unsafe { vertex_descriptor.layouts().objectAtIndexedSubscript(binding as _) };
			unsafe {
				layout.setStride(stride as _);
				layout.setStepRate(1);
			}
			layout.setStepFunction(MTLVertexStepFunction::PerVertex);
		}

		VertexLayout {
			elements,
			strides,
			vertex_descriptor,
		}
	}

	fn build_library(&self, data: &[u8]) -> Retained<ProtocolObject<dyn MTLLibrary>> {
		let data = DispatchData::from_bytes(data);
		self.device.newLibraryWithData_error(&data).expect(
			"Metal library creation failed. The most likely cause is that the provided bytes were not a valid metallib binary.",
		)
	}
}

#[derive(Clone)]
pub struct Pipeline {
	pub(crate) pipeline: PipelineState,
	pub(crate) depth_stencil_state: Option<Retained<ProtocolObject<dyn MTLDepthStencilState>>>,
	pub(crate) layout: PipelineLayout,
	pub(crate) vertex_layout: Option<VertexLayout>,
	pub(crate) shader_handles: HashMap<graphics_hardware_interface::ShaderHandle, [u8; 32]>,
	pub(crate) resource_access: Vec<((u32, u32), (crate::Stages, crate::AccessPolicies))>,
	pub(crate) compute_threadgroup_size: Option<Extent>,
	pub(crate) object_threadgroup_size: Option<Extent>,
	pub(crate) mesh_threadgroup_size: Option<Extent>,
	pub(crate) face_winding: crate::pipelines::raster::FaceWinding,
	pub(crate) cull_mode: crate::pipelines::raster::CullMode,
}

#[derive(Clone)]
pub struct ComputePipeline {
	pub(crate) pipeline: PipelineState,
	pub(crate) depth_stencil_state: Option<Retained<ProtocolObject<dyn MTLDepthStencilState>>>,
	pub(crate) layout: PipelineLayout,
	pub(crate) shader_handles: HashMap<graphics_hardware_interface::ShaderHandle, [u8; 32]>,
	pub(crate) resource_access: Vec<((u32, u32), (crate::Stages, crate::AccessPolicies))>,
	pub(crate) compute_threadgroup_size: Option<Extent>,
	pub(crate) object_threadgroup_size: Option<Extent>,
	pub(crate) mesh_threadgroup_size: Option<Extent>,
	pub(crate) face_winding: crate::pipelines::raster::FaceWinding,
	pub(crate) cull_mode: crate::pipelines::raster::CullMode,
}
