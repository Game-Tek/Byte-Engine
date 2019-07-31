#pragma once

#include "Core.h"

#include "RAPI/Vulkan/VulkanBase.h"

MAKE_VK_HANDLE(VkPhysicalDevice)

class Vk_Instance;

enum VkPhysicalDeviceType;

GS_CLASS Vk_PhysicalDevice
{
	VkPhysicalDevice PhysicalDevice = nullptr;

	static uint8 GetDeviceTypeScore(VkPhysicalDeviceType _PDT);
public:
	Vk_PhysicalDevice(const Vk_Instance& _Instance);
	~Vk_PhysicalDevice() = default;

	INLINE operator VkPhysicalDevice() const { return PhysicalDevice; }
};