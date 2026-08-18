use objc2_foundation::NSString;
use objc2_metal::MTL4Compiler as _;

use super::*;

/// Returns the previous drawable size, new drawable size, and scale factor.
pub(crate) fn get_layer_sizes(layer: &CAMetalLayer, view: &NSView) -> (NSSize, NSSize, f64) {
	let logical_size = view.frame().size;
	let drawable_size = view.convertSizeToBacking(logical_size);
	let scale_factor = if logical_size.width > 0.0 {
		(drawable_size.width / logical_size.width).max(1.0)
	} else if logical_size.height > 0.0 {
		(drawable_size.height / logical_size.height).max(1.0)
	} else {
		1.0
	};

	let current_size = layer.drawableSize();
	let new_size = NSSize::new(drawable_size.width, drawable_size.height);

	(current_size, new_size, scale_factor)
}

pub(crate) fn get_layer_extent(layer: &CAMetalLayer, view: &NSView) -> Extent {
	let (_, new_size, _) = get_layer_sizes(layer, view);

	Extent::rectangle(
		new_size.width.round().max(0.0) as u32,
		new_size.height.round().max(0.0) as u32,
	)
}

/// Updates the CAMetalLayer's drawable size to match the view's backing size, but only when
/// the size has actually changed. Calling `setDrawableSize` unconditionally invalidates the
/// layer's drawable pool, forcing Metal to allocate new drawables every frame.
pub(crate) fn update_layer_extent(layer: &CAMetalLayer, view: &NSView) -> Extent {
	let (current_size, new_size, scale_factor) = get_layer_sizes(layer, view);

	if (current_size.width - new_size.width).abs() > 0.5 || (current_size.height - new_size.height).abs() > 0.5 {
		layer.setContentsScale(scale_factor);
		layer.setDrawableSize(new_size);
	}

	Extent::rectangle(
		new_size.width.round().max(0.0) as u32,
		new_size.height.round().max(0.0) as u32,
	)
}

/// Applies one GHI specialization constant entry to a Metal function constant table.
pub(crate) fn apply_specialization_map_entry(
	constant_values: &mtl::MTLFunctionConstantValues,
	specialization_map_entry: &crate::pipelines::SpecializationMapEntry,
) {
	let value = specialization_map_entry.get_data().as_ptr() as *const c_void as *mut c_void;
	let value = NonNull::new(value).expect(
		"Metal specialization constant value pointer was null. The most likely cause is an empty specialization entry.",
	);
	let constant_id = specialization_map_entry.get_constant_id() as usize;

	match specialization_map_entry.get_type().as_str() {
		"bool" => unsafe { constant_values.setConstantValue_type_atIndex(value, mtl::MTLDataType::Bool, constant_id) },
		"i32" => unsafe { constant_values.setConstantValue_type_atIndex(value, mtl::MTLDataType::Int, constant_id) },
		"u32" => unsafe { constant_values.setConstantValue_type_atIndex(value, mtl::MTLDataType::UInt, constant_id) },
		"f32" => unsafe { constant_values.setConstantValue_type_atIndex(value, mtl::MTLDataType::Float, constant_id) },
		"vec2f" => unsafe {
			constant_values.setConstantValues_type_withRange(value, mtl::MTLDataType::Float, NSRange::new(constant_id, 2))
		},
		"vec3f" => unsafe {
			constant_values.setConstantValues_type_withRange(value, mtl::MTLDataType::Float, NSRange::new(constant_id, 3))
		},
		"vec4f" => unsafe {
			constant_values.setConstantValues_type_withRange(value, mtl::MTLDataType::Float, NSRange::new(constant_id, 4))
		},
		_ => panic!(
			"Unsupported Metal specialization constant type. The most likely cause is that the Metal backend was not updated for a new specialization entry type."
		),
	}
}

/// Rejects vertex attributes that overlap the fixed push-constant and nested argument-buffer bindings.
pub(crate) fn validate_vertex_binding(binding: u32) {
	assert!(
		binding < command_buffer::PUSH_CONSTANT_BINDING_INDEX,
		"Metal vertex binding is reserved. The most likely cause is that a vertex attribute uses binding 15 or higher. binding={binding}",
	);
}

