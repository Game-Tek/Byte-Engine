#include "Vulkan.h"
#include "VulkanRenderer.h"

struct QueueInfo
{
	VkDeviceQueueCreateInfo DeviceQueueCreateInfo;
	VkQueueFlagBits QueueFlagBits;
	float QueuePriority;
};

VulkanRenderer::VulkanRenderer()
{
}


VulkanRenderer::~VulkanRenderer()
{
}

Vulkan_Instance::Vulkan_Instance(const FVector<const char*>& _Extensions)
{
	VkApplicationInfo AppInfo = { VK_STRUCTURE_TYPE_APPLICATION_INFO };
	AppInfo.apiVersion = VK_API_VERSION_1_1;	//Should check if version is available vi vkEnumerateInstanceVersion().
	AppInfo.applicationVersion = VK_MAKE_VERSION(1, 0, 0);
	AppInfo.engineVersion = VK_MAKE_VERSION(1, 0, 0);
	AppInfo.pApplicationName = "Hello Triangle!";
	AppInfo.pEngineName = "Game Studio";
	AppInfo.pNext = nullptr;

	const char* InstanceLayers[] = { "VK_LAYER_LUNARG_standard_validation" };

	VkInstanceCreateInfo InstanceCreateInfo = { VK_STRUCTURE_TYPE_INSTANCE_CREATE_INFO };
	InstanceCreateInfo.pApplicationInfo = &AppInfo;
	InstanceCreateInfo.enabledLayerCount = 1;
	InstanceCreateInfo.ppEnabledLayerNames = InstanceLayers;
	InstanceCreateInfo.enabledExtensionCount = _Extensions.length();
	InstanceCreateInfo.ppEnabledExtensionNames = _Extensions.data();

	GS_VK_CHECK(vkCreateInstance(&InstanceCreateInfo, ALLOCATOR, &Instance), "Failed to create Instance!")
}

Vulkan_Instance::~Vulkan_Instance()
{
	vkDestroyInstance(Instance, ALLOCATOR);
}

Vulkan_Device::Vulkan_Device(VkInstance _Instance) : PhysicalDevice(_Instance)
{
	////////////////////////////////////////
	//      DEVICE CREATION/SELECTION     //
	////////////////////////////////////////

	VkPhysicalDeviceFeatures deviceFeatures = {};	//COME BACK TO

	const char* DeviceExtensions[] = { VK_KHR_SWAPCHAIN_EXTENSION_NAME };

	FVector<QueueInfo> QueueInfos(1);

	QueueInfos[1].QueueFlagBits = VK_QUEUE_GRAPHICS_BIT;
	QueueInfos[1].QueuePriority = 1.0f;

	for (uint8 i = 0; i < QueueInfos.length(); i++)
	{
		CreateQueueInfo(QueueInfos[i], PhysicalDevice.GetVkPhysicalDevice());
	}

	VkDeviceCreateInfo CreateInfo = { VK_STRUCTURE_TYPE_DEVICE_CREATE_INFO };
	CreateInfo.pQueueCreateInfos = &QueueInfos.data()->DeviceQueueCreateInfo;
	CreateInfo.queueCreateInfoCount = QueueInfos.length();
	CreateInfo.pEnabledFeatures = &deviceFeatures;
	CreateInfo.enabledExtensionCount = 1;
	CreateInfo.ppEnabledExtensionNames = DeviceExtensions;

	GS_VK_CHECK(vkCreateDevice(PhysicalDevice, &CreateInfo, ALLOCATOR, &m_Device), "Failed to create logical device!")
}

Vulkan_Device::~Vulkan_Device()
{
	vkDestroyDevice(m_Device, ALLOCATOR);
}

void Vulkan_Device::CreateQueueInfo(QueueInfo& _QI, VkPhysicalDevice _PD)
{
	_QI.DeviceQueueCreateInfo.sType = VK_STRUCTURE_TYPE_DEVICE_QUEUE_CREATE_INFO;

	uint32_t QueueFamiliesCount = 0;
	vkGetPhysicalDeviceQueueFamilyProperties(_PD, &QueueFamiliesCount, nullptr);	//Get the amount of queue families there are in the physical device.
	FVector<VkQueueFamilyProperties> queueFamilies(QueueFamiliesCount);
	vkGetPhysicalDeviceQueueFamilyProperties(_PD, &QueueFamiliesCount, queueFamilies.data());

	uint8 i = 0;
	while (true)
	{
		if (queueFamilies[i].queueCount > 0 && queueFamilies[i].queueFlags & _QI.QueueFlagBits)
		{
			break;
		}

		i++;
	}

	_QI.DeviceQueueCreateInfo.queueFamilyIndex = i;
	_QI.DeviceQueueCreateInfo.queueCount = 1;
	_QI.DeviceQueueCreateInfo.pQueuePriorities = &_QI.QueuePriority;
}

uint8 Vulkan__Physical__Device::GetDeviceTypeScore(VkPhysicalDeviceType _Type)
{
	switch (_Type)
	{
	case VK_PHYSICAL_DEVICE_TYPE_DISCRETE_GPU: return 255;
	case VK_PHYSICAL_DEVICE_TYPE_INTEGRATED_GPU: return 254;
	case VK_PHYSICAL_DEVICE_TYPE_CPU: return 253;
	default: return 0;
	}
}

Vulkan__Physical__Device::Vulkan__Physical__Device(VkInstance _Instance)
{
	////////////////////////////////////////
	// PHYSICAL DEVICE CREATION/SELECTION //
	////////////////////////////////////////

	uint32_t DeviceCount = 0;
	vkEnumeratePhysicalDevices(_Instance, &DeviceCount, nullptr);	//Get the amount of physical devices(GPUs) there are.

	FVector<VkPhysicalDevice> PhysicalDevices(DeviceCount);
	vkEnumeratePhysicalDevices(_Instance, &DeviceCount, PhysicalDevices.data());	//Fill the array with VkPhysicalDevice handles to every physical device (GPU) there is.

	FVector<VkPhysicalDeviceProperties> PhysicalDevicesProperties;	//Create a vector to store the physical device properties associated with every physical device we queried before.
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

			PhysicalDevicesProperties.erase(i);

			i--;
		}

		i++;
	}

	PhysicalDevice = PhysicalDevices[i];	//Set the VulkanDevice's physical device as the one which resulted a winner from the sort.
}

vulkanQueue::vulkanQueue(VkDevice _Device, VkPhysicalDevice _PD, VkQueueFlagBits _QueueType, float * _QP)
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
	QueueCreateInfo.pQueuePriorities = QP;
}

