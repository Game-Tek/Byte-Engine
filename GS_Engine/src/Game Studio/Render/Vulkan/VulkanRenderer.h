#pragma once

#include "Core.h"

#include "..\Renderer.h"

#include "VulkanBase.h"

GS_CLASS VulkanRenderer final : public Renderer
{
	Vulkan_Instance Instance;
	Vulkan_Device Device;
public:
	VulkanRenderer();
	~VulkanRenderer();

	
};

MAKE_VK_HANDLE(VkInstance)

GS_CLASS Vulkan_Instance
{
	VkInstance Instance = nullptr;
public:
	Vulkan_Instance(const char* _AppName, const FVector<const char*> & _Extensions);
	~Vulkan_Instance();

	INLINE VkInstance GetVkInstance() const { return Instance; }
};

MAKE_VK_HANDLE(VkQueue)
struct VkDeviceQueueCreateInfo;
struct QueueInfo;

GS_CLASS Vulkan_Device
{
	VkDevice Device = nullptr;
	FVector<VkQueue> Queues;
	VkPhysicalDevice PhysicalDevice = nullptr;

	static void CreateQueueInfo(QueueInfo& _DQCI, VkPhysicalDevice _PD);
	static void CreatePhysicalDevice(VkPhysicalDevice& _PD, VkInstance _Instance);
	static uint8 GetDeviceTypeScore(VkPhysicalDeviceType _Type);
public:
	Vulkan_Device(VkInstance _Instance);
	~Vulkan_Device();

	INLINE VkDevice GetVkDevice() const { return Device; }
	INLINE VkPhysicalDevice GetVkPhysicalDevice() const { return PhysicalDevice; }
};