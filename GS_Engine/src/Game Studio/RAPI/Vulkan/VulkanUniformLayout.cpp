#include "VulkanUniformLayout.h"

#include "RAPI/Vulkan/Vulkan.h"

#include "Containers/Array.hpp"

#include "RAPI/RenderContext.h"
#include "Native/VKDevice.h"
#include "VulkanImage.h"
#include "VulkanUniformBuffer.h"
#include "RAPI/RenderDevice.h"
#include "VulkanRenderer.h"

VKDescriptorSetLayoutCreator VulkanUniformLayout::CreateDescriptorSetLayout(VKDevice* _Device, const UniformLayoutCreateInfo& _PLCI)
{
	VkDescriptorSetLayoutCreateInfo DescriptorSetLayoutCreateInfo = { VK_STRUCTURE_TYPE_DESCRIPTOR_SET_LAYOUT_CREATE_INFO };

	Array<VkDescriptorSetLayoutBinding, MAX_DESCRIPTORS_PER_SET> DescriptorBindings;
	{
		for (uint8 i = 0; i < _PLCI.PipelineUniformSets.getLength(); ++i)
		{
			DescriptorBindings[i].binding = i;
			DescriptorBindings[i].descriptorCount = _PLCI.PipelineUniformSets[i].UniformSetUniformsCount;
			DescriptorBindings[i].descriptorType = UniformTypeToVkDescriptorType(_PLCI.PipelineUniformSets[i].UniformSetType);
			DescriptorBindings[i].stageFlags = ShaderTypeToVkShaderStageFlagBits(_PLCI.PipelineUniformSets[i].ShaderStage);
		}
	}

	DescriptorBindings.setLength(_PLCI.PipelineUniformSets.getLength());

	DescriptorSetLayoutCreateInfo.bindingCount = DescriptorBindings.getLength();
	DescriptorSetLayoutCreateInfo.pBindings = DescriptorBindings.getData();

	return VKDescriptorSetLayoutCreator(_Device, &DescriptorSetLayoutCreateInfo);
}

VKDescriptorPoolCreator VulkanUniformLayout::CreateDescriptorPool(VKDevice* _Device, const UniformLayoutCreateInfo& _PLCI)
{
	Array<VkDescriptorPoolSize, MAX_DESCRIPTORS_PER_SET> PoolSizes;
	{
		for (uint8 i = 0; i < _PLCI.PipelineUniformSets.getLength(); ++i)
		{
			PoolSizes[i].descriptorCount = _PLCI.RenderContext->GetMaxFramesInFlight();
			PoolSizes[i].type = UniformTypeToVkDescriptorType(_PLCI.PipelineUniformSets[i].UniformSetType);
		}
	}

	PoolSizes.setLength(_PLCI.PipelineUniformSets.getLength());

	VkDescriptorPoolCreateInfo DescriptorPoolCreateInfo = { VK_STRUCTURE_TYPE_DESCRIPTOR_POOL_CREATE_INFO };
	DescriptorPoolCreateInfo.maxSets = _PLCI.RenderContext->GetMaxFramesInFlight();
	DescriptorPoolCreateInfo.poolSizeCount = PoolSizes.getLength();
	DescriptorPoolCreateInfo.pPoolSizes = PoolSizes.getData();

	return VKDescriptorPoolCreator(_Device, &DescriptorPoolCreateInfo);
}

void VulkanUniformLayout::CreateDescriptorSet(VKDevice* _Device, const UniformLayoutCreateInfo& _PLCI)
{
	VkDescriptorSetAllocateInfo DescriptorSetAllocateInfo = { VK_STRUCTURE_TYPE_DESCRIPTOR_SET_ALLOCATE_INFO };
	DescriptorSetAllocateInfo.descriptorPool = DescriptorPool.GetHandle();
	DescriptorSetAllocateInfo.descriptorSetCount = _PLCI.RenderContext->GetMaxFramesInFlight();

	FVector<VkDescriptorSetLayout> SetLayouts(_PLCI.RenderContext->GetMaxFramesInFlight(), DescriptorSetLayout.GetHandle());

	DescriptorSetAllocateInfo.pSetLayouts = SetLayouts.getData();

	DescriptorPool.AllocateDescriptorSets(&DescriptorSetAllocateInfo, DescriptorSets.getData());
}


