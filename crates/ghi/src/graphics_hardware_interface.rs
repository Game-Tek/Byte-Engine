//! The [`GraphicsHardwareInterface`] implements easy to use rendering functionality.
//! It provides useful abstractions to interact with the GPU.
//! It's not tied to any particular render pipeline implementation.

use utils::{Extent, RGBA};

use crate::{
	command_buffer::CommandBufferType,
	descriptors::{self, DescriptorType},
	shader::BindingDescriptor,
	AccessPolicies, DataTypes, Encodings, Formats, Layouts, Stages, WorkloadTypes,
};

// HANDLES

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct QueueHandle(pub(crate) u64);

/// The `BaseBufferHandle` allows addressing any static buffer irregardless of it's underlying type.
#[derive(PartialEq, Eq, Clone, Copy, Hash, Debug, PartialOrd, Ord)]
pub struct BaseBufferHandle(pub(super) u64);

/// The `BufferHandle` allows addressing a buffer static buffer with a specific underlying type.
#[derive(PartialEq, Eq, Clone, Copy, Hash, Debug)]
pub struct BufferHandle<T>(pub(super) u64, pub(super) std::marker::PhantomData<T>);

/// The `DynamicBufferHandle` allows addressing a dynamic buffer with a specific underlying type.
#[derive(PartialEq, Eq, Clone, Copy, Hash, Debug)]
pub struct DynamicBufferHandle<T>(pub(super) u64, pub(super) std::marker::PhantomData<T>);

/// The `DynamicImageHandle` struct addresses a frame-local image that can be written independently for each frame in flight.
#[derive(PartialEq, Eq, Clone, Copy, Hash, Debug)]
pub struct DynamicImageHandle(pub(super) u64);

pub trait ImageHandleLike: Copy {
	#[doc(hidden)]
	fn into_image_handle(self) -> ImageHandle;
}

#[derive(PartialEq, Eq, Clone, Copy, Hash, Debug)]
pub struct TopLevelAccelerationStructureHandle(pub(super) u64);

#[derive(PartialEq, Eq, Clone, Copy, Hash, Debug)]
pub struct BottomLevelAccelerationStructureHandle(pub(super) u64);

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct CommandBufferHandle(pub(super) u64);

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct ShaderHandle(pub(super) u64);

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct PipelineHandle(pub(super) u64);

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct ImageHandle(pub(super) u64);

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct MeshHandle(pub(super) u64);

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct SynchronizerHandle(pub(super) u64);

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct DescriptorSetTemplateHandle(pub(super) u64);

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct DescriptorSetHandle(pub(super) u64);

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct DescriptorSetBindingHandle(pub(super) u64);

/// Handle to a Pipeline Layout
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct PipelineLayoutHandle(pub(super) u64);

/// Handle to a Sampler
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct SamplerHandle(pub(super) u64);

/// Handle to a Sampler
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct SwapchainHandle(pub(super) u64);

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct AllocationHandle(pub(crate) u64);

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct TextureCopyHandle(pub(crate) u64);

impl<T: Copy> Into<BaseBufferHandle> for BufferHandle<T> {
	fn into(self) -> BaseBufferHandle {
		BaseBufferHandle(self.0)
	}
}

impl<T: Copy> Into<BaseBufferHandle> for DynamicBufferHandle<T> {
	fn into(self) -> BaseBufferHandle {
		BaseBufferHandle(self.0)
	}
}

impl ImageHandleLike for ImageHandle {
	fn into_image_handle(self) -> ImageHandle {
		self
	}
}

impl ImageHandleLike for DynamicImageHandle {
	fn into_image_handle(self) -> ImageHandle {
		ImageHandle(self.0)
	}
}

impl Into<Handle> for DynamicImageHandle {
	fn into(self) -> Handle {
		self.into_image_handle().into()
	}
}

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum Handle {
	Buffer(BaseBufferHandle),
	// AccelerationStructure(AccelerationStructureHandle),
	TopLevelAccelerationStructure(TopLevelAccelerationStructureHandle),
	CommandBuffer(CommandBufferHandle),
	Shader(ShaderHandle),
	Pipeline(PipelineHandle),
	Image(ImageHandle),
	Mesh(MeshHandle),
	Synchronizer(SynchronizerHandle),
	DescriptorSetLayout(DescriptorSetTemplateHandle),
	DescriptorSet(DescriptorSetHandle),
	PipelineLayout(PipelineLayoutHandle),
	Sampler(SamplerHandle),
	Swapchain(SwapchainHandle),
	Allocation(AllocationHandle),
	TextureCopy(TextureCopyHandle),
	BottomLevelAccelerationStructure(BottomLevelAccelerationStructureHandle),
}

impl Into<Handle> for BaseBufferHandle {
	fn into(self) -> Handle {
		Handle::Buffer(self)
	}
}

impl Into<Handle> for ImageHandle {
	fn into(self) -> Handle {
		Handle::Image(self)
	}
}

impl Into<Handle> for SynchronizerHandle {
	fn into(self) -> Handle {
		Handle::Synchronizer(self)
	}
}

// HANDLES

#[derive(Clone, PartialEq, Eq, Hash, Debug)]
pub(crate) struct Consumption {
	pub handle: Handle,
	pub stages: Stages,
	pub access: AccessPolicies,
	pub layout: Layouts,
}

/// Describes the dimesions of a dispatch operation.
pub struct DispatchExtent {
	workgroup_extent: Extent,
	dispatch_extent: Extent,
}

impl DispatchExtent {
	/// Creates a new dispatch extent.
	/// # Arguments
	/// * `dispatch_extent` - The extent of the dispatch. (How many threads to have in each dimension).
	/// * `workgroup_extent` - The extent of the workgroup. (The workgroup extent defined in the shader).
	pub fn new(dispatch_extent: Extent, workgroup_extent: Extent) -> Self {
		Self {
			workgroup_extent,
			dispatch_extent,
		}
	}

	/// Returns the extent for a dispatch operation.
	/// # Returns
	/// The extent for a dispatch operation, which is the result of dividing the dispatch extent by the workgroup extent, rounded up.
	pub fn get_extent(&self) -> Extent {
		Extent::new(
			self.dispatch_extent.width().div_ceil(self.workgroup_extent.width()),
			self.dispatch_extent.height().div_ceil(self.workgroup_extent.height()),
			self.dispatch_extent.depth().div_ceil(self.workgroup_extent.depth()),
		)
	}
}

pub enum BottomLevelAccelerationStructureDescriptions {
	Mesh {
		vertex_count: u32,
		vertex_position_encoding: Encodings,
		triangle_count: u32,
		index_format: DataTypes,
	},
	AABB {
		transform_count: u32,
	},
}

pub struct BottomLevelAccelerationStructure {
	pub description: BottomLevelAccelerationStructureDescriptions,
}

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum Ranges {
	Size(usize),
	Whole,
}

pub struct BufferSplitter<'a, T: Copy> {
	buffer: &'a mut [T],
	offset: usize,
}

impl<'a, T: Copy> BufferSplitter<'a, T> {
	pub fn new(buffer: &'a mut [T], offset: usize) -> Self {
		Self { buffer, offset }
	}

