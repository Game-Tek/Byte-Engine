#pragma once

#include "VulkanBase.h"

#include "..\Buffer.h"

enum VkBufferUsageFlagBits;

MAKE_VK_HANDLE(VkPhysicalDevice)
MAKE_VK_HANDLE(VkBuffer)
MAKE_VK_HANDLE(VkDeviceMemory)

class Vulkan_Device;

GS_CLASS VulkanBuffer : public Buffer, public VulkanObject
{
	VkBuffer Buffer = nullptr;
	VkDeviceMemory Memory = nullptr;
public:
	VulkanBuffer(VkDevice _Device, const Vulkan_Device& _VKD, void* _Data, size_t _BufferSize, VkBufferUsageFlagBits _BufferFlag);
	~VulkanBuffer();
};