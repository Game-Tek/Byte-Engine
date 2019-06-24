#pragma once

#include "Core.h"

#include "..\Fence.h"
#include "..\Semaphore.h"
#include "VulkanBase.h"

MAKE_VK_HANDLE(VkFence)
MAKE_VK_HANDLE(VkSemaphore)

GS_CLASS VulkanFence final : public Fence, public VulkanObject
{
public:
	VkFence Fence;

	VulkanFence(VkDevice _Device);
	~VulkanFence();
};

GS_CLASS VulkanSemaphore final : public Semaphore, public VulkanObject
{
public:
	VkSemaphore Semaphore;

	VulkanSemaphore(VkDevice _Device);
	~VulkanSemaphore();
};