/// Builds the Metal vertex descriptor and matching GHI vertex-layout metadata.
pub(crate) fn build_vertex_layout(vertex_elements: &[crate::pipelines::VertexElement]) -> VertexLayout {
	for element in vertex_elements {
		validate_vertex_binding(element.binding);
	}

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
	let mut binding_offsets = vec![0usize; max_binding];
	let vertex_descriptor = mtl::MTLVertexDescriptor::vertexDescriptor();

	for (attribute_index, element) in elements.iter().enumerate() {
		strides[element.binding as usize] += element.format.size() as u32;

		let offset = binding_offsets[element.binding as usize];
		let attribute = unsafe { vertex_descriptor.attributes().objectAtIndexedSubscript(attribute_index as _) };
		attribute.setFormat(utils::vertex_format(element.format));
		unsafe {
			attribute.setOffset(offset as _);
			attribute.setBufferIndex(element.binding as _);
		}

		binding_offsets[element.binding as usize] += element.format.size();
	}

	for (binding, stride) in strides.iter().copied().enumerate() {
		let layout = unsafe { vertex_descriptor.layouts().objectAtIndexedSubscript(binding as _) };
		unsafe {
			layout.setStride(stride as _);
			layout.setStepRate(1);
		}
		layout.setStepFunction(mtl::MTLVertexStepFunction::PerVertex);
	}

	VertexLayout {
		elements,
		strides,
		vertex_descriptor,
	}
}

/// Builds a Metal texture descriptor from GHI image creation parameters.
pub(crate) fn build_texture_descriptor(
	format: crate::Formats,
	extent: Extent,
	resource_uses: crate::Uses,
	device_accesses: crate::DeviceAccesses,
	array_layers: u32,
	cube_compatible: bool,
	cube_array_compatible: bool,
	mip_levels: u32,
) -> Retained<mtl::MTLTextureDescriptor> {
	if cube_compatible {
		assert!(
			array_layers == 6 && extent.width() == extent.height() && extent.depth().max(1) == 1,
			"Invalid Metal cubemap image. The most likely cause is that cube compatibility was requested for a non-square image or an image without six faces."
		);
	}
	if cube_array_compatible {
		assert!(
			array_layers > 0
				&& array_layers.is_multiple_of(6)
				&& extent.width() == extent.height()
				&& extent.depth().max(1) == 1,
			"Invalid Metal cubemap-array image. The most likely cause is that cube-array compatibility was requested for a non-square image or an array layer count not divisible by six."
		);
	}
	let descriptor = unsafe {
		mtl::MTLTextureDescriptor::texture2DDescriptorWithPixelFormat_width_height_mipmapped(
			utils::to_pixel_format(format),
			extent.width().max(1) as _,
			extent.height().max(1) as _,
			mip_levels > 1,
		)
	};

	if extent.depth() > 1 {
		descriptor.setTextureType(mtl::MTLTextureType::Type3D);
	} else if cube_array_compatible {
		descriptor.setTextureType(mtl::MTLTextureType::TypeCubeArray);
	} else if cube_compatible {
		descriptor.setTextureType(mtl::MTLTextureType::TypeCube);
	} else if array_layers > 1 {
		descriptor.setTextureType(mtl::MTLTextureType::Type2DArray);
	}
	descriptor.setUsage(utils::texture_usage_from_uses(resource_uses));
	descriptor.setStorageMode(utils::storage_mode_from_access(device_accesses));
	unsafe {
		descriptor.setArrayLength(if cube_compatible {
			1
		} else if cube_array_compatible {
			array_layers / 6
		} else {
			array_layers
		} as _);
		descriptor.setMipmapLevelCount(mip_levels as _);
	}

	descriptor
}

/// Builds a Metal sampler descriptor from a GHI sampler builder.
pub(crate) fn build_sampler_descriptor(builder: &crate::sampler::Builder) -> Retained<mtl::MTLSamplerDescriptor> {
	let descriptor = mtl::MTLSamplerDescriptor::new();
	descriptor.setMinFilter(utils::sampler_min_mag_filter(builder.filtering_mode));
	descriptor.setMagFilter(utils::sampler_min_mag_filter(builder.filtering_mode));
	descriptor.setMipFilter(utils::sampler_mip_filter(builder.mip_map_mode));
	descriptor.setReductionMode(utils::sampler_reduction_mode(builder.reduction_mode));
	descriptor.setSAddressMode(utils::sampler_address_mode(builder.addressing_mode));
	descriptor.setTAddressMode(utils::sampler_address_mode(builder.addressing_mode));
	descriptor.setRAddressMode(utils::sampler_address_mode(builder.addressing_mode));
	descriptor.setLodMinClamp(builder.min_lod);
	descriptor.setLodMaxClamp(builder.max_lod);
	descriptor.setSupportArgumentBuffers(true);

	if let Some(anisotropy) = builder.anisotropy {
		descriptor.setMaxAnisotropy(anisotropy as _);
	}

	descriptor
}

