#include "VulkanBindings.h"

#include "VulkanRenderDevice.h"

#include "Native/VKDevice.h"

#include "VulkanUniformBuffer.h"
#include "VulkanTexture.h"

VulkanBindingsPool::VulkanBindingsPool(VulkanRenderDevice* device,
                                       const BindingsPoolCreateInfo& descriptorPoolCreateInfo)
{
	Array<VkDescriptorPoolSize, MAX_BINDINGS_PER_SET> descriptor_pool_sizes;
	descriptor_pool_sizes.resize(descriptorPoolCreateInfo.BindingsSetLayout.getLength());
	{
		uint8 i = 0;

		for (auto& descriptor_pool_size : descriptor_pool_sizes)
		{
			//Type of the descriptor pool.
			descriptor_pool_size.type = UniformTypeToVkDescriptorType(
				descriptorPoolCreateInfo.BindingsSetLayout[i].BindingType);
			//Max number of descriptors of VkDescriptorPoolSize::type we can allocate.
			descriptor_pool_size.descriptorCount = descriptorPoolCreateInfo.BindingsSetCount;

			++i;
		}
	}

	VkDescriptorPoolCreateInfo vk_descriptor_pool_create_info = {VK_STRUCTURE_TYPE_DESCRIPTOR_POOL_CREATE_INFO};
	//Is the total number of sets that can be allocated from the pool.
	vk_descriptor_pool_create_info.maxSets = descriptorPoolCreateInfo.BindingsSetCount;
	vk_descriptor_pool_create_info.poolSizeCount = descriptor_pool_sizes.getLength();
	vk_descriptor_pool_create_info.pPoolSizes = descriptor_pool_sizes.getData();

	vkCreateDescriptorPool(
		static_cast<VulkanRenderDevice*>(descriptorPoolCreateInfo.RenderDevice)->GetVkDevice().GetVkDevice(),
		&vk_descriptor_pool_create_info, ALLOCATOR, &vkDescriptorPool);
}

void VulkanBindingsPool::FreeBindingsSet()
{
	//vkFreeDescriptorSets(, vkDescriptorPool, );
}

void VulkanBindingsPool::FreePool(const FreeBindingsPoolInfo& freeDescriptorPoolInfo)
{
	vkResetDescriptorPool(
		static_cast<VulkanRenderDevice*>(freeDescriptorPoolInfo.RenderDevice)->GetVkDevice().GetVkDevice(),
		vkDescriptorPool, 0);
}

VulkanBindingsSet::VulkanBindingsSet(VulkanRenderDevice* device, const BindingsSetCreateInfo& descriptorSetCreateInfo)
{
	VkDescriptorSetLayoutCreateInfo vk_descriptor_set_layout_create_info = {
		VK_STRUCTURE_TYPE_DESCRIPTOR_SET_LAYOUT_CREATE_INFO
	};

	Array<VkDescriptorSetLayoutBinding, MAX_BINDINGS_PER_SET> descriptor_set_layout_bindings;
	descriptor_set_layout_bindings.resize(descriptorSetCreateInfo.BindingsSetLayout.getLength());
	{
		uint8 i = 0;

		for (auto& binding : descriptor_set_layout_bindings)
		{
			binding.binding = i;
			binding.descriptorCount = descriptorSetCreateInfo.BindingsSetLayout[i].ArrayLength;
			binding.descriptorType = UniformTypeToVkDescriptorType(
				descriptorSetCreateInfo.BindingsSetLayout[i].BindingType);
			binding.stageFlags = ShaderTypeToVkShaderStageFlagBits(
				descriptorSetCreateInfo.BindingsSetLayout[i].ShaderStage);
			binding.pImmutableSamplers = nullptr;

			++i;
		}
	}

	vk_descriptor_set_layout_create_info.bindingCount = descriptor_set_layout_bindings.getLength();
	vk_descriptor_set_layout_create_info.pBindings = descriptor_set_layout_bindings.getData();

	vkCreateDescriptorSetLayout(
		static_cast<VulkanRenderDevice*>(descriptorSetCreateInfo.RenderDevice)->GetVkDevice().GetVkDevice(),
		&vk_descriptor_set_layout_create_info, ALLOCATOR, &vkDescriptorSetLayout);

	VkDescriptorSetAllocateInfo vk_descriptor_set_allocate_info = {VK_STRUCTURE_TYPE_DESCRIPTOR_SET_ALLOCATE_INFO};
	vk_descriptor_set_allocate_info.descriptorPool = static_cast<VulkanBindingsPool*>(descriptorSetCreateInfo.
		BindingsPool)->GetVkDescriptorPool();
	vk_descriptor_set_allocate_info.descriptorSetCount = descriptorSetCreateInfo.BindingsSetCount;

	FVector<VkDescriptorSetLayout> SetLayouts(descriptorSetCreateInfo.BindingsSetCount, descriptorSetCreateInfo.BindingsSetCount);

	vk_descriptor_set_allocate_info.pSetLayouts = SetLayouts.getData();

	vkDescriptorSets.resize(vk_descriptor_set_allocate_info.descriptorSetCount);

	vkAllocateDescriptorSets(
		static_cast<VulkanRenderDevice*>(descriptorSetCreateInfo.RenderDevice)->GetVkDevice().GetVkDevice(),
		&vk_descriptor_set_allocate_info, vkDescriptorSets.getData());
}