	pub fn take(&mut self, size: usize) -> &'a mut [T] {
		let buffer = &mut self.buffer[self.offset..][..size];
		self.offset += size;
		// SAFETY: We know that the buffer is valid for the lifetime of the splitter.
		unsafe { std::mem::transmute(buffer) }
	}
}

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct FrameKey {
	/// The index of the frame.
	pub(crate) frame_index: u32,
	pub(crate) sequence_index: u8,
}

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct PresentKey {
	/// The index of the acquired swapchain image.
	pub(crate) image_index: u8,
	/// The index corresponding to the frame index.
	pub(crate) sequence_index: u8,
	/// The swapchain handle corresponding to the presentation request that this key is associated with.
	pub(crate) swapchain: SwapchainHandle,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RGBAu8 {
	r: u8,
	g: u8,
	b: u8,
	a: u8,
}

#[derive(Clone, Copy, Debug)]
pub enum PresentationModes {
	Inmediate,
	FIFO,
	Mailbox,
}

impl Default for PresentationModes {
	fn default() -> Self {
		Self::FIFO
	}
}

#[derive(Clone, Copy)]
pub enum ClearValue {
	None,
	Color(RGBA),
	Integer(u32, u32, u32, u32),
	Depth(f32),
}

#[derive(Clone, Copy)]
/// Stores the information of an attachment.
pub struct AttachmentInformation {
	/// The image view of the attachment.
	pub(crate) image: ImageHandle,
	/// The format of the attachment.
	pub(crate) format: Formats,
	/// The layout of the attachment.
	pub(crate) layout: Layouts,
	/// The clear color of the attachment.
	pub(crate) clear: ClearValue,
	/// Whether to load the contents of the attchment when starting a render pass.
	pub(crate) load: bool,
	/// Whether to store the contents of the attachment when ending a render pass.
	pub(crate) store: bool,
	/// The image layer index for the attachment.
	pub(crate) layer: Option<u32>,
}

impl AttachmentInformation {
	pub fn new(
		image: impl ImageHandleLike,
		format: Formats,
		layout: Layouts,
		clear: ClearValue,
		load: bool,
		store: bool,
	) -> Self {
		Self {
			image: image.into_image_handle(),
			format,
			layout,
			clear,
			load,
			store,
			layer: None,
		}
	}

	pub fn layer(mut self, layer: u32) -> Self {
		self.layer = Some(layer);
		self
	}
}

/// The `DescriptorSetBindingType` trait brands descriptor set binding templates with a compile-time descriptor type.
pub trait DescriptorSetBindingType {
	const DESCRIPTOR_TYPE: DescriptorType;
}

/// The `UniformBufferDescriptorBinding` struct brands a descriptor set binding template as a uniform-buffer binding.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct UniformBufferDescriptorBinding;

impl DescriptorSetBindingType for UniformBufferDescriptorBinding {
	const DESCRIPTOR_TYPE: DescriptorType = DescriptorType::UniformBuffer;
}

/// The `StorageBufferDescriptorBinding` struct brands a descriptor set binding template as a storage-buffer binding.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct StorageBufferDescriptorBinding;

impl DescriptorSetBindingType for StorageBufferDescriptorBinding {
	const DESCRIPTOR_TYPE: DescriptorType = DescriptorType::StorageBuffer;
}

/// The `SampledImageDescriptorBinding` struct brands a descriptor set binding template as a sampled-image binding.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct SampledImageDescriptorBinding;

impl DescriptorSetBindingType for SampledImageDescriptorBinding {
	const DESCRIPTOR_TYPE: DescriptorType = DescriptorType::SampledImage;
}

/// The `CombinedImageSamplerDescriptorBinding` struct brands a descriptor set binding template as a combined-image-sampler binding.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct CombinedImageSamplerDescriptorBinding;

impl DescriptorSetBindingType for CombinedImageSamplerDescriptorBinding {
	const DESCRIPTOR_TYPE: DescriptorType = DescriptorType::CombinedImageSampler;
}

/// The `StorageImageDescriptorBinding` struct brands a descriptor set binding template as a storage-image binding.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct StorageImageDescriptorBinding;

impl DescriptorSetBindingType for StorageImageDescriptorBinding {
	const DESCRIPTOR_TYPE: DescriptorType = DescriptorType::StorageImage;
}

/// The `InputAttachmentDescriptorBinding` struct brands a descriptor set binding template as an input-attachment binding.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct InputAttachmentDescriptorBinding;

impl DescriptorSetBindingType for InputAttachmentDescriptorBinding {
	const DESCRIPTOR_TYPE: DescriptorType = DescriptorType::InputAttachment;
}

/// The `SamplerDescriptorBinding` struct brands a descriptor set binding template as a sampler binding.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct SamplerDescriptorBinding;

impl DescriptorSetBindingType for SamplerDescriptorBinding {
	const DESCRIPTOR_TYPE: DescriptorType = DescriptorType::Sampler;
}

/// The `AccelerationStructureDescriptorBinding` struct brands a descriptor set binding template as an acceleration-structure binding.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct AccelerationStructureDescriptorBinding;

impl DescriptorSetBindingType for AccelerationStructureDescriptorBinding {
	const DESCRIPTOR_TYPE: DescriptorType = DescriptorType::AccelerationStructure;
}

/// Stores the information of a descriptor set layout binding.
#[derive(Clone)]
pub struct DescriptorSetBindingTemplate {
	/// The binding of the descriptor set layout binding.
	pub(crate) binding: u32,
	/// The descriptor type of the descriptor set layout binding.
	pub(crate) descriptor_type: DescriptorType,
	/// The number of descriptors in the descriptor set layout binding.
	pub(crate) descriptor_count: u32,
	/// The stages the descriptor set layout binding will be used in.
	pub(crate) stages: Stages,
	/// The immutable samplers of the descriptor set layout binding.
	pub(crate) immutable_samplers: Option<Vec<SamplerHandle>>,
}

/// The `TypedDescriptorSetBindingTemplate` struct provides branded descriptor-set binding templates for compile-time descriptor-type safety.
#[derive(Clone)]
pub struct TypedDescriptorSetBindingTemplate<T: DescriptorSetBindingType> {
	template: DescriptorSetBindingTemplate,
	type_brand: std::marker::PhantomData<T>,
}

impl<T: DescriptorSetBindingType> TypedDescriptorSetBindingTemplate<T> {
	pub const fn new(binding: u32, stages: Stages) -> Self {
		Self {
			template: DescriptorSetBindingTemplate::new(binding, T::DESCRIPTOR_TYPE, stages),
			type_brand: std::marker::PhantomData,
		}
	}

	pub const fn new_array(binding: u32, stages: Stages, count: u32) -> Self {
		Self {
			template: DescriptorSetBindingTemplate::new_array(binding, T::DESCRIPTOR_TYPE, stages, count),
			type_brand: std::marker::PhantomData,
		}
	}

	pub fn as_raw(&self) -> &DescriptorSetBindingTemplate {
		&self.template
	}

	pub fn into_raw(self) -> DescriptorSetBindingTemplate {
		self.template
	}

	pub fn binding(&self) -> u32 {
		self.template.binding()
	}
}

impl TypedDescriptorSetBindingTemplate<SamplerDescriptorBinding> {
	pub fn new_with_immutable_samplers(binding: u32, stages: Stages, samplers: Option<Vec<SamplerHandle>>) -> Self {
		Self {
			template: DescriptorSetBindingTemplate::new_with_immutable_samplers(binding, stages, samplers),
			type_brand: std::marker::PhantomData,
		}
	}
}

impl<T: DescriptorSetBindingType> AsRef<DescriptorSetBindingTemplate> for TypedDescriptorSetBindingTemplate<T> {
	fn as_ref(&self) -> &DescriptorSetBindingTemplate {
		self.as_raw()
	}
}

impl<T: DescriptorSetBindingType> From<TypedDescriptorSetBindingTemplate<T>> for DescriptorSetBindingTemplate {
	fn from(value: TypedDescriptorSetBindingTemplate<T>) -> Self {
		value.into_raw()
	}
}

pub type UniformBufferDescriptorSetBindingTemplate = TypedDescriptorSetBindingTemplate<UniformBufferDescriptorBinding>;
pub type StorageBufferDescriptorSetBindingTemplate = TypedDescriptorSetBindingTemplate<StorageBufferDescriptorBinding>;
pub type SampledImageDescriptorSetBindingTemplate = TypedDescriptorSetBindingTemplate<SampledImageDescriptorBinding>;
pub type CombinedImageSamplerDescriptorSetBindingTemplate =
	TypedDescriptorSetBindingTemplate<CombinedImageSamplerDescriptorBinding>;
pub type StorageImageDescriptorSetBindingTemplate = TypedDescriptorSetBindingTemplate<StorageImageDescriptorBinding>;
pub type InputAttachmentDescriptorSetBindingTemplate = TypedDescriptorSetBindingTemplate<InputAttachmentDescriptorBinding>;
pub type SamplerDescriptorSetBindingTemplate = TypedDescriptorSetBindingTemplate<SamplerDescriptorBinding>;
pub type AccelerationStructureDescriptorSetBindingTemplate =
	TypedDescriptorSetBindingTemplate<AccelerationStructureDescriptorBinding>;

impl DescriptorSetBindingTemplate {
	pub const fn new(binding: u32, descriptor_type: DescriptorType, stages: Stages) -> Self {
		Self {
			binding,
			descriptor_type,
			descriptor_count: 1,
			stages,
			immutable_samplers: None,
		}
	}

	pub const fn new_array(binding: u32, descriptor_type: DescriptorType, stages: Stages, count: u32) -> Self {
		Self {
			binding,
			descriptor_type,
			descriptor_count: count,
			stages,
			immutable_samplers: None,
		}
	}

	pub const fn uniform_buffer(binding: u32, stages: Stages) -> Self {
		Self::new(binding, DescriptorType::UniformBuffer, stages)
	}

	pub const fn uniform_buffer_array(binding: u32, stages: Stages, count: u32) -> Self {
		Self::new_array(binding, DescriptorType::UniformBuffer, stages, count)
	}

	pub const fn storage_buffer(binding: u32, stages: Stages) -> Self {
		Self::new(binding, DescriptorType::StorageBuffer, stages)
	}

	pub const fn storage_buffer_array(binding: u32, stages: Stages, count: u32) -> Self {
		Self::new_array(binding, DescriptorType::StorageBuffer, stages, count)
	}

	pub const fn sampled_image(binding: u32, stages: Stages) -> Self {
		Self::new(binding, DescriptorType::SampledImage, stages)
	}

	pub const fn sampled_image_array(binding: u32, stages: Stages, count: u32) -> Self {
		Self::new_array(binding, DescriptorType::SampledImage, stages, count)
	}

	pub const fn combined_image_sampler(binding: u32, stages: Stages) -> Self {
		Self::new(binding, DescriptorType::CombinedImageSampler, stages)
	}

	pub const fn combined_image_sampler_array(binding: u32, stages: Stages, count: u32) -> Self {
		Self::new_array(binding, DescriptorType::CombinedImageSampler, stages, count)
	}

	pub const fn storage_image(binding: u32, stages: Stages) -> Self {
		Self::new(binding, DescriptorType::StorageImage, stages)
	}

	pub const fn storage_image_array(binding: u32, stages: Stages, count: u32) -> Self {
		Self::new_array(binding, DescriptorType::StorageImage, stages, count)
	}

	pub const fn input_attachment(binding: u32, stages: Stages) -> Self {
		Self::new(binding, DescriptorType::InputAttachment, stages)
	}

	pub const fn input_attachment_array(binding: u32, stages: Stages, count: u32) -> Self {
		Self::new_array(binding, DescriptorType::InputAttachment, stages, count)
	}

	pub const fn sampler(binding: u32, stages: Stages) -> Self {
		Self::new(binding, DescriptorType::Sampler, stages)
	}

	pub const fn sampler_array(binding: u32, stages: Stages, count: u32) -> Self {
		Self::new_array(binding, DescriptorType::Sampler, stages, count)
	}

	pub const fn acceleration_structure(binding: u32, stages: Stages) -> Self {
		Self::new(binding, DescriptorType::AccelerationStructure, stages)
	}

	pub const fn acceleration_structure_array(binding: u32, stages: Stages, count: u32) -> Self {
		Self::new_array(binding, DescriptorType::AccelerationStructure, stages, count)
	}

	pub fn new_with_immutable_samplers(binding: u32, stages: Stages, samplers: Option<Vec<SamplerHandle>>) -> Self {
		Self {
			binding,
			descriptor_type: DescriptorType::Sampler,
			descriptor_count: 1,
			stages,
			immutable_samplers: samplers,
		}
	}

	pub fn into_shader_binding_descriptor(&self, set: u32, access_policies: AccessPolicies) -> BindingDescriptor {
		BindingDescriptor::new(set, self.binding, access_policies)
	}

	/// Returns the binding index of the descriptor set layout binding.
	pub fn binding(&self) -> u32 {
		self.binding
	}
}

pub struct BindingConstructor<'a> {
	pub(super) descriptor_set_binding_template: &'a DescriptorSetBindingTemplate,
	/// The index of the array element to write to in the binding(if the binding is an array).
	pub(super) array_element: u32,
	/// Information describing the descriptor.
	pub(super) descriptor: descriptors::WriteData,
	pub(super) frame_offset: Option<i8>,
}

impl<'a> BindingConstructor<'a> {
	pub fn buffer(descriptor_set_binding_template: &'a DescriptorSetBindingTemplate, buffer_handle: BaseBufferHandle) -> Self {
		Self {
			descriptor_set_binding_template,
			array_element: 0,
			descriptor: descriptors::WriteData::Buffer {
				handle: buffer_handle,
				size: Ranges::Whole,
			},
			frame_offset: None,
		}
	}

	pub fn image(
		descriptor_set_binding_template: &'a DescriptorSetBindingTemplate,
		image_handle: impl ImageHandleLike,
	) -> Self {
		Self {
			descriptor_set_binding_template,
			array_element: 0,
			descriptor: descriptors::WriteData::Image {
				handle: image_handle.into_image_handle(),
				layout: crate::Layouts::General,
			},
			frame_offset: None,
		}
	}

	pub fn sampler(descriptor_set_binding_template: &'a DescriptorSetBindingTemplate, sampler_handle: SamplerHandle) -> Self {
		Self {
			descriptor_set_binding_template,
			array_element: 0,
			descriptor: descriptors::WriteData::Sampler(sampler_handle),
			frame_offset: None,
		}
	}

	pub fn combined_image_sampler(
		descriptor_set_binding_template: &'a DescriptorSetBindingTemplate,
		image_handle: impl ImageHandleLike,
		sampler_handle: SamplerHandle,
		layout: Layouts,
	) -> Self {
		Self {
			descriptor_set_binding_template,
			array_element: 0,
			descriptor: descriptors::WriteData::CombinedImageSampler {
				image_handle: image_handle.into_image_handle(),
				sampler_handle,
				layout,
				layer: None,
			},
			frame_offset: None,
		}
	}

	pub fn combined_image_sampler_array(descriptor_set_binding_template: &'a DescriptorSetBindingTemplate) -> Self {
		Self {
			descriptor_set_binding_template,
			array_element: 0,
			descriptor: descriptors::WriteData::CombinedImageSamplerArray,
			frame_offset: None,
		}
	}

	pub fn combined_image_sampler_layer(
		descriptor_set_binding_template: &'a DescriptorSetBindingTemplate,
		image_handle: ImageHandle,
		sampler_handle: SamplerHandle,
		layout: Layouts,
		layer_index: u32,
	) -> Self {
		Self {
			descriptor_set_binding_template,
			array_element: 0,
			descriptor: descriptors::WriteData::CombinedImageSampler {
				image_handle,
				sampler_handle,
				layout,
				layer: Some(layer_index),
			},
			frame_offset: None,
		}
	}

	pub fn sampler_with_immutable_samplers(descriptor_set_binding_template: &'a DescriptorSetBindingTemplate) -> Self {
		Self {
			descriptor_set_binding_template,
			array_element: 0,
			descriptor: descriptors::WriteData::StaticSamplers,
			frame_offset: None,
		}
	}

	pub fn acceleration_structure(
		bindings: &'a DescriptorSetBindingTemplate,
		top_level_acceleration_structure: TopLevelAccelerationStructureHandle,
	) -> Self {
		BindingConstructor {
			descriptor_set_binding_template: bindings,
			array_element: 0,
			descriptor: descriptors::WriteData::AccelerationStructure {
				handle: top_level_acceleration_structure,
			},
			frame_offset: None,
		}
	}

	pub fn frame(mut self, frame_offset: i8) -> Self {
		self.frame_offset = Some(frame_offset);
		self
	}

	pub fn layout(mut self, layout: crate::Layouts) -> Self {
		match &mut self.descriptor {
			descriptors::WriteData::Image { layout: old_layout, .. } => {
				*old_layout = layout;
			}
			_ => (),
		}

		self
	}

	pub fn array_element(&self) -> u32 {
		self.array_element
	}
}

/// Describes the details of the memory layout of a particular image.
pub struct ImageSubresourceLayout {
	/// The offset inside a memory region where the texture will read it's first texel from.
	pub offset: usize,
	/// The size of the texture in bytes.
	pub size: usize,
	/// The row pitch of the texture.
	pub row_pitch: usize,
	/// The array pitch of the texture.
	pub array_pitch: usize,
	/// The depth pitch of the texture.
	pub depth_pitch: usize,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
/// Enumerates the states of a swapchain's validity for presentation.
pub enum SwapchainStates {
	/// The swapchain is valid for presentation.
	Ok,
	/// The swapchain is suboptimal for presentation.
	Suboptimal,
	/// The swapchain can't be used for presentation.
	Invalid,
}

pub enum AccelerationStructureTypes {
	TopLevel {
		instance_count: u32,
	},
	BottomLevel {
		vertex_count: u32,
		triangle_count: u32,
		vertex_position_format: DataTypes,
		index_format: DataTypes,
	},
}

pub struct QueueSelection {
	pub(crate) r#type: WorkloadTypes,
}

impl QueueSelection {
	pub fn new(r#type: WorkloadTypes) -> Self {
		Self { r#type }
	}
}

#[cfg(test)]
pub(super) mod tests {
	use std::borrow::Borrow as _;

	use crate::{
		command_buffer::{
			BoundComputePipelineMode as _, BoundPipelineLayoutMode as _, BoundRasterizationPipelineMode as _,
			BoundRayTracingPipelineMode as _, CommandBufferRecording as _, CommonCommandBufferMode as _,
			RasterizationRenderPassMode as _,
		},
		device::Device,
		frame::Frame as _,
		pipelines::{self, raster::AttachmentDescriptor, PushConstantRange, ShaderParameter, VertexElement},
		rt::{
			BindingTables, BottomLevelAccelerationStructureBuild, BottomLevelAccelerationStructureBuildDescriptions,
			TopLevelAccelerationStructureBuild, TopLevelAccelerationStructureBuildDescriptions,
		},
		shader::Sources,
		window::Window,
		BufferDescriptor, BufferStridedRange, ChannelBitSize, ChannelLayout, DeviceAccesses, FilteringModes,
		SamplerAddressingModes, SamplingReductionModes, ShaderTypes, Size as _, UseCases, Uses,
	};

	use resource_management::glsl;

	use super::*;