/// Falls back to standard Metal sampling when the device cannot execute sampler reductions.
pub(crate) fn apply_sampler_reduction_fallback(
	device: &ProtocolObject<dyn mtl::MTLDevice>,
	descriptor: &mtl::MTLSamplerDescriptor,
) {
	let reduction_mode =
		sampler_reduction_mode_for_device(descriptor.reductionMode(), device.supportsFamily(mtl::MTLGPUFamily::Apple10));
	descriptor.setReductionMode(reduction_mode);
}

/// Selects the native Metal sampler mode without changing the cross-backend sampler contract.
pub(crate) fn sampler_reduction_mode_for_device(
	requested: mtl::MTLSamplerReductionMode,
	supports_reduction: bool,
) -> mtl::MTLSamplerReductionMode {
	if supports_reduction {
		requested
	} else {
		mtl::MTLSamplerReductionMode::WeightedAverage
	}
}

/// Creates the compiler shared by context-local and detached Metal 4 pipeline builds.
pub(crate) fn create_metal4_compiler(
	device: &ProtocolObject<dyn mtl::MTLDevice>,
	debug_labels: bool,
) -> Result<Retained<ProtocolObject<dyn mtl::MTL4Compiler>>, &'static str> {
	let descriptor = mtl::MTL4CompilerDescriptor::new();
	if cfg!(debug_assertions) && debug_labels {
		descriptor.setLabel(Some(&NSString::from_str("Byte Engine")));
	}
	device.newCompilerWithDescriptor_error(&descriptor).map_err(|error| {
		eprintln!(
			"Metal 4 compiler creation failed: {}. The most likely cause is that Metal could not allocate a compiler for this device.",
			error.localizedDescription(),
		);
		"Metal 4 compiler creation failed. The most likely cause is that Metal could not allocate a compiler for this device."
	})
}

/// Builds a Metal 4 function descriptor and applies any pipeline specialization constants.
pub(crate) fn build_metal4_function_descriptor(
	shader: &Shader,
	specialization_map: &[crate::pipelines::SpecializationMapEntry],
) -> Option<Retained<mtl::MTL4FunctionDescriptor>> {
	let library = shader.metal_library.as_ref()?;
	let entry_point = NSString::from_str(shader.metal_entry_point.as_ref()?);
	let library_function = mtl::MTL4LibraryFunctionDescriptor::new();
	library_function.setLibrary(Some(library.as_ref()));
	library_function.setName(Some(&entry_point));
	let library_function = unsafe { Retained::cast_unchecked::<mtl::MTL4FunctionDescriptor>(library_function) };

	if specialization_map.is_empty() {
		return Some(library_function);
	}

	let constant_values = mtl::MTLFunctionConstantValues::new();
	for specialization in specialization_map {
		apply_specialization_map_entry(&constant_values, specialization);
	}
	let specialized_function = mtl::MTL4SpecializedFunctionDescriptor::new();
	specialized_function.setFunctionDescriptor(Some(&library_function));
	specialized_function.setConstantValues(Some(&constant_values));
	Some(unsafe { Retained::cast_unchecked::<mtl::MTL4FunctionDescriptor>(specialized_function) })
}

/// Configures one Metal 4 color attachment with the GHI format and blend mode.
fn configure_metal4_color_attachment(
	color_attachment: &mtl::MTL4RenderPipelineColorAttachmentDescriptor,
	attachment: &crate::pipelines::raster::AttachmentDescriptor,
) {
	color_attachment.setPixelFormat(utils::to_pixel_format(attachment.format));
	match attachment.blend {
		crate::pipelines::raster::BlendMode::None => color_attachment.setBlendingState(mtl::MTL4BlendState::Disabled),
		crate::pipelines::raster::BlendMode::Alpha => {
			color_attachment.setBlendingState(mtl::MTL4BlendState::Enabled);
			color_attachment.setRgbBlendOperation(mtl::MTLBlendOperation::Add);
			color_attachment.setAlphaBlendOperation(mtl::MTLBlendOperation::Add);
			color_attachment.setSourceRGBBlendFactor(mtl::MTLBlendFactor::SourceAlpha);
			color_attachment.setDestinationRGBBlendFactor(mtl::MTLBlendFactor::OneMinusSourceAlpha);
			color_attachment.setSourceAlphaBlendFactor(mtl::MTLBlendFactor::One);
			color_attachment.setDestinationAlphaBlendFactor(mtl::MTLBlendFactor::OneMinusSourceAlpha);
		}
	}
}

/// Configures the packed color outputs shared by Metal 4 vertex and mesh render descriptors.
fn configure_metal4_render_targets(
	color_attachments: &mtl::MTL4RenderPipelineColorAttachmentDescriptorArray,
	render_targets: &[crate::pipelines::raster::AttachmentDescriptor],
) {
	for (index, attachment) in render_targets
		.iter()
		.filter(|attachment| attachment.format.channel_layout() != crate::ChannelLayout::Depth)
		.enumerate()
	{
		let color_attachment = unsafe { color_attachments.objectAtIndexedSubscript(index as _) };
		configure_metal4_color_attachment(&color_attachment, attachment);
	}
}

