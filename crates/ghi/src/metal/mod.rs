#[link(name = "CoreGraphics", kind = "framework")]
extern "C" {}

use std::cell::RefCell;
use std::ffi::c_void;
use std::ptr::NonNull;
use std::sync::atomic::AtomicU64;

use ::utils::hash::HashMap;
use ::utils::Extent;
use objc2::rc::Retained;
use objc2::runtime::ProtocolObject;
use objc2_app_kit::NSView;
use objc2_foundation::{NSArray, NSRange, NSSize};
use objc2_metal as mtl;
use objc2_metal::MTLArgumentEncoder as _;
use objc2_metal::MTLDevice as _;
use objc2_quartz_core::{CAMetalDrawable, CAMetalLayer};
use smallvec::SmallVec;

use crate::buffer::BufferHandle;
use crate::graphics_hardware_interface;
use crate::image::ImageHandle;
use crate::sampler::SamplerHandle;
use crate::PrivateHandles;

#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub(super) enum Descriptor {
	Image {
		image: ImageHandle,
		layout: crate::Layouts,
	},
	CombinedImageSampler {
		image: ImageHandle,
		sampler: SamplerHandle,
		layout: crate::Layouts,
	},
	Buffer {
		buffer: BufferHandle,
		size: graphics_hardware_interface::Ranges,
	},
	Sampler {
		sampler: SamplerHandle,
	},
	Swapchain {
		handle: crate::swapchain::SwapchainHandle,
	},
	AccelerationStructure {
		handle: TopLevelAccelerationStructureHandle,
	},
}

impl Descriptor {
	pub(super) fn tracked_resource(self) -> Option<PrivateHandles> {
		match self {
			Descriptor::Buffer { buffer, .. } => Some(PrivateHandles::Buffer(buffer)),
			Descriptor::Image { image, .. } => Some(PrivateHandles::Image(image)),
			Descriptor::CombinedImageSampler { image, .. } => Some(PrivateHandles::Image(image)),
			Descriptor::Sampler { .. } => None,
			Descriptor::Swapchain { handle } => Some(PrivateHandles::Swapchain(handle)),
			Descriptor::AccelerationStructure { .. } => None,
		}
	}
}

#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub(super) struct TopLevelAccelerationStructureHandle(pub(super) u64);

#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub(super) struct BottomLevelAccelerationStructureHandle(pub(super) u64);

#[derive(Clone, Copy, PartialEq)]
pub(super) struct Consumption {
	pub(super) handle: PrivateHandles,
	pub(super) stages: crate::Stages,
	pub(super) access: crate::AccessPolicies,
	pub(super) layout: crate::Layouts,
}

#[derive(Clone, PartialEq)]
pub(super) struct MetalConsumption {
	pub(super) handle: PrivateHandles,
	pub(super) stages: crate::Stages,
	pub(super) access: crate::AccessPolicies,
	pub(super) layout: crate::Layouts,
}

const MAX_FRAMES_IN_FLIGHT: usize = 3;
const MAX_SWAPCHAIN_IMAGES: usize = 8;

