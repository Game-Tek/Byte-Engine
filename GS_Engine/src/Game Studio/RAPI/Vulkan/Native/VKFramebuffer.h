#pragma once

#include "Core.h"

#include "RAPI/Vulkan/VulkanBase.h"

MAKE_VK_HANDLE(VkFramebuffer)

struct VkFramebufferCreateInfo;

struct VKFramebufferCreator final : VKObjectCreator<VkFramebuffer>
{
	VKFramebufferCreator(VKDevice* _Device, const VkFramebufferCreateInfo* _VkFCI);
};

class VKFramebuffer final : public VKObject<VkFramebuffer>
{
public:
	VKFramebuffer(const VKFramebufferCreator& _VKFC) : VKObject<VkFramebuffer>(_VKFC)
	{
	}

	~VKFramebuffer();
};
