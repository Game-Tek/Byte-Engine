#pragma once

#include "Core.h"

#include "RAPI/Vulkan/VulkanBase.h"

MAKE_VK_HANDLE(VkDescriptorSetLayout)

struct VkDescriptorSetLayoutCreateInfo;

struct VKDescriptorSetLayoutCreator : VKObjectCreator<VkDescriptorSetLayout>
{
	VKDescriptorSetLayoutCreator(VKDevice* _Device, const VkDescriptorSetLayoutCreateInfo* _VkDSLCI);
};

class VKDescriptorSetLayout final : public VKObject<VkDescriptorSetLayout>
{
public:
	VKDescriptorSetLayout(const VKDescriptorSetLayoutCreator& _VKDSLC) : VKObject(_VKDSLC)
	{
	}

	~VKDescriptorSetLayout();
};