	#[test]
	fn test_formats_encoding() {
		// Test floating point formats
		assert_eq!(Formats::R8F.encoding(), Some(Encodings::FloatingPoint));
		assert_eq!(Formats::R16F.encoding(), Some(Encodings::FloatingPoint));
		assert_eq!(Formats::R32F.encoding(), Some(Encodings::FloatingPoint));
		assert_eq!(Formats::RG8F.encoding(), Some(Encodings::FloatingPoint));
		assert_eq!(Formats::RG16F.encoding(), Some(Encodings::FloatingPoint));
		assert_eq!(Formats::RGB8F.encoding(), Some(Encodings::FloatingPoint));
		assert_eq!(Formats::RGB16F.encoding(), Some(Encodings::FloatingPoint));
		assert_eq!(Formats::RGBA8F.encoding(), Some(Encodings::FloatingPoint));
		assert_eq!(Formats::RGBA16F.encoding(), Some(Encodings::FloatingPoint));
		assert_eq!(Formats::Depth32.encoding(), Some(Encodings::FloatingPoint));

		// Test unsigned normalized formats
		assert_eq!(Formats::R8UNORM.encoding(), Some(Encodings::UnsignedNormalized));
		assert_eq!(Formats::R16UNORM.encoding(), Some(Encodings::UnsignedNormalized));
		assert_eq!(Formats::R32UNORM.encoding(), Some(Encodings::UnsignedNormalized));
		assert_eq!(Formats::RG8UNORM.encoding(), Some(Encodings::UnsignedNormalized));
		assert_eq!(Formats::RG16UNORM.encoding(), Some(Encodings::UnsignedNormalized));
		assert_eq!(Formats::RGB8UNORM.encoding(), Some(Encodings::UnsignedNormalized));
		assert_eq!(Formats::RGB16UNORM.encoding(), Some(Encodings::UnsignedNormalized));
		assert_eq!(Formats::RGBA8UNORM.encoding(), Some(Encodings::UnsignedNormalized));
		assert_eq!(Formats::RGBA16UNORM.encoding(), Some(Encodings::UnsignedNormalized));
		assert_eq!(Formats::RGBu11u11u10.encoding(), Some(Encodings::UnsignedNormalized));
		assert_eq!(Formats::BGRAu8.encoding(), Some(Encodings::UnsignedNormalized));

		// Test signed normalized formats
		assert_eq!(Formats::R8SNORM.encoding(), Some(Encodings::SignedNormalized));
		assert_eq!(Formats::R16SNORM.encoding(), Some(Encodings::SignedNormalized));
		assert_eq!(Formats::R32SNORM.encoding(), Some(Encodings::SignedNormalized));
		assert_eq!(Formats::RG8SNORM.encoding(), Some(Encodings::SignedNormalized));
		assert_eq!(Formats::RG16SNORM.encoding(), Some(Encodings::SignedNormalized));
		assert_eq!(Formats::RGB8SNORM.encoding(), Some(Encodings::SignedNormalized));
		assert_eq!(Formats::RGB16SNORM.encoding(), Some(Encodings::SignedNormalized));
		assert_eq!(Formats::RGBA8SNORM.encoding(), Some(Encodings::SignedNormalized));
		assert_eq!(Formats::RGBA16SNORM.encoding(), Some(Encodings::SignedNormalized));

		// Test sRGB formats
		assert_eq!(Formats::R8sRGB.encoding(), Some(Encodings::sRGB));
		assert_eq!(Formats::R16sRGB.encoding(), Some(Encodings::sRGB));
		assert_eq!(Formats::R32sRGB.encoding(), Some(Encodings::sRGB));
		assert_eq!(Formats::RG8sRGB.encoding(), Some(Encodings::sRGB));
		assert_eq!(Formats::RG16sRGB.encoding(), Some(Encodings::sRGB));
		assert_eq!(Formats::RGB8sRGB.encoding(), Some(Encodings::sRGB));
		assert_eq!(Formats::RGB16sRGB.encoding(), Some(Encodings::sRGB));
		assert_eq!(Formats::RGBA8sRGB.encoding(), Some(Encodings::sRGB));
		assert_eq!(Formats::RGBA16sRGB.encoding(), Some(Encodings::sRGB));
		assert_eq!(Formats::BGRAsRGB.encoding(), Some(Encodings::sRGB));

		// Test formats without encoding
		assert_eq!(Formats::U32.encoding(), None);
		assert_eq!(Formats::BC5.encoding(), None);
		assert_eq!(Formats::BC7.encoding(), None);
	}

	#[test]
	fn descriptor_set_binding_template_type_specific_variants() {
		let stages = Stages::COMPUTE;

		let templates = [
			DescriptorSetBindingTemplate::uniform_buffer(0, stages),
			DescriptorSetBindingTemplate::storage_buffer(1, stages),
			DescriptorSetBindingTemplate::sampled_image(2, stages),
			DescriptorSetBindingTemplate::combined_image_sampler(3, stages),
			DescriptorSetBindingTemplate::storage_image(4, stages),
			DescriptorSetBindingTemplate::input_attachment(5, stages),
			DescriptorSetBindingTemplate::sampler(6, stages),
			DescriptorSetBindingTemplate::acceleration_structure(7, stages),
		];

		assert!(matches!(templates[0].descriptor_type, DescriptorType::UniformBuffer));
		assert!(matches!(templates[1].descriptor_type, DescriptorType::StorageBuffer));
		assert!(matches!(templates[2].descriptor_type, DescriptorType::SampledImage));
		assert!(matches!(templates[3].descriptor_type, DescriptorType::CombinedImageSampler));
		assert!(matches!(templates[4].descriptor_type, DescriptorType::StorageImage));
		assert!(matches!(templates[5].descriptor_type, DescriptorType::InputAttachment));
		assert!(matches!(templates[6].descriptor_type, DescriptorType::Sampler));
		assert!(matches!(templates[7].descriptor_type, DescriptorType::AccelerationStructure));

		for template in templates {
			assert_eq!(template.descriptor_count, 1);
		}

		let array_templates = [
			DescriptorSetBindingTemplate::uniform_buffer_array(8, stages, 2),
			DescriptorSetBindingTemplate::storage_buffer_array(9, stages, 3),
			DescriptorSetBindingTemplate::sampled_image_array(10, stages, 4),
			DescriptorSetBindingTemplate::combined_image_sampler_array(11, stages, 5),
			DescriptorSetBindingTemplate::storage_image_array(12, stages, 6),
			DescriptorSetBindingTemplate::input_attachment_array(13, stages, 7),
			DescriptorSetBindingTemplate::sampler_array(14, stages, 8),
			DescriptorSetBindingTemplate::acceleration_structure_array(15, stages, 9),
		];

		assert_eq!(array_templates[0].descriptor_count, 2);
		assert_eq!(array_templates[1].descriptor_count, 3);
		assert_eq!(array_templates[2].descriptor_count, 4);
		assert_eq!(array_templates[3].descriptor_count, 5);
		assert_eq!(array_templates[4].descriptor_count, 6);
		assert_eq!(array_templates[5].descriptor_count, 7);
		assert_eq!(array_templates[6].descriptor_count, 8);
		assert_eq!(array_templates[7].descriptor_count, 9);
	}

	#[test]
	fn typed_descriptor_set_binding_templates() {
		let stages = Stages::COMPUTE;

		let storage_buffer = StorageBufferDescriptorSetBindingTemplate::new(0, stages);
		let storage_image = StorageImageDescriptorSetBindingTemplate::new(1, stages);
		let storage_buffer_array = StorageBufferDescriptorSetBindingTemplate::new_array(2, stages, 8);
		let sampler = SamplerDescriptorSetBindingTemplate::new_with_immutable_samplers(3, stages, None);

		assert!(matches!(
			storage_buffer.as_raw().descriptor_type,
			DescriptorType::StorageBuffer
		));
		assert!(matches!(storage_image.as_raw().descriptor_type, DescriptorType::StorageImage));
		assert_eq!(storage_buffer_array.as_raw().descriptor_count, 8);
		assert!(matches!(sampler.as_raw().descriptor_type, DescriptorType::Sampler));

		let raw_template: DescriptorSetBindingTemplate = storage_buffer.into();
		assert!(matches!(raw_template.descriptor_type, DescriptorType::StorageBuffer));
	}

	#[test]
	fn test_formats_channel_bit_size() {
		// Test 8-bit formats
		assert_eq!(Formats::R8F.channel_bit_size(), ChannelBitSize::Bits8);
		assert_eq!(Formats::R8UNORM.channel_bit_size(), ChannelBitSize::Bits8);
		assert_eq!(Formats::R8SNORM.channel_bit_size(), ChannelBitSize::Bits8);
		assert_eq!(Formats::R8sRGB.channel_bit_size(), ChannelBitSize::Bits8);
		assert_eq!(Formats::RG8F.channel_bit_size(), ChannelBitSize::Bits8);
		assert_eq!(Formats::RGB8UNORM.channel_bit_size(), ChannelBitSize::Bits8);
		assert_eq!(Formats::RGBA8SNORM.channel_bit_size(), ChannelBitSize::Bits8);
		assert_eq!(Formats::BGRAu8.channel_bit_size(), ChannelBitSize::Bits8);
		assert_eq!(Formats::BGRAsRGB.channel_bit_size(), ChannelBitSize::Bits8);

		// Test 16-bit formats
		assert_eq!(Formats::R16F.channel_bit_size(), ChannelBitSize::Bits16);
		assert_eq!(Formats::R16UNORM.channel_bit_size(), ChannelBitSize::Bits16);
		assert_eq!(Formats::RG16SNORM.channel_bit_size(), ChannelBitSize::Bits16);
		assert_eq!(Formats::RGB16F.channel_bit_size(), ChannelBitSize::Bits16);
		assert_eq!(Formats::RGBA16UNORM.channel_bit_size(), ChannelBitSize::Bits16);

		// Test 32-bit formats
		assert_eq!(Formats::R32F.channel_bit_size(), ChannelBitSize::Bits32);
		assert_eq!(Formats::R32UNORM.channel_bit_size(), ChannelBitSize::Bits32);
		assert_eq!(Formats::Depth32.channel_bit_size(), ChannelBitSize::Bits32);
		assert_eq!(Formats::U32.channel_bit_size(), ChannelBitSize::Bits32);

		// Test special formats
		assert_eq!(Formats::RGBu11u11u10.channel_bit_size(), ChannelBitSize::Bits11_11_10);
		assert_eq!(Formats::BC5.channel_bit_size(), ChannelBitSize::Compressed);
		assert_eq!(Formats::BC7.channel_bit_size(), ChannelBitSize::Compressed);
	}

	#[test]
	fn test_formats_channel_layout() {
		// Test single channel formats
		assert_eq!(Formats::R8F.channel_layout(), ChannelLayout::R);
		assert_eq!(Formats::R16UNORM.channel_layout(), ChannelLayout::R);
		assert_eq!(Formats::R32SNORM.channel_layout(), ChannelLayout::R);
		assert_eq!(Formats::R8sRGB.channel_layout(), ChannelLayout::R);

		// Test dual channel formats
		assert_eq!(Formats::RG8F.channel_layout(), ChannelLayout::RG);
		assert_eq!(Formats::RG16UNORM.channel_layout(), ChannelLayout::RG);
		assert_eq!(Formats::RG8SNORM.channel_layout(), ChannelLayout::RG);

		// Test triple channel formats
		assert_eq!(Formats::RGB8F.channel_layout(), ChannelLayout::RGB);
		assert_eq!(Formats::RGB16UNORM.channel_layout(), ChannelLayout::RGB);
		assert_eq!(Formats::RGB8sRGB.channel_layout(), ChannelLayout::RGB);
		assert_eq!(Formats::RGBu11u11u10.channel_layout(), ChannelLayout::RGB);

		// Test quad channel formats
		assert_eq!(Formats::RGBA8F.channel_layout(), ChannelLayout::RGBA);
		assert_eq!(Formats::RGBA16UNORM.channel_layout(), ChannelLayout::RGBA);
		assert_eq!(Formats::RGBA8SNORM.channel_layout(), ChannelLayout::RGBA);

		// Test BGRA format
		assert_eq!(Formats::BGRAu8.channel_layout(), ChannelLayout::BGRA);
		assert_eq!(Formats::BGRAsRGB.channel_layout(), ChannelLayout::BGRA);

		// Test depth format
		assert_eq!(Formats::Depth32.channel_layout(), ChannelLayout::Depth);

		// Test packed format
		assert_eq!(Formats::U32.channel_layout(), ChannelLayout::Packed);

		// Test block compressed formats
		assert_eq!(Formats::BC5.channel_layout(), ChannelLayout::BC);
		assert_eq!(Formats::BC7.channel_layout(), ChannelLayout::BC);
	}

	#[test]
	fn test_formats_size() {
		// Test single channel formats
		assert_eq!(Formats::R8F.size(), 1);
		assert_eq!(Formats::R8UNORM.size(), 1);
		assert_eq!(Formats::R16F.size(), 2);
		assert_eq!(Formats::R16UNORM.size(), 2);
		assert_eq!(Formats::R32F.size(), 4);
		assert_eq!(Formats::R32SNORM.size(), 4);

		// Test dual channel formats
		assert_eq!(Formats::RG8F.size(), 2);
		assert_eq!(Formats::RG8UNORM.size(), 2);
		assert_eq!(Formats::RG16F.size(), 4);
		assert_eq!(Formats::RG16SNORM.size(), 4);

		// Test triple channel formats
		assert_eq!(Formats::RGB8F.size(), 3);
		assert_eq!(Formats::RGB8UNORM.size(), 3);
		assert_eq!(Formats::RGB16F.size(), 6);
		assert_eq!(Formats::RGB16SNORM.size(), 6);

		// Test quad channel formats
		assert_eq!(Formats::RGBA8F.size(), 4);
		assert_eq!(Formats::RGBA8UNORM.size(), 4);
		assert_eq!(Formats::RGBA16F.size(), 8);
		assert_eq!(Formats::RGBA16UNORM.size(), 8);

		// Test special formats
		assert_eq!(Formats::RGBu11u11u10.size(), 4);
		assert_eq!(Formats::BGRAu8.size(), 4);
		assert_eq!(Formats::BGRAsRGB.size(), 4);
		assert_eq!(Formats::Depth32.size(), 4);
		assert_eq!(Formats::U32.size(), 4);
		assert_eq!(Formats::BC5.size(), 1);
		assert_eq!(Formats::BC7.size(), 1);
	}

