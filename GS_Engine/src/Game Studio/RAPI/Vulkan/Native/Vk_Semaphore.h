#pragma once

#include "Core.h"

#include "RAPI/Vulkan/VulkanBase.h"

MAKE_VK_HANDLE(VkSemaphore)

GS_CLASS Vk_Semaphore final : public VulkanObject
{
	VkSemaphore Semaphore = nullptr;

public:
	Vk_Semaphore(const Vk_Device& _Device);
	~Vk_Semaphore();

	INLINE operator VkSemaphore() const { return Semaphore; }
};