/// Compiles one Metal 4 vertex/fragment render pipeline.
pub(crate) fn compile_metal4_render_pipeline(
	compiler: &ProtocolObject<dyn mtl::MTL4Compiler>,
	name: Option<&str>,
	vertex_function: &mtl::MTL4FunctionDescriptor,
	fragment_function: Option<&mtl::MTL4FunctionDescriptor>,
	vertex_descriptor: Option<&mtl::MTLVertexDescriptor>,
	render_targets: &[crate::pipelines::raster::AttachmentDescriptor],
) -> Retained<ProtocolObject<dyn mtl::MTLRenderPipelineState>> {
	let descriptor = mtl::MTL4RenderPipelineDescriptor::new();
	descriptor.setLabel(name.map(NSString::from_str).as_deref());
	descriptor.setVertexFunctionDescriptor(Some(vertex_function));
	descriptor.setFragmentFunctionDescriptor(fragment_function);
	descriptor.setVertexDescriptor(vertex_descriptor);
	descriptor.setInputPrimitiveTopology(mtl::MTLPrimitiveTopologyClass::Triangle);
	configure_metal4_render_targets(&descriptor.colorAttachments(), render_targets);

	compiler
		.newRenderPipelineStateWithDescriptor_compilerTaskOptions_error(&descriptor, None)
		.unwrap_or_else(|error| {
			panic!(
				"Metal 4 raster pipeline creation failed: {}. The most likely cause is invalid shader functions or render-target state in the pipeline descriptor.",
				error.localizedDescription(),
			)
		})
}

/// Compiles one Metal 4 object/mesh/fragment render pipeline.
pub(crate) fn compile_metal4_mesh_pipeline(
	compiler: &ProtocolObject<dyn mtl::MTL4Compiler>,
	name: Option<&str>,
	object_function: Option<&mtl::MTL4FunctionDescriptor>,
	mesh_function: &mtl::MTL4FunctionDescriptor,
	fragment_function: Option<&mtl::MTL4FunctionDescriptor>,
	render_targets: &[crate::pipelines::raster::AttachmentDescriptor],
) -> Retained<ProtocolObject<dyn mtl::MTLRenderPipelineState>> {
	let descriptor = mtl::MTL4MeshRenderPipelineDescriptor::new();
	descriptor.setLabel(name.map(NSString::from_str).as_deref());
	descriptor.setObjectFunctionDescriptor(object_function);
	descriptor.setMeshFunctionDescriptor(Some(mesh_function));
	descriptor.setFragmentFunctionDescriptor(fragment_function);
	configure_metal4_render_targets(&descriptor.colorAttachments(), render_targets);

	compiler
		.newRenderPipelineStateWithDescriptor_compilerTaskOptions_error(&descriptor, None)
		.unwrap_or_else(|error| {
			panic!(
				"Metal 4 mesh pipeline creation failed: {}. The most likely cause is invalid object, mesh, or fragment shader state in the pipeline descriptor.",
				error.localizedDescription(),
			)
		})
}

/// Compiles one Metal 4 compute pipeline.
pub(crate) fn compile_metal4_compute_pipeline(
	compiler: &ProtocolObject<dyn mtl::MTL4Compiler>,
	name: Option<&str>,
	compute_function: &mtl::MTL4FunctionDescriptor,
) -> Retained<ProtocolObject<dyn mtl::MTLComputePipelineState>> {
	let descriptor = mtl::MTL4ComputePipelineDescriptor::new();
	descriptor.setLabel(name.map(NSString::from_str).as_deref());
	descriptor.setComputeFunctionDescriptor(Some(compute_function));

	compiler
		.newComputePipelineStateWithDescriptor_compilerTaskOptions_error(&descriptor, None)
		.unwrap_or_else(|error| {
			panic!(
				"Metal 4 compute pipeline creation failed: {}. The most likely cause is invalid compute shader state in the pipeline descriptor.",
				error.localizedDescription(),
			)
		})
}

/// The `StageArgumentLayout` struct provides one pipeline-wide Metal argument-buffer layout shared by its shader stages.
#[derive(Clone)]
pub(crate) struct StageArgumentLayout {
	pub(crate) stage: crate::Stages,
	pub(crate) bindings: Vec<StageArgumentBinding>,
	pub(crate) argument_encoder: Retained<ProtocolObject<dyn mtl::MTLArgumentEncoder>>,
	pub(crate) encoded_length: usize,
}

