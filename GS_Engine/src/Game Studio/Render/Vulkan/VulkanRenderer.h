#pragma once

#include "Core.h"

#include "..\Renderer.h"

#include "VulkanBase.h"

GS_CLASS VulkanRenderer final : public Renderer
{
public:
	VulkanRenderer();
	~VulkanRenderer();

	
};

MAKE_VK_HANDLE(VkInstance)

GS_CLASS Vulkan_Instance
{
	VkInstance Instance = nullptr;
public:
	Vulkan_Instance(const FVector<const char*> & _Extensions);
	~Vulkan_Instance();

	INLINE VkInstance GetVkInstance() const { return Instance; }
};

MAKE_VK_HANDLE(VkQueue)
struct VkDeviceQueueCreateInfo;
struct QueueInfo;

GS_CLASS Vulkan_Device final : public VulkanObject
{
	Vulkan__Physical__Device PhysicalDevice;

	static void CreateQueueInfo(QueueInfo& _DQCI, VkPhysicalDevice _PD);

public:
	Vulkan_Device(VkInstance _Instance);

	~Vulkan_Device();
};

GS_STRUCT Vulkan__Physical__Device
{
	VkPhysicalDevice PhysicalDevice = nullptr;

	static uint8 GetDeviceTypeScore(VkPhysicalDeviceType _Type);
public:
	Vulkan__Physical__Device(VkInstance _Instance);

	INLINE VkPhysicalDevice GetVkPhysicalDevice() const { return PhysicalDevice; }
};