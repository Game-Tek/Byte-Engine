#pragma once

#include "Core.h"

#include "RAPI/Vulkan/VulkanBase.h"

MAKE_VK_HANDLE(VkDescriptorPool)

struct VkDescriptorPoolCreateInfo;

GS_STRUCT VKDescriptorPoolCreator final : VKObjectCreator<VkDescriptorPool>
{
	VKDescriptorPoolCreator(VKDevice* _Device, const VkDescriptorPoolCreateInfo* _VkDPCI);
};

GS_CLASS VKDescriptorPool final : VKObject<VkDescriptorPool>
{
public:
	VKDescriptorPool(const VKDescriptorPoolCreator& _VKDPC) : VKObject<VkDescriptorPool>(_VKDPC)
	{
	}

	~VKDescriptorPool();
};