use dispatch2::DispatchData;
use objc2_foundation::{NSArray, NSString};
use objc2_metal::MTL4Compiler as _;
use objc2_metal::MTLComputePipelineState as _;
#[cfg(debug_assertions)]
use objc2_metal::MTLLibrary as _;
use objc2_metal::MTLVisibleFunctionTable as _;

use super::*;

/// The Metal Shading Language version source shaders compile at: 4.1, which macOS 27 introduced.
///
/// objc2-metal names versions only up to 4.0, so the value is built the way Metal encodes one: the major version in the
/// high 16 bits and the minor version in the low 16.
const LANGUAGE_VERSION: mtl::MTLLanguageVersion = mtl::MTLLanguageVersion((4 << 16) + 1);

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

/// Returns the first argument ID of each resource of `layout`, and how many IDs the layout takes.
///
/// IDs are dense and follow slot order: a resource takes one ID per array element, and a combined image sampler
/// takes its textures first and then as many samplers. An ID is also the index of the resource's 64-bit entry in the
/// set's argument buffer. The MSL generator in resource-management declares its argument-buffer structs by the same
/// rule, so every shader that declares a layout reads one buffer.
pub(crate) fn argument_bases(layout: &crate::shader::DescriptorSetLayout) -> (Box<[u32]>, u32) {
	let mut next = 0u32;
	let bases = layout
		.resources()
		.iter()
		.map(|resource| {
			let base = next;
			let ids = match resource.kind() {
				crate::shader::ResourceKind::CombinedImageSampler => resource.count().checked_mul(2),
				_ => Some(resource.count()),
			};
			next = ids.and_then(|ids| next.checked_add(ids)).expect(
				"Metal argument ID overflowed. The most likely cause is a resource array too large for one descriptor set.",
			);
			base
		})
		.collect();
	(bases, next)
}

/// The `PipelineResource` struct is one layout resource a pipeline's shaders use, with the stages that use it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct PipelineResource {
	/// The resource's position in its layout.
	pub(crate) resource_index: usize,
	pub(crate) descriptor: crate::shader::ShaderResourceDescriptor,
	pub(crate) stages: crate::Stages,
}

/// The `PipelineSet` struct is one descriptor set layout a pipeline's shaders declare, and where each stage reads it.
#[derive(Clone)]
pub(crate) struct PipelineSet {
	pub(crate) layout: crate::shader::DescriptorSetLayout,
	/// The layout resources the shaders use, ordered by position in the layout. Only they are the pipeline's
	/// accesses, and only they need a resource written.
	pub(crate) resources: Vec<PipelineResource>,
	/// The stages that read the set and the buffer index their argument tables take it at.
	pub(crate) bindings: SmallVec<[(crate::Stages, u32); 3]>,
	/// A value that is equal for two pipelines that use the same resources of the layout from the same stages, so
	/// they share the set's resource uses. See [`SetUses`].
	pub(crate) usage: u64,
}

/// The `PipelineLayout` struct exists to retain what a pipeline needs bound: the descriptor sets its shaders declare
/// and its push-constant storage.
#[derive(Clone)]
pub(crate) struct PipelineLayout {
	pub(crate) sets: Vec<PipelineSet>,
	pub(crate) push_constant_size: usize,
}

/// The `DescriptorBindingKey` struct identifies what one binding of descriptor sets was resolved from: a pipeline and
/// every bound frame-local descriptor set at its version.
///
/// Recording compares keys to decide whether an encoder's bound sets, or their retained resource uses, still apply.
#[derive(Clone, PartialEq)]
pub(crate) struct DescriptorBindingKey {
	pub(crate) pipeline: graphics_hardware_interface::PipelineHandle,
	pub(crate) sets: SmallVec<[(crate::descriptors::DescriptorSetHandle, u64); 4]>,
}

