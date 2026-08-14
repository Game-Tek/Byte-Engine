//! Backend-independent GHI descriptors types.

use super::*;
use crate::{
	descriptors::{self, DescriptorType},
	Layouts, Stages,
};

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

/// The `DescriptorSetBindingTemplate` struct defines one resource binding in a retained descriptor set.
#[derive(Clone)]
pub struct DescriptorSetBindingTemplate {
	/// The shader-visible binding index.
	pub(crate) binding: u32,
	/// The resource type expected at the binding.
	pub(crate) descriptor_type: DescriptorType,
	/// The number of resources in the binding.
	pub(crate) descriptor_count: u32,
	/// The shader stages that can access the binding.
	pub(crate) stages: Stages,
	/// The immutable samplers assigned to the binding.
	pub(crate) immutable_samplers: Option<Vec<SamplerHandle>>,
	/// The texture view type expected by this binding when it references textures.
	pub(crate) texture_view_type: TextureViewTypes,
	/// The structured element byte stride expected by this binding when it references buffers.
	pub(crate) buffer_stride: u32,
	/// Whether a storage buffer uses read-only binding on APIs that distinguish access.
	pub(crate) buffer_read_only: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum TextureViewTypes {
	Texture2D,
	Texture2DArray,
	TextureCube,
	TextureCubeArray,
	Texture3D,
}

/// The `TypedDescriptorSetBindingTemplate` struct provides branded descriptor-set binding templates for compile-time descriptor-type safety.
#[derive(Clone)]
pub struct TypedDescriptorSetBindingTemplate<T: DescriptorSetBindingType> {
	pub(crate) template: DescriptorSetBindingTemplate,
	pub(crate) type_brand: std::marker::PhantomData<T>,
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
	pub(crate) descriptor_set_binding_template: &'a DescriptorSetBindingTemplate,
	/// The array element to update when the binding is an array.
	pub(crate) array_element: u32,
	/// The resource update to apply.
	pub(crate) descriptor: descriptors::WriteData,
	pub(crate) frame_offset: Option<i8>,
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
