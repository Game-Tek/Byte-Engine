#pragma once

#include "Core.h"

#include "RAPI/Vulkan/VulkanBase.h"

MAKE_VK_HANDLE(VkInstance)

GS_CLASS VKInstance
{
	VkInstance Instance = nullptr;

public:
	VKInstance(const char* _AppName);
	~VKInstance();

	INLINE VkInstance GetVkInstance() const { return Instance; }

	INLINE operator VkInstance() const { return Instance; }
};