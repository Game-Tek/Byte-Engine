#pragma once

#include "VulkanBase.h"

#include "..\Buffer.h"

class Vk_Queue;
enum VkBufferUsageFlagBits;

MAKE_VK_HANDLE(VkPhysicalDevice)
MAKE_VK_HANDLE(VkBuffer)
MAKE_VK_HANDLE(VkDeviceMemory)

class Vulkan_Device;

GS_CLASS Vk_Buffer final : public VulkanObject
{
	VkBuffer Buffer = nullptr;
	VkDeviceMemory Memory = nullptr;
public:
	Vk_Buffer(VkDevice _Device, void* _Data, size_t _BufferSize, VkBufferUsageFlagBits _BufferFlag, const Vk_Queue& _Queue, VkCommandPool _CP, const Vulkan_Device& _VD);
	~Vk_Buffer();
};

GS_CLASS VulkanBuffer final : public Buffer
{
	Vk_Buffer Buffer;
public:
	VulkanBuffer(VkDevice _Device, void* _Data, size_t _BufferSize, BufferType _BufferType, const Vk_Queue& _Queue, VkCommandPool _CP, const Vulkan_Device& _VD);
	~VulkanBuffer();
};