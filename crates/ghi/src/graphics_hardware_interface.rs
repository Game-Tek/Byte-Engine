//! The graphics hardware interface implements easy to use rendering functionality.
//! It provides useful abstractions to interact with the GPU.
//! It's not tied to any particular render pipeline implementation.

use utils::{Extent, RGBA};

#[cfg(all(test, target_os = "linux"))]
use crate::AccessPolicies;
use crate::{
	descriptors::{self, DescriptorType},
	DataTypes, Encodings, Formats, Layouts, Stages, WorkloadTypes,
};

// HANDLES

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct QueueHandle(pub(crate) u64);

/// The `BaseBufferHandle` allows addressing any static buffer irregardless of it's underlying type.
#[derive(PartialEq, Eq, Clone, Copy, Hash, Debug, PartialOrd, Ord)]
pub struct BaseBufferHandle(pub(super) u64);

impl MasterHandle for BaseBufferHandle {
	fn new(i: u64) -> Self {
		BaseBufferHandle(i)
	}

	fn index(&self) -> u64 {
		self.0
	}
}

/// The `BufferHandle` allows addressing a buffer static buffer with a specific underlying type.
#[derive(PartialEq, Eq, Clone, Copy, Hash, Debug)]
pub struct BufferHandle<T>(pub(super) BaseBufferHandle, pub(super) std::marker::PhantomData<T>);

/// The `DynamicBufferHandle` allows addressing a dynamic buffer with a specific underlying type.
#[derive(PartialEq, Eq, Clone, Copy, Hash, Debug)]
pub struct DynamicBufferHandle<T>(pub(super) BaseBufferHandle, pub(super) std::marker::PhantomData<T>);

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct BaseImageHandle(pub(super) u64);

impl MasterHandle for BaseImageHandle {
	fn new(i: u64) -> Self {
		BaseImageHandle(i)
	}

	fn index(&self) -> u64 {
		self.0
	}
}

impl From<BaseImageHandle> for Handles {
	fn from(value: BaseImageHandle) -> Self {
		Handles::Image(ImageHandle(value))
	}
}

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct ImageHandle(pub(super) BaseImageHandle);

impl From<ImageHandle> for BaseImageHandle {
	fn from(value: ImageHandle) -> Self {
		value.0
	}
}

/// The `DynamicImageHandle` struct addresses a frame-local image that can be written independently for each frame in flight.
#[derive(PartialEq, Eq, Clone, Copy, Hash, Debug)]
pub struct DynamicImageHandle(pub(super) BaseImageHandle);

impl From<DynamicImageHandle> for BaseImageHandle {
	fn from(value: DynamicImageHandle) -> Self {
		value.0
	}
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
pub struct MeshHandle(pub(super) u64);

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct SynchronizerHandle(pub(super) u64);

impl MasterHandle for SynchronizerHandle {
	fn new(i: u64) -> Self {
		Self(i)
	}