/// Returns the previous drawable size, new drawable size, and scale factor.
fn get_layer_sizes(layer: &CAMetalLayer, view: &NSView) -> (NSSize, NSSize, f64) {
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

fn get_layer_extent(layer: &CAMetalLayer, view: &NSView) -> Extent {
	let (_, new_size, _) = get_layer_sizes(layer, view);

	Extent::rectangle(
		new_size.width.round().max(0.0) as u32,
		new_size.height.round().max(0.0) as u32,
	)
}

/// Updates the CAMetalLayer's drawable size to match the view's backing size, but only when
/// the size has actually changed. Calling `setDrawableSize` unconditionally invalidates the
/// layer's drawable pool, forcing Metal to allocate new drawables every frame.
fn update_layer_extent(layer: &CAMetalLayer, view: &NSView) -> Extent {
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
fn apply_specialization_map_entry(
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

/// Builds the Metal vertex descriptor and matching GHI vertex-layout metadata.
fn build_vertex_layout(vertex_elements: &[crate::pipelines::VertexElement]) -> VertexLayout {
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
fn build_texture_descriptor(
	format: crate::Formats,
	extent: Extent,
	resource_uses: crate::Uses,
	device_accesses: crate::DeviceAccesses,
	array_layers: u32,
	mip_levels: u32,
) -> Retained<mtl::MTLTextureDescriptor> {
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
	} else if array_layers > 1 {
		descriptor.setTextureType(mtl::MTLTextureType::Type2DArray);
	}
	descriptor.setUsage(utils::texture_usage_from_uses(resource_uses));
	descriptor.setStorageMode(utils::storage_mode_from_access(device_accesses));
	unsafe {
		descriptor.setArrayLength(array_layers as _);
	}

	descriptor
}

/// Builds a Metal sampler descriptor from a GHI sampler builder.
fn build_sampler_descriptor(builder: &crate::sampler::Builder) -> Retained<mtl::MTLSamplerDescriptor> {
	let descriptor = mtl::MTLSamplerDescriptor::new();
	descriptor.setMinFilter(utils::sampler_min_mag_filter(builder.filtering_mode));
	descriptor.setMagFilter(utils::sampler_min_mag_filter(builder.filtering_mode));
	descriptor.setMipFilter(utils::sampler_mip_filter(builder.mip_map_mode));
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

/// Configures one Metal color attachment with the GHI format and blend mode.
fn configure_color_attachment(
	color_attachment: &mtl::MTLRenderPipelineColorAttachmentDescriptor,
	attachment: &crate::pipelines::raster::AttachmentDescriptor,
) {
	color_attachment.setPixelFormat(utils::to_pixel_format(attachment.format));
	match attachment.blend {
		crate::pipelines::raster::BlendMode::None => color_attachment.setBlendingEnabled(false),
		crate::pipelines::raster::BlendMode::Alpha => {
			color_attachment.setBlendingEnabled(true);
			color_attachment.setRgbBlendOperation(mtl::MTLBlendOperation::Add);
			color_attachment.setAlphaBlendOperation(mtl::MTLBlendOperation::Add);
			color_attachment.setSourceRGBBlendFactor(mtl::MTLBlendFactor::SourceAlpha);
			color_attachment.setDestinationRGBBlendFactor(mtl::MTLBlendFactor::OneMinusSourceAlpha);
			color_attachment.setSourceAlphaBlendFactor(mtl::MTLBlendFactor::One);
			color_attachment.setDestinationAlphaBlendFactor(mtl::MTLBlendFactor::OneMinusSourceAlpha);
		}
	}
}

/// Configures render-target formats for a Metal mesh pipeline descriptor.
fn configure_mesh_render_targets(
	descriptor: &mtl::MTLMeshRenderPipelineDescriptor,
	render_targets: &[crate::pipelines::raster::AttachmentDescriptor],
) {
	for (index, attachment) in render_targets.iter().enumerate() {
		if attachment.format.channel_layout() == crate::ChannelLayout::Depth {
			descriptor.setDepthAttachmentPixelFormat(utils::to_pixel_format(attachment.format));
		} else {
			let color_attachment = unsafe { descriptor.colorAttachments().objectAtIndexedSubscript(index as _) };
			configure_color_attachment(&color_attachment, attachment);
		}
	}
}

/// Configures render-target formats for a Metal raster pipeline descriptor.
fn configure_render_targets(
	descriptor: &mtl::MTLRenderPipelineDescriptor,
	render_targets: &[crate::pipelines::raster::AttachmentDescriptor],
) {
	for (index, attachment) in render_targets.iter().enumerate() {
		if attachment.format.channel_layout() == crate::ChannelLayout::Depth {
			descriptor.setDepthAttachmentPixelFormat(utils::to_pixel_format(attachment.format));
		} else {
			let color_attachment = unsafe { descriptor.colorAttachments().objectAtIndexedSubscript(index as _) };
			configure_color_attachment(&color_attachment, attachment);
		}
	}
}

/// The `StageArgumentLayout` struct exists to translate one shader stage's flat resource interface into Metal argument IDs.
#[derive(Clone)]
pub(crate) struct StageArgumentLayout {
	stage: crate::Stages,
	bindings: Vec<StageArgumentBinding>,
	argument_encoder: Retained<ProtocolObject<dyn mtl::MTLArgumentEncoder>>,
	encoded_length: usize,
}

/// The `StageArgumentBinding` struct exists to retain the Metal argument IDs assigned to one flat resource slot.
#[derive(Clone)]
pub(crate) struct StageArgumentBinding {
	descriptor: crate::shader::ShaderResourceDescriptor,
	argument_slots: ArgumentBindingSlots,
}

/// The `ArgumentSlotRange` struct identifies one dense run of native argument IDs.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct ArgumentSlotRange {
	base: u32,
	count: u32,
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
	descriptor: crate::shader::ShaderResourceDescriptor,
	stages: crate::Stages,
}

/// The `PipelineLayout` struct exists to retain the native resource layouts derived from a pipeline's shaders.
#[derive(Clone)]
pub(crate) struct PipelineLayout {
	resources: Vec<PipelineResourceDescriptor>,
	stage_argument_layouts: Vec<StageArgumentLayout>,
	push_constant_ranges: Vec<crate::pipelines::PushConstantRange>,
	push_constant_size: usize,
}

/// The `MaterializationKey` struct identifies one pipeline's frame-resolved union of retained descriptor sets.
#[derive(Clone, PartialEq, Eq, Hash)]
pub(crate) struct MaterializationKey {
	descriptor_sets: SmallVec<[crate::descriptors::DescriptorSetHandle; 4]>,
	sequence_index: u8,
}

/// The `Materialization` struct retains immutable native argument buffers until their logical set versions change.
#[derive(Clone)]
pub(crate) struct Materialization {
	versions: SmallVec<[u64; 4]>,
	argument_buffers: SmallVec<[(crate::Stages, Retained<ProtocolObject<dyn mtl::MTLBuffer>>); 5]>,
}

#[derive(Clone)]
pub(crate) struct VertexLayout {
	elements: Vec<VertexElementDescriptor>,
	strides: Vec<u32>,
	vertex_descriptor: Retained<mtl::MTLVertexDescriptor>,
}

#[derive(Clone, PartialEq, Eq, Hash)]
struct VertexLayoutKey {
	elements: Vec<VertexElementDescriptor>,
}

#[derive(Clone, PartialEq, Eq, Hash)]
pub(crate) struct VertexElementDescriptor {
	name: String,
	format: crate::DataTypes,
	binding: u32,
}

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub(crate) struct VertexLayoutHandle(pub(crate) u64);

#[derive(Clone)]
pub(crate) struct Shader {
	name: Option<String>,
	stage: crate::Stages,
	shader_resource_descriptors: Vec<crate::shader::ShaderResourceDescriptor>,
	metal_library: Option<Retained<ProtocolObject<dyn mtl::MTLLibrary>>>,
	metal_entry_point: Option<String>,
	threadgroup_size: Option<Extent>,
}

#[derive(Clone)]
pub(crate) struct Pipeline {
	pipeline: PipelineState,
	depth_stencil_state: Option<Retained<ProtocolObject<dyn mtl::MTLDepthStencilState>>>,
	layout: graphics_hardware_interface::PipelineLayoutHandle,
	vertex_layout: Option<VertexLayoutHandle>,
	shader_handles: HashMap<graphics_hardware_interface::ShaderHandle, [u8; 32]>,
	materializations: RefCell<HashMap<MaterializationKey, Materialization>>,
	compute_threadgroup_size: Option<Extent>,
	object_threadgroup_size: Option<Extent>,
	mesh_threadgroup_size: Option<Extent>,
	face_winding: crate::pipelines::raster::FaceWinding,
	cull_mode: crate::pipelines::raster::CullMode,
}

#[derive(Clone)]
pub(crate) enum PipelineState {
	Raster(Option<Retained<ProtocolObject<dyn mtl::MTLRenderPipelineState>>>),
	Compute(Option<Retained<ProtocolObject<dyn mtl::MTLComputePipelineState>>>),
	RayTracing,
}

fn resource_ranges_overlap(
	left: crate::shader::ShaderResourceDescriptor,
	right: crate::shader::ShaderResourceDescriptor,
) -> bool {
	let left_start = left.slot().index();
	let left_end = resource_range_end(left);
	let right_start = right.slot().index();
	let right_end = resource_range_end(right);
	left_start < right_end && right_start < left_end
}

fn resource_range_end(descriptor: crate::shader::ShaderResourceDescriptor) -> u32 {
	descriptor
		.slot()
		.index()
		.checked_add(descriptor.count())
		.expect("Metal shader resource range overflowed. The most likely cause is an invalid flat slot or resource count.")
}

fn resource_accepts_retained_slot_key(
	descriptor: crate::shader::ShaderResourceDescriptor,
	stored_slot: crate::shader::ResourceSlot,
) -> bool {
	let base = descriptor.slot().index();
	let stored = stored_slot.index();
	stored <= base || stored >= resource_range_end(descriptor)
}

fn resource_representations_match(
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
fn canonicalize_stage_resources(
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

/// Reserves a dense run of native argument IDs for one shader resource array.
fn reserve_argument_slots(next_argument_index: &mut u32, count: u32) -> ArgumentSlotRange {
	let start = *next_argument_index;
	let end = start
		.checked_add(count)
		.expect("Metal argument index range overflowed. The most likely cause is an invalid shader resource count.");
	*next_argument_index = end;
	ArgumentSlotRange { base: start, count }
}

/// Assigns the dense Metal argument IDs needed to represent one flat GHI resource.
fn allocate_argument_binding_slots(
	kind: crate::shader::ResourceKind,
	count: u32,
	next_argument_index: &mut u32,
) -> ArgumentBindingSlots {
	match kind {
		crate::shader::ResourceKind::UniformBuffer | crate::shader::ResourceKind::StorageBuffer => {
			ArgumentBindingSlots::Buffer(reserve_argument_slots(next_argument_index, count))
		}
		crate::shader::ResourceKind::SampledImage
		| crate::shader::ResourceKind::StorageImage
		| crate::shader::ResourceKind::InputAttachment => {
			ArgumentBindingSlots::Texture(reserve_argument_slots(next_argument_index, count))
		}
		crate::shader::ResourceKind::Sampler => {
			ArgumentBindingSlots::Sampler(reserve_argument_slots(next_argument_index, count))
		}
		crate::shader::ResourceKind::CombinedImageSampler => ArgumentBindingSlots::CombinedImageSampler {
			textures: reserve_argument_slots(next_argument_index, count),
			samplers: reserve_argument_slots(next_argument_index, count),
		},
		crate::shader::ResourceKind::AccelerationStructure => {
			ArgumentBindingSlots::AccelerationStructure(reserve_argument_slots(next_argument_index, count))
		}
	}
}

fn stage_argument_interface_matches(
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

/// Builds one dense Metal argument-buffer layout from the flat resources used by a shader stage.
fn build_stage_argument_layout(
	device: &ProtocolObject<dyn mtl::MTLDevice>,
	stage: crate::Stages,
	resources: &[crate::shader::ShaderResourceDescriptor],
) -> StageArgumentLayout {
	let mut next_argument_index = 0u32;
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
			let argument_slots = allocate_argument_binding_slots(resource.kind(), resource.count(), &mut next_argument_index);
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

/// Builds the private Metal pipeline layout by merging the resource interfaces of every shader stage.
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
				// Identical interfaces can use the same immutable argument buffer at index 16 for every matching stage.
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
mod flat_binding_tests {
	use super::*;

	fn resource(
		slot: u32,
		kind: crate::shader::ResourceKind,
		count: u32,
		access: crate::AccessPolicies,
	) -> crate::shader::ShaderResourceDescriptor {
		crate::shader::ShaderResourceDescriptor::new(crate::shader::ResourceSlot::new(slot), kind, count, access)
	}

	#[test]
	fn flat_resource_ranges_treat_arrays_as_reserved_slot_intervals() {
		let array = resource(
			9,
			crate::shader::ResourceKind::SampledImage,
			1024,
			crate::AccessPolicies::READ,
		);
		let inside = resource(10, crate::shader::ResourceKind::Sampler, 1, crate::AccessPolicies::READ);
		let after = resource(1033, crate::shader::ResourceKind::Sampler, 1, crate::AccessPolicies::READ);

		assert!(resource_ranges_overlap(array, inside));
		assert!(!resource_ranges_overlap(array, after));
	}

	#[test]
	fn active_array_interiors_are_not_independent_retained_slot_keys() {
		let array = resource(9, crate::shader::ResourceKind::SampledImage, 4, crate::AccessPolicies::READ);

		assert!(resource_accepts_retained_slot_key(array, crate::shader::ResourceSlot::new(9)));
		assert!(!resource_accepts_retained_slot_key(
			array,
			crate::shader::ResourceSlot::new(10)
		));
		assert!(!resource_accepts_retained_slot_key(
			array,
			crate::shader::ResourceSlot::new(12)
		));
		assert!(resource_accepts_retained_slot_key(
			array,
			crate::shader::ResourceSlot::new(13)
		));
	}

	#[test]
	#[should_panic(expected = "Overlapping Metal shader resources")]
	fn canonical_stage_interface_rejects_overlapping_ranges() {
		canonicalize_stage_resources(&[
			resource(4, crate::shader::ResourceKind::StorageBuffer, 4, crate::AccessPolicies::READ),
			resource(7, crate::shader::ResourceKind::Sampler, 1, crate::AccessPolicies::READ),
		]);
	}

	#[test]
	fn combined_image_sampler_arrays_pack_dense_texture_then_sampler_ids() {
		let mut next = 3;
		let combined = allocate_argument_binding_slots(crate::shader::ResourceKind::CombinedImageSampler, 2, &mut next);
		let buffer = allocate_argument_binding_slots(crate::shader::ResourceKind::UniformBuffer, 1, &mut next);

		assert_eq!(
			combined,
			ArgumentBindingSlots::CombinedImageSampler {
				textures: ArgumentSlotRange { base: 3, count: 2 },
				samplers: ArgumentSlotRange { base: 5, count: 2 },
			}
		);
		assert_eq!(buffer, ArgumentBindingSlots::Buffer(ArgumentSlotRange { base: 7, count: 1 }));
		assert_eq!(next, 8);
	}

	#[test]
	fn canonical_stage_interfaces_share_only_when_representation_and_access_match() {
		let split_declarations = canonicalize_stage_resources(&[
			resource(8, crate::shader::ResourceKind::Sampler, 1, crate::AccessPolicies::READ),
			resource(2, crate::shader::ResourceKind::StorageBuffer, 1, crate::AccessPolicies::READ),
			resource(2, crate::shader::ResourceKind::StorageBuffer, 1, crate::AccessPolicies::WRITE),
		]);
		let merged_declaration = canonicalize_stage_resources(&[
			resource(
				2,
				crate::shader::ResourceKind::StorageBuffer,
				1,
				crate::AccessPolicies::READ_WRITE,
			),
			resource(8, crate::shader::ResourceKind::Sampler, 1, crate::AccessPolicies::READ),
		]);
		let read_only = canonicalize_stage_resources(&[
			resource(2, crate::shader::ResourceKind::StorageBuffer, 1, crate::AccessPolicies::READ),
			resource(8, crate::shader::ResourceKind::Sampler, 1, crate::AccessPolicies::READ),
		]);

		assert_eq!(split_declarations, merged_declaration);
		assert_ne!(split_declarations, read_only);
	}

	/// Exercises the production material ordering where scalar resources follow the bindless texture table.
	#[test]
	fn retained_material_resources_after_bindless_array_reach_metal() {
		use crate::{
			command_buffer::{BoundComputePipelineMode as _, BoundPipelineLayoutMode as _, CommonCommandBufferMode as _},
			device::Device as _,
			queue::{FrameRequest, Queue as _, QueueExecution as _},
		};

		const TEXTURES_SLOT: crate::shader::ResourceSlot = crate::shader::ResourceSlot::new(9);
		const MATERIAL_SLOT: crate::shader::ResourceSlot = crate::shader::ResourceSlot::new(1046);
		const AO_SLOT: crate::shader::ResourceSlot = crate::shader::ResourceSlot::new(1051);
		const OUTPUT_SLOT: crate::shader::ResourceSlot = crate::shader::ResourceSlot::new(1054);
		const TEXTURE_INDEX: u32 = 7;

		let source = r#"
			#include <metal_stdlib>
			using namespace metal;

			struct _resources {
				texture2d<float> textures [[id(0)]][1024];
				sampler textures_sampler [[id(1024)]][1024];
				constant uint* material_texture_index [[id(2048)]];
				texture2d<float> ao [[id(2049)]];
				sampler ao_sampler [[id(2050)]];
				device uint* output [[id(2051)]];
			};

			kernel void retained_material_probe(
				uint2 gid [[thread_position_in_grid]],
				constant _resources& resources [[buffer(16)]]) {
				if (gid.x != 0 || gid.y != 0) { return; }
				uint texture_index = resources.material_texture_index[0];
				float material = resources.textures[texture_index].sample(
					resources.textures_sampler[texture_index], float2(0.5)).r;
				float ao = resources.ao.sample(resources.ao_sampler, float2(0.5)).r;
				resources.output[0] = uint(round(material * 255.0)) | (uint(round(ao * 255.0)) << 8);
			}
		"#;

		let mut instance = super::Instance::new(crate::device::Features::new())
			.expect("Failed to create a Metal instance. The most likely cause is unavailable Metal device support.");
		let mut queue_handle = None;
		let mut context = instance
			.create_device(
				crate::device::Features::new(),
				&mut [(crate::QueueSelection::new(crate::WorkloadTypes::COMPUTE), &mut queue_handle)],
			)
			.expect("Failed to create a Metal device. The most likely cause is unavailable compute queue support.")
			.create_context()
			.expect("Failed to create a Metal context. The most likely cause is unavailable Metal command support.");
		let queue_handle = queue_handle.expect(
			"Missing Metal compute queue. The most likely cause is that device selection did not return the requested queue.",
		);

		let texture_resource = resource(
			TEXTURES_SLOT.index(),
			crate::shader::ResourceKind::CombinedImageSampler,
			1024,
			crate::AccessPolicies::READ,
		);
		let material_resource = resource(
			MATERIAL_SLOT.index(),
			crate::shader::ResourceKind::StorageBuffer,
			1,
			crate::AccessPolicies::READ,
		);
		let ao_resource = resource(
			AO_SLOT.index(),
			crate::shader::ResourceKind::CombinedImageSampler,
			1,
			crate::AccessPolicies::READ,
		);
		let output_resource = resource(
			OUTPUT_SLOT.index(),
			crate::shader::ResourceKind::StorageBuffer,
			1,
			crate::AccessPolicies::WRITE,
		);
		let shader = context
			.create_shader(
				Some("Retained Material Binding Probe"),
				crate::shader::Sources::MTL {
					source,
					entry_point: "retained_material_probe",
				},
				crate::ShaderTypes::Compute,
				[output_resource, ao_resource, texture_resource, material_resource],
			)
			.expect("Failed to create the material binding probe. The most likely cause is invalid Metal test source.");
		let pipeline = context.create_compute_pipeline(crate::pipelines::compute::Builder::new(
			&[],
			crate::pipelines::ShaderParameter::new(&shader, crate::ShaderTypes::Compute),
		));

		let material_index = context.build_buffer::<u32>(
			crate::buffer::Builder::new(crate::Uses::Storage)
				.name("Material Texture Index Probe")
				.device_accesses(crate::DeviceAccesses::HostToDevice),
		);
		*context.get_mut_buffer_slice(material_index) = TEXTURE_INDEX;
		let output = context.build_buffer::<u32>(
			crate::buffer::Builder::new(crate::Uses::Storage)
				.name("Material Binding Probe Output")
				.device_accesses(crate::DeviceAccesses::CpuWrite | crate::DeviceAccesses::GpuWrite),
		);
		*context.get_mut_buffer_slice(output) = 0;

		let material_texture = context.build_image(
			crate::image::Builder::new(crate::Formats::RGBA8UNORM, crate::Uses::Image)
				.name("Material Binding Probe Texture")
				.extent(Extent::square(1))
				.device_accesses(crate::DeviceAccesses::HostToDevice),
		);
		context.write_texture(material_texture, |bytes| bytes.copy_from_slice(&[64, 0, 0, 255]));
		let ao_texture = context.build_image(
			crate::image::Builder::new(crate::Formats::R8UNORM, crate::Uses::Image)
				.name("Material Binding Probe AO")
				.extent(Extent::square(1))
				.device_accesses(crate::DeviceAccesses::HostToDevice),
		);
		context.write_texture(ao_texture, |bytes| bytes.copy_from_slice(&[192]));
		let sampler = context.build_sampler(
			crate::sampler::Builder::new()
				.filtering_mode(crate::FilteringModes::Closest)
				.mip_map_mode(crate::FilteringModes::Closest),
		);

		let scene_set = context.create_descriptor_set(Some("Material Binding Probe Scene Set"));
		let material_set = context.create_descriptor_set(Some("Material Binding Probe Material Set"));
		context.write(&[
			crate::DescriptorWrite::combined_image_sampler_array(
				scene_set,
				TEXTURES_SLOT,
				material_texture,
				sampler,
				crate::Layouts::Read,
				TEXTURE_INDEX,
			),
			crate::DescriptorWrite::buffer(material_set, MATERIAL_SLOT, material_index.into()),
			crate::DescriptorWrite::combined_image_sampler(material_set, AO_SLOT, ao_texture, sampler, crate::Layouts::Read),
			crate::DescriptorWrite::buffer(material_set, OUTPUT_SLOT, output.into()),
		]);

		let command_buffer = context
			.queue(queue_handle)
			.create_command_buffer(Some("Material Binding Probe"));
		let signal = context.create_synchronizer(Some("Material Binding Probe Signal"), true);
		context.queue(queue_handle).execute(
			Some(FrameRequest {
				index: 0,
				synchronizer: signal,
			}),
			&[],
			signal,
			|execution| {
				execution.record(command_buffer, |recording| {
					recording
						.bind_compute_pipeline(pipeline)
						.bind_descriptor_sets(&[scene_set, material_set])
						.dispatch(crate::DispatchExtent::new(Extent::square(1), Extent::square(1)));
				});
				[]
			},
		);
		context.wait();

		assert_eq!(
			*context.get_buffer_slice(output),
			64 | (192 << 8),
			"Material resources after the bindless table reached the wrong Metal argument IDs. The most likely cause is that retained materialization and MSL dense-ID allocation disagree.",
		);
	}
}

#[derive(Clone)]
pub(super) struct CommandBufferInternal {
	queue: Retained<ProtocolObject<dyn mtl::MTLCommandQueue>>,
	command_buffer: Retained<ProtocolObject<dyn mtl::MTLCommandBuffer>>,
}

#[derive(Clone)]
pub(crate) struct StoredCommandBuffer {
	queue_handle: graphics_hardware_interface::QueueHandle,
	name: Option<String>,
}

pub struct CommandBuffer<'a> {
	pub(crate) device: &'a mut context::Context,
	pub(crate) command_buffer_handle: graphics_hardware_interface::CommandBufferHandle,
}

#[derive(Clone)]
pub(crate) struct Allocation {
	buffer: Retained<ProtocolObject<dyn mtl::MTLBuffer>>,
	pointer: *mut u8,
	size: usize,
}

pub(crate) struct DebugCallbackData {
	error_count: AtomicU64,
	error_log_function: fn(&str),
}

#[derive(PartialEq, Eq, Clone, Copy)]
pub(crate) struct TransitionState {
	layout: crate::Layouts,
}

pub(crate) struct Mesh {
	vertex_buffers: Vec<Option<Retained<ProtocolObject<dyn mtl::MTLBuffer>>>>,
	index_buffer: Retained<ProtocolObject<dyn mtl::MTLBuffer>>,
	vertex_count: u32,
	index_count: u32,
	vertex_size: usize,
}

pub(crate) struct AccelerationStructure {
	structure: Option<Retained<ProtocolObject<dyn mtl::MTLAccelerationStructure>>>,
	buffer: Option<Retained<ProtocolObject<dyn mtl::MTLBuffer>>>,
}

#[derive(Clone, Copy)]
/// The `MemoryBackedResourceCreationResult` struct provides a resource and its memory requirements for allocation.
pub struct MemoryBackedResourceCreationResult<T> {
	/// The resource.
	resource: T,
	/// The final size of the resource.
	size: usize,
	/// The memory flags that need used to create the resource.
	memory_flags: u32,
}

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
struct BuildImage {
	previous: ImageHandle,
	master: graphics_hardware_interface::ImageHandle,
}

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
struct BuildBuffer {
	previous: BufferHandle,
	master: graphics_hardware_interface::BaseBufferHandle,
}

#[derive(Debug, Clone, Copy, Hash, PartialEq, Eq)]
pub(crate) enum Tasks {
	/// Deletes a Metal texture at the frame selected by [`Task`].
	DeleteMetalTexture {
		handle: ImageHandle,
	},
	/// Deletes a Metal buffer at the frame selected by [`Task`].
	DeleteMetalBuffer {
		handle: BufferHandle,
	},
	/// Patch all descriptors that reference the buffer.
	/// Usually, this is done when the buffer is resized because the Metal buffer will be swapped.
	UpdateBufferDescriptors {
		handle: BufferHandle,
	},
	/// Patch all descriptors that reference the image.
	/// Usually, this is done when the image is resized because the Metal texture will be swapped.
	UpdateImageDescriptors {
		handle: ImageHandle,
	},
	/// Resize an image.
	ResizeImage {
		handle: ImageHandle,
		extent: Extent,
	},
	BuildImage(BuildImage),
	BuildBuffer(BuildBuffer),
}

#[derive(Debug, Clone, PartialEq)]
/// The `Task` struct schedules backend work for a required time or frame.
pub(crate) struct Task {
	pub(crate) task: Tasks,
	pub(crate) frame: Option<u8>,
}

impl Task {
	pub(crate) fn new(task: Tasks, frame: Option<u8>) -> Self {
		Self { task, frame }
	}

	pub(crate) fn delete_metal_texture(handle: ImageHandle, frame: u8) -> Self {
		Self {
			task: Tasks::DeleteMetalTexture { handle },
			frame: Some(frame),
		}
	}

	pub(crate) fn delete_metal_buffer(handle: BufferHandle, frame: Option<u8>) -> Self {
		Self {
			task: Tasks::DeleteMetalBuffer { handle },
			frame,
		}
	}

	pub(crate) fn update_buffer_descriptor(handle: BufferHandle, frame: Option<u8>) -> Self {
		Self {
			task: Tasks::UpdateBufferDescriptors { handle },
			frame,
		}
	}

	pub(crate) fn update_image_descriptor(handle: ImageHandle, frame: Option<u8>) -> Self {
		Self {
			task: Tasks::UpdateImageDescriptors { handle },
			frame,
		}
	}

	pub(crate) fn frame(&self) -> Option<u8> {
		self.frame
	}

	pub(crate) fn task(&self) -> &Tasks {
		&self.task
	}

	pub(crate) fn into_task(self) -> Tasks {
		self.task
	}
}

mod utils {
	use objc2_metal as mtl;
	use utils::Extent;

	use crate::{DeviceAccesses, FilteringModes, Formats, SamplerAddressingModes, Uses};

	pub(crate) fn parse_threadgroup_size_metadata(source: &str) -> Option<Extent> {
		let metadata_prefix = "// besl-threadgroup-size:";
		let metadata = source.lines().find_map(|line| line.trim().strip_prefix(metadata_prefix))?;
		let mut extents = metadata.split(',').map(|value| value.trim().parse::<u32>().ok());

		Some(Extent::new(extents.next()??, extents.next()??, extents.next()??))
	}

	pub(crate) fn to_pixel_format(format: Formats) -> mtl::MTLPixelFormat {
		match format {
			Formats::R8UNORM => mtl::MTLPixelFormat::R8Unorm,
			Formats::R8SNORM => mtl::MTLPixelFormat::R8Snorm,
			Formats::R8F => mtl::MTLPixelFormat::R8Unorm,
			Formats::R8sRGB => mtl::MTLPixelFormat::R8Unorm,

			Formats::R16F => mtl::MTLPixelFormat::R16Float,
			Formats::R16UNORM => mtl::MTLPixelFormat::R16Unorm,
			Formats::R16SNORM => mtl::MTLPixelFormat::R16Snorm,
			Formats::R16sRGB => mtl::MTLPixelFormat::R16Unorm,

			Formats::R32F => mtl::MTLPixelFormat::R32Float,
			Formats::R32UNORM => mtl::MTLPixelFormat::R32Uint,
			Formats::R32SNORM => mtl::MTLPixelFormat::R32Sint,
			Formats::R32sRGB => mtl::MTLPixelFormat::R32Uint,

			Formats::RG8UNORM => mtl::MTLPixelFormat::RG8Unorm,
			Formats::RG8SNORM => mtl::MTLPixelFormat::RG8Snorm,
			Formats::RG8F => mtl::MTLPixelFormat::RG8Unorm,
			Formats::RG8sRGB => mtl::MTLPixelFormat::RG8Unorm,

			Formats::RG16F => mtl::MTLPixelFormat::RG16Float,
			Formats::RG16UNORM => mtl::MTLPixelFormat::RG16Unorm,
			Formats::RG16SNORM => mtl::MTLPixelFormat::RG16Snorm,
			Formats::RG16sRGB => mtl::MTLPixelFormat::RG16Unorm,

			Formats::RGB8UNORM => mtl::MTLPixelFormat::RGBA8Unorm,
			Formats::RGB8SNORM => mtl::MTLPixelFormat::RGBA8Snorm,
			Formats::RGB8F => mtl::MTLPixelFormat::RGBA8Unorm,
			Formats::RGB8sRGB => mtl::MTLPixelFormat::RGBA8Unorm_sRGB,

			Formats::RGB16F => mtl::MTLPixelFormat::RGBA16Float,
			Formats::RGB16UNORM => mtl::MTLPixelFormat::RGBA16Unorm,
			Formats::RGB16SNORM => mtl::MTLPixelFormat::RGBA16Snorm,
			Formats::RGB16sRGB => mtl::MTLPixelFormat::RGBA16Unorm,

			Formats::RGBA8UNORM => mtl::MTLPixelFormat::RGBA8Unorm,
			Formats::RGBA8SNORM => mtl::MTLPixelFormat::RGBA8Snorm,
			Formats::RGBA8F => mtl::MTLPixelFormat::RGBA8Unorm,
			Formats::RGBA8sRGB => mtl::MTLPixelFormat::RGBA8Unorm_sRGB,

			Formats::RGBA16F => mtl::MTLPixelFormat::RGBA16Float,
			Formats::RGBA16UNORM => mtl::MTLPixelFormat::RGBA16Unorm,
			Formats::RGBA16SNORM => mtl::MTLPixelFormat::RGBA16Snorm,
			Formats::RGBA16sRGB => mtl::MTLPixelFormat::RGBA16Unorm,

			Formats::RGBu11u11u10 => mtl::MTLPixelFormat::RG11B10Float,
			Formats::BGRAu8 => mtl::MTLPixelFormat::BGRA8Unorm,
			Formats::BGRAsRGB => mtl::MTLPixelFormat::BGRA8Unorm_sRGB,
			Formats::Depth32 => mtl::MTLPixelFormat::Depth32Float,
			Formats::U32 => mtl::MTLPixelFormat::R32Uint,

			Formats::BC5 => mtl::MTLPixelFormat::BC5_RGUnorm,
			Formats::BC5SNORM => mtl::MTLPixelFormat::BC5_RGSnorm,
			Formats::BC7 => mtl::MTLPixelFormat::BC7_RGBAUnorm,
			Formats::BC7SRGB => mtl::MTLPixelFormat::BC7_RGBAUnorm_sRGB,
		}
	}

	pub(crate) fn storage_mode_from_access(access: DeviceAccesses) -> mtl::MTLStorageMode {
		if access == DeviceAccesses::DeviceOnly {
			mtl::MTLStorageMode::Private
		} else if access.contains(DeviceAccesses::CpuRead) {
			mtl::MTLStorageMode::Managed
		} else {
			mtl::MTLStorageMode::Shared
		}
	}

	pub(crate) fn resource_options_from_access(access: DeviceAccesses) -> mtl::MTLResourceOptions {
		match storage_mode_from_access(access) {
			mtl::MTLStorageMode::Private => mtl::MTLResourceOptions::StorageModePrivate,
			mtl::MTLStorageMode::Managed => mtl::MTLResourceOptions::StorageModeManaged,
			_ => mtl::MTLResourceOptions::StorageModeShared,
		}
	}

	pub(crate) fn texture_usage_from_uses(uses: Uses) -> mtl::MTLTextureUsage {
		let mut usage = mtl::MTLTextureUsage::empty();

		if uses.intersects(Uses::Image | Uses::Storage | Uses::ShaderBindingTable) {
			usage |= mtl::MTLTextureUsage::ShaderRead;
		}

		if uses.contains(Uses::Storage) {
			usage |= mtl::MTLTextureUsage::ShaderWrite;
		}

		if uses.intersects(Uses::RenderTarget | Uses::DepthStencil) {
			usage |= mtl::MTLTextureUsage::RenderTarget;
		}

		usage
	}

	pub(crate) fn sampler_min_mag_filter(filter: FilteringModes) -> mtl::MTLSamplerMinMagFilter {
		match filter {
			FilteringModes::Closest => mtl::MTLSamplerMinMagFilter::Nearest,
			FilteringModes::Linear => mtl::MTLSamplerMinMagFilter::Linear,
		}
	}

	pub(crate) fn sampler_mip_filter(filter: FilteringModes) -> mtl::MTLSamplerMipFilter {
		match filter {
			FilteringModes::Closest => mtl::MTLSamplerMipFilter::Nearest,
			FilteringModes::Linear => mtl::MTLSamplerMipFilter::Linear,
		}
	}

	pub(crate) fn sampler_address_mode(mode: SamplerAddressingModes) -> mtl::MTLSamplerAddressMode {
		match mode {
			SamplerAddressingModes::Repeat => mtl::MTLSamplerAddressMode::Repeat,
			SamplerAddressingModes::Mirror => mtl::MTLSamplerAddressMode::MirrorRepeat,
			SamplerAddressingModes::Clamp => mtl::MTLSamplerAddressMode::ClampToEdge,
			SamplerAddressingModes::Border { .. } => mtl::MTLSamplerAddressMode::ClampToBorderColor,
		}
	}

	pub(crate) fn texture_upload_layout(format: Formats, extent: Extent) -> Option<(usize, usize, usize)> {
		Some(format.compact_copy_layout(extent.width().max(1), extent.height().max(1)))
	}

	pub(crate) fn texture_copy_size(_format: Formats, extent: Extent) -> mtl::MTLSize {
		mtl::MTLSize {
			width: extent.width().max(1) as _,
			height: extent.height().max(1) as _,
			depth: extent.depth().max(1) as _,
		}
	}

	pub(crate) fn is_block_compressed(format: Formats) -> bool {
		format.bc_bytes_per_block().is_some()
	}

	pub(crate) fn vertex_format(format: crate::DataTypes) -> mtl::MTLVertexFormat {
		match format {
			crate::DataTypes::Float => mtl::MTLVertexFormat::Float,
			crate::DataTypes::Float2 => mtl::MTLVertexFormat::Float2,
			crate::DataTypes::Float3 => mtl::MTLVertexFormat::Float3,
			crate::DataTypes::Float4 => mtl::MTLVertexFormat::Float4,
			crate::DataTypes::U8 => mtl::MTLVertexFormat::UChar,
			crate::DataTypes::U16 => mtl::MTLVertexFormat::UShort,
			crate::DataTypes::U32 | crate::DataTypes::UInt => mtl::MTLVertexFormat::UInt,
			crate::DataTypes::Int => mtl::MTLVertexFormat::Int,
			crate::DataTypes::Int2 => mtl::MTLVertexFormat::Int2,
			crate::DataTypes::Int3 => mtl::MTLVertexFormat::Int3,
			crate::DataTypes::Int4 => mtl::MTLVertexFormat::Int4,
			crate::DataTypes::UInt2 => mtl::MTLVertexFormat::UInt2,
			crate::DataTypes::UInt3 => mtl::MTLVertexFormat::UInt3,
			crate::DataTypes::UInt4 => mtl::MTLVertexFormat::UInt4,
		}
	}

	pub(crate) fn load_action(load: bool) -> mtl::MTLLoadAction {
		if load {
			mtl::MTLLoadAction::Load
		} else {
			mtl::MTLLoadAction::Clear
		}
	}

	pub(crate) fn store_action(store: bool) -> mtl::MTLStoreAction {
		if store {
			mtl::MTLStoreAction::Store
		} else {
			mtl::MTLStoreAction::DontCare
		}
	}

	pub(crate) fn clear_color(clear: crate::ClearValue) -> mtl::MTLClearColor {
		match clear {
			crate::ClearValue::None => mtl::MTLClearColor {
				red: 0.0,
				green: 0.0,
				blue: 0.0,
				alpha: 0.0,
			},
			crate::ClearValue::Color(color) => mtl::MTLClearColor {
				red: color.r as f64,
				green: color.g as f64,
				blue: color.b as f64,
				alpha: color.a as f64,
			},
			crate::ClearValue::Integer(r, g, b, a) => mtl::MTLClearColor {
				red: r as f64,
				green: g as f64,
				blue: b as f64,
				alpha: a as f64,
			},
			crate::ClearValue::Depth(depth) => mtl::MTLClearColor {
				red: depth as f64,
				green: 0.0,
				blue: 0.0,
				alpha: 0.0,
			},
		}
	}

	pub(crate) fn clear_depth(clear: crate::ClearValue) -> std::os::raw::c_double {
		match clear {
			crate::ClearValue::Depth(depth) => depth as _,
			_ => 0.0,
		}
	}

	pub(crate) fn winding(winding: crate::pipelines::raster::FaceWinding) -> mtl::MTLWinding {
		match winding {
			crate::pipelines::raster::FaceWinding::Clockwise => mtl::MTLWinding::Clockwise,
			crate::pipelines::raster::FaceWinding::CounterClockwise => mtl::MTLWinding::CounterClockwise,
		}
	}

	pub(crate) fn cull_mode(cull_mode: crate::pipelines::raster::CullMode) -> mtl::MTLCullMode {
		match cull_mode {
			crate::pipelines::raster::CullMode::None => mtl::MTLCullMode::None,
			crate::pipelines::raster::CullMode::Front => mtl::MTLCullMode::Front,
			crate::pipelines::raster::CullMode::Back => mtl::MTLCullMode::Back,
		}
	}

	#[cfg(not(debug_assertions))]
	pub(crate) fn debug_compressed_upload(
		_enabled: bool,
		_format: Formats,
		_mip_index: usize,
		_slice_index: usize,
		_extent: Extent,
		_bytes_per_row: usize,
		_bytes_per_image: usize,
		_source_offset: usize,
	) {
	}

	#[cfg(test)]
	mod tests {
		use super::*;

		#[test]
		fn upload_layout_preserves_bc_block_rows_and_minimum_extent() {
			let extent = Extent::rectangle(5, 7);

			let (bytes_per_row, row_count, bytes_per_image) = texture_upload_layout(Formats::BC7, extent).unwrap();

			assert_eq!(bytes_per_row, 2 * 16);
			assert_eq!(row_count, 2);
			assert_eq!(bytes_per_image, 2 * 2 * 16);
			assert_eq!(
				texture_upload_layout(Formats::RGBA8UNORM, Extent::rectangle(0, 0)),
				Some((4, 1, 4))
			);
		}

		#[test]
		fn bc_copy_size_uses_texel_extent_not_padded_block_extent() {
			let size = texture_copy_size(Formats::BC7, Extent::rectangle(5, 7));

			assert_eq!(size.width, 5);
			assert_eq!(size.height, 7);
			assert_eq!(size.depth, 1);
		}

		#[test]
		fn bc_format_mapping_preserves_linear_and_srgb_variants() {
			assert_eq!(to_pixel_format(Formats::BC5), mtl::MTLPixelFormat::BC5_RGUnorm);
			assert_eq!(to_pixel_format(Formats::BC5SNORM), mtl::MTLPixelFormat::BC5_RGSnorm);
			assert_eq!(to_pixel_format(Formats::BC7), mtl::MTLPixelFormat::BC7_RGBAUnorm);
			assert_eq!(to_pixel_format(Formats::BC7SRGB), mtl::MTLPixelFormat::BC7_RGBAUnorm_sRGB);
		}

		#[test]
		fn specialization_map_entry_supports_i32_constants() {
			let constant_values = mtl::MTLFunctionConstantValues::new();
			let entry = crate::pipelines::SpecializationMapEntry::new(0, "i32".to_string(), -1i32);

			super::super::apply_specialization_map_entry(&constant_values, &entry);
		}
	}
}

pub mod queue {
	use super::*;

	#[derive(Clone)]
	pub(crate) struct StoredQueue {
		pub(crate) queue: Retained<ProtocolObject<dyn mtl::MTLCommandQueue>>,
		pub(crate) workloads: crate::WorkloadTypes,
	}

	/// The `Queue` struct owns the queue submission entry point without borrowing the device.
	pub struct Queue {
		pub(crate) device: std::ptr::NonNull<context::Context>,
		pub(crate) queue_handle: graphics_hardware_interface::QueueHandle,
	}

	unsafe impl Send for Queue {}

	/// The `QueueReference` struct preserves the borrowed queue API while queue ownership is being split out.
	pub struct QueueReference<'a> {
		pub(crate) device: &'a mut context::Context,
		pub(crate) queue_handle: graphics_hardware_interface::QueueHandle,
	}

	/// The `Execution` struct gathers Metal command-buffer recordings before queue submission.
	pub struct Execution<'a> {
		frame: Option<super::Frame<'a>>,
		completed_frame: Option<graphics_hardware_interface::FrameKey>,
		command_buffers: SmallVec<[super::FinishedCommandBuffer<'static>; 4]>,
	}

	impl<'a> crate::queue::QueueExecution<'a> for Execution<'a> {
		type Frame = super::Frame<'a>;

		fn frame(&mut self) -> Option<&mut Self::Frame> {
			self.frame.as_mut()
		}

		fn completed_frame(&self) -> Option<graphics_hardware_interface::FrameKey> {
			self.completed_frame
		}

		fn record<'record>(
			&'record mut self,
			command_buffer_handle: graphics_hardware_interface::CommandBufferHandle,
			record: impl FnOnce(&mut <Self::Frame as crate::frame::Frame<'a>>::CBR<'record>),
		) where
			Self::Frame: 'record,
		{
			let frame = self.frame.as_mut().expect(
				"Frame is required to record a frame command buffer. The most likely cause is that Queue::execute was called with None and the closure tried to record frame work.",
			);
			let mut command_buffer = frame.create_command_buffer_recording(command_buffer_handle);
			record(&mut command_buffer);
			self.command_buffers.push(command_buffer.into_finished());
		}
	}

	impl Queue {
		/// Returns mutable device access for the queue wrapper until device state is split out.
		fn device_mut(&mut self) -> &mut context::Context {
			// The owned queue is created from a live Device and must not outlive it.
			// Thread-safe ownership will require moving queue-local state out of Device.
			unsafe { self.device.as_mut() }
		}
	}

	impl crate::queue::Queue for Queue {
		type Frame<'a> = super::Frame<'a>;
		type Execution<'a> = Execution<'a>;

		fn create_command_buffer(&mut self, name: Option<&str>) -> graphics_hardware_interface::CommandBufferHandle {
			let queue_handle = self.queue_handle;
			self.device_mut().create_command_buffer(name, queue_handle)
		}

		fn start_frame<'a>(
			&'a mut self,
			index: u32,
			synchronizer_handle: graphics_hardware_interface::SynchronizerHandle,
		) -> crate::queue::StartedFrame<Self::Frame<'a>> {
			self.device_mut().start_frame(index, synchronizer_handle)
		}

		fn execute<'a, P>(
			&'a mut self,
			frame: Option<crate::queue::FrameRequest>,
			wait_for: &[graphics_hardware_interface::SynchronizerHandle],
			synchronizer: graphics_hardware_interface::SynchronizerHandle,
			execute: impl FnOnce(&mut Self::Execution<'a>) -> P,
		) where
			P: AsRef<[graphics_hardware_interface::PresentKey]>,
		{
			let device = self.device_mut();
			for &wait_synchronizer in wait_for {
				device.wait_for_synchronizer(wait_synchronizer);
			}

			let frame = frame.map(|frame| device.start_frame(frame.index, frame.synchronizer));
			let completed_frame = frame.as_ref().and_then(|frame| frame.completed_frame);
			let frame = frame.map(|frame| frame.frame);
			let mut execution = Execution {
				frame,
				completed_frame,
				command_buffers: SmallVec::new(),
			};
			let present_keys = execute(&mut execution);

			let Some(mut frame) = execution.frame else {
				return;
			};
			let last_index = execution.command_buffers.len().saturating_sub(1);
			for (index, command_buffer) in execution.command_buffers.into_iter().enumerate() {
				let present_keys = if index == last_index { present_keys.as_ref() } else { &[] };
				frame.execute_finished(command_buffer, present_keys, synchronizer);
			}
		}
	}

	impl crate::queue::Queue for QueueReference<'_> {
		type Frame<'a> = super::Frame<'a>;
		type Execution<'a> = Execution<'a>;

		fn create_command_buffer(&mut self, name: Option<&str>) -> graphics_hardware_interface::CommandBufferHandle {
			self.device.create_command_buffer(name, self.queue_handle)
		}

		fn start_frame<'a>(
			&'a mut self,
			index: u32,
			synchronizer_handle: graphics_hardware_interface::SynchronizerHandle,
		) -> crate::queue::StartedFrame<Self::Frame<'a>> {
			self.device.start_frame(index, synchronizer_handle)
		}

		fn execute<'a, P>(
			&'a mut self,
			frame: Option<crate::queue::FrameRequest>,
			wait_for: &[graphics_hardware_interface::SynchronizerHandle],
			synchronizer: graphics_hardware_interface::SynchronizerHandle,
			execute: impl FnOnce(&mut Self::Execution<'a>) -> P,
		) where
			P: AsRef<[graphics_hardware_interface::PresentKey]>,
		{
			for &wait_synchronizer in wait_for {
				self.device.wait_for_synchronizer(wait_synchronizer);
			}

			let frame = frame.map(|frame| self.device.start_frame(frame.index, frame.synchronizer));
			let completed_frame = frame.as_ref().and_then(|frame| frame.completed_frame);
			let frame = frame.map(|frame| frame.frame);
			let mut execution = Execution {
				frame,
				completed_frame,
				command_buffers: SmallVec::new(),
			};
			let present_keys = execute(&mut execution);

			let Some(mut frame) = execution.frame else {
				return;
			};
			let last_index = execution.command_buffers.len().saturating_sub(1);
			for (index, command_buffer) in execution.command_buffers.into_iter().enumerate() {
				let present_keys = if index == last_index { present_keys.as_ref() } else { &[] };
				frame.execute_finished(command_buffer, present_keys, synchronizer);
			}
		}
	}
}

