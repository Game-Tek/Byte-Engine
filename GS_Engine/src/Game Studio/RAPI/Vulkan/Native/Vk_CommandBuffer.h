#pragma once

#include "Core.h"

#include "RAPI/Vulkan/VulkanBase.h"

MAKE_VK_HANDLE(VkCommandBuffer)

class Vk_CommandPool;
struct VkCommandBufferBeginInfo;

GS_CLASS Vk_CommandBuffer final : public VulkanObject
{
	VkCommandBuffer CommandBuffer = nullptr;
public:
	Vk_CommandBuffer(const Vk_Device& _Device, const Vk_CommandPool& _CP);
	~Vk_CommandBuffer() = default;

	void Free(const Vk_CommandPool& _CP);
	void Begin(VkCommandBufferBeginInfo* _CBBI);
	void End();

	INLINE VkCommandBuffer GetVkCommandBuffer() const { return CommandBuffer; }

	INLINE operator VkCommandBuffer() const	{ return CommandBuffer;	}

	INLINE operator const VkCommandBuffer*() const
	{
		return &CommandBuffer;
	}
};