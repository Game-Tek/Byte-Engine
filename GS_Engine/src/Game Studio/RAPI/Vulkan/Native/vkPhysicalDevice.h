#pragma once

#include "Core.h"

#include "RAPI/Vulkan/VulkanBase.h"

MAKE_VK_HANDLE(VkPhysicalDevice)

class VKInstance;

enum VkPhysicalDeviceType;

class GS_API vkPhysicalDevice
{
	VkPhysicalDevice PhysicalDevice = nullptr;

	static uint8 GetDeviceTypeScore(VkPhysicalDeviceType _PDT);
public:
	vkPhysicalDevice(const VKInstance& _Instance);
	~vkPhysicalDevice() = default;

	INLINE operator VkPhysicalDevice() const { return PhysicalDevice; }
};
