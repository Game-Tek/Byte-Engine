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

GS_CLASS Vulkan_Device final : public VulkanObject
{
	Vulkan__Physical__Device PhysicalDevice;
	Vulkan__Queue Queue;
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

MAKE_VK_HANDLE(VkQueue)

GS_STRUCT Vulkan__Queue
{
	VkQueue Queue = nullptr;
public:
	Vulkan__Queue() = default;

	Vulkan__Queue(VkDevice _Device, VkPhysicalDevice _PD, VkQueueFlagBits _QueueType)
	{
		VkDeviceQueueCreateInfo QueueCreateInfo = { VK_STRUCTURE_TYPE_DEVICE_QUEUE_CREATE_INFO };

		uint32_t QueueFamiliesCount = 0;
		vkGetPhysicalDeviceQueueFamilyProperties(_PD, &QueueFamiliesCount, nullptr);	//Get the amount of queue families there are in the physical device.

		FVector<VkQueueFamilyProperties> queueFamilies(QueueFamiliesCount);
		vkGetPhysicalDeviceQueueFamilyProperties(_PD, &QueueFamiliesCount, queueFamilies.data());

		uint8 i = 0;
		while (true)
		{
			if (queueFamilies[i].queueCount > 0 && queueFamilies[i].queueFlags & _QueueType)
			{
				break;
			}

			i++;
		}

		QueueCreateInfo.queueFamilyIndex = i;
		QueueCreateInfo.queueCount = 1;
		float queuePriority = 1.0f;
		QueueCreateInfo.pQueuePriorities = &queuePriority;

		vkGetDeviceQueue(_Device, QueueCreateInfo.queueFamilyIndex, 0, &Queue);
	}

	INLINE VkQueue GetVkQueue() const { return Queue; }
};