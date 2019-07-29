#pragma once

#include "Core.h"

#include "RAPI/Vulkan/VulkanBase.h"

MAKE_VK_HANDLE(VkInstance)

GS_CLASS Vk_Instance
{
	VkInstance Instance = nullptr;

public:
	Vk_Instance(const char* _AppName);
	~Vk_Instance();

	INLINE VkInstance GetVkInstance() const { return Instance; }

	INLINE operator VkInstance() const { return Instance; }
};