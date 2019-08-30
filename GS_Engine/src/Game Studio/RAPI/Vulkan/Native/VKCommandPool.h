#pragma once

#include "Core.h"

#include "RAPI/Vulkan/VulkanBase.h"

MAKE_VK_HANDLE(VkCommandPool)

struct VkCommandPoolCreateInfo;

GS_STRUCT VKCommandPoolCreator final : public VKObject<VkCommandPool>
{
	VKCommandPoolCreator(const VKDevice & _Device, const VkCommandPoolCreateInfo* _VkCPCI);
};

struct VKCommandBufferCreator;

GS_CLASS VKCommandPool final : public VKObject<VkCommandPool>
{
public:
	VKCommandPool(const VKCommandPoolCreator& _VKCPC) : VKObject<VkCommandPool>(_VKCPC)
	{
	}

	~VKCommandPool();

	VKCommandBufferCreator CreateCommandBuffer() const;

	void Reset() const;
};