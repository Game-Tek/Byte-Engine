#include "VulkanUniformLayout.h"

#include "RAPI/Vulkan/Vulkan.h"

#include "Containers/Array.hpp"

#include "RAPI/RenderContext.h"
#include "Native/VKDevice.h"
#include "VulkanImage.h"
#include "RAPI/Buffer.h"
#include "VulkanUniformBuffer.h"

VKDescriptorSetLayoutCreator VulkanUniformLayout::CreateDescriptorSetLayout(VKDevice* _Device,	const PipelineLayoutCreateInfo& _PLCI)
{
	VkDescriptorSetLayoutCreateInfo DescriptorSetLayoutCreateInfo = { VK_STRUCTURE_TYPE_DESCRIPTOR_SET_LAYOUT_CREATE_INFO };

	Array<VkDescriptorSetLayoutBinding, MAX_DESCRIPTORS_PER_SET> DescriptorBindings;
	{
		for(uint8 i = 0; i < _PLCI.PipelineUniformSets.length(); ++i)
		{
			DescriptorBindings[i].binding = i;
			DescriptorBindings[i].descriptorCount = _PLCI.PipelineUniformSets[i].UniformSetUniformsCount;
			DescriptorBindings[i].descriptorType = UniformTypeToVkDescriptorType(_PLCI.PipelineUniformSets[i].UniformSetType);
			DescriptorBindings[i].stageFlags = ShaderTypeToVkShaderStageFlagBits(_PLCI.PipelineUniformSets[i].ShaderStage);
		}
	}

	DescriptorSetLayoutCreateInfo.bindingCount = DescriptorBindings.length();
	DescriptorSetLayoutCreateInfo.pBindings = DescriptorBindings.data();

	return VKDescriptorSetLayoutCreator(_Device, &DescriptorSetLayoutCreateInfo);
}

VKDescriptorPoolCreator VulkanUniformLayout::CreateDescriptorPool(VKDevice* _Device, const PipelineLayoutCreateInfo& _PLCI)
{
	Array<VkDescriptorPoolSize, MAX_DESCRIPTORS_PER_SET> PoolSizes;
	{
		for(uint8 i = 0; i < _PLCI.PipelineUniformSets.length(); ++i)
		{
			PoolSizes[i].descriptorCount = _PLCI.RenderContext->GetMaxFramesInFlight();
			PoolSizes[i].type = UniformTypeToVkDescriptorType(_PLCI.PipelineUniformSets[i].UniformSetType);
		}
	}

	VkDescriptorPoolCreateInfo DescriptorPoolCreateInfo = { VK_STRUCTURE_TYPE_DESCRIPTOR_POOL_CREATE_INFO };
	DescriptorPoolCreateInfo.maxSets = _PLCI.RenderContext->GetMaxFramesInFlight();
	DescriptorPoolCreateInfo.poolSizeCount = PoolSizes.length();
	DescriptorPoolCreateInfo.pPoolSizes = PoolSizes.data();

	return VKDescriptorPoolCreator(_Device, &DescriptorPoolCreateInfo);
}

void VulkanUniformLayout::CreateDescriptorSet(VKDevice* _Device, const PipelineLayoutCreateInfo& _PLCI)
{
	VkDescriptorSetAllocateInfo DescriptorSetAllocateInfo = { VK_STRUCTURE_TYPE_DESCRIPTOR_SET_ALLOCATE_INFO };
	DescriptorSetAllocateInfo.descriptorPool = DescriptorPool.GetHandle();
	DescriptorSetAllocateInfo.descriptorSetCount = _PLCI.RenderContext->GetMaxFramesInFlight();

	VkDescriptorSetLayout pSetLayouts = DescriptorSetLayout.GetHandle();

	DescriptorSetAllocateInfo.pSetLayouts = &pSetLayouts;

	DescriptorPool.AllocateDescriptorSets(&DescriptorSetAllocateInfo, DescriptorSets.data());

	UpdateDescriptorSet(_Device, _PLCI);
}


VKPipelineLayoutCreator VulkanUniformLayout::CreatePipelineLayout(VKDevice* _Device, const PipelineLayoutCreateInfo& _PLCI) const
{
	VkPipelineLayoutCreateInfo PipelineLayoutCreateInfo = { VK_STRUCTURE_TYPE_PIPELINE_LAYOUT_CREATE_INFO };
	PipelineLayoutCreateInfo.setLayoutCount = 1;

	VkDescriptorSetLayout pDescriptorSetLayouts = DescriptorSetLayout.GetHandle();

	PipelineLayoutCreateInfo.pSetLayouts = &pDescriptorSetLayouts;

	return VKPipelineLayoutCreator(_Device, &PipelineLayoutCreateInfo);
}