	#[test]
	fn test_formats_comprehensive() {
		// Test that encoding, channel_bit_size, and channel_layout are consistent
		// For R8FloatingPoint
		let format = Formats::R8F;
		assert_eq!(format.encoding(), Some(Encodings::FloatingPoint));
		assert_eq!(format.channel_bit_size(), ChannelBitSize::Bits8);
		assert_eq!(format.channel_layout(), ChannelLayout::R);
		assert_eq!(format.size(), 1);

		// For RGBA16UnsignedNormalized
		let format = Formats::RGBA16UNORM;
		assert_eq!(format.encoding(), Some(Encodings::UnsignedNormalized));
		assert_eq!(format.channel_bit_size(), ChannelBitSize::Bits16);
		assert_eq!(format.channel_layout(), ChannelLayout::RGBA);
		assert_eq!(format.size(), 8);

		// For RGB8sRGB
		let format = Formats::RGB8sRGB;
		assert_eq!(format.encoding(), Some(Encodings::sRGB));
		assert_eq!(format.channel_bit_size(), ChannelBitSize::Bits8);
		assert_eq!(format.channel_layout(), ChannelLayout::RGB);
		assert_eq!(format.size(), 3);

		// For special format RGBu11u11u10
		let format = Formats::RGBu11u11u10;
		assert_eq!(format.encoding(), Some(Encodings::UnsignedNormalized));
		assert_eq!(format.channel_bit_size(), ChannelBitSize::Bits11_11_10);
		assert_eq!(format.channel_layout(), ChannelLayout::RGB);
		assert_eq!(format.size(), 4);

		// For BGRAu8
		let format = Formats::BGRAu8;
		assert_eq!(format.encoding(), Some(Encodings::UnsignedNormalized));
		assert_eq!(format.channel_bit_size(), ChannelBitSize::Bits8);
		assert_eq!(format.channel_layout(), ChannelLayout::BGRA);
		assert_eq!(format.size(), 4);

		// For BGRAsRGB
		let format = Formats::BGRAsRGB;
		assert_eq!(format.encoding(), Some(Encodings::sRGB));
		assert_eq!(format.channel_bit_size(), ChannelBitSize::Bits8);
		assert_eq!(format.channel_layout(), ChannelLayout::BGRA);
		assert_eq!(format.size(), 4);

		// For Depth32
		let format = Formats::Depth32;
		assert_eq!(format.encoding(), Some(Encodings::FloatingPoint));
		assert_eq!(format.channel_bit_size(), ChannelBitSize::Bits32);
		assert_eq!(format.channel_layout(), ChannelLayout::Depth);
		assert_eq!(format.size(), 4);

		// For BC7
		let format = Formats::BC7;
		assert_eq!(format.encoding(), None);
		assert_eq!(format.channel_bit_size(), ChannelBitSize::Compressed);
		assert_eq!(format.channel_layout(), ChannelLayout::BC);
		assert_eq!(format.size(), 1);
	}

	fn compile_shaders() -> (glsl::CompiledShader, glsl::CompiledShader) {
		let vertex_shader_code = "
			#version 450
			#pragma shader_stage(vertex)

			layout(location = 0) in vec3 in_position;
			layout(location = 1) in vec4 in_color;

			layout(location = 0) out vec4 out_color;

			void main() {
				out_color = in_color;
				gl_Position = vec4(in_position, 1.0);
			}
		";

		let fragment_shader_code = "
			#version 450
			#pragma shader_stage(fragment)

			layout(location = 0) in vec4 in_color;

			layout(location = 0) out vec4 out_color;

			void main() {
				out_color = in_color;
			}
		";

		let vertex_shader_artifact = glsl::compile(vertex_shader_code, "vertex").unwrap();
		let fragment_shader_artifact = glsl::compile(fragment_shader_code, "fragment").unwrap();

		(vertex_shader_artifact, fragment_shader_artifact)
	}

	fn compile_shaders_with_model_matrix() -> (glsl::CompiledShader, glsl::CompiledShader) {
		let vertex_shader_code = "
			#version 450
			#pragma shader_stage(vertex)

			layout(location = 0) in vec3 in_position;
			layout(location = 1) in vec4 in_color;

			layout(location = 0) out vec4 out_color;

			layout(push_constant) uniform ModelMatrix {
				mat4 model_matrix;
			} push_constants;

			void main() {
				out_color = in_color;
				gl_Position = push_constants.model_matrix * vec4(in_position, 1.0);
			}
		";

		let fragment_shader_code = "
			#version 450
			#pragma shader_stage(fragment)

			layout(location = 0) in vec4 in_color;

			layout(location = 0) out vec4 out_color;

			void main() {
				out_color = in_color;
			}
		";

		let vertex_shader_artifact = glsl::compile(vertex_shader_code, "vertex").unwrap();
		let fragment_shader_artifact = glsl::compile(fragment_shader_code, "fragment").unwrap();

		(vertex_shader_artifact, fragment_shader_artifact)
	}

	#[test]
	fn dispatch_extent() {
		let dispatch_extent = DispatchExtent::new(Extent::new(10, 10, 10), Extent::new(5, 5, 5));
		assert_eq!(dispatch_extent.get_extent(), Extent::new(2, 2, 2));

		let dispatch_extent = DispatchExtent::new(Extent::new(10, 10, 10), Extent::new(3, 3, 3));
		assert_eq!(dispatch_extent.get_extent(), Extent::new(4, 4, 4));
	}

	fn check_triangle(pixels: &[RGBAu8], extent: Extent) {
		assert_eq!(pixels.len(), (extent.width() * extent.height()) as usize);

		let pixel = pixels[0]; // top left
		assert_eq!(
			pixel,
			RGBAu8 {
				r: 0,
				g: 0,
				b: 0,
				a: 255
			}
		);

		if extent.width() % 2 != 0 {
			let pixel = pixels[(extent.width() / 2) as usize]; // middle top center
			assert_eq!(
				pixel,
				RGBAu8 {
					r: 255,
					g: 0,
					b: 0,
					a: 255
				}
			);
		}

		let pixel = pixels[(extent.width() - 1) as usize]; // top right
		assert_eq!(
			pixel,
			RGBAu8 {
				r: 0,
				g: 0,
				b: 0,
				a: 255
			}
		);

		let pixel = pixels[(extent.width() * (extent.height() - 1)) as usize]; // bottom left
		assert_eq!(
			pixel,
			RGBAu8 {
				r: 0,
				g: 0,
				b: 255,
				a: 255
			}
		);

		let pixel = pixels[(extent.width() * extent.height() - (extent.width() / 2)) as usize]; // middle bottom center
		assert!(
			pixel
				== RGBAu8 {
					r: 0,
					g: 127,
					b: 127,
					a: 255
				} || pixel
				== RGBAu8 {
					r: 0,
					g: 128,
					b: 127,
					a: 255
				}
		); // different implementations render slightly differently

		let pixel = pixels[(extent.width() * extent.height() - 1) as usize]; // bottom right
		assert_eq!(
			pixel,
			RGBAu8 {
				r: 0,
				g: 255,
				b: 0,
				a: 255
			}
		);
	}

	pub(crate) fn render_triangle(device: &mut impl Device, queue_handle: QueueHandle) {
		let signal = device.create_synchronizer(None, false);

		let floats: [f32; 21] = [
			0.0, 1.0, 0.0, 1.0, 0.0, 0.0, 1.0, 1.0, -1.0, 0.0, 0.0, 1.0, 0.0, 1.0, -1.0, -1.0, 0.0, 0.0, 0.0, 1.0, 1.0,
		];

		let vertex_layout = [
			VertexElement::new("POSITION", DataTypes::Float3, 0),
			VertexElement::new("COLOR", DataTypes::Float4, 0),
		];

		let mesh = unsafe {
			device.add_mesh_from_vertices_and_indices(
				3,
				3,
				std::slice::from_raw_parts(floats.as_ptr() as *const u8, (3 * 4 + 4 * 4) * 3),
				std::slice::from_raw_parts([0u16, 1u16, 2u16].as_ptr() as *const u8, 3 * 2),
				&vertex_layout,
			)
		};

		let (vertex_shader_artifact, fragment_shader_artifact) = compile_shaders();

		let vertex_shader = device
			.create_shader(
				None,
				Sources::SPIRV(vertex_shader_artifact.borrow().into()),
				ShaderTypes::Vertex,
				[],
			)
			.expect("Failed to create vertex shader");
		let fragment_shader = device
			.create_shader(
				None,
				Sources::SPIRV(fragment_shader_artifact.borrow().into()),
				ShaderTypes::Fragment,
				[],
			)
			.expect("Failed to create fragment shader");

		// Use and odd width to make sure there is a middle/center pixel
		let extent = Extent::rectangle(1921, 1080);

		let render_target = device.build_image(
			crate::image::Builder::new(Formats::RGBA8UNORM, Uses::RenderTarget)
				.extent(extent)
				.device_accesses(DeviceAccesses::DeviceToHost)
				.use_case(UseCases::STATIC),
		);

		let attachments = [AttachmentDescriptor::new(Formats::RGBA8UNORM)];

		let pipeline = device.create_raster_pipeline(pipelines::raster::Builder::new(
			&[],
			&[],
			&vertex_layout,
			&[
				ShaderParameter::new(&vertex_shader, ShaderTypes::Vertex),
				ShaderParameter::new(&fragment_shader, ShaderTypes::Fragment),
			],
			&attachments,
		));

		let command_buffer_handle = device.create_command_buffer(None, queue_handle);

		device.start_frame_capture();

		let mut command_buffer_recording = device.create_command_buffer_recording(command_buffer_handle);

		let attachments = [AttachmentInformation::new(
			render_target,
			Formats::RGBA8UNORM,
			Layouts::RenderTarget,
			ClearValue::Color(RGBA::black()),
			false,
			true,
		)];

		let render_pass_command = command_buffer_recording.start_render_pass(extent, &attachments);

		let raster_pipeline_command = render_pass_command.bind_raster_pipeline(pipeline);

		raster_pipeline_command.draw_mesh(&mesh);

		render_pass_command.end_render_pass();

		let texture_copy_handles = command_buffer_recording.transfer_textures(&[render_target]);

		command_buffer_recording.execute(signal);

		device.end_frame_capture();

		device.wait();

		assert!(!device.has_errors());

		// Get image data and cast u8 slice to rgbau8
		let pixels = unsafe {
			std::slice::from_raw_parts(
				device.get_image_data(texture_copy_handles[0]).as_ptr() as *const RGBAu8,
				(extent.width() * extent.height()) as usize,
			)
		};

		check_triangle(pixels, extent);
	}

	pub(crate) fn present(renderer: &mut impl Device, queue_handle: QueueHandle) {
		// Use and odd width to make sure there is a middle/center pixel
		let extent = Extent::rectangle(1921, 1080);

		let mut window = Window::new("Present Test", extent).expect("Failed to create window");

		let os_handles = window.os_handles();

		let swapchain = renderer.bind_to_window(&os_handles, Default::default(), extent, Uses::RenderTarget);

		let floats: [f32; 21] = [
			0.0, 1.0, 0.0, 1.0, 0.0, 0.0, 1.0, 1.0, -1.0, 0.0, 0.0, 1.0, 0.0, 1.0, -1.0, -1.0, 0.0, 0.0, 0.0, 1.0, 1.0,
		];

		let vertex_layout = [
			VertexElement::new("POSITION", DataTypes::Float3, 0),
			VertexElement::new("COLOR", DataTypes::Float4, 0),
		];

		let mesh = unsafe {
			renderer.add_mesh_from_vertices_and_indices(
				3,
				3,
				std::slice::from_raw_parts(floats.as_ptr() as *const u8, (3 * 4 + 4 * 4) * 3),
				std::slice::from_raw_parts([0u16, 1u16, 2u16].as_ptr() as *const u8, 3 * 2),
				&vertex_layout,
			)
		};

		let (vertex_shader_artifact, fragment_shader_artifact) = compile_shaders();

		let vertex_shader = renderer
			.create_shader(
				None,
				Sources::SPIRV(vertex_shader_artifact.borrow().into()),
				ShaderTypes::Vertex,
				[],
			)
			.expect("Failed to create vertex shader");
		let fragment_shader = renderer
			.create_shader(
				None,
				Sources::SPIRV(fragment_shader_artifact.borrow().into()),
				ShaderTypes::Fragment,
				[],
			)
			.expect("Failed to create fragment shader");

		let render_target = renderer.build_image(
			crate::image::Builder::new(Formats::RGBA8UNORM, Uses::RenderTarget)
				.extent(extent)
				.device_accesses(DeviceAccesses::DeviceOnly)
				.use_case(UseCases::STATIC),
		);

		let attachments = [AttachmentDescriptor::new(Formats::RGBA8UNORM)];

		let pipeline = renderer.create_raster_pipeline(pipelines::raster::Builder::new(
			&[],
			&[],
			&vertex_layout,
			&[
				ShaderParameter::new(&vertex_shader, ShaderTypes::Vertex),
				ShaderParameter::new(&fragment_shader, ShaderTypes::Fragment),
			],
			&attachments,
		));

		let command_buffer_handle = renderer.create_command_buffer(None, queue_handle);

		let render_finished_synchronizer = renderer.create_synchronizer(None, true);

		for _ in window.poll() {}

		renderer.start_frame_capture();

		let mut frame = renderer.start_frame(0, render_finished_synchronizer);

		let (present_key, _) = frame.acquire_swapchain_image(swapchain);

		let mut command_buffer_recording = frame.create_command_buffer_recording(command_buffer_handle);

		let attachments = [AttachmentInformation::new(
			render_target,
			Formats::RGBA8UNORM,
			Layouts::RenderTarget,
			ClearValue::Color(RGBA::black()),
			false,
			true,
		)];

		let render_pass_command = command_buffer_recording.start_render_pass(extent, &attachments);

		let raster_pipeline_command = render_pass_command.bind_raster_pipeline(pipeline);

		raster_pipeline_command.draw_mesh(&mesh);

		render_pass_command.end_render_pass();

		command_buffer_recording.copy_to_swapchain(render_target, present_key, swapchain);

		let present_keys = [present_key];
		let terminated_command_buffer = command_buffer_recording.end(&present_keys);
		frame.execute(terminated_command_buffer, render_finished_synchronizer);

		renderer.end_frame_capture();

		for _ in window.poll() {}

		// TODO: assert rendering results

		assert!(!renderer.has_errors())
	}

