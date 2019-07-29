#pragma once

#include "Core.h"

#include "RAPI/Vulkan/VulkanBase.h"

MAKE_VK_HANDLE(VkPhysicalDevice)

class Vk_Instance;

enum VkPhysicalDeviceType;

GS_CLASS Vk_PhysicalDevice final : public VulkanObject
{
	VkPhysicalDevice PhysicalDevice = nullptr;

	static uint8 GetDeviceTypeScore(VkPhysicalDeviceType _PDT);
public:
	Vk_PhysicalDevice(const Vk_Device & _Device, const Vk_Instance& _Instance);
	~Vk_PhysicalDevice() = default;
};