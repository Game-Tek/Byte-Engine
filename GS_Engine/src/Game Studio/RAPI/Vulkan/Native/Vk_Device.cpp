#include "Vk_Device.h"

#include "RAPI/Vulkan/Vulkan.h"

struct QueueInfo
{
	VkQueueFlagBits QueueFlag = {};
	float QueuePriority = 1.0f;
};

VkPhysicalDeviceMemoryProperties MemoryProperties = {};

Vk_Device::Vk_Device(VkInstance _Instance)
{
	////////////////////////////////////////
	//      DEVICE CREATION/SELECTION     //
	////////////////////////////////////////

	VkPhysicalDeviceFeatures deviceFeatures = {};	//COME BACK TO

	const char* DeviceExtensions[] = { VK_KHR_SWAPCHAIN_EXTENSION_NAME };

	PhysicalDevice = CreatePhysicalDevice(_Instance);

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

	FVector<VkDeviceQueueCreateInfo> QueueCreateInfos = CreateQueueInfos(QueueInfos, 3, PhysicalDevice);

	VkDeviceCreateInfo DeviceCreateInfo = { VK_STRUCTURE_TYPE_DEVICE_CREATE_INFO };
	DeviceCreateInfo.queueCreateInfoCount = QueueCreateInfos.length();
	DeviceCreateInfo.pQueueCreateInfos = QueueCreateInfos.data();
	DeviceCreateInfo.enabledExtensionCount = 1;
	DeviceCreateInfo.pEnabledFeatures = &deviceFeatures;
	DeviceCreateInfo.ppEnabledExtensionNames = DeviceExtensions;

	auto ff = vkCreateDevice(PhysicalDevice, &DeviceCreateInfo, ALLOCATOR, &Device);

	Vk_Queue* Queues[] = { &GraphicsQueue, &ComputeQueue, &TransferQueue };

	SetVk_Queues(Queues, QueueCreateInfos);

	vkGetPhysicalDeviceMemoryProperties(PhysicalDevice, &MemoryProperties);
}

Vk_Device::~Vk_Device()
{
	vkDeviceWaitIdle(Device);
	vkDestroyDevice(Device, ALLOCATOR);
}

void Vk_Device::SetVk_Queues(Vk_Queue* _Queue[], const FVector<VkDeviceQueueCreateInfo>& _QCI)
{
	for (uint8 i = 0; i < _QCI.length(); ++i)
	{
		for (uint8 j = 0; j < _QCI[i].queueCount; ++j)
		{
			vkGetDeviceQueue(Device, _QCI[i].queueFamilyIndex, j, &_Queue[i + j]->GetVkQueue());
		}
	}
}

FVector<VkDeviceQueueCreateInfo> Vk_Device::CreateQueueInfos(QueueInfo * _QI, uint8 _QueueCount, VkPhysicalDevice _PD)
{
	uint32_t QueueFamiliesCount = 0;
	vkGetPhysicalDeviceQueueFamilyProperties(_PD, &QueueFamiliesCount, VK_NULL_HANDLE);	//Get the amount of queue families there are in the physical device.
	FVector<VkQueueFamilyProperties> QueueFamilies(QueueFamiliesCount);
	vkGetPhysicalDeviceQueueFamilyProperties(_PD, &QueueFamiliesCount, QueueFamilies.data());


	FVector<VkDeviceQueueCreateInfo> QueueCreateInfos(QueueFamiliesCount);
	FVector<bool> UsedFamilies(QueueFamiliesCount);
	for (uint8 i = 0; i < QueueFamiliesCount; ++i)
	{
		UsedFamilies[i] = false;
	}

	for (uint8 q = 0; q < _QueueCount; ++q)	//For each queue
	{
		for (uint8 f = 0; f < QueueFamiliesCount; ++f)
		{
			if (QueueFamilies[f].queueCount > 0 && QueueFamilies[f].queueFlags & _QI[q].QueueFlag)
			{
				if (UsedFamilies[f] == true)
				{
					QueueCreateInfos[f].queueCount++;
					break;
				}

				QueueCreateInfos.push_back(VkDeviceQueueCreateInfo());
				QueueCreateInfos[f].queueCount = 1;
				QueueCreateInfos[f].queueFamilyIndex = f;
				UsedFamilies[f] = true;
				break;
			}
		}

		QueueCreateInfos[q].pQueuePriorities = &_QI[q].QueuePriority;
	}

	for (uint8 i = 0; i < QueueCreateInfos.length(); ++i)
	{
		QueueCreateInfos[i].sType = VK_STRUCTURE_TYPE_DEVICE_QUEUE_CREATE_INFO;
	}

	return QueueCreateInfos;
}

VkPhysicalDevice Vk_Device::CreatePhysicalDevice(VkInstance _Instance)
{

}

uint8 Vk_Device::GetDeviceTypeScore(VkPhysicalDeviceType _Type)
{

}

uint32 Vk_Device::FindMemoryType(uint32 _TypeFilter, uint32 _Properties) const
{
	for (uint32 i = 0; i < MemoryProperties.memoryTypeCount; i++)
	{
		if ((_TypeFilter & (1 << i)) && (MemoryProperties.memoryTypes[i].propertyFlags & _Properties) == _Properties)
		{
			return i;
		}
	}
}