pub mod buffer {
	use super::*;
	use crate::{DeviceAccesses, Uses};

	#[derive(Clone)]
	pub(crate) struct Buffer {
		pub(crate) name: Option<String>,
		pub(crate) staging: Option<BufferHandle>,
		pub(crate) buffer: Retained<ProtocolObject<dyn mtl::MTLBuffer>>,
		pub(crate) size: usize,
		pub(crate) gpu_address: u64,
		pub(crate) pointer: *mut u8,
		pub(crate) uses: Uses,
		pub(crate) access: DeviceAccesses,
	}
}

pub mod image {
	use super::*;
	use crate::{DeviceAccesses, Formats, Uses};

	#[derive(Clone)]
	pub(crate) struct Image {
		pub(crate) name: Option<String>,
		pub(crate) texture: Retained<ProtocolObject<dyn mtl::MTLTexture>>,
		pub(crate) extent: Extent,
		pub(crate) format: Formats,
		pub(crate) uses: Uses,
		pub(crate) access: DeviceAccesses,
		pub(crate) array_layers: u32,
		pub(crate) staging: Option<Vec<u8>>,
	}
}

pub mod sampler {
	use super::*;

	#[derive(Clone)]
	pub(crate) struct Sampler {
		pub(crate) sampler: Retained<ProtocolObject<dyn mtl::MTLSamplerState>>,
	}
}

