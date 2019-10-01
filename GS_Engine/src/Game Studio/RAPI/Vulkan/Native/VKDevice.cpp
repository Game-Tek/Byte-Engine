#include "VKDevice.h"

#include "RAPI/Vulkan/Vulkan.h"
#include "vkPhysicalDevice.h"
#include <vector>

struct QueueInfo
{
	VkQueueFlagBits QueueFlag = {};
	float QueuePriority = 1.0f;
};

VkPhysicalDeviceMemoryProperties MemoryProperties = {};

VKDevice::VKDevice(const VKInstance& _Instance, const vkPhysicalDevice& _PD)
{
	VkPhysicalDeviceFeatures deviceFeatures = {};	//COME BACK TO

	const char* DeviceExtensions[] = { VK_KHR_SWAPCHAIN_EXTENSION_NAME };

	QueueInfo GraphicsQueueInfo;
	QueueInfo ComputeQueueInfo;
	QueueInfo TransferQueueInfo;

	GraphicsQueueInfo.QueueFlag = VK_QUEUE_GRAPHICS_BIT;
	GraphicsQueueInfo.QueuePriority = 1.0f;
	ComputeQueueInfo.QueueFlag = VK_QUEUE_COMPUTE_BIT;
	ComputeQueueInfo.QueuePriority = 1.0f;
	TransferQueueInfo.QueueFlag = VK_QUEUE_TRANSFER_BIT;
	TransferQueueInfo.QueuePriority = 1.0f;

	QueueInfo QueueInfos[] = { GraphicsQueueInfo, ComputeQueueInfo, TransferQueueInfo };

	FVector<VkDeviceQueueCreateInfo> QueueCreateInfos = CreateQueueInfos(QueueInfos, 3, _PD);

	VkDeviceCreateInfo DeviceCreateInfo = { VK_STRUCTURE_TYPE_DEVICE_CREATE_INFO };
	DeviceCreateInfo.queueCreateInfoCount = QueueCreateInfos.length();
	DeviceCreateInfo.pQueueCreateInfos = QueueCreateInfos.data();
	DeviceCreateInfo.enabledExtensionCount = 1;
	DeviceCreateInfo.pEnabledFeatures = &deviceFeatures;
	DeviceCreateInfo.ppEnabledExtensionNames = DeviceExtensions;

	GS_VK_CHECK(vkCreateDevice(_PD, &DeviceCreateInfo, ALLOCATOR, &Device), "Failed to create Device!");

	vkQueue* Queues[] = { &GraphicsQueue, &ComputeQueue, &TransferQueue };

	SetVk_Queues(Queues, QueueCreateInfos);

	vkGetPhysicalDeviceMemoryProperties(_PD, &MemoryProperties);
}

VKDevice::~VKDevice()
{
	vkDeviceWaitIdle(Device);
	vkDestroyDevice(Device, ALLOCATOR);
}

void VKDevice::SetVk_Queues(vkQueue* _Queue[], const FVector<VkDeviceQueueCreateInfo>& _QCI)
{
	for (uint8 i = 0; i < _QCI.length(); ++i)
	{
		for (uint8 j = 0; j < _QCI[i].queueCount; ++j)
		{
			vkGetDeviceQueue(Device, _QCI[i].queueFamilyIndex, j, &_Queue[i + j]->GetVkQueue());
		}
	}
}

FVector<VkDeviceQueueCreateInfo> VKDevice::CreateQueueInfos(QueueInfo* _QI, uint8 _QueueCount, const vkPhysicalDevice& _PD)
{
	uint32_t QueueFamiliesCount = 0;
	vkGetPhysicalDeviceQueueFamilyProperties(_PD, &QueueFamiliesCount, VK_NULL_HANDLE);	//Get the amount of queue families there are in the physical device.
	FVector<VkQueueFamilyProperties> QueueFamilies(QueueFamiliesCount);
	vkGetPhysicalDeviceQueueFamilyProperties(_PD, &QueueFamiliesCount, QueueFamilies.data());


	FVector<VkDeviceQueueCreateInfo> QueueCreateInfos(QueueFamiliesCount);
	FVector<bool> UsedFamilies(QueueFamiliesCount, false);

	for (uint8 q = 0; q < _QueueCount; ++q)
	{
		for (uint8 f = 0; f < QueueFamiliesCount; ++f)
		{
			if (QueueFamilies[f].queueCount > 0 && QueueFamilies[f].queueFlags & _QI[f].QueueFlag)
			{
				if (UsedFamilies[f])
				{
					QueueCreateInfos[f].queueCount++;
					break;
				}

				QueueCreateInfos.push_back(VkDeviceQueueCreateInfo());
				QueueCreateInfos[f].queueCount = 1;
				QueueCreateInfos[f].queueFamilyIndex = f;
				//QueueCreateInfos[f].pQueuePriorities = &_QI[f].QueuePriority;
				UsedFamilies[f] = true;
				break;
			}
		}
		QueueCreateInfos[q].pQueuePriorities = &_QI[q].QueuePriority;
	}

	for (auto& QueueCreateInfo : QueueCreateInfos)
	{
		QueueCreateInfo.sType = VK_STRUCTURE_TYPE_DEVICE_QUEUE_CREATE_INFO;
	}

	return QueueCreateInfos;
}

uint32 VKDevice::FindMemoryType(uint32 _TypeFilter, uint32 _Properties) const
{
	for (uint32 i = 0; i < MemoryProperties.memoryTypeCount; i++)
	{
		if ((_TypeFilter & (1 << i)) && (MemoryProperties.memoryTypes[i].propertyFlags & _Properties) == _Properties)
		{
			return i;
		}
	}

	return 0;
}