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
	~VulkanCommandBuffer() = default;

	INLINE VkCommandBuffer GetVkCommandBuffer() const { return CommandBuffer; }

	void BeginRecording() final override;
	void EndRecording() final override;
};

GS_CLASS Vulkan_Command_Pool final : public VulkanObject
{
	VkCommandPool CommandPool = nullptr;
public:
	Vulkan_Command_Pool(VkDevice _Device);
	~Vulkan_Command_Pool();

	INLINE VkCommandPool GetVkCommandPool() const { return CommandPool; }
};