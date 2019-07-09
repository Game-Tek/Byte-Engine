#pragma once

#include "Core.h"

#include "VulkanBase.h"

MAKE_VK_HANDLE(VkCommandPool)
MAKE_VK_HANDLE(VkCommandBuffer)

GS_CLASS Vk_CommandBuffer final : public VulkanObject
{
	VkCommandBuffer CommandBuffer = nullptr;
public:
	Vk_CommandBuffer(VkDevice _Device, VkCommandPool _CP);
	~Vk_CommandBuffer();

	INLINE VkCommandBuffer GetVkCommandBuffer() const { return CommandBuffer; }
};

GS_CLASS Vk_CommandPool final : public VulkanObject
{
	VkCommandPool CommandPool = nullptr;
public:
	Vk_CommandPool(VkDevice _Device, uint32 _QueueIndex);
	~Vk_CommandPool();

	INLINE VkCommandPool GetVkCommandPool() const { return CommandPool; }
};
