#pragma once

#include "Core.h"

#include "RAPI/Fence.h"
#include "RAPI/Semaphore.h"
#include "VulkanBase.h"

MAKE_VK_HANDLE(VkFence)
MAKE_VK_HANDLE(VkSemaphore)

GS_CLASS VulkanFence final : public Fence, public VulkanObject
{
public:
	VkFence Fence = nullptr;

	VulkanFence(VkDevice _Device, bool _StateInitialized);
	~VulkanFence();
};

GS_CLASS VulkanSemaphore final : public Semaphore, public VulkanObject
{
	VkSemaphore Semaphore = nullptr;
public:
	VulkanSemaphore(VkDevice _Device);
	~VulkanSemaphore();

	INLINE VkSemaphore GetVkSemaphore() const { return Semaphore; }
};