pub mod descriptor_set {
	use super::*;
	use crate::descriptors::DescriptorSetHandle;

	/// The `DescriptorSet` struct provides Metal descriptor state for one frame.
	#[derive(Clone)]
	pub(crate) struct DescriptorSet {
		pub next: Option<DescriptorSetHandle>,
		pub version: u64,
		pub descriptors: HashMap<crate::shader::ResourceSlot, HashMap<u32, Descriptor>>,
	}
}

pub mod synchronizer {
	use std::cell::{Cell, RefCell};

	use super::*;
	use crate::synchronizer::SynchronizerHandle;

	/// The `Synchronizer` struct owns the Metal workloads associated with one GHI synchronization point.
	pub(crate) struct Synchronizer {
		pub next: Option<SynchronizerHandle>,
		signaled: Cell<bool>,
		workloads: RefCell<SmallVec<[Retained<ProtocolObject<dyn mtl::MTLCommandBuffer>>; 4]>>,
	}

	impl Synchronizer {
		pub(crate) fn new(signaled: bool) -> Self {
			Self {
				next: None,
				signaled: Cell::new(signaled),
				workloads: RefCell::new(SmallVec::new()),
			}
		}

		pub(crate) fn reset(&self) {
			// Reset only after previous work is complete so diagnostics are not lost for in-flight submissions.
			self.wait();
			self.signaled.set(false);
		}

