#include "VulkanUniformLayout.h"

#include "RAPI/Vulkan/Vulkan.h"

#include "Containers/Array.hpp"

#include "RAPI/RenderContext.h"
#include "Native/VKDevice.h"
#include "VulkanImage.h"
#include "VulkanUniformBuffer.h"
#include "RAPI/RenderDevice.h"
#include "VulkanRenderer.h"
#include "VulkanTexture.h"

VulkanUniformLayout::VulkanUniformLayout(VKDevice* _Device, const UniformLayoutCreateInfo& _PLCI)
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
			DescriptorBindings[i].pImmutableSamplers = nullptr;
		}
	}

	DescriptorBindings.setLength(_PLCI.PipelineUniformSets.getLength());

	DescriptorSetLayoutCreateInfo.bindingCount = DescriptorBindings.getLength();
	DescriptorSetLayoutCreateInfo.pBindings = DescriptorBindings.getData();

	vkCreateDescriptorSetLayout(_Device->GetVkDevice(), &DescriptorSetLayoutCreateInfo, ALLOCATOR, &descriptorSetLayout);

	Array<VkDescriptorPoolSize, MAX_DESCRIPTORS_PER_SET> PoolSizes;
	{
		for (uint8 i = 0; i < _PLCI.PipelineUniformSets.getLength(); ++i)
		{
			PoolSizes[i].descriptorCount = _PLCI.DescriptorCount;
			PoolSizes[i].type = UniformTypeToVkDescriptorType(_PLCI.PipelineUniformSets[i].UniformSetType);
		}
	}

	PoolSizes.setLength(_PLCI.PipelineUniformSets.getLength());

	VkDescriptorPoolCreateInfo DescriptorPoolCreateInfo = { VK_STRUCTURE_TYPE_DESCRIPTOR_POOL_CREATE_INFO };
	DescriptorPoolCreateInfo.maxSets = _PLCI.RenderContext->GetMaxFramesInFlight();
	DescriptorPoolCreateInfo.poolSizeCount = PoolSizes.getLength();
	DescriptorPoolCreateInfo.pPoolSizes = PoolSizes.getData();

	vkCreateDescriptorPool(_Device->GetVkDevice(), &DescriptorPoolCreateInfo, ALLOCATOR, &descriptorPool);

	VkDescriptorSetAllocateInfo DescriptorSetAllocateInfo = { VK_STRUCTURE_TYPE_DESCRIPTOR_SET_ALLOCATE_INFO };
	DescriptorSetAllocateInfo.descriptorPool = descriptorPool;
	DescriptorSetAllocateInfo.descriptorSetCount = _PLCI.RenderContext->GetMaxFramesInFlight();

	FVector<VkDescriptorSetLayout> SetLayouts(_PLCI.RenderContext->GetMaxFramesInFlight(), descriptorSetLayout);

	DescriptorSetAllocateInfo.pSetLayouts = SetLayouts.getData();

	descriptorSets.resize(DescriptorSetAllocateInfo.descriptorSetCount);
	
	vkAllocateDescriptorSets(_Device->GetVkDevice(), &DescriptorSetAllocateInfo, descriptorSets.getData());

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

	VkDescriptorSetLayout pDescriptorSetLayouts = descriptorSetLayout;
	PipelineLayoutCreateInfo.setLayoutCount = 1;
	PipelineLayoutCreateInfo.pSetLayouts = &pDescriptorSetLayouts;

	vkCreatePipelineLayout(_Device->GetVkDevice(), &PipelineLayoutCreateInfo, ALLOCATOR, &pipelineLayout);
}

void VulkanUniformLayout::UpdateUniformSet(const UniformLayoutUpdateInfo& _ULUI)
{
	DArray<VkWriteDescriptorSet> write_descriptors(_ULUI.PipelineUniformSets.getLength());
	
	for (uint8 i = 0; i < _ULUI.PipelineUniformSets.getLength(); ++i)
	{
		switch (_ULUI.PipelineUniformSets[i].UniformSetType)
		{
		case UniformType::SAMPLER:
		case UniformType::COMBINED_IMAGE_SAMPLER:
		case UniformType::SAMPLED_IMAGE:

			VkDescriptorImageInfo DescriptorImageInfo;
			DescriptorImageInfo.imageView = SCAST(VulkanTexture*, _ULUI.PipelineUniformSets[i].UniformData)->GetImageView();
			DescriptorImageInfo.imageLayout = ImageLayoutToVkImageLayout(SCAST(VulkanTexture*, _ULUI.PipelineUniformSets[i].UniformData)->GetImageLayout());
			DescriptorImageInfo.sampler = static_cast<VulkanTexture*>(_ULUI.PipelineUniformSets[i].UniformData)->GetImageSampler();
			
			write_descriptors[i].sType = VK_STRUCTURE_TYPE_WRITE_DESCRIPTOR_SET;
			write_descriptors[i].pNext = nullptr;
			write_descriptors[i].dstSet = descriptorSets[0];
			write_descriptors[i].dstBinding = i;
			write_descriptors[i].dstArrayElement = 0;
			write_descriptors[i].descriptorCount = _ULUI.PipelineUniformSets[i].UniformSetUniformsCount;
			write_descriptors[i].descriptorType = UniformTypeToVkDescriptorType(_ULUI.PipelineUniformSets[i].UniformSetType);
			write_descriptors[i].pImageInfo = &DescriptorImageInfo;
			write_descriptors[i].pTexelBufferView = nullptr;
			write_descriptors[i].pBufferInfo = nullptr;

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

			write_descriptors[i].sType = VK_STRUCTURE_TYPE_WRITE_DESCRIPTOR_SET;
			write_descriptors[i].pNext = nullptr;
			write_descriptors[i].dstSet = descriptorSets[0];
			write_descriptors[i].dstBinding = i;
			write_descriptors[i].dstArrayElement = 0;
			write_descriptors[i].descriptorCount = _ULUI.PipelineUniformSets[i].UniformSetUniformsCount;
			write_descriptors[i].descriptorType = UniformTypeToVkDescriptorType(_ULUI.PipelineUniformSets[i].UniformSetType);
			write_descriptors[i].pImageInfo = nullptr;
			write_descriptors[i].pTexelBufferView = nullptr;
			write_descriptors[i].pBufferInfo = &DescriptorBufferInfo;

			break;

			//case UniformType::UNIFORM_BUFFER_DYNAMIC: break;
			//case UniformType::STORAGE_BUFFER_DYNAMIC: break;
			//case UniformType::INPUT_ATTACHMENT: break;
		default:;
		}
	}

	vkUpdateDescriptorSets(SCAST(VulkanRenderDevice*, RenderDevice::Get())->GetVKDevice().GetVkDevice(), write_descriptors.getCapacity(), write_descriptors.getData(), 0, nullptr);
}