/// The `StageArgumentBinding` struct retains stable Metal argument IDs derived from one flat resource slot.
#[derive(Clone)]
pub(crate) struct StageArgumentBinding {
	pub(crate) descriptor: crate::shader::ShaderResourceDescriptor,
	pub(crate) argument_slots: ArgumentBindingSlots,
}

/// The `ArgumentSlotRange` struct identifies one dense run of native argument IDs.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct ArgumentSlotRange {
	pub(crate) base: u32,
	pub(crate) count: u32,
}

impl ArgumentSlotRange {
	fn slot(self, array_element: u32) -> u32 {
		assert!(
			array_element < self.count,
			"Metal argument array element is out of range. The most likely cause is that descriptor validation was bypassed.",
		);
		self.base
			.checked_add(array_element)
			.expect("Metal argument index overflowed. The most likely cause is an invalid argument base or array element.")
	}
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum ArgumentBindingSlots {
	Buffer(ArgumentSlotRange),
	Texture(ArgumentSlotRange),
	Sampler(ArgumentSlotRange),
	AccelerationStructure(ArgumentSlotRange),
	CombinedImageSampler {
		textures: ArgumentSlotRange,
		samplers: ArgumentSlotRange,
	},
}

impl StageArgumentLayout {
	pub(crate) fn binding(&self, slot: crate::shader::ResourceSlot) -> Option<&StageArgumentBinding> {
		self.bindings
			.iter()
			.find(|layout_binding| layout_binding.descriptor.slot() == slot)
	}
}

impl StageArgumentBinding {
	pub(crate) fn slot_for_array_element(&self, array_element: u32) -> DescriptorBindingSlot {
		match &self.argument_slots {
			ArgumentBindingSlots::Buffer(range) => DescriptorBindingSlot::Buffer(range.slot(array_element)),
			ArgumentBindingSlots::Texture(range) => DescriptorBindingSlot::Texture(range.slot(array_element)),
			ArgumentBindingSlots::Sampler(range) => DescriptorBindingSlot::Sampler(range.slot(array_element)),
			ArgumentBindingSlots::AccelerationStructure(range) => {
				DescriptorBindingSlot::AccelerationStructure(range.slot(array_element))
			}
			ArgumentBindingSlots::CombinedImageSampler { textures, samplers } => DescriptorBindingSlot::CombinedImageSampler {
				texture: textures.slot(array_element),
				sampler: samplers.slot(array_element),
			},
		}
	}
}

impl ArgumentBindingSlots {
	/// Visits each native argument range without allocating a flattened list of array elements.
	fn for_each_metal_argument(&self, mut visit: impl FnMut(u32, u32, mtl::MTLDataType)) {
		let mut visit_range = |range: ArgumentSlotRange, data_type| {
			visit(range.base, range.count, data_type);
		};

		match self {
			Self::Buffer(range) => visit_range(*range, mtl::MTLDataType::Pointer),
			Self::Texture(range) => visit_range(*range, mtl::MTLDataType::Texture),
			Self::Sampler(range) => visit_range(*range, mtl::MTLDataType::Sampler),
			Self::AccelerationStructure(range) => visit_range(*range, mtl::MTLDataType::InstanceAccelerationStructure),
			Self::CombinedImageSampler { textures, samplers } => {
				visit_range(*textures, mtl::MTLDataType::Texture);
				visit_range(*samplers, mtl::MTLDataType::Sampler);
			}
		}
	}
}

#[derive(Clone, Copy)]
pub(crate) enum DescriptorBindingSlot {
	Buffer(u32),
	Texture(u32),
	Sampler(u32),
	AccelerationStructure(u32),
	CombinedImageSampler { texture: u32, sampler: u32 },
}

/// The `PipelineResourceDescriptor` struct exists to retain the merged stage visibility for one flat pipeline resource.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) struct PipelineResourceDescriptor {
	pub(crate) descriptor: crate::shader::ShaderResourceDescriptor,
	pub(crate) stages: crate::Stages,
}

/// The `PipelineLayout` struct exists to retain the native resource layouts derived from a pipeline's shaders.
#[derive(Clone)]
pub(crate) struct PipelineLayout {
	pub(crate) resources: Vec<PipelineResourceDescriptor>,
	pub(crate) stage_argument_layouts: Vec<StageArgumentLayout>,
	pub(crate) push_constant_ranges: Vec<crate::pipelines::PushConstantRange>,
	pub(crate) push_constant_size: usize,
}

/// The `MaterializationKey` struct identifies one pipeline's frame-resolved union of retained descriptor sets.
#[derive(Clone, PartialEq, Eq, Hash)]
pub(crate) struct MaterializationKey {
	pub(crate) descriptor_sets: SmallVec<[crate::descriptors::DescriptorSetHandle; 4]>,
	pub(crate) sequence_index: u8,
}