/// The `SetUses` struct keeps the resource accesses of every pipeline that uses one descriptor set the same way, so
/// hazard tracking resolves them once per set, and plans a set's reads once per encoder however many pipelines bind
/// it there.
///
/// The descriptor set retains it while the set keeps the version it records.
#[derive(Clone)]
pub(crate) struct SetUses {
	/// The [`PipelineSet::usage`] the uses were resolved for.
	pub(crate) usage: u64,
	/// The set version the uses were resolved from.
	pub(crate) version: u64,
	pub(crate) uses: std::sync::Arc<synchronization::DescriptorUses>,
}

/// The `Shader` struct keeps one loaded Metal library entry point and the resources it declares until pipelines use it.
#[derive(Clone)]
pub(crate) struct Shader {
	pub(crate) name: Option<String>,
	pub(crate) stage: crate::Stages,
	pub(crate) descriptor_sets: Vec<crate::shader::ShaderDescriptorSet>,
	pub(crate) library: Retained<ProtocolObject<dyn mtl::MTLLibrary>>,
	pub(crate) entry_point: String,
	pub(crate) threadgroup_size: Option<Extent>,
	/// The machine code of a closest-hit shader, once for each set of specialization constants a pipeline used it
	/// with. Ray tracing pipelines link these instead of compiling the shader again, so building a pipeline with one
	/// more hit group compiles only that group.
	pub(crate) binary_functions: Vec<(Box<[crate::pipelines::SpecializationMapEntry]>, BinaryFunction)>,
}

/// A visible function compiled to machine code, which any ray tracing pipeline of the device can link.
pub(crate) type BinaryFunction = Retained<ProtocolObject<dyn mtl::MTL4BinaryFunction>>;

/// The `Pipeline` struct owns one compiled Metal pipeline and the resource layout its shaders declare.
///
/// A [`Factory`] can build it away from the render thread; pass it to [`Frame::intern_raster_pipeline`],
/// [`Frame::intern_compute_pipeline`], or [`Frame::intern_ray_tracing_pipeline`] to get a handle for recording.
#[derive(Clone)]
pub struct Pipeline {
	pub(crate) pipeline: PipelineState,
	pub(crate) layout: PipelineLayout,
}

// SAFETY: Metal pipeline states, depth state, and argument encoders are immutable and documented for cross-thread use.
unsafe impl Send for Pipeline {}

impl Pipeline {
	/// Returns the threadgroup size a compute shader declared, or `None` for other pipelines.
	pub(crate) fn compute_threadgroup_size(&self) -> Option<Extent> {
		match &self.pipeline {
			PipelineState::Compute { threadgroup_size, .. } => *threadgroup_size,
			PipelineState::Raster(_) | PipelineState::RayTracing { .. } => None,
		}
	}

	/// Returns the raster state that draws apply.
	///
	/// Draws bind the render pipeline before they read it, which already rejects other pipelines, so a failure here
	/// means recording state is corrupt.
	pub(crate) fn raster(&self) -> &RasterState {
		match &self.pipeline {
			PipelineState::Raster(raster) => raster,
			PipelineState::Compute { .. } | PipelineState::RayTracing { .. } => unreachable!(
				"Metal draw state is missing. The most likely cause is that a draw read its pipeline before binding a raster pipeline."
			),
		}
	}
}

/// The `PipelineState` enum keeps only the native state each kind of pipeline sets on its encoder.
#[derive(Clone)]
pub(crate) enum PipelineState {
	Raster(RasterState),
	Compute {
		state: Retained<ProtocolObject<dyn mtl::MTLComputePipelineState>>,
		threadgroup_size: Option<Extent>,
	},
	/// Metal has no ray tracing pipeline state: the ray generation shader is a compute kernel that calls each hit's
	/// closest hit through a visible function table, which a shader binding table fills from `hit_groups`.
	RayTracing {
		state: Retained<ProtocolObject<dyn mtl::MTLComputePipelineState>>,
		/// The closest hit of each hit group, as the function a table entry of this pipeline can call.
		hit_groups: Box<[Retained<ProtocolObject<dyn mtl::MTLFunctionHandle>>]>,
	},
}

