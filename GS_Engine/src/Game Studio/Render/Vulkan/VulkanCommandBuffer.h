#pragma once

#include "..\CommandBuffer.h"
#include "VulkanBase.h"

MAKE_VK_HANDLE(VkCommandBuffer)
MAKE_VK_HANDLE(VkCommandPool)

GS_CLASS VulkanCommandBuffer final : public CommandBuffer, public VulkanObject
{
	VkCommandBuffer CommandBuffer = nullptr;
public:
	VulkanCommandBuffer(VkDevice _Device, VkCommandPool _CP);
	~VulkanCommandBuffer();

	INLINE VkCommandBuffer GetVkCommandBuffer() const { return CommandBuffer; }

	void BeginRecording() final override;
	void EndRecording() final override;
};