use dispatch2::DispatchData;
use objc2_foundation::NSString;
use objc2_metal::MTL4Compiler as _;

use super::*;

/// Updates the CAMetalLayer's drawable size to match the view's backing size, but only when
/// the size has actually changed. Calling `setDrawableSize` unconditionally invalidates the
/// layer's drawable pool, forcing Metal to allocate new drawables every frame.
pub(crate) fn update_layer_extent(layer: &CAMetalLayer, view: &NSView) -> Extent {
	let logical_size = view.frame().size;
	let new_size = view.convertSizeToBacking(logical_size);
	let current_size = layer.drawableSize();

	if (current_size.width - new_size.width).abs() > 0.5 || (current_size.height - new_size.height).abs() > 0.5 {
		let scale_factor = if logical_size.width > 0.0 {
			(new_size.width / logical_size.width).max(1.0)
		} else if logical_size.height > 0.0 {
			(new_size.height / logical_size.height).max(1.0)
		} else {
			1.0
		};
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
	let (data_type, count) = match specialization_map_entry.get_type().as_str() {
		"bool" => (mtl::MTLDataType::Bool, 1),
		"i32" => (mtl::MTLDataType::Int, 1),
		"u32" => (mtl::MTLDataType::UInt, 1),
		"f32" => (mtl::MTLDataType::Float, 1),
		"vec2f" => (mtl::MTLDataType::Float, 2),
		"vec3f" => (mtl::MTLDataType::Float, 3),
		"vec4f" => (mtl::MTLDataType::Float, 4),
		_ => panic!(
			"Unsupported Metal specialization constant type. The most likely cause is that the Metal backend was not updated for a new specialization entry type."
		),
	};
	let range = NSRange::new(specialization_map_entry.get_constant_id() as usize, count);
	// SAFETY: The specialization entry owns `count` contiguous values of `data_type` for the duration of this call.
	unsafe { constant_values.setConstantValues_type_withRange(value, data_type, range) };
}

/// Rejects vertex attributes that overlap the fixed push-constant and nested argument-buffer bindings.
pub(crate) fn validate_vertex_binding(binding: u32) {
	assert!(
		binding < command_buffer::PUSH_CONSTANT_BINDING_INDEX,
		"Metal vertex binding is reserved. The most likely cause is that a vertex attribute uses binding 15 or higher. binding={binding}",
	);
}

/// Builds the Metal vertex descriptor for a raster pipeline, or `None` when the pipeline reads no vertex attributes.
fn build_vertex_descriptor(vertex_elements: &[crate::pipelines::VertexElement]) -> Option<Retained<mtl::MTLVertexDescriptor>> {
	let max_binding = vertex_elements.iter().map(|element| element.binding as usize + 1).max()?;
	let mut strides = vec![0usize; max_binding];
	let vertex_descriptor = mtl::MTLVertexDescriptor::vertexDescriptor();

	for (attribute_index, element) in vertex_elements.iter().enumerate() {
		validate_vertex_binding(element.binding);
		let stride = &mut strides[element.binding as usize];
		// SAFETY: Metal vertex descriptor arrays materialize entries for every valid attribute index.
		let attribute = unsafe { vertex_descriptor.attributes().objectAtIndexedSubscript(attribute_index as _) };
		attribute.setFormat(utils::vertex_format(element.format));
		// SAFETY: The offset is the running size of this binding's earlier attributes, so it stays inside its stride.
		unsafe { attribute.setOffset(*stride as _) };
		// SAFETY: `validate_vertex_binding` excludes Metal's reserved buffer indices.
		unsafe { attribute.setBufferIndex(element.binding as _) };
		*stride += element.format.size();
	}

	for (binding, stride) in strides.into_iter().enumerate() {
		// SAFETY: Each binding came from a vertex element and therefore has a corresponding layout entry.
		let layout = unsafe { vertex_descriptor.layouts().objectAtIndexedSubscript(binding as _) };
		// SAFETY: The stride is the checked sum of this binding's attribute sizes.
		unsafe { layout.setStride(stride as _) };
		// SAFETY: A per-vertex layout advances once for every vertex.
		unsafe { layout.setStepRate(1) };
		layout.setStepFunction(mtl::MTLVertexStepFunction::PerVertex);
	}

	Some(vertex_descriptor)
}

/// Creates a Metal image, with a host staging copy when the CPU may access it.
///
/// Both [`Context`] and [`Factory`] create images through this function.
pub(crate) fn build_image(
	device: &ProtocolObject<dyn mtl::MTLDevice>,
	name: Option<&str>,
	description: image::ImageDescription,
	debug_labels: bool,
) -> image::Image {
	let texture = device
		.newTextureWithDescriptor(&build_texture_descriptor(description))
		.expect("Metal texture creation failed. The most likely cause is that the device is out of memory.");

	#[cfg(debug_assertions)]
	if let Some(name) = name.filter(|_| debug_labels) {
		texture.setLabel(Some(&NSString::from_str(name)));
	}

	// Device-only images are written through uploads or transfers, so they need no host copy.
	let staging = description
		.access
		.intersects(crate::DeviceAccesses::CpuRead | crate::DeviceAccesses::CpuWrite)
		.then(|| {
			let (_, _, bytes_per_image) = utils::texture_upload_layout(description.format, description.extent);
			vec![0u8; bytes_per_image * description.extent.depth().max(1) as usize * description.array_layers as usize]
		});

	image::Image {
		name: crate::debug_name(name),
		texture,
		description,
		staging,
	}
}

/// Builds a Metal texture descriptor from GHI image creation parameters.
fn build_texture_descriptor(
	image::ImageDescription {
		extent,
		format,
		uses,
		access,
		array_layers,
		cube_compatible,
		cube_array_compatible,
		mip_levels,
	}: image::ImageDescription,
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
	// SAFETY: The format and nonzero physical dimensions form a valid Metal 2D texture descriptor.
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
	descriptor.setUsage(utils::texture_usage_from_uses(uses));
	descriptor.setStorageMode(utils::storage_mode_from_access(access));
	let array_length = if cube_compatible {
		1
	} else if cube_array_compatible {
		array_layers / 6
	} else {
		array_layers
	};
	// SAFETY: Cube validation and the image builder guarantee a valid native array length.
	unsafe { descriptor.setArrayLength(array_length as _) };
	// SAFETY: The image builder supplies at least one mip level and validates it against the extent.
	unsafe { descriptor.setMipmapLevelCount(mip_levels as _) };

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

/// Creates a Metal sampler, falling back to standard sampling when the device cannot execute sampler reductions.
pub(crate) fn build_sampler(
	device: &ProtocolObject<dyn mtl::MTLDevice>,
	builder: &crate::sampler::Builder,
) -> sampler::Sampler {
	let descriptor = build_sampler_descriptor(builder);
	let reduction_mode =
		sampler_reduction_mode_for_device(descriptor.reductionMode(), device.supportsFamily(mtl::MTLGPUFamily::Apple10));
	descriptor.setReductionMode(reduction_mode);

	sampler::Sampler {
		sampler: device
			.newSamplerStateWithDescriptor(&descriptor)
			.expect("Metal sampler creation failed. The most likely cause is that the device is out of sampler resources."),
	}
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
fn build_metal4_function_descriptor(
	shader: &Shader,
	specialization_map: &[crate::pipelines::SpecializationMapEntry],
) -> Retained<mtl::MTL4FunctionDescriptor> {
	let library_function = mtl::MTL4LibraryFunctionDescriptor::new();
	library_function.setLibrary(Some(shader.library.as_ref()));
	library_function.setName(Some(&NSString::from_str(&shader.entry_point)));
	// SAFETY: MTL4LibraryFunctionDescriptor conforms to the MTL4FunctionDescriptor protocol consumed by pipeline descriptors.
	let library_function = unsafe { Retained::cast_unchecked::<mtl::MTL4FunctionDescriptor>(library_function) };

	if specialization_map.is_empty() {
		return library_function;
	}

	let constant_values = mtl::MTLFunctionConstantValues::new();
	for specialization in specialization_map {
		apply_specialization_map_entry(&constant_values, specialization);
	}
	let specialized_function = mtl::MTL4SpecializedFunctionDescriptor::new();
	specialized_function.setFunctionDescriptor(Some(&library_function));
	specialized_function.setConstantValues(Some(&constant_values));
	// SAFETY: MTL4SpecializedFunctionDescriptor conforms to the MTL4FunctionDescriptor protocol.
	unsafe { Retained::cast_unchecked::<mtl::MTL4FunctionDescriptor>(specialized_function) }
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
		// SAFETY: The index is bounded by the filtered render-target slice and Metal exposes at least that many attachment slots.
		let color_attachment = unsafe { color_attachments.objectAtIndexedSubscript(index as _) };
		configure_metal4_color_attachment(&color_attachment, attachment);
	}
}

/// Compiles one Metal 4 vertex/fragment render pipeline.
fn compile_metal4_render_pipeline(
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
fn compile_metal4_mesh_pipeline(
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
fn compile_metal4_compute_pipeline(
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
	pub(crate) push_constant_size: usize,
}

/// The `Materialization` struct owns one encoded argument-buffer snapshot and its hazard metadata.
///
/// The first bound descriptor set retains the snapshot. It stays valid while
/// every set in `descriptor_sets` keeps the version recorded in `versions`.
#[derive(Clone)]
pub(crate) struct Materialization {
	pub(crate) pipeline: graphics_hardware_interface::PipelineHandle,
	pub(crate) descriptor_sets: SmallVec<[crate::descriptors::DescriptorSetHandle; 4]>,
	pub(crate) versions: SmallVec<[u64; 4]>,
	/// One encoded range per stage layout: the stage, its backing buffer, and the range's byte offset.
	pub(crate) argument_buffers: SmallVec<[(crate::Stages, Retained<ProtocolObject<dyn mtl::MTLBuffer>>, usize); 5]>,
	pub(crate) resource_uses: SmallVec<[synchronization::MetalResourceUse; 16]>,
	// Metal argument buffers do not retain texture views. Keep selected mip views alive with their bindings.
	pub(crate) _texture_views: SmallVec<[Retained<ProtocolObject<dyn mtl::MTLTexture>>; 4]>,
}

/// The `Shader` struct keeps one loaded Metal library entry point and the resources it declares until pipelines use it.
#[derive(Clone)]
pub(crate) struct Shader {
	pub(crate) name: Option<String>,
	pub(crate) stage: crate::Stages,
	pub(crate) shader_resource_descriptors: Vec<crate::shader::ShaderResourceDescriptor>,
	pub(crate) library: Retained<ProtocolObject<dyn mtl::MTLLibrary>>,
	pub(crate) entry_point: String,
	pub(crate) threadgroup_size: Option<Extent>,
}

/// The `Pipeline` struct owns one compiled Metal pipeline and the resource layout its shaders declare.
///
/// A [`Factory`] can build it away from the render thread; pass it to [`Frame::intern_raster_pipeline`] or
/// [`Frame::intern_compute_pipeline`] to get a handle for recording.
#[derive(Clone)]
pub struct Pipeline {
	pub(crate) pipeline: PipelineState,
	pub(crate) depth_stencil_state: Option<Retained<ProtocolObject<dyn mtl::MTLDepthStencilState>>>,
	pub(crate) layout: PipelineLayout,
	pub(crate) compute_threadgroup_size: Option<Extent>,
	pub(crate) object_threadgroup_size: Option<Extent>,
	pub(crate) mesh_threadgroup_size: Option<Extent>,
	pub(crate) face_winding: crate::pipelines::raster::FaceWinding,
	pub(crate) cull_mode: crate::pipelines::raster::CullMode,
	pub(crate) fill_mode: crate::pipelines::raster::FillMode,
}

// SAFETY: Metal pipeline states, depth state, and argument encoders are immutable and documented for cross-thread use.
unsafe impl Send for Pipeline {}

impl Pipeline {
	/// Wraps compute state, which also serves ray-generation dispatches, with default raster state it never reads.
	fn compute(pipeline: PipelineState, layout: PipelineLayout, compute_threadgroup_size: Option<Extent>) -> Self {
		Self {
			pipeline,
			depth_stencil_state: None,
			layout,
			compute_threadgroup_size,
			object_threadgroup_size: None,
			mesh_threadgroup_size: None,
			face_winding: crate::pipelines::raster::FaceWinding::Clockwise,
			cull_mode: crate::pipelines::raster::CullMode::Back,
			fill_mode: crate::pipelines::raster::FillMode::Solid,
		}
	}
}

#[derive(Clone)]
pub(crate) enum PipelineState {
	Raster(Retained<ProtocolObject<dyn mtl::MTLRenderPipelineState>>),
	Compute(Retained<ProtocolObject<dyn mtl::MTLComputePipelineState>>),
	/// Metal has no ray-tracing pipeline state: a ray-generation function is a compute function that resolves hits
	/// through the bound acceleration structure, so tracing rays dispatches this compute state.
	RayTracing(Retained<ProtocolObject<dyn mtl::MTLComputePipelineState>>),
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

/// Returns `descriptor` with the access of `other` added. Both must describe the same resource representation.
fn with_merged_access(
	descriptor: crate::shader::ShaderResourceDescriptor,
	other: crate::shader::ShaderResourceDescriptor,
) -> crate::shader::ShaderResourceDescriptor {
	crate::shader::ShaderResourceDescriptor::new(
		descriptor.slot(),
		descriptor.kind(),
		descriptor.count(),
		descriptor.access() | other.access(),
	)
	.texture_view_type(descriptor.texture_view())
	.buffer_stride(descriptor.buffer_element_stride())
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
				*previous = with_merged_access(*previous, descriptor);
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
	secondary.checked_add(count).expect(
		"Metal argument reservation overflowed. The most likely cause is a flat resource range too large for the fixed Metal ABI.",
	);
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
fn build_pipeline_layout<'a>(
	device: &ProtocolObject<dyn mtl::MTLDevice>,
	shaders: impl IntoIterator<Item = &'a Shader>,
	push_constant_ranges: &[crate::pipelines::PushConstantRange],
) -> PipelineLayout {
	let mut resources = Vec::<PipelineResourceDescriptor>::new();
	let mut stage_argument_layouts = Vec::<StageArgumentLayout>::new();

	for Shader {
		stage,
		shader_resource_descriptors,
		..
	} in shaders
	{
		let stage_descriptors = canonicalize_stage_resources(shader_resource_descriptors);
		if !stage_descriptors.is_empty() {
			if let Some(existing) = stage_argument_layouts.iter_mut().find(|layout| {
				layout
					.bindings
					.iter()
					.map(|binding| binding.descriptor)
					.eq(stage_descriptors.iter().copied())
			}) {
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
				existing.descriptor = with_merged_access(existing.descriptor, descriptor);
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
		push_constant_size,
	}
}

/// Loads or compiles one Metal shader library and records the resource interface it declares.
///
/// Both [`Context`] and [`Factory`] create shaders through this function; pipelines refer to the result by index.
pub(crate) fn build_shader(
	device: &ProtocolObject<dyn mtl::MTLDevice>,
	name: Option<&str>,
	source: crate::shader::Sources,
	stage: crate::ShaderTypes,
	shader_resource_descriptors: impl IntoIterator<Item = crate::shader::ShaderResourceDescriptor>,
) -> Result<Shader, ()> {
	let (library, entry_point, threadgroup_size) = match source {
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
			let library = device
				.newLibraryWithData_error(&DispatchData::from_bytes(binary))
				.map_err(|error| eprintln!("Metal shader library load failed: {}", error.localizedDescription()))?;
			(library, entry_point, threadgroup_size)
		}
		crate::shader::Sources::MTL { source, entry_point } => {
			let threadgroup_size = matches!(
				stage,
				crate::ShaderTypes::Task | crate::ShaderTypes::Mesh | crate::ShaderTypes::Compute
			)
			.then(|| utils::parse_threadgroup_size_metadata(source))
			.flatten();
			let library = device
				.newLibraryWithSource_options_error(&NSString::from_str(source), Some(&mtl::MTLCompileOptions::new()))
				.map_err(|error| eprintln!("Metal shader compilation failed: {}", error.localizedDescription()))?;
			(library, entry_point, threadgroup_size)
		}
	};

	Ok(Shader {
		name: crate::debug_name(name),
		stage: stage.into(),
		shader_resource_descriptors: shader_resource_descriptors.into_iter().collect(),
		library,
		entry_point: entry_point.to_owned(),
		threadgroup_size,
	})
}

/// Compiles a raster pipeline from shaders created by the same [`Context`] or [`Factory`].
pub(crate) fn build_raster_pipeline(
	device: &ProtocolObject<dyn mtl::MTLDevice>,
	compiler: &ProtocolObject<dyn mtl::MTL4Compiler>,
	shaders: &[Shader],
	debug_labels: bool,
	builder: crate::pipelines::raster::Builder,
) -> Pipeline {
	let mut object_function = None;
	let mut vertex_function = None;
	let mut mesh_function = None;
	let mut fragment_function = None;
	let mut object_threadgroup_size = None;
	let mut mesh_threadgroup_size = None;
	for shader_parameter in builder.shaders.iter() {
		let shader = &shaders[shader_parameter.handle.0 as usize];
		let function = Some(build_metal4_function_descriptor(shader, shader_parameter.specialization_map));
		match shader_parameter.stage {
			crate::ShaderTypes::Task => {
				object_function = function;
				object_threadgroup_size = Some(shader.threadgroup_size.unwrap_or(Extent::new(1, 1, 1)));
			}
			crate::ShaderTypes::Vertex => vertex_function = function,
			crate::ShaderTypes::Mesh => {
				mesh_function = function;
				mesh_threadgroup_size = shader.threadgroup_size;
			}
			crate::ShaderTypes::Fragment => fragment_function = function,
			_ => {}
		}
	}

	let name = builder.name.filter(|_| cfg!(debug_assertions) && debug_labels);
	let render_targets = builder.render_targets.as_ref();
	let pipeline = if let Some(mesh_function) = mesh_function.as_deref() {
		compile_metal4_mesh_pipeline(
			compiler,
			name,
			object_function.as_deref(),
			mesh_function,
			fragment_function.as_deref(),
			render_targets,
		)
	} else if let Some(vertex_function) = vertex_function.as_deref() {
		let vertex_descriptor = build_vertex_descriptor(builder.vertex_elements.as_ref());
		compile_metal4_render_pipeline(
			compiler,
			name,
			vertex_function,
			fragment_function.as_deref(),
			vertex_descriptor.as_deref(),
			render_targets,
		)
	} else {
		panic!(
			"Metal raster pipeline creation failed because no vertex or mesh shader was supplied. The most likely cause is that the raster pipeline builder received only task or fragment shaders. Pipeline: {:?}",
			builder.name,
		);
	};

	let has_depth_attachment = render_targets
		.iter()
		.any(|attachment| attachment.format.channel_layout() == crate::ChannelLayout::Depth);
	let depth_stencil_state = has_depth_attachment
		.then(|| {
			let descriptor = mtl::MTLDepthStencilDescriptor::new();
			descriptor.setDepthCompareFunction(mtl::MTLCompareFunction::GreaterEqual);
			descriptor.setDepthWriteEnabled(builder.depth_write);
			device.newDepthStencilStateWithDescriptor(&descriptor)
		})
		.flatten();

	Pipeline {
		pipeline: PipelineState::Raster(pipeline),
		depth_stencil_state,
		layout: build_pipeline_layout(
			device,
			builder
				.shaders
				.iter()
				.map(|shader_parameter| &shaders[shader_parameter.handle.0 as usize]),
			builder.push_constant_ranges.as_ref(),
		),
		compute_threadgroup_size: None,
		object_threadgroup_size,
		mesh_threadgroup_size,
		face_winding: builder.face_winding,
		cull_mode: builder.cull_mode,
		fill_mode: builder.fill_mode,
	}
}

/// Compiles a compute pipeline from a shader created by the same [`Context`] or [`Factory`].
pub(crate) fn build_compute_pipeline(
	device: &ProtocolObject<dyn mtl::MTLDevice>,
	compiler: &ProtocolObject<dyn mtl::MTL4Compiler>,
	shaders: &[Shader],
	debug_labels: bool,
	builder: crate::pipelines::compute::Builder,
) -> Pipeline {
	let shader = &shaders[builder.shader.handle.0 as usize];
	assert!(
		shader.stage == crate::Stages::COMPUTE,
		"Metal compute pipeline creation requires a compute shader. The most likely cause is that a non-compute shader was passed to compute::Builder.",
	);
	let function = build_metal4_function_descriptor(shader, builder.shader.specialization_map);
	let name = builder.name.filter(|_| cfg!(debug_assertions) && debug_labels);

	Pipeline::compute(
		PipelineState::Compute(compile_metal4_compute_pipeline(compiler, name, &function)),
		build_pipeline_layout(device, [shader], builder.push_constant_ranges),
		shader.threadgroup_size,
	)
}

/// Compiles a ray-tracing pipeline into the compute state Metal dispatches for its ray-generation shader.
///
/// Metal resolves hit and miss behaviour inside the ray-generation function through the bound acceleration
/// structure, so only that shader becomes pipeline state. Every shader still contributes to the resource layout.
pub(crate) fn build_ray_tracing_pipeline(
	device: &ProtocolObject<dyn mtl::MTLDevice>,
	compiler: &ProtocolObject<dyn mtl::MTL4Compiler>,
	shaders: &[Shader],
	debug_labels: bool,
	builder: crate::pipelines::ray_tracing::Builder,
) -> Pipeline {
	let raygen = builder
		.shaders
		.iter()
		.find(|shader_parameter| matches!(shader_parameter.stage, crate::ShaderTypes::RayGen))
		.expect(
			"Metal ray tracing pipeline creation requires a ray generation shader. The most likely cause is that ray_tracing::Builder received only hit or miss shaders.",
		);
	let shader = &shaders[raygen.handle.0 as usize];
	let function = build_metal4_function_descriptor(shader, raygen.specialization_map);
	let name = shader.name.as_deref().filter(|_| cfg!(debug_assertions) && debug_labels);

	Pipeline::compute(
		PipelineState::RayTracing(compile_metal4_compute_pipeline(compiler, name, &function)),
		build_pipeline_layout(
			device,
			builder
				.shaders
				.iter()
				.map(|shader_parameter| &shaders[shader_parameter.handle.0 as usize]),
			builder.push_constant_ranges.as_ref(),
		),
		shader.threadgroup_size,
	)
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
