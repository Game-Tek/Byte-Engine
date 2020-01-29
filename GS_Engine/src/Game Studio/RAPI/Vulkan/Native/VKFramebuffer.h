#pragma once

#include "Core.h"

#include "RAPI/Vulkan/VulkanBase.h"

MAKE_VK_HANDLE(VkFramebuffer)

struct VkFramebufferCreateInfo;

struct GS_API VKFramebufferCreator final : VKObjectCreator<VkFramebuffer>
{
	VKFramebufferCreator(VKDevice* _Device, const VkFramebufferCreateInfo* _VkFCI);
};

class GS_API VKFramebuffer final : public VKObject<VkFramebuffer>
{
public:
	VKFramebuffer(const VKFramebufferCreator& _VKFC) : VKObject<VkFramebuffer>(_VKFC)
	{
	}

	~VKFramebuffer();
};
