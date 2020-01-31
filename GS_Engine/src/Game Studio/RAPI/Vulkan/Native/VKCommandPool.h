#pragma once

#include "Core.h"

#include "RAPI/Vulkan/VulkanBase.h"

MAKE_VK_HANDLE(VkCommandPool)

struct VkCommandPoolCreateInfo;

struct VKCommandPoolCreator final : public VKObject<VkCommandPool>
{
	VKCommandPoolCreator(VKDevice* _Device, const VkCommandPoolCreateInfo* _VkCPCI);
};

struct VKCommandBufferCreator;

class VKCommandPool final : public VKObject<VkCommandPool>
{
public:
	VKCommandPool(const VKCommandPoolCreator& _VKCPC) : VKObject<VkCommandPool>(_VKCPC)
	{
	}

	~VKCommandPool();

	[[nodiscard]] VKCommandBufferCreator CreateCommandBuffer() const;

	void Reset() const;
};