	fn index(&self) -> u64 {
		self.0
	}
}

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct DescriptorSetTemplateHandle(pub(super) u64);

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
/// The `DescriptorSetHandle` struct identifies a retained group of flat shader resource writes.
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

impl<T: Copy> From<BufferHandle<T>> for BaseBufferHandle {
	fn from(val: BufferHandle<T>) -> Self {
		val.0
	}
}

impl<T: Copy> From<DynamicBufferHandle<T>> for BaseBufferHandle {
	fn from(val: DynamicBufferHandle<T>) -> Self {
		val.0
	}
}

impl From<DynamicImageHandle> for Handles {
	fn from(val: DynamicImageHandle) -> Self {
		val.0.into()
	}
}

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum Handles {
	Buffer(BaseBufferHandle),
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

impl From<BaseBufferHandle> for Handles {
	fn from(val: BaseBufferHandle) -> Self {
		Handles::Buffer(val)
	}
}

impl From<ImageHandle> for Handles {
	fn from(val: ImageHandle) -> Self {
		Handles::Image(val)
	}
}

impl From<SynchronizerHandle> for Handles {
	fn from(val: SynchronizerHandle) -> Self {
		Handles::Synchronizer(val)
	}
}

pub(crate) trait MasterHandle: Sized + Copy {
	fn new(i: u64) -> Self;
	fn index(&self) -> u64;
}

impl<T: Copy> MasterHandle for BufferHandle<T> {
	fn new(i: u64) -> Self {
		Self(BaseBufferHandle(i), std::marker::PhantomData)
	}

	fn index(&self) -> u64 {
		self.0 .0
	}
}

pub(crate) trait PrivateHandle: Copy {
	fn new(i: u64) -> Self;
	fn index(&self) -> u64;
}

// HANDLES

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
			self.dispatch_extent
				.width()
				.max(1)
				.div_ceil(self.workgroup_extent.width().max(1)),
			self.dispatch_extent
				.height()
				.max(1)
				.div_ceil(self.workgroup_extent.height().max(1)),
			self.dispatch_extent
				.depth()
				.max(1)
				.div_ceil(self.workgroup_extent.depth().max(1)),
		)
	}

	pub fn get_workgroup_extent(&self) -> Extent {
		self.workgroup_extent
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

#[derive(Clone, Copy, Debug, Default)]
pub enum PresentationModes {
	Inmediate,
	#[default]
	FIFO,
	Mailbox,
}

#[derive(Clone, Copy)]
pub enum ClearValue {
	None,
	Color(RGBA),
	Integer(u32, u32, u32, u32),
	Depth(f32),
}

#[derive(Clone, Copy)]
pub enum ImageOrSwapchain {
	Image(BaseImageHandle),
	Swapchain(SwapchainHandle),
}

impl From<BaseImageHandle> for ImageOrSwapchain {
	fn from(value: BaseImageHandle) -> Self {
		Self::Image(value)
	}
}

impl From<ImageHandle> for ImageOrSwapchain {
	fn from(value: ImageHandle) -> Self {
		Self::Image(value.into())
	}
}

impl From<DynamicImageHandle> for ImageOrSwapchain {
	fn from(value: DynamicImageHandle) -> Self {
		Self::Image(value.into())
	}
}

impl From<SwapchainHandle> for ImageOrSwapchain {
	fn from(value: SwapchainHandle) -> Self {
		Self::Swapchain(value)
	}
}

#[derive(Clone, Copy)]
/// Stores the information of an attachment.
pub struct AttachmentInformation {
	/// The image view of the attachment.
	pub(crate) target: ImageOrSwapchain,
	/// The format of the attachment. If `None`, the format will be determined by the target image.
	pub(crate) format: Option<Formats>,
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
	pub fn new(target: impl Into<ImageOrSwapchain>, layout: Layouts, clear: ClearValue, load: bool, store: bool) -> Self {
		Self {
			target: target.into(),
			format: None,
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
	/// The texture view type expected by this binding when it references textures.
	pub(crate) texture_view_type: TextureViewTypes,
	/// The structured element byte stride expected by this binding when it references buffers.
	pub(crate) buffer_stride: u32,
	/// Whether a storage-buffer binding is read-only and should use SRV-style binding on APIs that distinguish it.
	pub(crate) buffer_read_only: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum TextureViewTypes {
	Texture2D,
	Texture2DArray,
	Texture3D,
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

// Generates paired convenience constructors so each single/array pair shares one descriptor type.
macro_rules! descriptor_template_constructors {
	($( $single:ident, $array:ident => $descriptor_type:ident; )+) => {
		$(
			pub const fn $single(binding: u32, stages: Stages) -> Self {
				Self::new(binding, DescriptorType::$descriptor_type, stages)
			}

			pub const fn $array(binding: u32, stages: Stages, count: u32) -> Self {
				Self::new_array(binding, DescriptorType::$descriptor_type, stages, count)
			}
		)+
	};
}

impl DescriptorSetBindingTemplate {
	pub const fn new(binding: u32, descriptor_type: DescriptorType, stages: Stages) -> Self {
		Self::new_array(binding, descriptor_type, stages, 1)
	}

	pub const fn new_array(binding: u32, descriptor_type: DescriptorType, stages: Stages, count: u32) -> Self {
		Self {
			binding,
			descriptor_type,
			descriptor_count: count,
			stages,
			immutable_samplers: None,
			texture_view_type: TextureViewTypes::Texture2D,
			buffer_stride: 4,
			buffer_read_only: false,
		}
	}

	pub const fn texture_view_type(mut self, texture_view_type: TextureViewTypes) -> Self {
		self.texture_view_type = texture_view_type;
		self
	}

	pub const fn buffer_stride(mut self, buffer_stride: u32) -> Self {
		self.buffer_stride = buffer_stride;
		self
	}

	pub const fn buffer_read_only(mut self, buffer_read_only: bool) -> Self {
		self.buffer_read_only = buffer_read_only;
		self
	}

	descriptor_template_constructors! {
		uniform_buffer, uniform_buffer_array => UniformBuffer;
		storage_buffer, storage_buffer_array => StorageBuffer;
		sampled_image, sampled_image_array => SampledImage;
		combined_image_sampler, combined_image_sampler_array => CombinedImageSampler;
		storage_image, storage_image_array => StorageImage;
		input_attachment, input_attachment_array => InputAttachment;
		sampler, sampler_array => Sampler;
		acceleration_structure, acceleration_structure_array => AccelerationStructure;
	}

	pub fn new_with_immutable_samplers(binding: u32, stages: Stages, samplers: Option<Vec<SamplerHandle>>) -> Self {
		let mut template = Self::sampler(binding, stages);
		template.immutable_samplers = samplers;
		template
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
	fn new(descriptor_set_binding_template: &'a DescriptorSetBindingTemplate, descriptor: descriptors::WriteData) -> Self {
		Self {
			descriptor_set_binding_template,
			array_element: 0,
			descriptor,
			frame_offset: None,
		}
	}

	pub fn buffer(descriptor_set_binding_template: &'a DescriptorSetBindingTemplate, buffer_handle: BaseBufferHandle) -> Self {
		Self::new(descriptor_set_binding_template, descriptors::WriteData::buffer(buffer_handle))
	}

	pub fn image(
		descriptor_set_binding_template: &'a DescriptorSetBindingTemplate,
		image_handle: impl Into<BaseImageHandle>,
	) -> Self {
		Self::new(
			descriptor_set_binding_template,
			descriptors::WriteData::image(image_handle, crate::Layouts::General),
		)
	}

	pub fn swapchain(
		descriptor_set_binding_template: &'a DescriptorSetBindingTemplate,
		swapchain_handle: SwapchainHandle,
	) -> Self {
		Self::new(
			descriptor_set_binding_template,
			descriptors::WriteData::Swapchain(swapchain_handle),
		)
	}

	pub fn sampler(descriptor_set_binding_template: &'a DescriptorSetBindingTemplate, sampler_handle: SamplerHandle) -> Self {
		Self::new(
			descriptor_set_binding_template,
			descriptors::WriteData::Sampler(sampler_handle),
		)
	}

	pub fn combined_image_sampler(
		descriptor_set_binding_template: &'a DescriptorSetBindingTemplate,
		image_handle: impl Into<BaseImageHandle>,
		sampler_handle: SamplerHandle,
		layout: Layouts,
	) -> Self {
		Self::new(
			descriptor_set_binding_template,
			descriptors::WriteData::combined_image_sampler(image_handle, sampler_handle, layout, None),
		)
	}

	pub fn combined_image_sampler_array(descriptor_set_binding_template: &'a DescriptorSetBindingTemplate) -> Self {
		Self::new(
			descriptor_set_binding_template,
			descriptors::WriteData::CombinedImageSamplerArray,
		)
	}

	pub fn combined_image_sampler_layer(
		descriptor_set_binding_template: &'a DescriptorSetBindingTemplate,
		image_handle: impl Into<BaseImageHandle>,
		sampler_handle: SamplerHandle,
		layout: Layouts,
		layer_index: u32,
	) -> Self {
		Self::new(
			descriptor_set_binding_template,
			descriptors::WriteData::combined_image_sampler(image_handle, sampler_handle, layout, Some(layer_index)),
		)
	}

	pub fn sampler_with_immutable_samplers(descriptor_set_binding_template: &'a DescriptorSetBindingTemplate) -> Self {
		Self::new(descriptor_set_binding_template, descriptors::WriteData::StaticSamplers)
	}

	pub fn acceleration_structure(
		bindings: &'a DescriptorSetBindingTemplate,
		top_level_acceleration_structure: TopLevelAccelerationStructureHandle,
	) -> Self {
		Self::new(
			bindings,
			descriptors::WriteData::acceleration_structure(top_level_acceleration_structure),
		)
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
			descriptors::WriteData::CombinedImageSampler { layout: old_layout, .. } => {
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

	use super::*;
	use crate::{
		command_buffer::{
			BoundComputePipelineMode as _, BoundPipelineLayoutMode as _, BoundRasterizationPipelineMode as _,
			BoundRayTracingPipelineMode as _, CommandBuffer as _, CommandBufferRecording as _, CommonCommandBufferMode as _,
			RasterizationRenderPassMode as _,
		},
		frame::Frame as _,
		pipelines::{self, raster::AttachmentDescriptor, PushConstantRange, ShaderParameter, VertexElement},
		queue::{FrameRequest, Queue as _, QueueExecution as _},
		rt::{
			BindingTables, BottomLevelAccelerationStructureBuild, BottomLevelAccelerationStructureBuildDescriptions,
			TopLevelAccelerationStructureBuild, TopLevelAccelerationStructureBuildDescriptions,
		},
		shader::{CompiledShaderSource, ShaderSource},
		BufferDescriptor, BufferStridedRange, DeviceAccesses, FilteringModes, SamplerAddressingModes, SamplingReductionModes,
		ShaderTypes, UseCases, Uses, Window,
	};
	use crate::{ChannelBitSize, ChannelLayout, Size as _};

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
		assert_eq!(Formats::BC7SRGB.encoding(), Some(Encodings::sRGB));

		// Test formats without encoding
		assert_eq!(Formats::U32.encoding(), None);
		assert_eq!(Formats::BC5.encoding(), None);
		assert_eq!(Formats::BC7.encoding(), None);
	}

	#[test]
	fn descriptor_write_constructors_preserve_set_slot_array_and_frame_semantics() {
		let set = DescriptorSetHandle(1);
		let slot = crate::shader::ResourceSlot::new(9);
		let buffer = BaseBufferHandle(2);
		let image = ImageHandle(BaseImageHandle(3));
		let sampler = SamplerHandle(4);
		let acceleration_structure = TopLevelAccelerationStructureHandle(5);

		let buffer_write = descriptors::DescriptorWrite::buffer(set, slot, buffer);
		assert_eq!(buffer_write.descriptor_set, set);
		assert_eq!(buffer_write.slot, slot);
		assert_eq!(buffer_write.array_element, 0);
		assert_eq!(buffer_write.frame_offset, None);
		assert!(matches!(
			buffer_write.descriptor,
			descriptors::WriteData::Buffer {
				handle,
				size: Ranges::Whole
			} if handle == buffer
		));

		let image_write = descriptors::DescriptorWrite::image_with_frame(set, slot, image, Layouts::General, -1);
		assert_eq!(image_write.frame_offset, Some(-1));
		assert!(matches!(
			image_write.descriptor,
			descriptors::WriteData::Image {
				handle,
				layout: Layouts::General
			} if handle == BaseImageHandle(3)
		));

		let array_write = descriptors::DescriptorWrite::combined_image_sampler_array_with_frame(
			set,
			slot,
			image,
			sampler,
			Layouts::Read,
			7,
			2,
		);
		assert_eq!(array_write.array_element, 7);
		assert_eq!(array_write.frame_offset, Some(2));
		assert!(matches!(
			array_write.descriptor,
			descriptors::WriteData::CombinedImageSampler {
				image_handle,
				sampler_handle,
				layout: Layouts::Read,
				layer: None,
			} if image_handle == BaseImageHandle(3) && sampler_handle == sampler
		));

		let sampler_write = descriptors::DescriptorWrite::sampler(set, slot, sampler);
		assert!(matches!(sampler_write.descriptor, descriptors::WriteData::Sampler(value) if value == sampler));
		let acceleration_write = descriptors::DescriptorWrite::acceleration_structure(set, slot, acceleration_structure);
		assert!(matches!(
			acceleration_write.descriptor,
			descriptors::WriteData::AccelerationStructure { handle } if handle == acceleration_structure
		));
	}

	#[test]
	fn descriptor_write_variants_without_frame_offsets_remain_frame_invariant() {
		let set = DescriptorSetHandle(8);
		let slot = crate::shader::ResourceSlot::new(12);
		let image = ImageHandle(BaseImageHandle(9));
		let sampler = SamplerHandle(10);

		let image_write = descriptors::DescriptorWrite::image(set, slot, image, Layouts::Read);
		let combined = descriptors::DescriptorWrite::combined_image_sampler(set, slot, image, sampler, Layouts::Read);
		let array = descriptors::DescriptorWrite::combined_image_sampler_array(set, slot, image, sampler, Layouts::Read, 3);

		assert_eq!(image_write.frame_offset, None);
		assert_eq!(combined.frame_offset, None);
		assert_eq!(array.frame_offset, None);
		assert_eq!(array.array_element, 3);
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
		assert_eq!(Formats::BC7SRGB.channel_bit_size(), ChannelBitSize::Compressed);
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
		assert_eq!(Formats::BC7SRGB.channel_layout(), ChannelLayout::BC);
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

		// For BC7 sRGB
		let format = Formats::BC7SRGB;
		assert_eq!(format.encoding(), Some(Encodings::sRGB));
		assert_eq!(format.channel_bit_size(), ChannelBitSize::Compressed);
		assert_eq!(format.channel_layout(), ChannelLayout::BC);
		assert_eq!(format.size(), 1);
	}

	fn compile_shaders() -> (CompiledShaderSource, CompiledShaderSource) {
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
		let vertex_shader_msl = r#"
			#include <metal_stdlib>
			using namespace metal;
			struct VertexInput {
				float3 position [[attribute(0)]];
				float4 color [[attribute(1)]];
			};
			struct VertexOutput {
				float4 position [[position]];
				float4 color;
			};
			vertex VertexOutput vertex_main(VertexInput input [[stage_in]]) {
				return VertexOutput { float4(input.position, 1.0), input.color };
			}
		"#;
		let fragment_shader_msl = r#"
			#include <metal_stdlib>
			using namespace metal;
			struct VertexOutput {
				float4 position [[position]];
				float4 color;
			};
			fragment float4 fragment_main(VertexOutput input [[stage_in]]) {
				return input.color;
			}
		"#;
		let vertex_shader_hlsl = r#"
			struct VertexInput { float3 position : POSITION; float4 color : COLOR0; };
			struct VertexOutput { float4 position : SV_POSITION; float4 color : COLOR0; };
			VertexOutput vertex_main(VertexInput input) {
				VertexOutput output;
				output.position = float4(input.position, 1.0);
				output.color = input.color;
				return output;
			}
		"#;
		let fragment_shader_hlsl = r#"
			struct VertexOutput { float4 position : SV_POSITION; float4 color : COLOR0; };
			float4 fragment_main(VertexOutput input) : SV_TARGET0 { return input.color; }
		"#;

		let vertex_shader_artifact = crate::shader::compile(
			"GHI test vertex shader",
			ShaderSource::PlatformNative {
				glsl: vertex_shader_code,
				msl: vertex_shader_msl,
				msl_entry_point: "vertex_main",
				hlsl: vertex_shader_hlsl,
				hlsl_entry_point: "vertex_main",
			},
		)
		.expect("Failed to compile GHI test vertex shader. The most likely cause is invalid native shader source.");
		let fragment_shader_artifact = crate::shader::compile(
			"GHI test fragment shader",
			ShaderSource::PlatformNative {
				glsl: fragment_shader_code,
				msl: fragment_shader_msl,
				msl_entry_point: "fragment_main",
				hlsl: fragment_shader_hlsl,
				hlsl_entry_point: "fragment_main",
			},
		)
		.expect("Failed to compile GHI test fragment shader. The most likely cause is invalid native shader source.");

		(vertex_shader_artifact, fragment_shader_artifact)
	}

	fn compile_shaders_with_model_matrix() -> (CompiledShaderSource, CompiledShaderSource) {
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
		let vertex_shader_msl = r#"
			#include <metal_stdlib>
			using namespace metal;
			struct VertexInput {
				float3 position [[attribute(0)]];
				float4 color [[attribute(1)]];
			};
			struct VertexOutput {
				float4 position [[position]];
				float4 color;
			};
			vertex VertexOutput vertex_main(
				VertexInput input [[stage_in]],
				constant float4x4& model_matrix [[buffer(15)]]) {
				return VertexOutput { model_matrix * float4(input.position, 1.0), input.color };
			}
		"#;
		let fragment_shader_msl = r#"
			#include <metal_stdlib>
			using namespace metal;
			struct VertexOutput {
				float4 position [[position]];
				float4 color;
			};
			fragment float4 fragment_main(VertexOutput input [[stage_in]]) {
				return input.color;
			}
		"#;
		let vertex_shader_hlsl = r#"
			struct VertexInput { float3 position : POSITION; float4 color : COLOR0; };
			struct VertexOutput { float4 position : SV_POSITION; float4 color : COLOR0; };
			struct PushConstant { float4x4 model_matrix; };
			ConstantBuffer<PushConstant> push_constant : register(b0, space0);
			VertexOutput vertex_main(VertexInput input) {
				VertexOutput output;
				output.position = mul(push_constant.model_matrix, float4(input.position, 1.0));
				output.color = input.color;
				return output;
			}
		"#;
		let fragment_shader_hlsl = r#"
			struct VertexOutput { float4 position : SV_POSITION; float4 color : COLOR0; };
			float4 fragment_main(VertexOutput input) : SV_TARGET0 { return input.color; }
		"#;

		let vertex_shader_artifact = crate::shader::compile(
			"GHI model-matrix test vertex shader",
			ShaderSource::PlatformNative {
				glsl: vertex_shader_code,
				msl: vertex_shader_msl,
				msl_entry_point: "vertex_main",
				hlsl: vertex_shader_hlsl,
				hlsl_entry_point: "vertex_main",
			},
		)
		.expect(
			"Failed to compile GHI model-matrix test vertex shader. The most likely cause is invalid native shader source.",
		);
		let fragment_shader_artifact = crate::shader::compile(
			"GHI model-matrix test fragment shader",
			ShaderSource::PlatformNative {
				glsl: fragment_shader_code,
				msl: fragment_shader_msl,
				msl_entry_point: "fragment_main",
				hlsl: fragment_shader_hlsl,
				hlsl_entry_point: "fragment_main",
			},
		)
		.expect("Failed to compile GHI test fragment shader. The most likely cause is invalid native shader source.");

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

		if !extent.width().is_multiple_of(2) {
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

	pub(crate) fn render_triangle(device: &mut impl crate::context::Context, queue_handle: QueueHandle) {
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
			.create_shader(None, vertex_shader_artifact.as_source(), ShaderTypes::Vertex, [])
			.expect("Failed to create vertex shader");
		let fragment_shader = device
			.create_shader(None, fragment_shader_artifact.as_source(), ShaderTypes::Fragment, [])
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
			&vertex_layout,
			&[
				ShaderParameter::new(&vertex_shader, ShaderTypes::Vertex),
				ShaderParameter::new(&fragment_shader, ShaderTypes::Fragment),
			],
			&attachments,
		));

		let command_buffer_handle = device.queue(queue_handle).create_command_buffer(None);

		device.start_frame_capture();

		let texture_copy_handles = {
			let mut command_buffer = device.command_buffer(command_buffer_handle);
			let mut command_buffer_recording = command_buffer.create_command_buffer_recording();

			let attachments = [AttachmentInformation::new(
				render_target,
				Layouts::RenderTarget,
				ClearValue::Color(RGBA::black()),
				false,
				true,
			)];

			let render_pass_command = command_buffer_recording.start_render_pass(extent, &attachments);

			let raster_pipeline_command = render_pass_command.bind_raster_pipeline(pipeline);

			raster_pipeline_command.draw_mesh(&mesh);

			render_pass_command.end_render_pass();

			let texture_copy_handles = command_buffer_recording.transfer_textures(&[render_target.into()]);

			command_buffer_recording.execute(signal);
			texture_copy_handles
		};

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

	pub(crate) fn present(renderer: &mut impl crate::context::Context, queue_handle: QueueHandle) {
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
			.create_shader(None, vertex_shader_artifact.as_source(), ShaderTypes::Vertex, [])
			.expect("Failed to create vertex shader");
		let fragment_shader = renderer
			.create_shader(None, fragment_shader_artifact.as_source(), ShaderTypes::Fragment, [])
			.expect("Failed to create fragment shader");

		let attachments = [AttachmentDescriptor::new(Formats::BGRAsRGB)];

		let pipeline = renderer.create_raster_pipeline(pipelines::raster::Builder::new(
			&[],
			&vertex_layout,
			&[
				ShaderParameter::new(&vertex_shader, ShaderTypes::Vertex),
				ShaderParameter::new(&fragment_shader, ShaderTypes::Fragment),
			],
			&attachments,
		));

		let command_buffer_handle = renderer.queue(queue_handle).create_command_buffer(None);

		let render_finished_synchronizer = renderer.create_synchronizer(None, true);

		for _ in window.poll() {}

		renderer.start_frame_capture();

		{
			let mut queue = renderer.queue(queue_handle);
			queue.execute(
				Some(FrameRequest {
					index: 0,
					synchronizer: render_finished_synchronizer,
				}),
				&[],
				render_finished_synchronizer,
				|execution| {
					let (present_key, _) = execution.frame().unwrap().acquire_swapchain_image(swapchain);
					let present_keys = [present_key];

					execution.record(command_buffer_handle, |command_buffer_recording| {
						let attachments = [AttachmentInformation::new(
							swapchain,
							Layouts::RenderTarget,
							ClearValue::Color(RGBA::black()),
							false,
							true,
						)];

						let render_pass_command = command_buffer_recording.start_render_pass(extent, &attachments);

						let raster_pipeline_command = render_pass_command.bind_raster_pipeline(pipeline);

						raster_pipeline_command.draw_mesh(&mesh);

						render_pass_command.end_render_pass();
					});

					present_keys
				},
			);
		}

		renderer.end_frame_capture();

		for _ in window.poll() {}

		// TODO: assert rendering results

		assert!(!renderer.has_errors())
	}

	pub(crate) fn multiframe_present(renderer: &mut impl crate::context::Context, queue_handle: QueueHandle) {
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
			.create_shader(None, vertex_shader_artifact.as_source(), ShaderTypes::Vertex, [])
			.expect("Failed to create vertex shader");
		let fragment_shader = renderer
			.create_shader(None, fragment_shader_artifact.as_source(), ShaderTypes::Fragment, [])
			.expect("Failed to create fragment shader");

		let attachments = [AttachmentDescriptor::new(Formats::BGRAsRGB)];

		let pipeline = renderer.create_raster_pipeline(pipelines::raster::Builder::new(
			&[],
			&vertex_layout,
			&[
				ShaderParameter::new(&vertex_shader, ShaderTypes::Vertex),
				ShaderParameter::new(&fragment_shader, ShaderTypes::Fragment),
			],
			&attachments,
		));

		let command_buffer_handle = renderer.queue(queue_handle).create_command_buffer(None);

		let render_finished_synchronizer = renderer.create_synchronizer(None, true);

		for i in 0..2 * 64 {
			renderer.start_frame_capture();

			{
				let mut queue = renderer.queue(queue_handle);
				queue.execute(
					Some(FrameRequest {
						index: i,
						synchronizer: render_finished_synchronizer,
					}),
					&[],
					render_finished_synchronizer,
					|execution| {
						let (present_key, _) = execution.frame().unwrap().acquire_swapchain_image(swapchain);
						let present_keys = [present_key];

						execution.record(command_buffer_handle, |command_buffer_recording| {
							let attachments = [AttachmentInformation::new(
								swapchain,
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
						});

						present_keys
					},
				);
			}

			renderer.end_frame_capture();

			assert!(!renderer.has_errors());
		}
	}

	pub(crate) fn multiframe_rendering(device: &mut impl crate::context::Context, queue_handle: QueueHandle) {
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
			.create_shader(None, vertex_shader_artifact.as_source(), ShaderTypes::Vertex, [])
			.expect("Failed to create vertex shader");
		let fragment_shader = device
			.create_shader(None, fragment_shader_artifact.as_source(), ShaderTypes::Fragment, [])
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
			&[PushConstantRange::new(0, 16 * 4)],
			&vertex_layout,
			&[
				ShaderParameter::new(&vertex_shader, ShaderTypes::Vertex),
				ShaderParameter::new(&fragment_shader, ShaderTypes::Fragment),
			],
			&attachments,
		));

		let command_buffer_handle = device.queue(queue_handle).create_command_buffer(None);

		let render_finished_synchronizer = device.create_synchronizer(None, true);

		for i in 0..FRAMES_IN_FLIGHT * 10 {
			device.start_frame_capture();

			let texture_copy_handles = {
				let mut queue = device.queue(queue_handle);
				let mut texture_copy_handles = Vec::new();
				queue.execute(
					Some(FrameRequest {
						index: i as u32,
						synchronizer: render_finished_synchronizer,
					}),
					&[],
					render_finished_synchronizer,
					|execution| {
						execution.record(command_buffer_handle, |command_buffer_recording| {
							let attachments = [AttachmentInformation::new(
								render_target,
								Layouts::RenderTarget,
								ClearValue::Color(RGBA::black()),
								false,
								true,
							)];

							let render_pass_command = command_buffer_recording.start_render_pass(extent, &attachments);

							let raster_pipeline_command = render_pass_command.bind_raster_pipeline(pipeline);

							raster_pipeline_command.draw_mesh(&mesh);

							raster_pipeline_command.end_render_pass();

							texture_copy_handles = command_buffer_recording.transfer_textures(&[render_target.into()]);
						});
						[]
					},
				);
				texture_copy_handles
			};

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

	pub(crate) fn change_frames(device: &mut impl crate::context::Context, queue_handle: QueueHandle) {
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
			.create_shader(None, vertex_shader_artifact.as_source(), ShaderTypes::Vertex, [])
			.expect("Failed to create vertex shader");
		let fragment_shader = device
			.create_shader(None, fragment_shader_artifact.as_source(), ShaderTypes::Fragment, [])
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
			&vertex_layout,
			&[
				ShaderParameter::new(&vertex_shader, ShaderTypes::Vertex),
				ShaderParameter::new(&fragment_shader, ShaderTypes::Fragment),
			],
			&attachments,
		));

		let command_buffer_handle = device.queue(queue_handle).create_command_buffer(None);

		let render_finished_synchronizer = device.create_synchronizer(None, true);

		for i in 0..FRAMES_IN_FLIGHT * 10 {
			if i == 2 {
				device.set_frames_in_flight(3); // Change from default 2 to 3
			}

			device.start_frame_capture();

			let texture_copy_handles = {
				let mut queue = device.queue(queue_handle);
				let mut texture_copy_handles = Vec::new();
				queue.execute(
					Some(FrameRequest {
						index: i as u32,
						synchronizer: render_finished_synchronizer,
					}),
					&[],
					render_finished_synchronizer,
					|execution| {
						execution.record(command_buffer_handle, |command_buffer_recording| {
							let attachments = [AttachmentInformation::new(
								render_target,
								Layouts::RenderTarget,
								ClearValue::Color(RGBA::black()),
								false,
								true,
							)];

							let render_pass_command = command_buffer_recording.start_render_pass(extent, &attachments);

							let raster_pipeline_command = render_pass_command.bind_raster_pipeline(pipeline);

							raster_pipeline_command.draw_mesh(&mesh);

							raster_pipeline_command.end_render_pass();

							texture_copy_handles = command_buffer_recording.transfer_textures(&[render_target.into()]);
						});
						[]
					},
				);
				texture_copy_handles
			};

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

	pub(crate) fn resize(device: &mut impl crate::context::Context, queue_handle: QueueHandle) {
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
			.create_shader(None, vertex_shader_artifact.as_source(), ShaderTypes::Vertex, [])
			.expect("Failed to create vertex shader");
		let fragment_shader = device
			.create_shader(None, fragment_shader_artifact.as_source(), ShaderTypes::Fragment, [])
			.expect("Failed to create fragment shader");

		let mut extent = Extent::rectangle(1280, 720);

		let render_target = device.build_dynamic_image(
			crate::image::Builder::new(Formats::RGBA8UNORM, Uses::RenderTarget)
				.extent(extent)
				.device_accesses(DeviceAccesses::DeviceToHost)
				.use_case(UseCases::DYNAMIC),
		);

		let attachments = [AttachmentDescriptor::new(Formats::RGBA8UNORM)];

		let pipeline = device.create_raster_pipeline(pipelines::raster::Builder::new(
			&[],
			&vertex_layout,
			&[
				ShaderParameter::new(&vertex_shader, ShaderTypes::Vertex),
				ShaderParameter::new(&fragment_shader, ShaderTypes::Fragment),
			],
			&attachments,
		));

		let command_buffer_handle = device.queue(queue_handle).create_command_buffer(None);

		let render_finished_synchronizer = device.create_synchronizer(None, true);

		for i in 0..FRAMES_IN_FLIGHT * 10 {
			device.start_frame_capture();

			let texture_copy_handles = {
				let mut queue = device.queue(queue_handle);
				let mut texture_copy_handles = Vec::new();

				queue.execute(
					Some(FrameRequest {
						index: i as u32,
						synchronizer: render_finished_synchronizer,
					}),
					&[],
					render_finished_synchronizer,
					|execution| {
						let frame = execution.frame().unwrap();

						if i == 2 {
							extent = Extent::rectangle(1920, 1080);
							frame.resize_image(render_target.into(), extent);
						}

						execution.record(command_buffer_handle, |command_buffer_recording| {
							let attachments = [AttachmentInformation::new(
								render_target,
								Layouts::RenderTarget,
								ClearValue::Color(RGBA::black()),
								false,
								true,
							)];

							let render_pass_command = command_buffer_recording.start_render_pass(extent, &attachments);

							let raster_pipeline_command = render_pass_command.bind_raster_pipeline(pipeline);

							raster_pipeline_command.draw_mesh(&mesh);

							raster_pipeline_command.end_render_pass();

							texture_copy_handles = command_buffer_recording.transfer_textures(&[render_target.into()]);
						});
						[]
					},
				);
				texture_copy_handles
			};

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

	pub(crate) fn dynamic_data(device: &mut impl crate::context::Context, queue_handle: QueueHandle) {
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
			.create_shader(None, vertex_shader_artifact.as_source(), ShaderTypes::Vertex, [])
			.expect("Failed to create vertex shader");
		let fragment_shader = device
			.create_shader(None, fragment_shader_artifact.as_source(), ShaderTypes::Fragment, [])
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
			&vertex_layout,
			&[
				ShaderParameter::new(&vertex_shader, ShaderTypes::Vertex),
				ShaderParameter::new(&fragment_shader, ShaderTypes::Fragment),
			],
			&attachments,
		));

		let _buffer =
			device.build_buffer::<u8>(crate::buffer::Builder::new(Uses::Storage).device_accesses(DeviceAccesses::HostToDevice));

		let command_buffer_handle = device.queue(queue_handle).create_command_buffer(None);

		let render_finished_synchronizer = device.create_synchronizer(None, true);

		for i in 0..FRAMES_IN_FLIGHT * 10 {
			device.start_frame_capture();

			let copy_texture_handles = {
				let mut queue = device.queue(queue_handle);
				let mut copy_texture_handles = Vec::new();
				queue.execute(
					Some(FrameRequest {
						index: i as u32,
						synchronizer: render_finished_synchronizer,
					}),
					&[],
					render_finished_synchronizer,
					|execution| {
						execution.record(command_buffer_handle, |command_buffer_recording| {
							let attachments = [AttachmentInformation::new(
								render_target,
								Layouts::RenderTarget,
								ClearValue::Color(RGBA::black()),
								false,
								true,
							)];

							let c = command_buffer_recording.start_render_pass(extent, &attachments);

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

							copy_texture_handles = command_buffer_recording.transfer_textures(&[render_target.into()]);
						});
						[]
					},
				);
				copy_texture_handles
			};

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

	pub(crate) fn dynamic_textures(device: &mut impl crate::context::Context, queue_handle: QueueHandle) {
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

		let command_buffer_handle = device.queue(queue_handle).create_command_buffer(None);
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
			let texture_copy_handles = {
				let mut queue = device.queue(queue_handle);
				let mut texture_copy_handles = Vec::new();
				queue.execute(
					Some(FrameRequest {
						index: frame_index as u32,
						synchronizer: render_finished_synchronizer,
					}),
					&[],
					render_finished_synchronizer,
					|execution| {
						let frame = execution.frame().unwrap();

						let texture_slice = frame.get_mut_dynamic_texture_slice(upload_image.into());
						let pixels =
							unsafe { std::slice::from_raw_parts_mut(texture_slice.as_mut_ptr() as *mut RGBAu8, pixel_count) };
						pixels.fill(expected_color);
						frame.sync_texture(upload_image.into());

						execution.record(command_buffer_handle, |command_buffer_recording| {
							command_buffer_recording.blit_image(
								upload_image.into(),
								Layouts::Transfer,
								readback_image.into(),
								Layouts::Transfer,
							);
							texture_copy_handles = command_buffer_recording.transfer_textures(&[readback_image.into()]);
						});
						[]
					},
				);
				texture_copy_handles
			};

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

	pub(crate) fn multiframe_resources(device: &mut impl crate::context::Context, queue_handle: QueueHandle) {
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
		let compute_shader_msl = r#"
			#include <metal_stdlib>
			using namespace metal;
			struct Resources {
				texture2d<float, access::write> image [[id(0)]];
				texture2d<float, access::read> last_frame_image [[id(1)]];
			};
			kernel void compute_main(
				uint2 gid [[thread_position_in_grid]],
				constant Resources& resources [[buffer(16)]],
				constant float& value [[buffer(15)]]) {
				resources.image.write(float4(value, value, value, 1.0), uint2(0, 0));
				resources.image.write(resources.last_frame_image.read(uint2(0, 0)), uint2(1, 0));
			}
		"#;
		let compute_shader_hlsl = r#"
			RWTexture2D<float4> image : register(u0, space0);
			RWTexture2D<float4> last_frame_image : register(u1, space0);
			struct PushConstant { float value; };
			ConstantBuffer<PushConstant> push_constant : register(b0, space0);
			[numthreads(1, 1, 1)]
			void compute_main(uint3 gid : SV_DispatchThreadID) {
				image[uint2(0, 0)] = float4(push_constant.value.xxx, 1.0);
				image[uint2(1, 0)] = last_frame_image[uint2(0, 0)];
			}
		"#;
		let compute_shader_artifact = crate::shader::compile(
			"GHI multiframe resource test compute shader",
			ShaderSource::PlatformNative {
				glsl: compute_shader_string,
				msl: compute_shader_msl,
				msl_entry_point: "compute_main",
				hlsl: compute_shader_hlsl,
				hlsl_entry_point: "compute_main",
			},
		)
		.expect("Failed to compile the multiframe resource shader. The most likely cause is invalid native shader source.");
		let image_resource = crate::shader::ShaderResourceDescriptor::single(
			crate::shader::ResourceSlot::new(0),
			crate::shader::ResourceKind::StorageImage,
			crate::AccessPolicies::WRITE,
		);
		let last_frame_image_resource = crate::shader::ShaderResourceDescriptor::single(
			crate::shader::ResourceSlot::new(1),
			crate::shader::ResourceKind::StorageImage,
			crate::AccessPolicies::READ,
		);

		let compute_shader = device
			.create_shader(
				None,
				compute_shader_artifact.as_source(),
				ShaderTypes::Compute,
				[image_resource, last_frame_image_resource],
			)
			.expect("Failed to create compute shader");

		let pipeline = device.create_compute_pipeline(pipelines::compute::Builder::new(
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

		let descriptor_set = device.create_descriptor_set(None);
		device.write(&[
			crate::DescriptorWrite::image(descriptor_set, image_resource.slot(), image, Layouts::General),
			crate::DescriptorWrite::image_with_frame(
				descriptor_set,
				last_frame_image_resource.slot(),
				image,
				Layouts::General,
				-1,
			),
		]);

		let command_buffer = device.queue(queue_handle).create_command_buffer(None);

		let signal = device.create_synchronizer(None, true);

		let copy_handles = {
			let mut queue = device.queue(queue_handle);
			let mut copy_handles = Vec::new();
			queue.execute(
				Some(FrameRequest {
					index: 0,
					synchronizer: signal,
				}),
				&[],
				signal,
				|execution| {
					execution.record(command_buffer, |command_buffer_recording| {
						let data = [0.5f32];

						let pipeline_command = command_buffer_recording.bind_compute_pipeline(pipeline);

						pipeline_command.write_push_constant(0, data);
						pipeline_command
							.bind_descriptor_sets(&[descriptor_set])
							.dispatch(DispatchExtent::new(Extent::square(1), Extent::square(1)));

						copy_handles = command_buffer_recording.transfer_textures(&[image.into()]);
					});
					[]
				},
			);
			copy_handles
		};

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

		let copy_handles = {
			let mut queue = device.queue(queue_handle);
			let mut copy_handles = Vec::new();
			queue.execute(
				Some(FrameRequest {
					index: 1,
					synchronizer: signal,
				}),
				&[],
				signal,
				|execution| {
					execution.record(command_buffer, |command_buffer_recording| {
						let data = [1.0f32];

						let pipeline_command = command_buffer_recording.bind_compute_pipeline(pipeline);

						pipeline_command.write_push_constant(0, data);
						pipeline_command
							.bind_descriptor_sets(&[descriptor_set])
							.dispatch(DispatchExtent::new(Extent::square(1), Extent::square(1)));

						copy_handles = command_buffer_recording.transfer_textures(&[image.into()]);
					});
					[]
				},
			);
			copy_handles
		};

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

		let copy_handles = {
			let mut queue = device.queue(queue_handle);
			let mut copy_handles = Vec::new();
			queue.execute(
				Some(FrameRequest {
					index: 2,
					synchronizer: signal,
				}),
				&[],
				signal,
				|execution| {
					execution.record(command_buffer, |command_buffer_recording| {
						copy_handles = command_buffer_recording.transfer_textures(&[image.into()]);
					});
					[]
				},
			);
			copy_handles
		};

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

		let copy_handles = {
			let mut queue = device.queue(queue_handle);
			let mut copy_handles = Vec::new();
			queue.execute(
				Some(FrameRequest {
					index: 3,
					synchronizer: signal,
				}),
				&[],
				signal,
				|execution| {
					execution.record(command_buffer, |command_buffer_recording| {
						copy_handles = command_buffer_recording.transfer_textures(&[image.into()]);
					});
					[]
				},
			);
			copy_handles
		};

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

	pub(crate) fn descriptor_sets(device: &mut impl crate::context::Context, queue_handle: QueueHandle) {
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
		let vertex_shader_msl = r#"
			#include <metal_stdlib>
			using namespace metal;
			struct VertexResources { constant float4x4* matrix [[id(0)]]; };
			struct VertexInput {
				float3 position [[attribute(0)]];
				float4 color [[attribute(1)]];
			};
			struct VertexOutput {
				float4 position [[position]];
				float4 color;
			};
			vertex VertexOutput vertex_main(
				VertexInput input [[stage_in]],
				constant VertexResources& resources [[buffer(16)]]) {
				return VertexOutput { resources.matrix[0] * float4(input.position, 1.0), input.color };
			}
		"#;
		let fragment_shader_msl = r#"
			#include <metal_stdlib>
			using namespace metal;
			struct FragmentResources {
				sampler texture_sampler [[id(0)]];
				texture2d<float> texture [[id(1)]];
			};
			struct VertexOutput {
				float4 position [[position]];
				float4 color;
			};
			fragment float4 fragment_main(
				VertexOutput input [[stage_in]],
				constant FragmentResources& resources [[buffer(16)]]) {
				return resources.texture.sample(resources.texture_sampler, float2(0.0));
			}
		"#;
		let vertex_shader_hlsl = r#"
			StructuredBuffer<float4x4> matrices : register(t1, space0);
			struct VertexInput { float3 position : POSITION; float4 color : COLOR0; };
			struct VertexOutput { float4 position : SV_POSITION; float4 color : COLOR0; };
			VertexOutput vertex_main(VertexInput input) {
				VertexOutput output;
				output.position = mul(matrices[0], float4(input.position, 1.0));
				output.color = input.color;
				return output;
			}
		"#;
		let fragment_shader_hlsl = r#"
			SamplerState texture_sampler : register(s0, space0);
			Texture2D<float4> texture_image : register(t2, space0);
			struct VertexOutput { float4 position : SV_POSITION; float4 color : COLOR0; };
			float4 fragment_main(VertexOutput input) : SV_TARGET0 {
				return texture_image.Sample(texture_sampler, float2(0.0, 0.0));
			}
		"#;
		let vertex_shader_artifact = crate::shader::compile(
			"GHI descriptor test vertex shader",
			ShaderSource::PlatformNative {
				glsl: vertex_shader_code,
				msl: vertex_shader_msl,
				msl_entry_point: "vertex_main",
				hlsl: vertex_shader_hlsl,
				hlsl_entry_point: "vertex_main",
			},
		)
		.expect("Failed to compile the descriptor test vertex shader. The most likely cause is invalid native shader source.");
		let fragment_shader_artifact = crate::shader::compile(
			"GHI descriptor test fragment shader",
			ShaderSource::PlatformNative {
				glsl: fragment_shader_code,
				msl: fragment_shader_msl,
				msl_entry_point: "fragment_main",
				hlsl: fragment_shader_hlsl,
				hlsl_entry_point: "fragment_main",
			},
		)
		.expect(
			"Failed to compile the descriptor test fragment shader. The most likely cause is invalid native shader source.",
		);
		let sampler_resource = crate::ShaderResourceDescriptor::single(
			crate::ResourceSlot::new(0),
			crate::ResourceKind::Sampler,
			crate::AccessPolicies::READ,
		);
		let buffer_resource = crate::ShaderResourceDescriptor::single(
			crate::ResourceSlot::new(1),
			crate::ResourceKind::StorageBuffer,
			crate::AccessPolicies::READ,
		);
		let image_resource = crate::ShaderResourceDescriptor::single(
			crate::ResourceSlot::new(2),
			crate::ResourceKind::SampledImage,
			crate::AccessPolicies::READ,
		);

		let vertex_shader = device
			.create_shader(
				None,
				vertex_shader_artifact.as_source(),
				ShaderTypes::Vertex,
				[buffer_resource],
			)
			.expect("Failed to create vertex shader");
		let fragment_shader = device
			.create_shader(
				None,
				fragment_shader_artifact.as_source(),
				ShaderTypes::Fragment,
				[sampler_resource, image_resource],
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

		let descriptor_set = device.create_descriptor_set(None);
		device.write(&[
			crate::DescriptorWrite::sampler(descriptor_set, sampler_resource.slot(), sampler),
			crate::DescriptorWrite::buffer(descriptor_set, buffer_resource.slot(), buffer.into()),
			crate::DescriptorWrite::image(descriptor_set, image_resource.slot(), sampled_texture, Layouts::Read),
		]);

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
			&[],
			&vertex_layout,
			&[
				ShaderParameter::new(&vertex_shader, ShaderTypes::Vertex),
				ShaderParameter::new(&fragment_shader, ShaderTypes::Fragment),
			],
			&attachments,
		));

		let command_buffer_handle = device.queue(queue_handle).create_command_buffer(None);

		device.start_frame_capture();

		let texure_copy_handles = {
			let mut queue = device.queue(queue_handle);
			let mut texure_copy_handles = Vec::new();
			queue.execute(
				Some(FrameRequest {
					index: 0,
					synchronizer: signal,
				}),
				&[],
				signal,
				|execution| {
					execution.record(command_buffer_handle, |command_buffer_recording| {
						command_buffer_recording.write_image_data(sampled_texture.into(), &pixels);

						let attachments = [AttachmentInformation::new(
							render_target,
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

						texure_copy_handles = command_buffer_recording.transfer_textures(&[render_target.into()]);
					});
					[]
				},
			);
			texure_copy_handles
		};

		device.end_frame_capture();

		device.wait();

		// assert colored triangle was drawn to texture
		let _pixels = device.get_image_data(texure_copy_handles[0]);

		// TODO: assert rendering results

		assert!(!device.has_errors());
	}

	pub(crate) fn ray_tracing(renderer: &mut impl crate::context::Context, queue_handle: QueueHandle) {
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

		// Metal ray tracing execution is still intentionally ignored, but native source keeps this shared test portable.
		let raygen_shader_artifact = crate::shader::compile(
			"GHI ray generation test shader",
			ShaderSource::PlatformNative {
				glsl: raygen_shader_code,
				msl: "#include <metal_stdlib>\nusing namespace metal; kernel void raygen_main() {}",
				msl_entry_point: "raygen_main",
				hlsl: "[shader(\"raygeneration\")] void raygen_main() {}",
				hlsl_entry_point: "raygen_main",
			},
		)
		.expect("Failed to compile the ray generation test shader. The most likely cause is invalid native shader source.");
		let closest_hit_shader_artifact = crate::shader::compile(
			"GHI closest-hit test shader",
			ShaderSource::PlatformNative {
				glsl: closest_hit_shader_code,
				msl: "#include <metal_stdlib>\nusing namespace metal; kernel void closest_hit_main() {}",
				msl_entry_point: "closest_hit_main",
				hlsl: "[shader(\"closesthit\")] void closest_hit_main() {}",
				hlsl_entry_point: "closest_hit_main",
			},
		)
		.expect("Failed to compile the closest-hit test shader. The most likely cause is invalid native shader source.");
		let miss_shader_artifact = crate::shader::compile(
			"GHI miss test shader",
			ShaderSource::PlatformNative {
				glsl: miss_shader_code,
				msl: "#include <metal_stdlib>\nusing namespace metal; kernel void miss_main() {}",
				msl_entry_point: "miss_main",
				hlsl: "[shader(\"miss\")] void miss_main() {}",
				hlsl_entry_point: "miss_main",
			},
		)
		.expect("Failed to compile the miss test shader. The most likely cause is invalid native shader source.");
		let acceleration_structure_resource = crate::ShaderResourceDescriptor::single(
			crate::ResourceSlot::new(0),
			crate::ResourceKind::AccelerationStructure,
			crate::AccessPolicies::READ,
		);
		let output_resource = crate::ShaderResourceDescriptor::single(
			crate::ResourceSlot::new(1),
			crate::ResourceKind::StorageImage,
			crate::AccessPolicies::WRITE,
		);
		let position_resource = crate::ShaderResourceDescriptor::single(
			crate::ResourceSlot::new(2),
			crate::ResourceKind::StorageBuffer,
			crate::AccessPolicies::READ,
		);
		let color_resource = crate::ShaderResourceDescriptor::single(
			crate::ResourceSlot::new(3),
			crate::ResourceKind::StorageBuffer,
			crate::AccessPolicies::READ,
		);
		let index_resource = crate::ShaderResourceDescriptor::single(
			crate::ResourceSlot::new(4),
			crate::ResourceKind::StorageBuffer,
			crate::AccessPolicies::READ,
		);

		let raygen_shader = renderer
			.create_shader(
				None,
				raygen_shader_artifact.as_source(),
				ShaderTypes::RayGen,
				[acceleration_structure_resource, output_resource],
			)
			.expect("Failed to create raygen shader");
		let closest_hit_shader = renderer
			.create_shader(
				None,
				closest_hit_shader_artifact.as_source(),
				ShaderTypes::ClosestHit,
				[position_resource, color_resource, index_resource],
			)
			.expect("Failed to create closest hit shader");
		let miss_shader = renderer
			.create_shader(None, miss_shader_artifact.as_source(), ShaderTypes::Miss, [])
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

		let descriptor_set = renderer.create_descriptor_set(None);

		let render_target = renderer.build_image(
			crate::image::Builder::new(Formats::RGBA8UNORM, Uses::Storage)
				.extent(extent)
				.device_accesses(DeviceAccesses::DeviceToHost)
				.use_case(UseCases::DYNAMIC),
		);

		renderer.write(&[
			crate::DescriptorWrite::acceleration_structure(
				descriptor_set,
				acceleration_structure_resource.slot(),
				top_level_acceleration_structure,
			),
			crate::DescriptorWrite::image(descriptor_set, output_resource.slot(), render_target, Layouts::General),
			crate::DescriptorWrite::buffer(descriptor_set, position_resource.slot(), vertex_positions_buffer.into()),
			crate::DescriptorWrite::buffer(descriptor_set, color_resource.slot(), vertex_colors_buffer.into()),
			crate::DescriptorWrite::buffer(descriptor_set, index_resource.slot(), index_buffer.into()),
		]);

		let pipeline = renderer.create_ray_tracing_pipeline(pipelines::ray_tracing::Builder::new(
			&[],
			&[
				ShaderParameter::new(&raygen_shader, ShaderTypes::RayGen),
				ShaderParameter::new(&closest_hit_shader, ShaderTypes::ClosestHit),
				ShaderParameter::new(&miss_shader, ShaderTypes::Miss),
			],
		));

		let rendering_command_buffer_handle = renderer.queue(queue_handle).create_command_buffer(None);

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

			let texure_copy_handles = {
				let mut queue = renderer.queue(queue_handle);
				let mut texure_copy_handles = Vec::new();
				queue.execute(
					Some(FrameRequest {
						index: i as u32,
						synchronizer: render_finished_synchronizer,
					}),
					&[],
					render_finished_synchronizer,
					|execution| {
						execution.record(rendering_command_buffer_handle, |command_buffer_recording| {
							{
								command_buffer_recording.build_bottom_level_acceleration_structures(&[
									BottomLevelAccelerationStructureBuild {
										acceleration_structure: bottom_level_acceleration_structure,
										description: BottomLevelAccelerationStructureBuildDescriptions::Mesh {
											vertex_buffer: BufferStridedRange::new(
												vertex_positions_buffer.into(),
												0,
												12,
												12 * 3,
											),
											vertex_count: 3,
											index_buffer: BufferStridedRange::new(index_buffer.into(), 0, 2, 2 * 3),
											vertex_position_encoding: Encodings::FloatingPoint,
											index_format: DataTypes::U16,
											triangle_count: 1,
										},
										scratch_buffer: BufferDescriptor::new(scratch_buffer),
									},
								]);

								command_buffer_recording.build_top_level_acceleration_structure(
									&TopLevelAccelerationStructureBuild {
										acceleration_structure: top_level_acceleration_structure,
										description: TopLevelAccelerationStructureBuildDescriptions::Instance {
											instances_buffer,
											instance_count: 1,
										},
										scratch_buffer: BufferDescriptor::new(scratch_buffer),
									},
								);
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

							texure_copy_handles = command_buffer_recording.transfer_textures(&[render_target.into()]);
						});
						[]
					},
				);
				texure_copy_handles
			};

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