void VulkanBindingsSet::Update(const BindingsSetUpdateInfo& uniformLayoutUpdateInfo)
{
	DArray<VkWriteDescriptorSet> write_descriptors(uniformLayoutUpdateInfo.BindingsSetLayout.getLength());

	for (uint8 i = 0; i < uniformLayoutUpdateInfo.BindingsSetLayout.getLength(); ++i)
	{
		switch (uniformLayoutUpdateInfo.BindingsSetLayout[i].BindingType)
		{
		case BindingType::SAMPLER:
		case BindingType::COMBINED_IMAGE_SAMPLER:
		case BindingType::SAMPLED_IMAGE:

			VkDescriptorImageInfo DescriptorImageInfo;
			DescriptorImageInfo.imageView = static_cast<VulkanTexture*>(uniformLayoutUpdateInfo.BindingsSetLayout[i].
				BindingResource)->GetImageView();
			DescriptorImageInfo.imageLayout = ImageLayoutToVkImageLayout(
				static_cast<VulkanTexture*>(uniformLayoutUpdateInfo.BindingsSetLayout[i].BindingResource)->
				GetImageLayout());
			DescriptorImageInfo.sampler = static_cast<VulkanTexture*>(uniformLayoutUpdateInfo.BindingsSetLayout[i].
				BindingResource)->GetImageSampler();

			write_descriptors[i].sType = VK_STRUCTURE_TYPE_WRITE_DESCRIPTOR_SET;
			write_descriptors[i].pNext = nullptr;
			write_descriptors[i].dstSet = vkDescriptorSets[uniformLayoutUpdateInfo.DestinationSet];
			write_descriptors[i].dstBinding = i;
			write_descriptors[i].dstArrayElement = 0;
			write_descriptors[i].descriptorCount = uniformLayoutUpdateInfo.BindingsSetLayout[i].ArrayLength;
			write_descriptors[i].descriptorType = UniformTypeToVkDescriptorType(
				uniformLayoutUpdateInfo.BindingsSetLayout[i].BindingType);
			write_descriptors[i].pImageInfo = &DescriptorImageInfo;
			write_descriptors[i].pTexelBufferView = nullptr;
			write_descriptors[i].pBufferInfo = nullptr;

			break;

			//case BindingType::STORAGE_IMAGE: break;

			//case BindingType::UNIFORM_TEXEL_BUFFER: break;
			//case BindingType::STORAGE_TEXEL_BUFFER: break;

		case BindingType::UNIFORM_BUFFER:
		case BindingType::STORAGE_BUFFER:

			VkDescriptorBufferInfo DescriptorBufferInfo;
			DescriptorBufferInfo.buffer = static_cast<VulkanUniformBuffer*>(uniformLayoutUpdateInfo.BindingsSetLayout[i].
				BindingResource)->GetVKBuffer().GetHandle();
			DescriptorBufferInfo.offset = 0; //TODO: Get offset from buffer itself
			DescriptorBufferInfo.range = VK_WHOLE_SIZE;

			write_descriptors[i].sType = VK_STRUCTURE_TYPE_WRITE_DESCRIPTOR_SET;
			write_descriptors[i].pNext = nullptr;
			write_descriptors[i].dstSet = vkDescriptorSets[uniformLayoutUpdateInfo.DestinationSet];
			write_descriptors[i].dstBinding = i;
			write_descriptors[i].dstArrayElement = 0;
			write_descriptors[i].descriptorCount = uniformLayoutUpdateInfo.BindingsSetLayout[i].ArrayLength;
			write_descriptors[i].descriptorType = UniformTypeToVkDescriptorType(
				uniformLayoutUpdateInfo.BindingsSetLayout[i].BindingType);
			write_descriptors[i].pImageInfo = nullptr;
			write_descriptors[i].pTexelBufferView = nullptr;
			write_descriptors[i].pBufferInfo = &DescriptorBufferInfo;

			break;

			//case BindingType::UNIFORM_BUFFER_DYNAMIC: break;
			//case BindingType::STORAGE_BUFFER_DYNAMIC: break;
			//case BindingType::INPUT_ATTACHMENT: break;
		default: ;
		}
	}

	vkUpdateDescriptorSets(
		static_cast<VulkanRenderDevice*>(uniformLayoutUpdateInfo.RenderDevice)->GetVkDevice().GetVkDevice(),
		write_descriptors.getCapacity(), write_descriptors.getData(), 0, nullptr);
}