	pub(crate) fn multiframe_present(renderer: &mut impl Device, queue_handle: QueueHandle) {
		// Use and odd width to make sure there is a middle/center pixel
		let extent = Extent::rectangle(1920, 1080);

		let window = Window::new("Present Test", extent).expect("Failed to create window");

		let os_handles = window.os_handles();

		let swapchain = renderer.bind_to_window(&os_handles, Default::default(), extent, Uses::RenderTarget);

		let floats: [f32; 21] = [
			0.0, 1.0, 0.0, 1.0, 0.0, 0.0, 1.0, 1.0, -1.0, 0.0, 0.0, 1.0, 0.0, 1.0, -1.0, -1.0, 0.0, 0.0, 0.0, 1.0, 1.0,
		];

		let vertex_layout = [
			VertexElement::new("POSITION", DataTypes::Float3, 0),
			VertexElement::new("COLOR", DataTypes::Float4, 0),
		];

		let mesh = unsafe {
			renderer.add_mesh_from_vertices_and_indices(
				3,
				3,
				std::slice::from_raw_parts(floats.as_ptr() as *const u8, (3 * 4 + 4 * 4) * 3),
				std::slice::from_raw_parts([0u16, 1u16, 2u16].as_ptr() as *const u8, 3 * 2),
				&vertex_layout,
			)
		};

		let (vertex_shader_artifact, fragment_shader_artifact) = compile_shaders();

		let vertex_shader = renderer
			.create_shader(
				None,
				Sources::SPIRV(vertex_shader_artifact.borrow().into()),
				ShaderTypes::Vertex,
				[],
			)
			.expect("Failed to create vertex shader");
		let fragment_shader = renderer
			.create_shader(
				None,
				Sources::SPIRV(fragment_shader_artifact.borrow().into()),
				ShaderTypes::Fragment,
				[],
			)
			.expect("Failed to create fragment shader");

		let render_target = renderer.build_image(
			crate::image::Builder::new(Formats::RGBA8UNORM, Uses::RenderTarget)
				.extent(extent)
				.device_accesses(DeviceAccesses::DeviceToHost)
				.use_case(UseCases::DYNAMIC),
		);

		let attachments = [AttachmentDescriptor::new(Formats::RGBA8UNORM)];

		let pipeline = renderer.create_raster_pipeline(pipelines::raster::Builder::new(
			&[],
			&[],
			&vertex_layout,
			&[
				ShaderParameter::new(&vertex_shader, ShaderTypes::Vertex),
				ShaderParameter::new(&fragment_shader, ShaderTypes::Fragment),
			],
			&attachments,
		));

		let command_buffer_handle = renderer.create_command_buffer(None, queue_handle);

		let render_finished_synchronizer = renderer.create_synchronizer(None, true);

		for i in 0..2 * 64 {
			renderer.start_frame_capture();

			let mut frame = renderer.start_frame(i, render_finished_synchronizer);

			let (present_key, _) = frame.acquire_swapchain_image(swapchain);

			let mut command_buffer_recording = frame.create_command_buffer_recording(command_buffer_handle);

			let attachments = [AttachmentInformation::new(
				render_target,
				Formats::RGBA8UNORM,
				Layouts::RenderTarget,
				ClearValue::Color(RGBA {
					r: 0.0,
					g: 0.0,
					b: 0.0,
					a: 1.0,
				}),
				false,
				true,
			)];

			let render_pass_command = command_buffer_recording.start_render_pass(extent, &attachments);

			let raster_pipeline_command = render_pass_command.bind_raster_pipeline(pipeline);

			raster_pipeline_command.draw_mesh(&mesh);

			raster_pipeline_command.end_render_pass();

			command_buffer_recording.copy_to_swapchain(render_target, present_key, swapchain);

			let present_keys = [present_key];
			let terminated_command_buffer = command_buffer_recording.end(&present_keys);
			frame.execute(terminated_command_buffer, render_finished_synchronizer);

			renderer.end_frame_capture();

			assert!(!renderer.has_errors());
		}
	}

	pub(crate) fn multiframe_rendering(device: &mut impl Device, queue_handle: QueueHandle) {
		//! Tests that the render system can perform rendering with multiple frames in flight.
		//! Having multiple frames in flight means allocating and managing multiple resources under a single handle, one for each frame.

		const FRAMES_IN_FLIGHT: usize = 2;

		// Use and odd width to make sure there is a middle/center pixel
		let _extent = Extent::rectangle(1920, 1080);

		let floats: [f32; 21] = [
			0.0, 1.0, 0.0, 1.0, 0.0, 0.0, 1.0, 1.0, -1.0, 0.0, 0.0, 1.0, 0.0, 1.0, -1.0, -1.0, 0.0, 0.0, 0.0, 1.0, 1.0,
		];

		let vertex_layout = [
			VertexElement::new("POSITION", DataTypes::Float3, 0),
			VertexElement::new("COLOR", DataTypes::Float4, 0),
		];

		let mesh = unsafe {
			device.add_mesh_from_vertices_and_indices(
				3,
				3,
				std::slice::from_raw_parts(floats.as_ptr() as *const u8, (3 * 4 + 4 * 4) * 3),
				std::slice::from_raw_parts([0u16, 1u16, 2u16].as_ptr() as *const u8, 3 * 2),
				&vertex_layout,
			)
		};

		let (vertex_shader_artifact, fragment_shader_artifact) = compile_shaders();

		let vertex_shader = device
			.create_shader(
				None,
				Sources::SPIRV(vertex_shader_artifact.borrow().into()),
				ShaderTypes::Vertex,
				[],
			)
			.expect("Failed to create vertex shader");
		let fragment_shader = device
			.create_shader(
				None,
				Sources::SPIRV(fragment_shader_artifact.borrow().into()),
				ShaderTypes::Fragment,
				[],
			)
			.expect("Failed to create fragment shader");

		// Use and odd width to make sure there is a middle/center pixel
		let extent = Extent::rectangle(1920, 1080);

		let render_target = device.build_image(
			crate::image::Builder::new(Formats::RGBA8UNORM, Uses::RenderTarget)
				.extent(extent)
				.device_accesses(DeviceAccesses::DeviceToHost)
				.use_case(UseCases::DYNAMIC),
		);

		let attachments = [AttachmentDescriptor::new(Formats::RGBA8UNORM)];

		let pipeline = device.create_raster_pipeline(pipelines::raster::Builder::new(
			&[],
			&[PushConstantRange::new(0, 16 * 4)],
			&vertex_layout,
			&[
				ShaderParameter::new(&vertex_shader, ShaderTypes::Vertex),
				ShaderParameter::new(&fragment_shader, ShaderTypes::Fragment),
			],
			&attachments,
		));

		let command_buffer_handle = device.create_command_buffer(None, queue_handle);

		let render_finished_synchronizer = device.create_synchronizer(None, true);

		for i in 0..FRAMES_IN_FLIGHT * 10 {
			device.start_frame_capture();

			let mut frame = device.start_frame(i as u32, render_finished_synchronizer);

			let mut command_buffer_recording = frame.create_command_buffer_recording(command_buffer_handle);

			let attachments = [AttachmentInformation::new(
				render_target,
				Formats::RGBA8UNORM,
				Layouts::RenderTarget,
				ClearValue::Color(RGBA::black()),
				false,
				true,
			)];

			let render_pass_command = command_buffer_recording.start_render_pass(extent, &attachments);

			let raster_pipeline_command = render_pass_command.bind_raster_pipeline(pipeline);

			raster_pipeline_command.draw_mesh(&mesh);

			raster_pipeline_command.end_render_pass();

			let texture_copy_handles = command_buffer_recording.transfer_textures(&[render_target]);

			let terminated_command_buffer = command_buffer_recording.end(&[]);
			frame.execute(terminated_command_buffer, render_finished_synchronizer);

			device.end_frame_capture();

			device.wait();

			assert!(!device.has_errors());

			let pixels = unsafe {
				std::slice::from_raw_parts(
					device.get_image_data(texture_copy_handles[0]).as_ptr() as *const RGBAu8,
					(extent.width() * extent.height()) as usize,
				)
			};

			check_triangle(pixels, extent);
		}
	}

	pub(crate) fn change_frames(device: &mut impl Device, queue_handle: QueueHandle) {
		//! Tests that the render system can perform rendering while changing the amount of frames in flight.
		//! Having multiple frames in flight means allocating and managing multiple resources under a single handle, one for each frame.

		const FRAMES_IN_FLIGHT: usize = 3;

		let floats: [f32; 21] = [
			0.0, 1.0, 0.0, 1.0, 0.0, 0.0, 1.0, 1.0, -1.0, 0.0, 0.0, 1.0, 0.0, 1.0, -1.0, -1.0, 0.0, 0.0, 0.0, 1.0, 1.0,
		];

		let vertex_layout = [
			VertexElement::new("POSITION", DataTypes::Float3, 0),
			VertexElement::new("COLOR", DataTypes::Float4, 0),
		];

		let mesh = unsafe {
			device.add_mesh_from_vertices_and_indices(
				3,
				3,
				std::slice::from_raw_parts(floats.as_ptr() as *const u8, (3 * 4 + 4 * 4) * 3),
				std::slice::from_raw_parts([0u16, 1u16, 2u16].as_ptr() as *const u8, 3 * 2),
				&vertex_layout,
			)
		};

		let (vertex_shader_artifact, fragment_shader_artifact) = compile_shaders();

		let vertex_shader = device
			.create_shader(
				None,
				Sources::SPIRV(vertex_shader_artifact.borrow().into()),
				ShaderTypes::Vertex,
				[],
			)
			.expect("Failed to create vertex shader");
		let fragment_shader = device
			.create_shader(
				None,
				Sources::SPIRV(fragment_shader_artifact.borrow().into()),
				ShaderTypes::Fragment,
				[],
			)
			.expect("Failed to create fragment shader");

		let extent = Extent::rectangle(1920, 1080);

		let render_target = device.build_image(
			crate::image::Builder::new(Formats::RGBA8UNORM, Uses::RenderTarget)
				.extent(extent)
				.device_accesses(DeviceAccesses::DeviceToHost)
				.use_case(UseCases::DYNAMIC),
		);

		let attachments = [AttachmentDescriptor::new(Formats::RGBA8UNORM)];

		let pipeline = device.create_raster_pipeline(pipelines::raster::Builder::new(
			&[],
			&[],
			&vertex_layout,
			&[
				ShaderParameter::new(&vertex_shader, ShaderTypes::Vertex),
				ShaderParameter::new(&fragment_shader, ShaderTypes::Fragment),
			],
			&attachments,
		));

		let command_buffer_handle = device.create_command_buffer(None, queue_handle);

		let render_finished_synchronizer = device.create_synchronizer(None, true);

		for i in 0..FRAMES_IN_FLIGHT * 10 {
			if i == 2 {
				device.set_frames_in_flight(3); // Change from default 2 to 3
			}

			device.start_frame_capture();

			let mut frame = device.start_frame(i as u32, render_finished_synchronizer);

			let mut command_buffer_recording = frame.create_command_buffer_recording(command_buffer_handle);

			let attachments = [AttachmentInformation::new(
				render_target,
				Formats::RGBA8UNORM,
				Layouts::RenderTarget,
				ClearValue::Color(RGBA::black()),
				false,
				true,
			)];

			let render_pass_command = command_buffer_recording.start_render_pass(extent, &attachments);

			let raster_pipeline_command = render_pass_command.bind_raster_pipeline(pipeline);

			raster_pipeline_command.draw_mesh(&mesh);

			raster_pipeline_command.end_render_pass();

			let texture_copy_handles = command_buffer_recording.transfer_textures(&[render_target]);

			let terminated_command_buffer = command_buffer_recording.end(&[]);
			frame.execute(terminated_command_buffer, render_finished_synchronizer);

			device.end_frame_capture();

			device.wait();

			assert!(!device.has_errors());

			let pixels = unsafe {
				std::slice::from_raw_parts(
					device.get_image_data(texture_copy_handles[0]).as_ptr() as *const RGBAu8,
					(extent.width() * extent.height()) as usize,
				)
			};

			check_triangle(pixels, extent);
		}
	}