/// The `Materialization` struct retains immutable native argument buffers until their logical set versions change.
#[derive(Clone)]
pub(crate) struct Materialization {
	pub(crate) versions: SmallVec<[u64; 4]>,
	pub(crate) argument_buffers: Rc<SmallVec<[(crate::Stages, Retained<ProtocolObject<dyn mtl::MTLBuffer>>); 5]>>,
	// Metal argument buffers do not retain texture views. Keep selected mip views alive with their bindings.
	pub(crate) _texture_views: Rc<SmallVec<[Retained<ProtocolObject<dyn mtl::MTLTexture>>; 4]>>,
}

#[derive(Clone)]
pub(crate) struct VertexLayout {
	pub(crate) elements: Vec<VertexElementDescriptor>,
	pub(crate) strides: Vec<u32>,
	pub(crate) vertex_descriptor: Retained<mtl::MTLVertexDescriptor>,
}

#[derive(Clone, PartialEq, Eq, Hash)]
pub(crate) struct VertexLayoutKey {
	pub(crate) elements: Vec<VertexElementDescriptor>,
}

#[derive(Clone, PartialEq, Eq, Hash)]
pub(crate) struct VertexElementDescriptor {
	pub(crate) name: String,
	pub(crate) format: crate::DataTypes,
	pub(crate) binding: u32,
}

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub(crate) struct VertexLayoutHandle(pub(crate) u64);

#[derive(Clone)]
pub(crate) struct Shader {
	pub(crate) name: Option<String>,
	pub(crate) stage: crate::Stages,
	pub(crate) shader_resource_descriptors: Vec<crate::shader::ShaderResourceDescriptor>,
	pub(crate) metal_library: Option<Retained<ProtocolObject<dyn mtl::MTLLibrary>>>,
	pub(crate) metal_entry_point: Option<String>,
	pub(crate) threadgroup_size: Option<Extent>,
}

#[derive(Clone)]
pub(crate) struct Pipeline {
	pub(crate) pipeline: PipelineState,
	pub(crate) depth_stencil_state: Option<Retained<ProtocolObject<dyn mtl::MTLDepthStencilState>>>,
	pub(crate) layout: graphics_hardware_interface::PipelineLayoutHandle,
	pub(crate) vertex_layout: Option<VertexLayoutHandle>,
	pub(crate) shader_handles: HashMap<graphics_hardware_interface::ShaderHandle, [u8; 32]>,
	pub(crate) materializations: RefCell<HashMap<MaterializationKey, Materialization>>,
	pub(crate) compute_threadgroup_size: Option<Extent>,
	pub(crate) object_threadgroup_size: Option<Extent>,
	pub(crate) mesh_threadgroup_size: Option<Extent>,
	pub(crate) face_winding: crate::pipelines::raster::FaceWinding,
	pub(crate) cull_mode: crate::pipelines::raster::CullMode,
}

#[derive(Clone)]
pub(crate) enum PipelineState {
	Raster(Retained<ProtocolObject<dyn mtl::MTLRenderPipelineState>>),
	Compute(Retained<ProtocolObject<dyn mtl::MTLComputePipelineState>>),
	RayTracing,
}

pub(crate) fn resource_ranges_overlap(
	left: crate::shader::ShaderResourceDescriptor,
	right: crate::shader::ShaderResourceDescriptor,
) -> bool {
	let left_start = left.slot().index();
	let left_end = resource_range_end(left);
	let right_start = right.slot().index();
	let right_end = resource_range_end(right);
	left_start < right_end && right_start < left_end
}

pub(crate) fn resource_range_end(descriptor: crate::shader::ShaderResourceDescriptor) -> u32 {
	descriptor
		.slot()
		.index()
		.checked_add(descriptor.count())
		.expect("Metal shader resource range overflowed. The most likely cause is an invalid flat slot or resource count.")
}

pub(crate) fn resource_accepts_retained_slot_key(
	descriptor: crate::shader::ShaderResourceDescriptor,
	stored_slot: crate::shader::ResourceSlot,
) -> bool {
	let base = descriptor.slot().index();
	let stored = stored_slot.index();
	stored <= base || stored >= resource_range_end(descriptor)
}

pub(crate) fn resource_representations_match(
	left: crate::shader::ShaderResourceDescriptor,
	right: crate::shader::ShaderResourceDescriptor,
) -> bool {
	left.slot() == right.slot()
		&& left.kind() == right.kind()
		&& left.count() == right.count()
		&& left.texture_view() == right.texture_view()
		&& left.buffer_element_stride() == right.buffer_element_stride()
}