VulkanUniformLayout::VulkanUniformLayout(VKDevice* _Device, const PipelineLayoutCreateInfo& _PLCI) : 
DescriptorSetLayout(CreateDescriptorSetLayout(_Device, _PLCI)), 
DescriptorPool(CreateDescriptorPool(_Device, _PLCI)),
DescriptorSets(_PLCI.RenderContext->GetMaxFramesInFlight()),
PipelineLayout(CreatePipelineLayout(_Device, _PLCI))
{
	CreateDescriptorSet(_Device, _PLCI);
}

void VulkanUniformLayout::UpdateDescriptorSet(VKDevice* _Device, const PipelineLayoutCreateInfo& _PLCI)
{
	DArray<VkWriteDescriptorSet> WriteDescriptors(DescriptorSets.length());
	for (uint8 i = 0; i < DescriptorSets.length(); ++i)
	{
		switch (_PLCI.PipelineUniformSets[i].UniformSetType)
		{
		case UniformType::SAMPLER:

			VkDescriptorImageInfo DescriptorImageInfo;
			DescriptorImageInfo.imageView = SCAST(VulkanImageBase*, _PLCI.PipelineUniformSets[i].UniformData)->GetVKImageView().GetHandle();
			DescriptorImageInfo.imageLayout = VK_IMAGE_LAYOUT_UNDEFINED;

			WriteDescriptors[i].sType = VK_STRUCTURE_TYPE_WRITE_DESCRIPTOR_SET;
			WriteDescriptors[i].pNext = nullptr;
			WriteDescriptors[i].dstSet = DescriptorSets[i];
			WriteDescriptors[i].dstBinding = 0;
			WriteDescriptors[i].dstArrayElement = 0;
			WriteDescriptors[i].descriptorCount = _PLCI.PipelineUniformSets[i].UniformSetUniformsCount;
			WriteDescriptors[i].descriptorType = UniformTypeToVkDescriptorType(_PLCI.PipelineUniformSets[i].UniformSetType);
			WriteDescriptors[i].pImageInfo = &DescriptorImageInfo;
			WriteDescriptors[i].pTexelBufferView = nullptr;
			WriteDescriptors[i].pBufferInfo = nullptr;

			break;

			//case UniformType::COMBINED_IMAGE_SAMPLER: break;

		case UniformType::SAMPLED_IMAGE:

			WriteDescriptors[i].sType = VK_STRUCTURE_TYPE_WRITE_DESCRIPTOR_SET;
			WriteDescriptors[i].pNext = nullptr;
			WriteDescriptors[i].dstSet = DescriptorSets[i];
			WriteDescriptors[i].dstBinding = 0;
			WriteDescriptors[i].dstArrayElement = 0;
			WriteDescriptors[i].descriptorCount = _PLCI.PipelineUniformSets[i].UniformSetUniformsCount;
			WriteDescriptors[i].descriptorType = UniformTypeToVkDescriptorType(_PLCI.PipelineUniformSets[i].UniformSetType);
			WriteDescriptors[i].pImageInfo;
			WriteDescriptors[i].pTexelBufferView = nullptr;
			WriteDescriptors[i].pBufferInfo = nullptr;

			break;

			//case UniformType::STORAGE_IMAGE: break;

			//case UniformType::UNIFORM_TEXEL_BUFFER: break;
			//case UniformType::STORAGE_TEXEL_BUFFER: break;

		case UniformType::UNIFORM_BUFFER:
		case UniformType::STORAGE_BUFFER:

			VkDescriptorBufferInfo DescriptorBufferInfo;
			DescriptorBufferInfo.buffer = SCAST(VulkanUniformBuffer*, _PLCI.PipelineUniformSets[i].UniformData)->GetVKBuffer().GetHandle();
			DescriptorBufferInfo.offset = 0; //TODO: Get offset from buffer itself
			DescriptorBufferInfo.range = VK_WHOLE_SIZE;

			WriteDescriptors[i].sType = VK_STRUCTURE_TYPE_WRITE_DESCRIPTOR_SET;
			WriteDescriptors[i].pNext = nullptr;
			WriteDescriptors[i].dstSet = DescriptorSets[i];
			WriteDescriptors[i].dstBinding = 0;
			WriteDescriptors[i].dstArrayElement = 0;
			WriteDescriptors[i].descriptorCount = _PLCI.PipelineUniformSets[i].UniformSetUniformsCount;
			WriteDescriptors[i].descriptorType = UniformTypeToVkDescriptorType(_PLCI.PipelineUniformSets[i].UniformSetType);
			WriteDescriptors[i].pImageInfo = nullptr;
			WriteDescriptors[i].pTexelBufferView = nullptr;
			WriteDescriptors[i].pBufferInfo = &DescriptorBufferInfo;

			break;

			//case UniformType::UNIFORM_BUFFER_DYNAMIC: break;
			//case UniformType::STORAGE_BUFFER_DYNAMIC: break;
			//case UniformType::INPUT_ATTACHMENT: break;
		default:;
		}
	}

	vkUpdateDescriptorSets(_Device->GetVkDevice(), WriteDescriptors.length(), WriteDescriptors.data(), 0, nullptr);
}
