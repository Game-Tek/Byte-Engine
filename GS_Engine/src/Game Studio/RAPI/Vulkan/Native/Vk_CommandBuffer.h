#pragma once

#include "Core.h"

#include "RAPI/Vulkan/VulkanBase.h"

MAKE_VK_HANDLE(VkCommandBuffer)
MAKE_VK_HANDLE(VkQueue)
MAKE_VK_HANDLE(VkFence)

class Vk_CommandPool;
struct VkCommandBufferBeginInfo;

GS_CLASS Vk_CommandBuffer final : public VulkanObject
{
	VkCommandBuffer CommandBuffer = nullptr;
public:
	Vk_CommandBuffer(VkDevice _Device, const Vk_CommandPool& _CP);
	~Vk_CommandBuffer() = default;

	void Free(VkCommandPool _CP);
	void Begin(VkCommandBufferBeginInfo* _CBBI);
	void End();

	INLINE VkCommandBuffer GetVkCommandBuffer() const { return CommandBuffer; }

	INLINE operator VkCommandBuffer() const	{ return CommandBuffer;	}

	INLINE operator const VkCommandBuffer*() const
	{
		return &CommandBuffer;
	}
};