/// Canonicalizes one stage interface so native layouts and materialization sharing do not depend on declaration order.
pub(crate) fn canonicalize_stage_resources(
	resources: &[crate::shader::ShaderResourceDescriptor],
) -> Vec<crate::shader::ShaderResourceDescriptor> {
	let mut sorted = resources.to_vec();
	sorted.sort_by_key(|descriptor| descriptor.slot());

	let mut canonical = Vec::<crate::shader::ShaderResourceDescriptor>::with_capacity(sorted.len());
	for descriptor in sorted {
		if let Some(previous) = canonical.last_mut() {
			if previous.slot() == descriptor.slot() {
				assert!(
					resource_representations_match(*previous, descriptor),
					"Conflicting Metal shader resources. The most likely cause is that one stage declared the same flat slot with incompatible representations.",
				);
				*previous = crate::shader::ShaderResourceDescriptor::new(
					previous.slot(),
					previous.kind(),
					previous.count(),
					previous.access() | descriptor.access(),
				)
				.texture_view_type(previous.texture_view())
				.buffer_stride(previous.buffer_element_stride());
				continue;
			}

			assert!(
				!resource_ranges_overlap(*previous, descriptor),
				"Overlapping Metal shader resources. The most likely cause is that one stage declared intersecting flat resource ranges.",
			);
		}
		canonical.push(descriptor);
	}

	canonical
}

/// Maps one logical flat-slot interval to its stable Metal argument-ID reservation.
pub(crate) fn fixed_argument_slot_ranges(
	slot: crate::shader::ResourceSlot,
	count: u32,
) -> (ArgumentSlotRange, ArgumentSlotRange) {
	let primary = slot.index().checked_mul(2).expect(
		"Metal argument index overflowed. The most likely cause is a flat resource slot too large for the fixed Metal ABI.",
	);
	let secondary = primary
		.checked_add(count)
		.expect("Metal argument index overflowed. The most likely cause is an invalid flat resource slot or resource count.");
	secondary
		.checked_add(count)
		.expect("Metal argument reservation overflowed. The most likely cause is a flat resource range too large for the fixed Metal ABI.");
	(
		ArgumentSlotRange { base: primary, count },
		ArgumentSlotRange { base: secondary, count },
	)
}

/// Assigns stable Metal argument IDs from one flat GHI resource interval.
pub(crate) fn allocate_argument_binding_slots(descriptor: crate::shader::ShaderResourceDescriptor) -> ArgumentBindingSlots {
	let (primary, secondary) = fixed_argument_slot_ranges(descriptor.slot(), descriptor.count());
	match descriptor.kind() {
		crate::shader::ResourceKind::UniformBuffer | crate::shader::ResourceKind::StorageBuffer => {
			ArgumentBindingSlots::Buffer(primary)
		}
		crate::shader::ResourceKind::SampledImage
		| crate::shader::ResourceKind::StorageImage
		| crate::shader::ResourceKind::InputAttachment => ArgumentBindingSlots::Texture(primary),
		crate::shader::ResourceKind::Sampler => ArgumentBindingSlots::Sampler(primary),
		crate::shader::ResourceKind::CombinedImageSampler => ArgumentBindingSlots::CombinedImageSampler {
			textures: primary,
			samplers: secondary,
		},
		crate::shader::ResourceKind::AccelerationStructure => ArgumentBindingSlots::AccelerationStructure(primary),
	}
}

/// Returns whether a native layout matches a stage's canonical packed resource interface.
pub(crate) fn stage_argument_interface_matches(
	layout: &StageArgumentLayout,
	resources: &[crate::shader::ShaderResourceDescriptor],
) -> bool {
	layout.bindings.len() == resources.len()
		&& layout
			.bindings
			.iter()
			.zip(resources)
			.all(|(binding, descriptor)| binding.descriptor == *descriptor)
}