		pub(crate) fn signal_workload(&self, command_buffer: Retained<ProtocolObject<dyn mtl::MTLCommandBuffer>>) {
			self.signaled.set(false);
			self.workloads.borrow_mut().push(command_buffer);
		}

		pub(crate) fn wait(&self) {
			if self.signaled.get() {
				return;
			}

			// Retain the command buffers until completion so asynchronous Metal submissions can be diagnosed later.
			let workloads = self.workloads.take();
			for command_buffer in &workloads {
				device::wait_for_metal_command_buffer(command_buffer.as_ref());
			}

			self.signaled.set(true);
		}
	}
}

pub mod swapchain {
	use super::*;
	use crate::image::ImageHandle;

	#[derive(Clone)]
	pub(crate) struct Swapchain {
		pub layer: Retained<CAMetalLayer>,
		pub view: Retained<NSView>,
		pub images: [Option<ImageHandle>; MAX_SWAPCHAIN_IMAGES],
		pub extent: Extent,
	}
}

pub mod command_buffer;
pub mod context;
pub mod device;
pub mod factory;
pub mod frame;
pub mod instance;

pub use self::command_buffer::*;
pub use self::context::*;
pub(crate) use self::descriptor_set::*;
pub use self::device::Device;
pub use self::factory::{ComputePipeline, Factory};
pub use self::frame::*;
pub use self::instance::*;
pub(crate) use self::synchronizer::*;