VKPipelineLayoutCreator VulkanUniformLayout::CreatePipelineLayout(VKDevice* _Device, const UniformLayoutCreateInfo& _PLCI) const
{
	VkPipelineLayoutCreateInfo PipelineLayoutCreateInfo = { VK_STRUCTURE_TYPE_PIPELINE_LAYOUT_CREATE_INFO };

	VkPushConstantRange PushConstantRange = {};
	if (_PLCI.PushConstant)
	{
		PushConstantRange.size = _PLCI.PushConstant->Size;
		PushConstantRange.offset = 0;
		PushConstantRange.stageFlags = VK_SHADER_STAGE_VERTEX_BIT;

		PipelineLayoutCreateInfo.pushConstantRangeCount = 1;
		PipelineLayoutCreateInfo.pPushConstantRanges = &PushConstantRange;
	}
	else
	{
		PipelineLayoutCreateInfo.pushConstantRangeCount = 0;
		PipelineLayoutCreateInfo.pPushConstantRanges = nullptr;
	}

	VkDescriptorSetLayout pDescriptorSetLayouts = DescriptorSetLayout.GetHandle();
	PipelineLayoutCreateInfo.setLayoutCount = 1;
	PipelineLayoutCreateInfo.pSetLayouts = &pDescriptorSetLayouts;

	return VKPipelineLayoutCreator(_Device, &PipelineLayoutCreateInfo);
}

VulkanUniformLayout::VulkanUniformLayout(VKDevice* _Device, const UniformLayoutCreateInfo& _PLCI) :
	DescriptorSetLayout(CreateDescriptorSetLayout(_Device, _PLCI)),
	DescriptorPool(CreateDescriptorPool(_Device, _PLCI)),
	DescriptorSets(_PLCI.RenderContext->GetMaxFramesInFlight()),
	PipelineLayout(CreatePipelineLayout(_Device, _PLCI))
{
	CreateDescriptorSet(_Device, _PLCI);
}

void VulkanUniformLayout::UpdateUniformSet(const UniformLayoutUpdateInfo& _ULUI)
{
	DArray<VkWriteDescriptorSet> WriteDescriptors(_ULUI.PipelineUniformSets.getLength());
	for (uint8 i = 0; i < _ULUI.PipelineUniformSets.getLength(); ++i)
	{
		switch (_ULUI.PipelineUniformSets[i].UniformSetType)
		{
		case UniformType::SAMPLER:

			VkDescriptorImageInfo DescriptorImageInfo;
			DescriptorImageInfo.imageView = SCAST(VulkanImageBase*, _ULUI.PipelineUniformSets[i].UniformData)->GetVKImageView().GetHandle();
			DescriptorImageInfo.imageLayout = VK_IMAGE_LAYOUT_UNDEFINED;

			WriteDescriptors[i].sType = VK_STRUCTURE_TYPE_WRITE_DESCRIPTOR_SET;
			WriteDescriptors[i].pNext = nullptr;
			WriteDescriptors[i].dstSet = DescriptorSets[i];
			WriteDescriptors[i].dstBinding = 0;
			WriteDescriptors[i].dstArrayElement = 0;
			WriteDescriptors[i].descriptorCount = _ULUI.PipelineUniformSets[i].UniformSetUniformsCount;
			WriteDescriptors[i].descriptorType = UniformTypeToVkDescriptorType(_ULUI.PipelineUniformSets[i].UniformSetType);
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
			WriteDescriptors[i].descriptorCount = _ULUI.PipelineUniformSets[i].UniformSetUniformsCount;
			WriteDescriptors[i].descriptorType = UniformTypeToVkDescriptorType(_ULUI.PipelineUniformSets[i].UniformSetType);
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
			DescriptorBufferInfo.buffer = SCAST(VulkanUniformBuffer*, _ULUI.PipelineUniformSets[i].UniformData)->GetVKBuffer().GetHandle();
			DescriptorBufferInfo.offset = 0; //TODO: Get offset from buffer itself
			DescriptorBufferInfo.range = VK_WHOLE_SIZE;

			WriteDescriptors[i].sType = VK_STRUCTURE_TYPE_WRITE_DESCRIPTOR_SET;
			WriteDescriptors[i].pNext = nullptr;
			WriteDescriptors[i].dstSet = DescriptorSets[i];
			WriteDescriptors[i].dstBinding = 0;
			WriteDescriptors[i].dstArrayElement = 0;
			WriteDescriptors[i].descriptorCount = _ULUI.PipelineUniformSets[i].UniformSetUniformsCount;
			WriteDescriptors[i].descriptorType = UniformTypeToVkDescriptorType(_ULUI.PipelineUniformSets[i].UniformSetType);
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

	vkUpdateDescriptorSets(SCAST(VulkanRenderDevice*, RenderDevice::Get())->GetVKDevice().GetVkDevice(), WriteDescriptors.getCapacity(), WriteDescriptors.getData(), 0, nullptr);
}