	pub(crate) fn resize(device: &mut impl Device, queue_handle: QueueHandle) {
		//! Tests that the render system can perform rendering while resize the render targets.

		const FRAMES_IN_FLIGHT: usize = 3;

		let floats: [f32; 21] = [
			0.0, 1.0, 0.0, 1.0, 0.0, 0.0, 1.0, 1.0, -1.0, 0.0, 0.0, 1.0, 0.0, 1.0, -1.0, -1.0, 0.0, 0.0, 0.0, 1.0, 1.0,
		];

		let vertex_layout = [
			VertexElement::new("POSITION", DataTypes::Float3, 0),
			VertexElement::new("COLOR", DataTypes::Float4, 0),
		];

		let mesh = unsafe {
			device.add_mesh_from_vertices_and_indices(
				3,
				3,
				std::slice::from_raw_parts(floats.as_ptr() as *const u8, (3 * 4 + 4 * 4) * 3),
				std::slice::from_raw_parts([0u16, 1u16, 2u16].as_ptr() as *const u8, 3 * 2),
				&vertex_layout,
			)
		};

		let (vertex_shader_artifact, fragment_shader_artifact) = compile_shaders();

		let vertex_shader = device
			.create_shader(
				None,
				Sources::SPIRV(vertex_shader_artifact.borrow().into()),
				ShaderTypes::Vertex,
				[],
			)
			.expect("Failed to create vertex shader");
		let fragment_shader = device
			.create_shader(
				None,
				Sources::SPIRV(fragment_shader_artifact.borrow().into()),
				ShaderTypes::Fragment,
				[],
			)
			.expect("Failed to create fragment shader");

		let mut extent = Extent::rectangle(1280, 720);

		let render_target = device.build_image(
			crate::image::Builder::new(Formats::RGBA8UNORM, Uses::RenderTarget)
				.extent(extent)
				.device_accesses(DeviceAccesses::DeviceToHost)
				.use_case(UseCases::DYNAMIC),
		);

		let attachments = [AttachmentDescriptor::new(Formats::RGBA8UNORM)];

		let pipeline = device.create_raster_pipeline(pipelines::raster::Builder::new(
			&[],
			&[],
			&vertex_layout,
			&[
				ShaderParameter::new(&vertex_shader, ShaderTypes::Vertex),
				ShaderParameter::new(&fragment_shader, ShaderTypes::Fragment),
			],
			&attachments,
		));

		let command_buffer_handle = device.create_command_buffer(None, queue_handle);

		let render_finished_synchronizer = device.create_synchronizer(None, true);

		for i in 0..FRAMES_IN_FLIGHT * 10 {
			device.start_frame_capture();

			let mut frame = device.start_frame(i as u32, render_finished_synchronizer);

			if i == 2 {
				extent = Extent::rectangle(1920, 1080);
				frame.resize_image(render_target, extent);
			}

			let mut command_buffer_recording = frame.create_command_buffer_recording(command_buffer_handle);

			let attachments = [AttachmentInformation::new(
				render_target,
				Formats::RGBA8UNORM,
				Layouts::RenderTarget,
				ClearValue::Color(RGBA::black()),
				false,
				true,
			)];

			let render_pass_command = command_buffer_recording.start_render_pass(extent, &attachments);

			let raster_pipeline_command = render_pass_command.bind_raster_pipeline(pipeline);

			raster_pipeline_command.draw_mesh(&mesh);

			raster_pipeline_command.end_render_pass();

			let texture_copy_handles = command_buffer_recording.transfer_textures(&[render_target]);

			let terminated_command_buffer = command_buffer_recording.end(&[]);
			frame.execute(terminated_command_buffer, render_finished_synchronizer);

			device.end_frame_capture();

			device.wait();

			assert!(!device.has_errors());

			let pixels = unsafe {
				std::slice::from_raw_parts(
					device.get_image_data(texture_copy_handles[0]).as_ptr() as *const RGBAu8,
					(extent.width() * extent.height()) as usize,
				)
			};

			assert_eq!(pixels.len(), (extent.width() * extent.height()) as usize);

			check_triangle(pixels, extent);
		}
	}

	pub(crate) fn dynamic_data(device: &mut impl Device, queue_handle: QueueHandle) {
		//! Tests that the render system can perform rendering with multiple frames in flight.
		//! Having multiple frames in flight means allocating and managing multiple resources under a single handle, one for each frame.

		const FRAMES_IN_FLIGHT: usize = 2;

		let floats: [f32; 21] = [
			0.0, 1.0, 0.0, 1.0, 0.0, 0.0, 1.0, 1.0, -1.0, 0.0, 0.0, 1.0, 0.0, 1.0, -1.0, -1.0, 0.0, 0.0, 0.0, 1.0, 1.0,
		];

		let vertex_layout = [
			VertexElement::new("POSITION", DataTypes::Float3, 0),
			VertexElement::new("COLOR", DataTypes::Float4, 0),
		];

		let mesh = unsafe {
			device.add_mesh_from_vertices_and_indices(
				3,
				3,
				std::slice::from_raw_parts(floats.as_ptr() as *const u8, (3 * 4 + 4 * 4) * 3),
				std::slice::from_raw_parts([0u16, 1u16, 2u16].as_ptr() as *const u8, 3 * 2),
				&vertex_layout,
			)
		};

		let (vertex_shader_artifact, fragment_shader_artifact) = compile_shaders_with_model_matrix();

		let vertex_shader = device
			.create_shader(
				None,
				Sources::SPIRV(vertex_shader_artifact.borrow().into()),
				ShaderTypes::Vertex,
				[],
			)
			.expect("Failed to create vertex shader");
		let fragment_shader = device
			.create_shader(
				None,
				Sources::SPIRV(fragment_shader_artifact.borrow().into()),
				ShaderTypes::Fragment,
				[],
			)
			.expect("Failed to create fragment shader");

		// Use and odd width to make sure there is a middle/center pixel
		let extent = Extent::rectangle(1920, 1080);

		let render_target = device.build_image(
			crate::image::Builder::new(Formats::RGBA8UNORM, Uses::RenderTarget)
				.extent(extent)
				.device_accesses(DeviceAccesses::DeviceToHost)
				.use_case(UseCases::DYNAMIC),
		);

		let attachments = [AttachmentDescriptor::new(Formats::RGBA8UNORM)];

		let pipeline = device.create_raster_pipeline(pipelines::raster::Builder::new(
			&[],
			&[],
			&vertex_layout,
			&[
				ShaderParameter::new(&vertex_shader, ShaderTypes::Vertex),
				ShaderParameter::new(&fragment_shader, ShaderTypes::Fragment),
			],
			&attachments,
		));

		let _buffer =
			device.build_buffer::<u8>(crate::buffer::Builder::new(Uses::Storage).device_accesses(DeviceAccesses::HostToDevice));

		let command_buffer_handle = device.create_command_buffer(None, queue_handle);

		let render_finished_synchronizer = device.create_synchronizer(None, true);

		for i in 0..FRAMES_IN_FLIGHT * 10 {
			device.start_frame_capture();

			let mut frame = device.start_frame(i as u32, render_finished_synchronizer);

			let mut cb = frame.create_command_buffer_recording(command_buffer_handle);

			let attachments = [AttachmentInformation::new(
				render_target,
				Formats::RGBA8UNORM,
				Layouts::RenderTarget,
				ClearValue::Color(RGBA::black()),
				false,
				true,
			)];

			let c = cb.start_render_pass(extent, &attachments);

			let angle = (i as f32) * (std::f32::consts::PI / 2.0f32);

			let matrix: [f32; 16] = [
				angle.cos(),
				-angle.sin(),
				0f32,
				0f32,
				angle.sin(),
				angle.cos(),
				0f32,
				0f32,
				0f32,
				0f32,
				1f32,
				0f32,
				0f32,
				0f32,
				0f32,
				1f32,
			];

			let c = c.bind_raster_pipeline(pipeline);

			c.write_push_constant(0, matrix);
			c.draw_mesh(&mesh);

			c.end_render_pass();

			let copy_texture_handles = cb.transfer_textures(&[render_target]);

			let terminated_command_buffer = cb.end(&[]);
			frame.execute(terminated_command_buffer, render_finished_synchronizer);

			device.end_frame_capture();

			device.wait();

			assert!(!device.has_errors());

			let pixels = unsafe {
				std::slice::from_raw_parts(
					device.get_image_data(copy_texture_handles[0]).as_ptr() as *const RGBAu8,
					(extent.width() * extent.height()) as usize,
				)
			};

			assert_eq!(pixels.len(), (extent.width() * extent.height()) as usize);

			// Track green corner as it should move through screen

			if i % 4 == 0 {
				let pixel = pixels[(extent.width() * extent.height() - 1) as usize]; // bottom right
				assert_eq!(
					pixel,
					RGBAu8 {
						r: 0,
						g: 255,
						b: 0,
						a: 255
					},
					"Pixel at bottom right corner did not match expected green color in frame: {i}"
				);
			} else if i % 4 == 1 {
				let pixel = pixels[(extent.width() * (extent.height() - 1)) as usize]; // bottom left
				assert_eq!(
					pixel,
					RGBAu8 {
						r: 0,
						g: 255,
						b: 0,
						a: 255
					},
					"Pixel at bottom left corner did not match expected green color in frame: {i}"
				);
			} else if i % 4 == 2 {
				let pixel = pixels[0]; // top left
				assert_eq!(
					pixel,
					RGBAu8 {
						r: 0,
						g: 255,
						b: 0,
						a: 255
					},
					"Pixel at top left corner did not match expected green color in frame: {i}"
				);
			} else if i % 4 == 3 {
				let pixel = pixels[(extent.width() - 1) as usize]; // top right
				assert_eq!(
					pixel,
					RGBAu8 {
						r: 0,
						g: 255,
						b: 0,
						a: 255
					},
					"Pixel at top right corner did not match expected green color in frame: {i}"
				);
			}
		}

		assert!(!device.has_errors())
	}

	pub(crate) fn dynamic_textures(device: &mut impl Device, queue_handle: QueueHandle) {
		//! Tests that dynamic textures write to the current frame image instead of always writing to the root image.

		let extent = Extent::square(2);
		let pixel_count = (extent.width() * extent.height()) as usize;

		let upload_image = device.build_dynamic_image(
			crate::image::Builder::new(Formats::RGBA8UNORM, Uses::Image | Uses::TransferSource)
				.extent(extent)
				.device_accesses(DeviceAccesses::HostToDevice),
		);

		let readback_image = device.build_dynamic_image(
			crate::image::Builder::new(Formats::RGBA8UNORM, Uses::Image | Uses::TransferDestination)
				.extent(extent)
				.device_accesses(DeviceAccesses::DeviceToHost),
		);

		let command_buffer_handle = device.create_command_buffer(None, queue_handle);
		let render_finished_synchronizer = device.create_synchronizer(None, true);

		let expected_colors = [
			RGBAu8 {
				r: 255,
				g: 0,
				b: 0,
				a: 255,
			},
			RGBAu8 {
				r: 0,
				g: 255,
				b: 0,
				a: 255,
			},
		];

		for (frame_index, expected_color) in expected_colors.into_iter().enumerate() {
			let mut frame = device.start_frame(frame_index as u32, render_finished_synchronizer);

			let texture_slice = frame.get_mut_dynamic_texture_slice(upload_image);
			let pixels = unsafe { std::slice::from_raw_parts_mut(texture_slice.as_mut_ptr() as *mut RGBAu8, pixel_count) };
			pixels.fill(expected_color);
			frame.sync_texture(upload_image);

			let mut command_buffer_recording = frame.create_command_buffer_recording(command_buffer_handle);
			command_buffer_recording.blit_image(upload_image, Layouts::Transfer, readback_image, Layouts::Transfer);
			let texture_copy_handles = command_buffer_recording.transfer_textures(&[readback_image]);
			let terminated_command_buffer = command_buffer_recording.end(&[]);
			frame.execute(terminated_command_buffer, render_finished_synchronizer);

			device.wait();

			let pixels = unsafe {
				std::slice::from_raw_parts(
					device.get_image_data(texture_copy_handles[0]).as_ptr() as *const RGBAu8,
					pixel_count,
				)
			};

			assert!(pixels.iter().all(|pixel| *pixel == expected_color));
			assert!(!device.has_errors());
		}
	}

