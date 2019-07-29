#pragma once

#include "RAPI/Vulkan/VulkanBase.h"

MAKE_VK_HANDLE(VkCommandPool)

class Vk_Queue;

enum VkCommandPoolCreateFlagBits;

GS_CLASS Vk_CommandPool final : public VulkanObject
{
	VkCommandPool CommandPool = nullptr;

public:
	Vk_CommandPool(const Vk_Device& _Device, const Vk_Queue& _Queue, VkCommandPoolCreateFlagBits _CPF);
	~Vk_CommandPool();

	INLINE VkCommandPool GetVkCommandPool() const { return CommandPool; }

	INLINE operator VkCommandPool() const {	return CommandPool;	}
};