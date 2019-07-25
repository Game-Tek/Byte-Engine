#pragma once

#include "Core.h"

#include "VulkanBase.h"

MAKE_VK_HANDLE(VkCommandPool)
MAKE_VK_HANDLE(VkCommandBuffer)
MAKE_VK_HANDLE(VkQueue)
MAKE_VK_HANDLE(VkFence)

struct VkCommandBufferBeginInfo;

GS_CLASS Vk_CommandBuffer final : public VulkanObject
{
	VkCommandBuffer CommandBuffer = nullptr;
public:
	Vk_CommandBuffer() : VulkanObject(nullptr)
	{
	}

	Vk_CommandBuffer(VkDevice _Device, VkCommandPool _CP);
	~Vk_CommandBuffer() = default;

	void Free(VkCommandPool _CP);
	void Begin(VkCommandBufferBeginInfo* _CBBI);
	void End();

	INLINE VkCommandBuffer GetVkCommandBuffer() const { return CommandBuffer; }

	INLINE operator VkCommandBuffer() const
	{
		return CommandBuffer;
	}

	INLINE operator const VkCommandBuffer*() const
	{
		return &CommandBuffer;
	}
};


enum VkCommandPoolCreateFlagBits;

GS_CLASS Vk_CommandPool final : public VulkanObject
{
	VkCommandPool CommandPool = nullptr;
public:
	Vk_CommandPool(VkDevice _Device, uint32 _QueueIndex, VkCommandPoolCreateFlagBits _CPF = SCAST(VkCommandPoolCreateFlagBits, 0));
	~Vk_CommandPool();

	INLINE VkCommandPool GetVkCommandPool() const { return CommandPool; }

	INLINE operator VkCommandPool() const
	{
		return CommandPool;
	}
};
