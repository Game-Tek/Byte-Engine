#pragma once

#include "Core.h"

#include "RAPI/Vulkan/VulkanBase.h"

MAKE_VK_HANDLE(VkDescriptorSetLayout)

struct VkDescriptorSetLayoutCreateInfo;

GS_STRUCT VKDescriptorSetLayoutCreator : VKObjectCreator<VkDescriptorSetLayout>
{
	VKDescriptorSetLayoutCreator(VKDevice* _Device, const VkDescriptorSetLayoutCreateInfo * _VkDSLCI);
};

GS_CLASS VKDescriptorSetLayout final : public VKObject<VkDescriptorSetLayout>
{
public:
	VKDescriptorSetLayout(const VKDescriptorSetLayoutCreator & _VKDSLC) : VKObject(_VKDSLC)
	{
	}

	~VKDescriptorSetLayout();
};