	pub(crate) fn multiframe_resources(device: &mut impl Device, queue_handle: QueueHandle) {
		// TODO: test multiframe resources for combined image samplers
		let compute_shader_string = "
			#version 450
			#pragma shader_stage(compute)

			layout(set=0,binding=0, rgba8) uniform image2D img;
			layout(set=0,binding=1, rgba8) uniform readonly image2D last_frame_img;

			layout(push_constant) uniform PushConstants {
				float value;
			} push_constants;

			layout(local_size_x = 1, local_size_y = 1, local_size_z = 1) in;
			void main() {
				imageStore(img, ivec2(0, 0), vec4(vec3(push_constants.value), 1));
				imageStore(img, ivec2(1, 0), imageLoad(last_frame_img, ivec2(0, 0)));
			}
		";

		let compute_shader_artifact = glsl::compile(compute_shader_string, "compute").unwrap();

		let compute_shader = device
			.create_shader(
				None,
				Sources::SPIRV(compute_shader_artifact.borrow().into()),
				ShaderTypes::Compute,
				[
					BindingDescriptor::new(0, 0, AccessPolicies::WRITE),
					BindingDescriptor::new(0, 1, AccessPolicies::READ),
				],
			)
			.expect("Failed to create compute shader");

		let image_binding_template = DescriptorSetBindingTemplate::new(0, DescriptorType::StorageImage, Stages::COMPUTE);
		let last_frame_image_binding_template =
			DescriptorSetBindingTemplate::new(1, DescriptorType::StorageImage, Stages::COMPUTE);

		let descriptor_set_template = device.create_descriptor_set_template(
			None,
			&[image_binding_template.clone(), last_frame_image_binding_template.clone()],
		);

		let pipeline = device.create_compute_pipeline(pipelines::compute::Builder::new(
			&[descriptor_set_template],
			&[PushConstantRange { offset: 0, size: 4 }],
			ShaderParameter::new(&compute_shader, ShaderTypes::Compute),
		));

		let image = device.build_image(
			crate::image::Builder::new(Formats::RGBA8UNORM, Uses::Storage)
				.name("Image")
				.extent(Extent::square(2))
				.device_accesses(DeviceAccesses::DeviceToHost)
				.use_case(UseCases::DYNAMIC),
		);

		let descriptor_set = device.create_descriptor_set(None, &descriptor_set_template);

		let _ = device.create_descriptor_binding(descriptor_set, BindingConstructor::image(&image_binding_template, image));
		let _ = device.create_descriptor_binding(
			descriptor_set,
			BindingConstructor::image(&last_frame_image_binding_template, image).frame(-1),
		);

		let command_buffer = device.create_command_buffer(None, queue_handle);

		let signal = device.create_synchronizer(None, true);

		let mut frame = device.start_frame(0, signal);

		let mut command_buffer_recording = frame.create_command_buffer_recording(command_buffer);

		let data = [0.5f32];

		let pipeline_command = command_buffer_recording.bind_compute_pipeline(pipeline);

		pipeline_command.write_push_constant(0, data);
		pipeline_command
			.bind_descriptor_sets(&[descriptor_set])
			.dispatch(DispatchExtent::new(Extent::square(1), Extent::square(1)));

		let copy_handles = command_buffer_recording.transfer_textures(&[image]);

		let terminated_command_buffer = command_buffer_recording.end(&[]);
		frame.execute(terminated_command_buffer, signal);

		device.wait();

		let pixels = unsafe { std::slice::from_raw_parts(device.get_image_data(copy_handles[0]).as_ptr() as *const RGBAu8, 4) };

		assert!(
			pixels[0]
				== RGBAu8 {
					r: 127,
					g: 127,
					b: 127,
					a: 255
				} || pixels[0]
				== RGBAu8 {
					r: 128,
					g: 128,
					b: 128,
					a: 255
				}
		); // Current frame image
		assert_eq!(pixels[1], RGBAu8 { r: 0, g: 0, b: 0, a: 0 }); // Current frame sample from last frame image

		assert!(!device.has_errors());

		let mut frame = device.start_frame(1, signal);

		let mut command_buffer_recording = frame.create_command_buffer_recording(command_buffer);

		let data = [1.0f32];

		let pipeline_command = command_buffer_recording.bind_compute_pipeline(pipeline);

		pipeline_command.write_push_constant(0, data);
		pipeline_command
			.bind_descriptor_sets(&[descriptor_set])
			.dispatch(DispatchExtent::new(Extent::square(1), Extent::square(1)));

		let copy_handles = command_buffer_recording.transfer_textures(&[image]);

		let terminated_command_buffer = command_buffer_recording.end(&[]);
		frame.execute(terminated_command_buffer, signal);

		device.wait();

		let pixels = unsafe { std::slice::from_raw_parts(device.get_image_data(copy_handles[0]).as_ptr() as *const RGBAu8, 4) };

		assert_eq!(
			pixels[0],
			RGBAu8 {
				r: 255,
				g: 255,
				b: 255,
				a: 255
			}
		);
		assert!(
			pixels[1]
				== RGBAu8 {
					r: 127,
					g: 127,
					b: 127,
					a: 255
				} || pixels[1]
				== RGBAu8 {
					r: 128,
					g: 128,
					b: 128,
					a: 255
				}
		); // Current frame sample from last frame image

		assert!(!device.has_errors());

		let mut frame = device.start_frame(2, signal);

		let mut command_buffer_recording = frame.create_command_buffer_recording(command_buffer);

		let copy_handles = command_buffer_recording.transfer_textures(&[image]);

		let terminated_command_buffer = command_buffer_recording.end(&[]);
		frame.execute(terminated_command_buffer, signal);

		device.wait();

		let pixels = unsafe { std::slice::from_raw_parts(device.get_image_data(copy_handles[0]).as_ptr() as *const RGBAu8, 4) };

		assert!(
			pixels[0]
				== RGBAu8 {
					r: 127,
					g: 127,
					b: 127,
					a: 255
				} || pixels[0]
				== RGBAu8 {
					r: 128,
					g: 128,
					b: 128,
					a: 255
				}
		);
		assert_eq!(pixels[1], RGBAu8 { r: 0, g: 0, b: 0, a: 0 });

		assert!(!device.has_errors());

		let mut frame = device.start_frame(3, signal);

		let mut command_buffer_recording = frame.create_command_buffer_recording(command_buffer);

		let copy_handles = command_buffer_recording.transfer_textures(&[image]);

		let terminated_command_buffer = command_buffer_recording.end(&[]);
		frame.execute(terminated_command_buffer, signal);

		device.wait();

		let pixels = unsafe { std::slice::from_raw_parts(device.get_image_data(copy_handles[0]).as_ptr() as *const RGBAu8, 4) };

		assert_eq!(
			pixels[0],
			RGBAu8 {
				r: 255,
				g: 255,
				b: 255,
				a: 255
			}
		);
		assert!(
			pixels[1]
				== RGBAu8 {
					r: 127,
					g: 127,
					b: 127,
					a: 255
				} || pixels[1]
				== RGBAu8 {
					r: 128,
					g: 128,
					b: 128,
					a: 255
				}
		);

		assert!(!device.has_errors());
	}

	pub(crate) fn descriptor_sets(device: &mut impl Device, queue_handle: QueueHandle) {
		let signal = device.create_synchronizer(None, true);

		let floats: [f32; 21] = [
			0.0, 1.0, 0.0, 1.0, 0.0, 0.0, 1.0, 1.0, -1.0, 0.0, 0.0, 1.0, 0.0, 1.0, -1.0, -1.0, 0.0, 0.0, 0.0, 1.0, 1.0,
		];

		let vertex_layout = [
			VertexElement::new("POSITION", DataTypes::Float3, 0),
			VertexElement::new("COLOR", DataTypes::Float4, 0),
		];

		let mesh = unsafe {
			device.add_mesh_from_vertices_and_indices(
				3,
				3,
				std::slice::from_raw_parts(floats.as_ptr() as *const u8, (3 * 4 + 4 * 4) * 3),
				std::slice::from_raw_parts([0u16, 1u16, 2u16].as_ptr() as *const u8, 3 * 2),
				&vertex_layout,
			)
		};

		let vertex_shader_code = "
			#version 450 core
			#pragma shader_stage(vertex)

			layout(location = 0) in vec3 in_position;
			layout(location = 1) in vec4 in_color;

			layout(location = 0) out vec4 out_color;

			layout(set=0, binding=1) uniform UniformBufferObject {
				mat4 matrix;
			} ubo;

			void main() {
				out_color = in_color;
				gl_Position = vec4(in_position, 1.0);
			}
		";

		let fragment_shader_code = "
			#version 450 core
			#pragma shader_stage(fragment)

			layout(location = 0) in vec4 in_color;

			layout(location = 0) out vec4 out_color;

			layout(set=0,binding=0) uniform sampler smpl;
			layout(set=0,binding=2) uniform texture2D tex;

			void main() {
				out_color = texture(sampler2D(tex, smpl), vec2(0, 0));
			}
		";

		let vertex_shader_artifact = glsl::compile(vertex_shader_code, "vertex").unwrap();
		let fragment_shader_artifact = glsl::compile(fragment_shader_code, "fragment").unwrap();

		let vertex_shader = device
			.create_shader(
				None,
				Sources::SPIRV(vertex_shader_artifact.borrow().into()),
				ShaderTypes::Vertex,
				[BindingDescriptor::new(0, 1, AccessPolicies::READ)],
			)
			.expect("Failed to create vertex shader");
		let fragment_shader = device
			.create_shader(
				None,
				Sources::SPIRV(fragment_shader_artifact.borrow().into()),
				ShaderTypes::Fragment,
				[
					BindingDescriptor::new(0, 0, AccessPolicies::READ),
					BindingDescriptor::new(0, 2, AccessPolicies::READ),
				],
			)
			.expect("Failed to create fragment shader");

		let buffer = device.build_dynamic_buffer::<[u8; 64]>(
			crate::buffer::Builder::new(Uses::Uniform | Uses::Storage).device_accesses(DeviceAccesses::HostToDevice),
		);

		let sampled_texture = device.build_image(
			crate::image::Builder::new(Formats::RGBA8UNORM, Uses::Image)
				.name("sampled texture")
				.extent(Extent::square(2))
				.device_accesses(DeviceAccesses::HostToDevice)
				.use_case(UseCases::STATIC),
		);

		let pixels = vec![
			RGBAu8 {
				r: 255,
				g: 0,
				b: 0,
				a: 255,
			},
			RGBAu8 {
				r: 0,
				g: 255,
				b: 0,
				a: 255,
			},
			RGBAu8 {
				r: 0,
				g: 0,
				b: 255,
				a: 255,
			},
			RGBAu8 {
				r: 255,
				g: 255,
				b: 0,
				a: 255,
			},
		];

		let sampler = device.build_sampler(
			crate::sampler::Builder::new()
				.filtering_mode(FilteringModes::Closest)
				.reduction_mode(SamplingReductionModes::WeightedAverage)
				.mip_map_mode(FilteringModes::Closest)
				.addressing_mode(SamplerAddressingModes::Repeat)
				.min_lod(0.0f32)
				.max_lod(0.0f32),
		);

		let descriptor_set_layout_handle = device.create_descriptor_set_template(
			None,
			&[
				DescriptorSetBindingTemplate::new_with_immutable_samplers(0, Stages::FRAGMENT, Some(vec![sampler])),
				DescriptorSetBindingTemplate::new(1, DescriptorType::StorageBuffer, Stages::VERTEX),
				DescriptorSetBindingTemplate::new(2, DescriptorType::SampledImage, Stages::FRAGMENT),
			],
		);

		let descriptor_set = device.create_descriptor_set(None, &descriptor_set_layout_handle);

		let _ = device.create_descriptor_binding(
			descriptor_set,
			BindingConstructor::sampler(
				&DescriptorSetBindingTemplate::new(0, DescriptorType::Sampler, Stages::FRAGMENT),
				sampler,
			),
		);
		let _ = device.create_descriptor_binding(
			descriptor_set,
			BindingConstructor::buffer(
				&DescriptorSetBindingTemplate::new(1, DescriptorType::StorageBuffer, Stages::VERTEX),
				buffer.into(),
			),
		);
		let _ = device.create_descriptor_binding(
			descriptor_set,
			BindingConstructor::image(
				&DescriptorSetBindingTemplate::new(2, DescriptorType::SampledImage, Stages::FRAGMENT),
				sampled_texture,
			)
			.layout(Layouts::Read),
		);

		assert!(!device.has_errors());

		// Use and odd width to make sure there is a middle/center pixel
		let extent = Extent::rectangle(1920, 1080);

		let render_target = device.build_image(
			crate::image::Builder::new(Formats::RGBA8UNORM, Uses::RenderTarget)
				.extent(extent)
				.device_accesses(DeviceAccesses::DeviceToHost)
				.use_case(UseCases::STATIC),
		);

		let attachments = [AttachmentDescriptor::new(Formats::RGBA8UNORM)];

		let pipeline = device.create_raster_pipeline(pipelines::raster::Builder::new(
			&[descriptor_set_layout_handle],
			&[],
			&vertex_layout,
			&[
				ShaderParameter::new(&vertex_shader, ShaderTypes::Vertex),
				ShaderParameter::new(&fragment_shader, ShaderTypes::Fragment),
			],
			&attachments,
		));

		let command_buffer_handle = device.create_command_buffer(None, queue_handle);

		device.start_frame_capture();

		let mut frame = device.start_frame(0, signal);

		let mut command_buffer_recording = frame.create_command_buffer_recording(command_buffer_handle);

		command_buffer_recording.write_image_data(sampled_texture, &pixels);

		let attachments = [AttachmentInformation::new(
			render_target,
			Formats::RGBA8UNORM,
			Layouts::RenderTarget,
			ClearValue::Color(RGBA {
				r: 0.0,
				g: 0.0,
				b: 0.0,
				a: 1.0,
			}),
			false,
			true,
		)];

		let raster_render_pass_command = command_buffer_recording.start_render_pass(extent, &attachments);

		let raster_pipeline_command = raster_render_pass_command.bind_raster_pipeline(pipeline);

		raster_pipeline_command.bind_descriptor_sets(&[descriptor_set]);

		raster_pipeline_command.draw_mesh(&mesh);

		raster_render_pass_command.end_render_pass();

		let texure_copy_handles = command_buffer_recording.transfer_textures(&[render_target]);

		let terminated_command_buffer = command_buffer_recording.end(&[]);
		frame.execute(terminated_command_buffer, signal);

		device.end_frame_capture();

		device.wait();

		// assert colored triangle was drawn to texture
		let _pixels = device.get_image_data(texure_copy_handles[0]);

		// TODO: assert rendering results

		assert!(!device.has_errors());
	}