/// The `ShaderBindingTable` struct keeps the records of a shader binding table as the visible function table a trace
/// binds, so a ray generation kernel runs a hit's closest hit by indexing the table with the hit's record.
pub(crate) struct ShaderBindingTable {
	pub(crate) name: Option<String>,
	/// The pipeline the last write was for and the table it filled, or `None` before the first write.
	///
	/// Each write fills a new table, and a recording keeps the table its traces bind until they complete, so traces
	/// recorded before a write keep their records.
	pub(crate) written: Option<(
		graphics_hardware_interface::PipelineHandle,
		Retained<ProtocolObject<dyn mtl::MTLVisibleFunctionTable>>,
	)>,
}

/// Fills a new visible function table of `pipeline` whose entry `i` calls the closest hit of hit group
/// `hit_groups[i]`.
///
/// # Panics
///
/// Panics when `pipeline` isn't a ray tracing pipeline, or when an entry names a hit group the pipeline doesn't have.
pub(crate) fn build_shader_binding_table(
	pipeline: &Pipeline,
	hit_groups: &[u32],
	label: Option<&str>,
) -> Retained<ProtocolObject<dyn mtl::MTLVisibleFunctionTable>> {
	let PipelineState::RayTracing {
		state,
		hit_groups: functions,
	} = &pipeline.pipeline
	else {
		panic!(
			"Metal shader binding table write failed because the pipeline isn't a ray tracing pipeline. The most likely cause is that a raster or compute pipeline handle was passed to write_shader_binding_table. Pass the handle of the ray tracing pipeline the table's traces bind."
		);
	};
	let descriptor = mtl::MTLVisibleFunctionTableDescriptor::new();
	// Metal doesn't define a table without entries, so a write without records still makes one, which no hit indexes.
	// SAFETY: The count is at least one, and every entry the loop below fills is below it.
	unsafe { descriptor.setFunctionCount(hit_groups.len().max(1)) };
	let table = state.newVisibleFunctionTableWithDescriptor(&descriptor).expect(
		"Metal shader binding table creation failed. The most likely cause is that the device ran out of memory for function tables. Release unused shader binding tables, or write fewer records.",
	);
	for (record, &group) in hit_groups.iter().enumerate() {
		let function = functions.get(group as usize).unwrap_or_else(|| {
			panic!(
				"Metal shader binding table write failed because record {record} names hit group {group}, and the pipeline has {} hit groups. The most likely cause is that the records were made for another version of the pipeline, with more hit groups. Write the table for the pipeline whose hit groups the records name.",
				functions.len(),
			)
		});
		// SAFETY: Every closest hit the pipeline links has the signature the table's entries share, and `record` is
		// below the table's function count.
		unsafe { table.setFunction_atIndex(Some(function), record) };
	}
	if let Some(label) = label {
		table.setLabel(Some(&NSString::from_str(label)));
	}
	table
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

/// Builds the private Metal pipeline layout from the descriptor set layouts each shader stage declares.
///
/// `parameters` select the stage shaders from `shaders`, the shaders of the factory that builds the pipeline. Each
/// stage takes the sets it uses at consecutive buffer indices, in the order its shader declares them, which is the
/// order its native source declares their argument buffers. Stages that declare the same layout share its set.
fn build_pipeline_layout(
	shaders: &[Shader],
	parameters: &[crate::pipelines::ShaderParameter],
	push_constant_ranges: &[crate::pipelines::PushConstantRange],
) -> PipelineLayout {
	let mut sets = Vec::<PipelineSet>::new();

	for parameter in parameters {
		let Shader {
			stage, descriptor_sets, ..
		} = &shaders[parameter.handle.0 as usize];

		// A layout the shader uses no resource of has no argument buffer in its source, so it takes no index.
		let used_sets = descriptor_sets.iter().filter(|set| set.used() != 0);
		for (index, shader_set) in used_sets.enumerate() {
			let binding = command_buffer::ARGUMENT_BUFFER_BINDING_BASE + index as u32;
			let position = match sets
				.iter()
				.position(|set| set.layout.identity() == shader_set.layout().identity())
			{
				Some(position) => position,
				None => {
					sets.push(PipelineSet {
						layout: shader_set.layout().clone(),
						resources: Vec::new(),
						bindings: SmallVec::new(),
						usage: 0,
					});
					sets.len() - 1
				}
			};
			let set = &mut sets[position];

			match set.bindings.iter_mut().find(|(_, existing)| *existing == binding) {
				Some((stages, _)) => *stages |= *stage,
				None => set.bindings.push((*stage, binding)),
			}
			add_used_resources(set, shader_set, *stage);
		}
	}

	finish_pipeline_layout(sets, push_constant_ranges)
}

/// Builds the private Metal pipeline layout of a ray tracing pipeline.
///
/// The ray generation kernel takes every set its shader declares at consecutive buffer indices, whether it uses the
/// set or not, because it hands each set's address to the closest hits it calls. A closest hit can therefore only
/// declare sets the kernel takes, at the same positions.
///
/// # Panics
///
/// Panics when a closest-hit shader declares a set the ray generation shader doesn't declare at the same position.
fn build_ray_tracing_layout(shaders: &[Shader], builder: &crate::pipelines::ray_tracing::Builder) -> PipelineLayout {
	let ray_generation = &shaders[builder.ray_generation.handle.0 as usize];
	let mut sets: Vec<PipelineSet> = ray_generation
		.descriptor_sets
		.iter()
		.enumerate()
		.map(|(index, shader_set)| {
			let binding = command_buffer::ARGUMENT_BUFFER_BINDING_BASE + index as u32;
			let mut set = PipelineSet {
				layout: shader_set.layout().clone(),
				resources: Vec::new(),
				bindings: SmallVec::from_elem((crate::Stages::RAYGEN, binding), 1),
				usage: 0,
			};
			add_used_resources(&mut set, shader_set, crate::Stages::RAYGEN);
			set
		})
		.collect();

	for hit_group in builder.hit_groups {
		let closest_hit = &shaders[hit_group.handle.0 as usize];
		for (index, shader_set) in closest_hit.descriptor_sets.iter().enumerate() {
			let set = sets
				.get_mut(index)
				.filter(|set| set.layout.identity() == shader_set.layout().identity())
				.unwrap_or_else(|| {
					panic!(
						"Metal ray tracing pipeline creation failed because closest-hit shader {:?} declares descriptor set {index} as layout `{}`, which the ray generation shader doesn't declare at that position. The most likely cause is that the closest-hit shader declares its sets in another order than the ray generation shader, or declares a set the ray generation shader leaves out. Declare the ray generation shader's first sets in the closest-hit shader, in the same order.",
						closest_hit.name,
						shader_set.layout().name(),
					)
				});
			add_used_resources(set, shader_set, crate::Stages::CLOSEST_HIT);
		}
	}

	finish_pipeline_layout(sets, builder.push_constant_ranges)
}

/// Adds the resources `shader_set` uses to the pipeline set `set`, as used from `stage`.
fn add_used_resources(set: &mut PipelineSet, shader_set: &crate::shader::ShaderDescriptorSet, stage: crate::Stages) {
	for (resource_index, descriptor) in shader_set.used_resources() {
		match set
			.resources
			.iter_mut()
			.find(|resource| resource.resource_index == resource_index)
		{
			Some(resource) => resource.stages |= stage,
			None => set.resources.push(PipelineResource {
				resource_index,
				descriptor,
				stages: stage,
			}),
		}
	}
}

/// Orders each set's resources, names the way the pipeline uses each set, and sizes the push constant storage.
fn finish_pipeline_layout(
	mut sets: Vec<PipelineSet>,
	push_constant_ranges: &[crate::pipelines::PushConstantRange],
) -> PipelineLayout {
	for set in &mut sets {
		set.resources.sort_by_key(|resource| resource.resource_index);
		// The layout already fixes each resource's access, so the resources and their stages name the usage.
		set.usage = set.resources.iter().fold(0xcbf2_9ce4_8422_2325, |hash: u64, resource| {
			(hash ^ resource.resource_index as u64).wrapping_mul(0x0000_0100_0000_01b3) ^ resource.stages.bits().rotate_left(17)
		});
	}
	let push_constant_size = push_constant_ranges
		.iter()
		.map(|range| range.offset as usize + range.size as usize)
		.max()
		.unwrap_or(0);

	PipelineLayout {
		sets,
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
	type RayTracingPipeline = RayTracingPipeline;

	fn allocator(&self) -> &Self::Allocator {
		&std::alloc::Global
	}

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
		descriptor_sets: impl IntoIterator<Item = crate::shader::ShaderDescriptorSet>,
	) -> Result<graphics_hardware_interface::ShaderHandle, ()> {
		let (library, entry_point, threadgroup_size) = match source {
			crate::shader::Sources::SPIRV(_) | crate::shader::Sources::KeyedSPIRV { .. } => {
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
				let options = mtl::MTLCompileOptions::new();
				options.setLanguageVersion(LANGUAGE_VERSION);
				let library = self
					.device
					.newLibraryWithSource_options_error(&NSString::from_str(source), Some(&options))
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
			descriptor_sets: descriptor_sets.into_iter().collect(),
			library,
			entry_point: entry_point.to_owned(),
			threadgroup_size,
			binary_functions: Vec::new(),
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
			layout: build_pipeline_layout(&self.shaders, builder.shaders, builder.push_constant_ranges),
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
				&self.shaders,
				std::slice::from_ref(&builder.shader),
				builder.push_constant_ranges,
			),
		}
	}

	/// Links a ray tracing pipeline from this factory's shaders: the ray generation kernel, with the binary function
	/// of each hit group's closest hit.
	///
	/// The factory compiles a closest hit only the first time a pipeline uses it with a set of specialization
	/// constants, so building the pipeline again with one more hit group compiles that group and links the rest.
	fn create_ray_tracing_pipeline(&mut self, builder: crate::pipelines::ray_tracing::Builder) -> Self::RayTracingPipeline {
		let ray_generation = &self.shaders[builder.ray_generation.handle.0 as usize];
		assert!(
			ray_generation.stage == crate::Stages::RAYGEN,
			"Metal ray tracing pipeline creation failed because its ray generation shader {:?} is a {:?} shader. The most likely cause is that the shaders were passed to ray_tracing::Builder::new in the wrong order. Pass the ray generation shader first and closest-hit shaders as the hit groups.",
			ray_generation.name,
			ray_generation.stage,
		);
		assert!(
			!builder.hit_groups.is_empty(),
			"Metal ray tracing pipeline creation failed because the pipeline has no hit group. The most likely cause is that it was built before any closest-hit shader was ready. Pass at least one closest-hit shader, such as a default one for surfaces whose own shader isn't ready.",
		);
		let ray_generation_function =
			build_metal4_function_descriptor(ray_generation, builder.ray_generation.specialization_map);
		let layout = build_ray_tracing_layout(&self.shaders, &builder);
		let functions: Vec<BinaryFunction> = builder
			.hit_groups
			.iter()
			.map(|hit_group| self.closest_hit_function(hit_group))
			.collect();

		// Hit groups that share a shader and its constants share one function, which the pipeline links once.
		let mut linked = Vec::<&ProtocolObject<dyn mtl::MTL4BinaryFunction>>::with_capacity(functions.len());
		for function in &functions {
			if !linked.iter().any(|linked| std::ptr::eq(*linked, &**function)) {
				linked.push(function);
			}
		}
		let name = builder.name.filter(|_| cfg!(debug_assertions) && self.settings.debug_labels);
		let descriptor = mtl::MTL4ComputePipelineDescriptor::new();
		descriptor.setLabel(name.map(NSString::from_str).as_deref());
		descriptor.setComputeFunctionDescriptor(Some(&ray_generation_function));
		descriptor.setSupportBinaryLinking(true);
		let linking = mtl::MTL4PipelineStageDynamicLinkingDescriptor::new();
		linking.setBinaryLinkedFunctions(Some(&NSArray::from_slice(&linked)));
		// The kernel calls closest hits, and closest hits call nothing.
		linking.setMaxCallStackDepth(1);
		let state = self
			.compiler
			.newComputePipelineStateWithDescriptor_dynamicLinkingDescriptor_compilerTaskOptions_error(
				&descriptor,
				Some(&linking),
				None,
			)
			.unwrap_or_else(|error| {
				panic!(
					"Metal ray tracing pipeline linking failed: {}. The most likely cause is that the ray generation shader calls a function that isn't one of the pipeline's closest hits. Call closest hits only through the visible function table the ray generation shader takes at buffer {}.",
					error.localizedDescription(),
					command_buffer::SHADER_BINDING_TABLE_BINDING_INDEX,
				)
			});
		let hit_groups = functions
			.iter()
			.map(|function| {
				state.functionHandleWithBinaryFunction(function).expect(
					"Metal ray tracing pipeline linking returned no handle for a closest hit it links. The most likely cause is a fault in the Metal compiler. Update macOS, and report the pipeline if the fault persists.",
				)
			})
			.collect();

		Pipeline {
			pipeline: PipelineState::RayTracing { state, hit_groups },
			layout,
		}
	}
}

impl Factory {
	/// Returns the binary function of a hit group's closest-hit shader with the group's specialization constants,
	/// compiling it the first time a pipeline of this factory uses that combination.
	///
	/// A closest hit takes tens of milliseconds to compile and microseconds to link, so pipelines rebuilt as materials
	/// load compile only the closest hits they did not use before.
	fn closest_hit_function(&mut self, hit_group: &crate::pipelines::ShaderParameter) -> BinaryFunction {
		let index = hit_group.handle.0 as usize;
		let shader = &self.shaders[index];
		assert!(
			shader.stage == crate::Stages::CLOSEST_HIT,
			"Metal ray tracing pipeline creation failed because hit group shader {:?} is a {:?} shader. The most likely cause is that a shader of another stage was passed as a hit group. Pass only closest-hit shaders as hit groups.",
			shader.name,
			shader.stage,
		);
		if let Some((_, function)) = shader
			.binary_functions
			.iter()
			.find(|(constants, _)| **constants == *hit_group.specialization_map)
		{
			return function.clone();
		}

		let descriptor = mtl::MTL4BinaryFunctionDescriptor::new();
		// Metal identifies a binary function by its name, and generated shaders all name their entry point the same,
		// so the name adds the shader and the specialization.
		let name = format!("{}.{index}.{}", shader.entry_point, shader.binary_functions.len());
		descriptor.setName(&NSString::from_str(&name));
		descriptor.setFunctionDescriptor(&build_metal4_function_descriptor(shader, hit_group.specialization_map));
		let function = self
			.compiler
			.newBinaryFunctionWithDescriptor_compilerTaskOptions_error(&descriptor, None)
			.unwrap_or_else(|error| {
				panic!(
					"Metal closest-hit compilation failed for shader {:?}: {}. The most likely cause is that the shader's function isn't declared [[visible]]. Declare the closest-hit function [[visible]] in its source.",
					shader.name,
					error.localizedDescription(),
				)
			});
		self.shaders[index]
			.binary_functions
			.push((hit_group.specialization_map.into(), function.clone()));
		function
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
