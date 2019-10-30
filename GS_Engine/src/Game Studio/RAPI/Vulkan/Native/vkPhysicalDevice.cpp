#include "vkPhysicalDevice.h"

#include "RAPI/Vulkan/Vulkan.h"
#include "VKInstance.h"

#include "Containers/FVector.hpp"

vkPhysicalDevice::vkPhysicalDevice(const VKInstance& _Instance)
{
	uint32_t DeviceCount = 0;
	vkEnumeratePhysicalDevices(_Instance, &DeviceCount, VK_NULL_HANDLE);	//Get the amount of physical devices(GPUs) there are.
	FVector<VkPhysicalDevice> PhysicalDevices(DeviceCount);
	vkEnumeratePhysicalDevices(_Instance, &DeviceCount, PhysicalDevices.data());	//Fill the array with VkPhysicalDevice handles to every physical device (GPU) there is.

	FVector<VkPhysicalDeviceProperties> PhysicalDevicesProperties(DeviceCount);	//Create a vector to store the physical device properties associated with every physical device we queried before.
	//Loop through every physical device there is while getting/querying the physical device properties of each one and storing them in the vector.
	for (size_t i = 0; i < DeviceCount; i++)
	{
		vkGetPhysicalDeviceProperties(PhysicalDevices[i], &PhysicalDevicesProperties[i]);
	}

	uint16 BestPhysicalDeviceIndex = 0;	//Variable to hold the index into the physical devices properties vector of the current winner of the GPU sorting processes.
	uint16 i = 0;
	//Sort by Device Type.
	while (PhysicalDevicesProperties.length() > i)
	{
		if (GetDeviceTypeScore(PhysicalDevicesProperties[i].deviceType) > GetDeviceTypeScore(PhysicalDevicesProperties[BestPhysicalDeviceIndex].deviceType))
		{
			BestPhysicalDeviceIndex = i;

			PhysicalDevicesProperties.pop(i);

			i--;
		}

		i++;
	}

	PhysicalDevice = PhysicalDevices[i];
}

uint8 vkPhysicalDevice::GetDeviceTypeScore(VkPhysicalDeviceType _PDT)
{
	switch (_PDT)
	{
	case VK_PHYSICAL_DEVICE_TYPE_DISCRETE_GPU:		return 255;
	case VK_PHYSICAL_DEVICE_TYPE_INTEGRATED_GPU:	return 254;
	case VK_PHYSICAL_DEVICE_TYPE_CPU:				return 253;
	default:										return 0;
	}
}