	pub(crate) fn ray_tracing(renderer: &mut impl Device, queue_handle: QueueHandle) {
		//! Tests that the render system can perform rendering with multiple frames in flight.
		//! Having multiple frames in flight means allocating and managing multiple resources under a single handle, one for each frame.

		const FRAMES_IN_FLIGHT: usize = 2;

		// let mut window_system = window_system::WindowSystem::new();

		// Use and odd width to make sure there is a middle/center pixel
		let extent = Extent::rectangle(1920, 1080);

		// let window_handle = window_system.create_window("Renderer Test", extent, "test");
		// let swapchain = renderer.bind_to_window(&window_system.get_os_handles_2(&window_handle));

		let positions: [f32; 3 * 3] = [0.0, 1.0, 0.0, 1.0, -1.0, 0.0, -1.0, -1.0, 0.0];

		let colors: [f32; 4 * 3] = [1.0, 0.0, 0.0, 1.0, 0.0, 1.0, 0.0, 1.0, 0.0, 0.0, 1.0, 1.0];

		let vertex_positions_buffer = renderer.build_buffer::<[f32; 3 * 3]>(
			crate::buffer::Builder::new(Uses::Storage | Uses::AccelerationStructureBuild)
				.device_accesses(DeviceAccesses::HostToDevice),
		);
		let vertex_colors_buffer = renderer.build_buffer::<[f32; 4 * 3]>(
			crate::buffer::Builder::new(Uses::Storage | Uses::AccelerationStructureBuild)
				.device_accesses(DeviceAccesses::HostToDevice),
		);
		let index_buffer = renderer.build_buffer::<[u16; 3]>(
			crate::buffer::Builder::new(Uses::Storage | Uses::AccelerationStructureBuild)
				.device_accesses(DeviceAccesses::HostToDevice),
		);

		renderer
			.get_mut_buffer_slice(vertex_positions_buffer)
			.copy_from_slice(&positions);
		renderer.get_mut_buffer_slice(vertex_colors_buffer).copy_from_slice(&colors);
		renderer
			.get_mut_buffer_slice(index_buffer)
			.copy_from_slice(&[0u16, 1u16, 2u16]);

		renderer.sync_buffer(vertex_positions_buffer);
		renderer.sync_buffer(index_buffer);

		let raygen_shader_code = "
#version 460 core
#pragma shader_stage(raygen)

#extension GL_EXT_scalar_block_layout: enable
#extension GL_EXT_buffer_reference: enable
#extension GL_EXT_buffer_reference2: enable
#extension GL_EXT_shader_16bit_storage: require
#extension GL_EXT_ray_tracing: require

layout(binding = 0, set = 0) uniform accelerationStructureEXT topLevelAS;
layout(binding = 1, set = 0, rgba8) uniform image2D image;

layout(location = 0) rayPayloadEXT vec3 hitValue;

void main() {
	const vec2 pixelCenter = vec2(gl_LaunchIDEXT.xy) + vec2(0.5);
	const vec2 inUV = pixelCenter/vec2(gl_LaunchSizeEXT.xy);
	vec2 d = inUV * 2.0 - 1.0;
	d.y *= -1.0;

	uint rayFlags = gl_RayFlagsOpaqueEXT;
	uint cullMask = 0xff;
	float tmin = 0.001;
	float tmax = 10.0;

	vec3 origin = vec3(d, -1.0);
	vec3 direction = vec3(0.0, 0.0, 1.0);

	traceRayEXT(topLevelAS, rayFlags, cullMask, 0, 0, 0, origin, tmin, direction, tmax, 0);

	imageStore(image, ivec2(gl_LaunchIDEXT.xy), vec4(hitValue, 1.0));
}
		";

		let closest_hit_shader_code = "
#version 460 core
#pragma shader_stage(closest)

#extension GL_EXT_scalar_block_layout: enable
#extension GL_EXT_buffer_reference: enable
#extension GL_EXT_buffer_reference2: enable
#extension GL_EXT_shader_16bit_storage: require
#extension GL_EXT_ray_tracing: require

layout(location = 0) rayPayloadInEXT vec3 hitValue;
hitAttributeEXT vec2 attribs;

layout(binding = 2, set = 0) buffer VertexPositions { vec3 positions[3]; };
layout(binding = 3, set = 0) buffer VertexColors { vec4 colors[3]; };
layout(binding = 4, set = 0) buffer Indices { uint16_t indices[3]; };

void main() {
	const vec3 barycentricCoords = vec3(1.0f - attribs.x - attribs.y, attribs.x, attribs.y);
	ivec3 index = ivec3(indices[3 * gl_PrimitiveID], indices[3 * gl_PrimitiveID + 1], indices[3 * gl_PrimitiveID + 2]);

	vec3[3] vertex_positions = vec3[3](positions[index.x], positions[index.y], positions[index.z]);
	vec4[3] vertex_colors = vec4[3](colors[index.x], colors[index.y], colors[index.z]);

	vec3 position = vertex_positions[0] * barycentricCoords.x + vertex_positions[1] * barycentricCoords.y + vertex_positions[2] * barycentricCoords.z;
	vec4 color = vertex_colors[0] * barycentricCoords.x + vertex_colors[1] * barycentricCoords.y + vertex_colors[2] * barycentricCoords.z;

	hitValue = color.xyz;
}
		";

		let miss_shader_code = "
#version 460 core
#pragma shader_stage(miss)

#extension GL_EXT_scalar_block_layout: enable
#extension GL_EXT_buffer_reference: enable
#extension GL_EXT_buffer_reference2: enable
#extension GL_EXT_shader_16bit_storage: require
#extension GL_EXT_ray_tracing: require

layout(location = 0) rayPayloadInEXT vec3 hitValue;

void main() {
    hitValue = vec3(0.0, 0.0, 0.0);
}
		";

		let raygen_shader_artifact = glsl::compile(raygen_shader_code, "raygen").unwrap();
		let closest_hit_shader_artifact = glsl::compile(closest_hit_shader_code, "closest_hit").unwrap();
		let miss_shader_artifact = glsl::compile(miss_shader_code, "miss").unwrap();

		let raygen_shader = renderer
			.create_shader(
				None,
				Sources::SPIRV(raygen_shader_artifact.borrow().into()),
				ShaderTypes::RayGen,
				[
					BindingDescriptor::new(0, 0, AccessPolicies::READ),
					BindingDescriptor::new(0, 1, AccessPolicies::WRITE),
				],
			)
			.expect("Failed to create raygen shader");
		let closest_hit_shader = renderer
			.create_shader(
				None,
				Sources::SPIRV(closest_hit_shader_artifact.borrow().into()),
				ShaderTypes::ClosestHit,
				[
					BindingDescriptor::new(0, 2, AccessPolicies::READ),
					BindingDescriptor::new(0, 3, AccessPolicies::READ),
					BindingDescriptor::new(0, 4, AccessPolicies::READ),
				],
			)
			.expect("Failed to create closest hit shader");
		let miss_shader = renderer
			.create_shader(
				None,
				Sources::SPIRV(miss_shader_artifact.borrow().into()),
				ShaderTypes::Miss,
				[],
			)
			.expect("Failed to create miss shader");

		let top_level_acceleration_structure = renderer.create_top_level_acceleration_structure(Some("Top Level"), 1);
		let bottom_level_acceleration_structure =
			renderer.create_bottom_level_acceleration_structure(&BottomLevelAccelerationStructure {
				description: BottomLevelAccelerationStructureDescriptions::Mesh {
					vertex_count: 3,
					vertex_position_encoding: Encodings::FloatingPoint,
					triangle_count: 1,
					index_format: DataTypes::U16,
				},
			});

		let bindings = [
			DescriptorSetBindingTemplate::new(0, DescriptorType::AccelerationStructure, Stages::RAYGEN),
			DescriptorSetBindingTemplate::new(1, DescriptorType::StorageImage, Stages::RAYGEN),
			DescriptorSetBindingTemplate::new(2, DescriptorType::StorageBuffer, Stages::CLOSEST_HIT),
			DescriptorSetBindingTemplate::new(3, DescriptorType::StorageBuffer, Stages::CLOSEST_HIT),
			DescriptorSetBindingTemplate::new(4, DescriptorType::StorageBuffer, Stages::CLOSEST_HIT),
		];

		let descriptor_set_layout_handle = renderer.create_descriptor_set_template(None, &bindings);

		let descriptor_set = renderer.create_descriptor_set(None, &descriptor_set_layout_handle);

		let render_target = renderer.build_image(
			crate::image::Builder::new(Formats::RGBA8UNORM, Uses::Storage)
				.extent(extent)
				.device_accesses(DeviceAccesses::DeviceToHost)
				.use_case(UseCases::DYNAMIC),
		);

		let _ = renderer.create_descriptor_binding(
			descriptor_set,
			BindingConstructor::acceleration_structure(&bindings[0], top_level_acceleration_structure),
		);
		let _ = renderer.create_descriptor_binding(descriptor_set, BindingConstructor::image(&bindings[1], render_target));
		let _ = renderer.create_descriptor_binding(
			descriptor_set,
			BindingConstructor::buffer(&bindings[2], vertex_positions_buffer.into()),
		);
		let _ = renderer.create_descriptor_binding(
			descriptor_set,
			BindingConstructor::buffer(&bindings[3], vertex_colors_buffer.into()),
		);
		let _ =
			renderer.create_descriptor_binding(descriptor_set, BindingConstructor::buffer(&bindings[4], index_buffer.into()));

		let pipeline = renderer.create_ray_tracing_pipeline(pipelines::ray_tracing::Builder::new(
			&[descriptor_set_layout_handle],
			&[],
			&[
				ShaderParameter::new(&raygen_shader, ShaderTypes::RayGen),
				ShaderParameter::new(&closest_hit_shader, ShaderTypes::ClosestHit),
				ShaderParameter::new(&miss_shader, ShaderTypes::Miss),
			],
		));

		let rendering_command_buffer_handle = renderer.create_command_buffer(None, queue_handle);

		let render_finished_synchronizer = renderer.create_synchronizer(None, true);

		let instances_buffer = renderer.create_acceleration_structure_instance_buffer(None, 1);

		renderer.write_instance(
			instances_buffer,
			0,
			[[1f32, 0f32, 0f32, 0f32], [0f32, 1f32, 0f32, 0f32], [0f32, 0f32, 1f32, 0f32]],
			0,
			0xFF,
			0,
			bottom_level_acceleration_structure,
		);

		let scratch_buffer = renderer.build_buffer::<[u8; 1024 * 1024]>(
			crate::buffer::Builder::new(Uses::AccelerationStructureBuildScratch).device_accesses(DeviceAccesses::DeviceOnly),
		);

		let raygen_sbt_buffer = renderer.build_buffer::<[u8; 64]>(
			crate::buffer::Builder::new(Uses::ShaderBindingTable).device_accesses(DeviceAccesses::HostToDevice),
		);
		let miss_sbt_buffer = renderer.build_buffer::<[u8; 64]>(
			crate::buffer::Builder::new(Uses::ShaderBindingTable).device_accesses(DeviceAccesses::HostToDevice),
		);
		let hit_sbt_buffer = renderer.build_buffer::<[u8; 64]>(
			crate::buffer::Builder::new(Uses::ShaderBindingTable).device_accesses(DeviceAccesses::HostToDevice),
		);

		renderer.write_sbt_entry(raygen_sbt_buffer.into(), 0, pipeline, raygen_shader);
		renderer.write_sbt_entry(miss_sbt_buffer.into(), 0, pipeline, miss_shader);
		renderer.write_sbt_entry(hit_sbt_buffer.into(), 0, pipeline, closest_hit_shader);

		for i in 0..FRAMES_IN_FLIGHT * 10 {
			renderer.start_frame_capture();

			let mut frame = renderer.start_frame(i as u32, render_finished_synchronizer);

			let mut command_buffer_recording = frame.create_command_buffer_recording(rendering_command_buffer_handle);

			{
				command_buffer_recording.build_bottom_level_acceleration_structures(&[BottomLevelAccelerationStructureBuild {
					acceleration_structure: bottom_level_acceleration_structure,
					description: BottomLevelAccelerationStructureBuildDescriptions::Mesh {
						vertex_buffer: BufferStridedRange::new(vertex_positions_buffer.into(), 0, 12, 12 * 3),
						vertex_count: 3,
						index_buffer: BufferStridedRange::new(index_buffer.into(), 0, 2, 2 * 3),
						vertex_position_encoding: Encodings::FloatingPoint,
						index_format: DataTypes::U16,
						triangle_count: 1,
					},
					scratch_buffer: BufferDescriptor::new(scratch_buffer),
				}]);

				command_buffer_recording.build_top_level_acceleration_structure(&TopLevelAccelerationStructureBuild {
					acceleration_structure: top_level_acceleration_structure,
					description: TopLevelAccelerationStructureBuildDescriptions::Instance {
						instances_buffer,
						instance_count: 1,
					},
					scratch_buffer: BufferDescriptor::new(scratch_buffer),
				});
			}

			let ray_tracing_pipeline_command = command_buffer_recording.bind_ray_tracing_pipeline(pipeline);

			ray_tracing_pipeline_command.bind_descriptor_sets(&[descriptor_set]);

			ray_tracing_pipeline_command.trace_rays(
				BindingTables {
					raygen: BufferStridedRange::new(raygen_sbt_buffer.into(), 0, 64, 64),
					hit: BufferStridedRange::new(hit_sbt_buffer.into(), 0, 64, 64),
					miss: BufferStridedRange::new(miss_sbt_buffer.into(), 0, 64, 64),
					callable: None,
				},
				1920,
				1080,
				1,
			);

			let texure_copy_handles = command_buffer_recording.transfer_textures(&[render_target]);

			let terminated_command_buffer = command_buffer_recording.end(&[]);
			frame.execute(terminated_command_buffer, render_finished_synchronizer);

			renderer.end_frame_capture();

			assert!(!renderer.has_errors());

			let pixels = unsafe {
				std::slice::from_raw_parts(
					renderer.get_image_data(texure_copy_handles[0]).as_ptr() as *const RGBAu8,
					(extent.width() * extent.height()) as usize,
				)
			};

			check_triangle(pixels, extent);
		}
	}
}
