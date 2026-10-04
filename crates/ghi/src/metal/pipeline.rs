use dispatch2::DispatchData;
use objc2_foundation::NSString;
use objc2_metal::MTL4Compiler as _;
#[cfg(debug_assertions)]
use objc2_metal::MTLLibrary as _;

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
	description: ImageDescription,
	debug_labels: bool,
) -> Image {
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

	Image {
		name: crate::debug_name(name),
		texture,
		description,
		staging,
		slot: None,
	}
}

/// Builds a Metal texture descriptor from GHI image creation parameters.
pub(crate) fn build_texture_descriptor(
	ImageDescription {
		extent,
		format,
		uses,
		access,
		array_layers,
		cube_compatible,
		cube_array_compatible,
		mip_levels,
	}: ImageDescription,
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
	descriptor.setResourceOptions(utils::resource_options_from_access(access));
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

/// Creates the Metal sampler state that recordings bind through descriptor writes.
pub(crate) fn build_sampler(
	device: &ProtocolObject<dyn mtl::MTLDevice>,
	builder: &crate::sampler::Builder,
	#[cfg_attr(not(debug_assertions), allow(unused_variables))] debug_labels: bool,
) -> Retained<ProtocolObject<dyn mtl::MTLSamplerState>> {
	let descriptor = mtl::MTLSamplerDescriptor::new();
	descriptor.setMinFilter(utils::sampler_min_mag_filter(builder.filtering_mode));
	descriptor.setMagFilter(utils::sampler_min_mag_filter(builder.filtering_mode));
	descriptor.setMipFilter(utils::sampler_mip_filter(builder.mip_map_mode));
	// A device that cannot execute sampler reductions falls back to standard sampling, without changing the
	// cross-backend sampler contract.
	descriptor.setReductionMode(if device.supportsFamily(mtl::MTLGPUFamily::Apple10) {
		utils::sampler_reduction_mode(builder.reduction_mode)
	} else {
		mtl::MTLSamplerReductionMode::WeightedAverage
	});
	descriptor.setSAddressMode(utils::sampler_address_mode(builder.addressing_mode));
	descriptor.setTAddressMode(utils::sampler_address_mode(builder.addressing_mode));
	descriptor.setRAddressMode(utils::sampler_address_mode(builder.addressing_mode));
	descriptor.setLodMinClamp(builder.min_lod);
	descriptor.setLodMaxClamp(builder.max_lod);
	descriptor.setSupportArgumentBuffers(true);

	if let Some(anisotropy) = builder.anisotropy {
		descriptor.setMaxAnisotropy(anisotropy as _);
	}
	// Samplers carry no name, so the label spells out the state that distinguishes them.
	#[cfg(debug_assertions)]
	if debug_labels {
		let label = format!(
			"Sampler: {:?}, mip {:?}, {:?}, {:?}",
			builder.filtering_mode, builder.mip_map_mode, builder.addressing_mode, builder.reduction_mode
		);
		descriptor.setLabel(Some(&NSString::from_str(&label)));
	}

	device
		.newSamplerStateWithDescriptor(&descriptor)
		.expect("Metal sampler creation failed. The most likely cause is that the device is out of sampler resources.")
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
	for entry in specialization_map {
		let (data_type, count) = match entry.r#type {
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
		// SAFETY: The specialization entry owns `count` contiguous values of `data_type` for the duration of this call.
		unsafe {
			constant_values.setConstantValues_type_withRange(
				NonNull::from(&*entry.value).cast(),
				data_type,
				NSRange::new(entry.constant_id as usize, count),
			)
		};
	}
	let specialized_function = mtl::MTL4SpecializedFunctionDescriptor::new();
	specialized_function.setFunctionDescriptor(Some(&library_function));
	specialized_function.setConstantValues(Some(&constant_values));
	// SAFETY: MTL4SpecializedFunctionDescriptor conforms to the MTL4FunctionDescriptor protocol.
	unsafe { Retained::cast_unchecked::<mtl::MTL4FunctionDescriptor>(specialized_function) }
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
		color_attachment.setPixelFormat(utils::to_pixel_format(attachment.format));
		let source_color_factor = match attachment.blend {
			crate::pipelines::raster::BlendMode::None => {
				color_attachment.setBlendingState(mtl::MTL4BlendState::Disabled);
				continue;
			}
			crate::pipelines::raster::BlendMode::Alpha => mtl::MTLBlendFactor::SourceAlpha,
			crate::pipelines::raster::BlendMode::Premultiplied => mtl::MTLBlendFactor::One,
		};
		color_attachment.setBlendingState(mtl::MTL4BlendState::Enabled);
		color_attachment.setRgbBlendOperation(mtl::MTLBlendOperation::Add);
		color_attachment.setAlphaBlendOperation(mtl::MTLBlendOperation::Add);
		color_attachment.setSourceRGBBlendFactor(source_color_factor);
		color_attachment.setDestinationRGBBlendFactor(mtl::MTLBlendFactor::OneMinusSourceAlpha);
		color_attachment.setSourceAlphaBlendFactor(mtl::MTLBlendFactor::One);
		color_attachment.setDestinationAlphaBlendFactor(mtl::MTLBlendFactor::OneMinusSourceAlpha);
	}
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
	fn for_each_metal_argument(&self, mut visit: impl FnMut(ArgumentSlotRange, mtl::MTLDataType)) {
		match *self {
			Self::Buffer(range) => visit(range, mtl::MTLDataType::Pointer),
			Self::Texture(range) => visit(range, mtl::MTLDataType::Texture),
			Self::Sampler(range) => visit(range, mtl::MTLDataType::Sampler),
			Self::AccelerationStructure(range) => visit(range, mtl::MTLDataType::InstanceAccelerationStructure),
			Self::CombinedImageSampler { textures, samplers } => {
				visit(textures, mtl::MTLDataType::Texture);
				visit(samplers, mtl::MTLDataType::Sampler);
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

/// The `DescriptorBindingKey` struct identifies what one argument-buffer snapshot was encoded from: a pipeline and
/// every bound frame-local descriptor set at its version.
///
/// Recording compares keys to decide whether an encoder's bound snapshot, or a retained one, still applies.
#[derive(Clone, PartialEq)]
pub(crate) struct DescriptorBindingKey {
	pub(crate) pipeline: graphics_hardware_interface::PipelineHandle,
	pub(crate) sets: SmallVec<[(crate::descriptors::DescriptorSetHandle, u64); 4]>,
}

/// The `Materialization` struct owns one encoded argument-buffer snapshot and its hazard metadata.
///
/// The first bound descriptor set retains the snapshot. It stays valid while every set in its key keeps the
/// version the key records.
#[derive(Clone)]
pub(crate) struct Materialization {
	pub(crate) key: DescriptorBindingKey,
	/// One encoded range per stage layout: the stage, its backing buffer, and the range's byte offset.
	pub(crate) argument_buffers: SmallVec<[(crate::Stages, Retained<ProtocolObject<dyn mtl::MTLBuffer>>, usize); 5]>,
	pub(crate) resource_uses: synchronization::DescriptorUses,
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
	pub(crate) layout: PipelineLayout,
}

// SAFETY: Metal pipeline states, depth state, and argument encoders are immutable and documented for cross-thread use.
unsafe impl Send for Pipeline {}

impl Pipeline {
	/// Returns the threadgroup size a compute or ray-generation shader declared, or `None` for raster pipelines.
	pub(crate) fn compute_threadgroup_size(&self) -> Option<Extent> {
		match &self.pipeline {
			PipelineState::Compute { threadgroup_size, .. } => *threadgroup_size,
			PipelineState::Raster(_) => None,
		}
	}

	/// Returns the raster state that draws apply.
	///
	/// Draws bind the render pipeline before they read it, which already rejects other pipelines, so a failure here
	/// means recording state is corrupt.
	pub(crate) fn raster(&self) -> &RasterState {
		match &self.pipeline {
			PipelineState::Raster(raster) => raster,
			PipelineState::Compute { .. } => unreachable!(
				"Metal draw state is missing. The most likely cause is that a draw read its pipeline before binding a raster pipeline."
			),
		}
	}
}

/// The `PipelineState` enum keeps only the native state each kind of pipeline sets on its encoder.
#[derive(Clone)]
pub(crate) enum PipelineState {
	Raster(RasterState),
	/// Metal has no ray-tracing pipeline state: a ray-generation function is a compute function that resolves hits
	/// through the bound acceleration structure, so ray-tracing pipelines dispatch compute state too.
	Compute {
		state: Retained<ProtocolObject<dyn mtl::MTLComputePipelineState>>,
		threadgroup_size: Option<Extent>,
	},
}

/// The `RasterState` struct keeps the render encoder state a raster pipeline applies before its draws.
#[derive(Clone)]
pub(crate) struct RasterState {
	pub(crate) state: Retained<ProtocolObject<dyn mtl::MTLRenderPipelineState>>,
	pub(crate) depth_stencil_state: Option<Retained<ProtocolObject<dyn mtl::MTLDepthStencilState>>>,
	pub(crate) face_winding: crate::pipelines::raster::FaceWinding,
	pub(crate) cull_mode: crate::pipelines::raster::CullMode,
	pub(crate) fill_mode: crate::pipelines::raster::FillMode,
	/// Present only for mesh pipelines with an object stage.
	pub(crate) object_threadgroup_size: Option<Extent>,
	/// Present only for mesh pipelines.
	pub(crate) mesh_threadgroup_size: Option<Extent>,
}

pub(crate) fn resource_ranges_overlap(
	left: crate::shader::ShaderResourceDescriptor,
	right: crate::shader::ShaderResourceDescriptor,
) -> bool {
	let (left_end, right_end) = (resource_range_end(left), resource_range_end(right));
	left.slot().index() < right_end && right.slot().index() < left_end
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
	let stored = stored_slot.index();
	stored <= descriptor.slot().index() || stored >= resource_range_end(descriptor)
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

/// Assigns stable Metal argument IDs from one flat GHI resource interval.
///
/// Slot `n` reserves IDs from `2n`: the primary range first, then a secondary range of the same length that only
/// combined image samplers use for their samplers.
pub(crate) fn allocate_argument_binding_slots(descriptor: crate::shader::ShaderResourceDescriptor) -> ArgumentBindingSlots {
	let count = descriptor.count();
	let primary = descriptor.slot().index().checked_mul(2).expect(
		"Metal argument index overflowed. The most likely cause is a flat resource slot too large for the fixed Metal ABI.",
	);
	let secondary = primary
		.checked_add(count)
		.expect("Metal argument index overflowed. The most likely cause is an invalid flat resource slot or resource count.");
	secondary.checked_add(count).expect(
		"Metal argument reservation overflowed. The most likely cause is a flat resource range too large for the fixed Metal ABI.",
	);
	let (primary, secondary) = (
		ArgumentSlotRange { base: primary, count },
		ArgumentSlotRange { base: secondary, count },
	);
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
			argument_slots.for_each_metal_argument(|range, data_type| {
				let descriptor = mtl::MTLArgumentDescriptor::argumentDescriptor();
				descriptor.setDataType(data_type);
				descriptor.setIndex(range.base as _);
				if range.count > 1 {
					descriptor.setArrayLength(range.count as _);
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
	let argument_descriptors = NSArray::from_retained_slice(&metal_argument_descriptors);
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
///
/// `parameters` select the stage shaders from `shaders`, the shaders of the factory that builds the pipeline.
fn build_pipeline_layout(
	device: &ProtocolObject<dyn mtl::MTLDevice>,
	shaders: &[Shader],
	parameters: &[crate::pipelines::ShaderParameter],
	push_constant_ranges: &[crate::pipelines::PushConstantRange],
) -> PipelineLayout {
	let mut resources = Vec::<PipelineResourceDescriptor>::new();
	let mut stage_argument_layouts = Vec::<StageArgumentLayout>::new();

	for parameter in parameters {
		let Shader {
			stage,
			shader_resource_descriptors,
			..
		} = &shaders[parameter.handle.0 as usize];
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

/// A [`Factory`] compiles every shader and pipeline, both detached ones and those a [`Context`] creates through
/// the factory it owns. Pipelines refer to shaders by the handles their own factory returned.
impl crate::device::Device for Factory {
	type Context = crate::metal::context::Context;
	type Allocator = std::alloc::Global;
	type RasterPipeline = Pipeline;
	type ComputePipeline = ComputePipeline;

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

	/// Loads or compiles one Metal shader library and records the resource interface it declares.
	fn create_shader(
		&mut self,
		name: Option<&str>,
		source: crate::shader::Sources,
		stage: crate::ShaderTypes,
		shader_resource_descriptors: impl IntoIterator<Item = crate::shader::ShaderResourceDescriptor>,
	) -> Result<graphics_hardware_interface::ShaderHandle, ()> {
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
				let library = self
					.device
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
				let library = self
					.device
					.newLibraryWithSource_options_error(&NSString::from_str(source), Some(&mtl::MTLCompileOptions::new()))
					.map_err(|error| eprintln!("Metal shader compilation failed: {}", error.localizedDescription()))?;
				(library, entry_point, threadgroup_size)
			}
		};

		// Every generated entry point is `besl_main`, so the library label is what tells shaders apart in captures.
		#[cfg(debug_assertions)]
		if let Some(name) = name.filter(|_| self.settings.debug_labels) {
			library.setLabel(Some(&NSString::from_str(name)));
		}

		self.shaders.push(Shader {
			name: crate::debug_name(name),
			stage: stage.into(),
			shader_resource_descriptors: shader_resource_descriptors.into_iter().collect(),
			library,
			entry_point: entry_point.to_owned(),
			threadgroup_size,
		});
		Ok(graphics_hardware_interface::ShaderHandle((self.shaders.len() - 1) as u64))
	}

	/// Compiles a raster pipeline from this factory's shaders, as a vertex or a mesh pipeline.
	fn create_raster_pipeline(&mut self, builder: crate::pipelines::raster::Builder) -> Self::RasterPipeline {
		let mut object_function = None;
		let mut vertex_function = None;
		let mut mesh_function = None;
		let mut fragment_function = None;
		let mut object_threadgroup_size = None;
		let mut mesh_threadgroup_size = None;
		for shader_parameter in builder.shaders.iter() {
			let shader = &self.shaders[shader_parameter.handle.0 as usize];
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

		let name = builder.name.filter(|_| cfg!(debug_assertions) && self.settings.debug_labels);
		let render_targets = builder.render_targets;
		let pipeline = if let Some(mesh_function) = mesh_function.as_deref() {
			let descriptor = mtl::MTL4MeshRenderPipelineDescriptor::new();
			descriptor.setLabel(name.map(NSString::from_str).as_deref());
			descriptor.setObjectFunctionDescriptor(object_function.as_deref());
			descriptor.setMeshFunctionDescriptor(Some(mesh_function));
			descriptor.setFragmentFunctionDescriptor(fragment_function.as_deref());
			configure_metal4_render_targets(&descriptor.colorAttachments(), render_targets);
			self.compiler
				.newRenderPipelineStateWithDescriptor_compilerTaskOptions_error(&descriptor, None)
				.unwrap_or_else(|error| {
					panic!(
						"Metal 4 mesh pipeline creation failed: {}. The most likely cause is invalid object, mesh, or fragment shader state in the pipeline descriptor.",
						error.localizedDescription(),
					)
				})
		} else if let Some(vertex_function) = vertex_function.as_deref() {
			let vertex_descriptor = build_vertex_descriptor(builder.vertex_elements);
			let descriptor = mtl::MTL4RenderPipelineDescriptor::new();
			descriptor.setLabel(name.map(NSString::from_str).as_deref());
			descriptor.setVertexFunctionDescriptor(Some(vertex_function));
			descriptor.setFragmentFunctionDescriptor(fragment_function.as_deref());
			descriptor.setVertexDescriptor(vertex_descriptor.as_deref());
			descriptor.setInputPrimitiveTopology(mtl::MTLPrimitiveTopologyClass::Triangle);
			configure_metal4_render_targets(&descriptor.colorAttachments(), render_targets);
			self.compiler
				.newRenderPipelineStateWithDescriptor_compilerTaskOptions_error(&descriptor, None)
				.unwrap_or_else(|error| {
					panic!(
						"Metal 4 raster pipeline creation failed: {}. The most likely cause is invalid shader functions or render-target state in the pipeline descriptor.",
						error.localizedDescription(),
					)
				})
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
				#[cfg(debug_assertions)]
				if self.settings.debug_labels {
					descriptor.setLabel(builder.name.map(NSString::from_str).as_deref());
				}
				self.device.newDepthStencilStateWithDescriptor(&descriptor)
			})
			.flatten();

		Pipeline {
			pipeline: PipelineState::Raster(RasterState {
				state: pipeline,
				depth_stencil_state,
				face_winding: builder.face_winding,
				cull_mode: builder.cull_mode,
				fill_mode: builder.fill_mode,
				object_threadgroup_size,
				mesh_threadgroup_size,
			}),
			layout: build_pipeline_layout(&self.device, &self.shaders, builder.shaders, builder.push_constant_ranges),
		}
	}

	/// Compiles a compute pipeline from one of this factory's compute shaders.
	fn create_compute_pipeline(&mut self, builder: crate::pipelines::compute::Builder) -> Self::ComputePipeline {
		let shader = &self.shaders[builder.shader.handle.0 as usize];
		assert!(
			shader.stage == crate::Stages::COMPUTE,
			"Metal compute pipeline creation requires a compute shader. The most likely cause is that a non-compute shader was passed to compute::Builder.",
		);
		let function = build_metal4_function_descriptor(shader, builder.shader.specialization_map);
		let name = builder.name.filter(|_| cfg!(debug_assertions) && self.settings.debug_labels);

		Pipeline {
			pipeline: PipelineState::Compute {
				state: compile_metal4_compute_pipeline(&self.compiler, name, &function),
				threadgroup_size: shader.threadgroup_size,
			},
			layout: build_pipeline_layout(
				&self.device,
				&self.shaders,
				std::slice::from_ref(&builder.shader),
				builder.push_constant_ranges,
			),
		}
	}
}

impl Factory {
	/// Compiles a ray-tracing pipeline into the compute state Metal dispatches for its ray-generation shader.
	///
	/// Metal resolves hit and miss behaviour inside the ray-generation function through the bound acceleration
	/// structure, so only that shader becomes pipeline state. Every shader still contributes to the resource layout.
	pub(crate) fn create_ray_tracing_pipeline(&self, builder: crate::pipelines::ray_tracing::Builder) -> Pipeline {
		let raygen = builder
			.shaders
			.iter()
			.find(|shader_parameter| matches!(shader_parameter.stage, crate::ShaderTypes::RayGen))
			.expect(
				"Metal ray tracing pipeline creation requires a ray generation shader. The most likely cause is that ray_tracing::Builder received only hit or miss shaders.",
			);
		let shader = &self.shaders[raygen.handle.0 as usize];
		let function = build_metal4_function_descriptor(shader, raygen.specialization_map);
		let name = shader
			.name
			.as_deref()
			.filter(|_| cfg!(debug_assertions) && self.settings.debug_labels);

		Pipeline {
			pipeline: PipelineState::Compute {
				state: compile_metal4_compute_pipeline(&self.compiler, name, &function),
				threadgroup_size: shader.threadgroup_size,
			},
			layout: build_pipeline_layout(&self.device, &self.shaders, builder.shaders, builder.push_constant_ranges),
		}
	}
}

#[cfg(test)]
mod tests {
	#[test]
	#[should_panic(expected = "Metal vertex binding is reserved")]
	fn vertex_binding_fifteen_is_rejected() {
		super::validate_vertex_binding(15);
	}
}