/// Builds one fixed-ID Metal argument-buffer layout matching one shader stage's packed resource struct.
pub(crate) fn build_stage_argument_layout(
	device: &ProtocolObject<dyn mtl::MTLDevice>,
	stage: crate::Stages,
	resources: &[crate::shader::ShaderResourceDescriptor],
) -> StageArgumentLayout {
	let mut metal_argument_descriptors = Vec::new();
	let bindings = resources
		.iter()
		.copied()
		.map(|resource| {
			let access = if resource.access().intersects(crate::AccessPolicies::WRITE) {
				mtl::MTLBindingAccess::ReadWrite
			} else {
				mtl::MTLBindingAccess::ReadOnly
			};
			let argument_slots = allocate_argument_binding_slots(resource);
			argument_slots.for_each_metal_argument(|slot, count, data_type| {
				let descriptor = mtl::MTLArgumentDescriptor::argumentDescriptor();
				descriptor.setDataType(data_type);
				descriptor.setIndex(slot as _);
				if count > 1 {
					descriptor.setArrayLength(count as _);
				}
				descriptor.setAccess(access);
				if data_type == mtl::MTLDataType::Texture {
					let texture_type = match resource.texture_view() {
						crate::TextureViewTypes::Texture2D => mtl::MTLTextureType::Type2D,
						crate::TextureViewTypes::Texture2DArray => mtl::MTLTextureType::Type2DArray,
						crate::TextureViewTypes::TextureCube => mtl::MTLTextureType::TypeCube,
						crate::TextureViewTypes::TextureCubeArray => mtl::MTLTextureType::TypeCubeArray,
						crate::TextureViewTypes::Texture3D => mtl::MTLTextureType::Type3D,
					};
					descriptor.setTextureType(texture_type);
				}
				metal_argument_descriptors.push(descriptor);
			});

			StageArgumentBinding {
				descriptor: resource,
				argument_slots,
			}
		})
		.collect::<Vec<_>>();
	let argument_descriptor_refs = metal_argument_descriptors
		.iter()
		.map(|descriptor| descriptor.as_ref())
		.collect::<Vec<_>>();
	let argument_descriptors = NSArray::from_slice(&argument_descriptor_refs);
	let argument_encoder = device
		.newArgumentEncoderWithArguments(&argument_descriptors)
		.expect("Metal argument layout creation failed. The most likely cause is an unsupported shader resource interface.");

	StageArgumentLayout {
		stage,
		bindings,
		encoded_length: argument_encoder.encodedLength().max(1),
		argument_encoder,
	}
}

/// Builds the private Metal pipeline layout from the packed resource interface of each shader stage.
pub(crate) fn build_pipeline_layout(
	device: &ProtocolObject<dyn mtl::MTLDevice>,
	stage_resources: &[(crate::Stages, Vec<crate::shader::ShaderResourceDescriptor>)],
	push_constant_ranges: &[crate::pipelines::PushConstantRange],
) -> PipelineLayout {
	let mut resources = Vec::<PipelineResourceDescriptor>::new();
	let mut stage_argument_layouts = Vec::with_capacity(stage_resources.len());

	for (stage, stage_descriptors) in stage_resources {
		let stage_descriptors = canonicalize_stage_resources(stage_descriptors);
		if !stage_descriptors.is_empty() {
			if let Some(existing) = stage_argument_layouts
				.iter_mut()
				.find(|layout| stage_argument_interface_matches(layout, &stage_descriptors))
			{
				// Identical stage structs can share one immutable argument buffer at index 16.
				existing.stage |= *stage;
			} else {
				stage_argument_layouts.push(build_stage_argument_layout(device, *stage, &stage_descriptors));
			}
		}

		for descriptor in stage_descriptors {
			if let Some(existing) = resources
				.iter_mut()
				.find(|existing| existing.descriptor.slot() == descriptor.slot())
			{
				assert!(
					resource_representations_match(existing.descriptor, descriptor),
					"Conflicting pipeline resource slot. The most likely cause is that shader stages declared incompatible resources at the same flat slot.",
				);
				existing.stages |= *stage;
				existing.descriptor = crate::shader::ShaderResourceDescriptor::new(
					descriptor.slot(),
					descriptor.kind(),
					descriptor.count(),
					existing.descriptor.access() | descriptor.access(),
				)
				.texture_view_type(descriptor.texture_view())
				.buffer_stride(descriptor.buffer_element_stride());
				continue;
			}

			assert!(
				resources
					.iter()
					.all(|existing| !resource_ranges_overlap(existing.descriptor, descriptor)),
				"Overlapping pipeline resource slots. The most likely cause is that shader resource arrays reserve intersecting flat slot ranges.",
			);
			resources.push(PipelineResourceDescriptor {
				descriptor,
				stages: *stage,
			});
		}
	}

	resources.sort_by_key(|resource| resource.descriptor.slot());
	let push_constant_size = push_constant_ranges
		.iter()
		.map(|range| range.offset as usize + range.size as usize)
		.max()
		.unwrap_or(0);

	PipelineLayout {
		resources,
		stage_argument_layouts,
		push_constant_ranges: push_constant_ranges.to_vec(),
		push_constant_size,
	}
}

#[cfg(test)]
mod tests {
	#[test]
	fn vertex_bindings_stop_before_reserved_shader_buffers() {
		super::validate_vertex_binding(14);
	}

	#[test]
	#[should_panic(expected = "Metal vertex binding is reserved")]
	fn vertex_binding_fifteen_is_rejected() {
		super::validate_vertex_binding(15);
	}
}
