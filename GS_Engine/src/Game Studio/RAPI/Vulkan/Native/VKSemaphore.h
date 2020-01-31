#pragma once

#include "Core.h"

#include "RAPI/Vulkan/VulkanBase.h"

MAKE_VK_HANDLE(VkSemaphore)

struct VkSemaphoreCreateInfo;

struct VKSemaphoreCreator final : VKObjectCreator<VkSemaphore>
{
	VKSemaphoreCreator(VKDevice* _Device, const VkSemaphoreCreateInfo* _VkSCI);
};

class VKSemaphore final : public VKObject<VkSemaphore>
{
public:
	VKSemaphore(const VKSemaphoreCreator& _VKSC) : VKObject<VkSemaphore>(_VKSC)
	{
	}

	~VKSemaphore();
};
