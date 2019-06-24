#pragma once

#include "VulkanBase.h"

enum VkBufferUsageFlagBits;

MAKE_VK_HANDLE(VkPhysicalDevice)
MAKE_VK_HANDLE(VkBuffer)
MAKE_VK_HANDLE(VkDeviceMemory)

GS_CLASS VulkanBuffer : public VulkanObject
{
	VkBuffer Buffer = nullptr;
	VkDeviceMemory Memory = nullptr;
public:
	VulkanBuffer(VkDevice _Device, VkPhysicalDevice _PD, void* _Data, size_t _BufferSize, VkBufferUsageFlagBits _BufferFlag);
	~VulkanBuffer();

	static uint32 FindMemoryType(VkPhysicalDevice _PD, uint32 _TypeFilter, VkMemoryPropertyFlags _Properties);
};

GS_CLASS VulkanVertexBuffer final : public VulkanBuffer
{
public:
	VulkanVertexBuffer(VkDevice _Device, VkPhysicalDevice _PD, void* _Data, size_t _BufferSize);
	~VulkanVertexBuffer() = delete;
};

GS_CLASS VulkanIndexBuffer final : public VulkanBuffer
{
public:
	VulkanIndexBuffer(VkDevice _Device, VkPhysicalDevice _PD, void* _Data, size_t _BufferSize);
	~VulkanIndexBuffer() = delete;
};