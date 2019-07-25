#include "Vulkan.h"
#include "VulkanRenderer.h"

#include "VulkanShader.h"
#include "VulkanRenderContext.h"
#include "VulkanBuffers.h"
#include "VulkanPipelines.h"
#include "VulkanRenderPass.h"

#include "RAPI/Window.h"

struct QueueInfo
{
	VkDeviceQueueCreateInfo DeviceQueueCreateInfo = { VK_STRUCTURE_TYPE_DEVICE_QUEUE_CREATE_INFO };
	VkQueueFlagBits QueueFlagBits = {};
	float QueuePriority = 1.0f;
};


//  VULKAN RENDERER

VulkanRenderer::VulkanRenderer() : Instance("Game Studio"), Device(Instance.GetVkInstance()),
                                   TransientCommandPool(Device, Device.GetTransferQueue().GetQueueIndex(),
                                                        VK_COMMAND_POOL_CREATE_TRANSIENT_BIT)
{
}

VulkanRenderer::~VulkanRenderer()
{
}

Shader* VulkanRenderer::CreateShader(const ShaderCreateInfo& _SI)
{
	return new VulkanShader(Device, _SI.ShaderName, _SI.Type);
}

Buffer* VulkanRenderer::CreateBuffer(const BufferCreateInfo& _BCI)
{
	return new VulkanBuffer(Device.GetVkDevice(), _BCI.Data, _BCI.Size, _BCI.Type, Device.GetTransferQueue(), TransientCommandPool, Device);
}

GraphicsPipeline* VulkanRenderer::CreateGraphicsPipeline(const GraphicsPipelineCreateInfo& _GPCI)
{
	return new VulkanGraphicsPipeline(Device, _GPCI.RenderPass, _GPCI.SwapchainSize, _GPCI.StagesInfo);
}

RenderPass* VulkanRenderer::CreateRenderPass(const RenderPassCreateInfo& _RPCI)
{
	return new VulkanRenderPass(Device, _RPCI.RPDescriptor);
}

ComputePipeline* VulkanRenderer::CreateComputePipeline(const ComputePipelineCreateInfo& _CPCI)
{
	return new ComputePipeline();
}

Framebuffer* VulkanRenderer::CreateFramebuffer(const FramebufferCreateInfo& _FCI)
{
	return new VulkanFramebuffer(Device, _FCI.RenderPass, _FCI.Extent, _FCI.Images, _FCI.ImagesCount);
}

RenderContext* VulkanRenderer::CreateRenderContext(const RenderContextCreateInfo& _RCCI)
{
	return new VulkanRenderContext(Device, Instance.GetVkInstance(), Device.GetVkPhysicalDevice(), _RCCI.Window);
}

//  VULKAN INSTANCE

Vulkan_Instance::Vulkan_Instance(const char * _AppName)
{
	VkApplicationInfo AppInfo = { VK_STRUCTURE_TYPE_APPLICATION_INFO };
	AppInfo.apiVersion = VK_API_VERSION_1_1;	//Should check if version is available vi vkEnumerateInstanceVersion().
	AppInfo.applicationVersion = VK_MAKE_VERSION(1, 0, 0);
	AppInfo.engineVersion = VK_MAKE_VERSION(1, 0, 0);
	AppInfo.pApplicationName = _AppName;
	AppInfo.pEngineName = "Game Studio";

#ifdef GS_DEBUG
	const char* InstanceLayers[] = { "VK_LAYER_LUNARG_standard_validation" };
#else
	const char* InstanceLayers[];
#endif // GS_DEBUG

	VkInstanceCreateInfo InstanceCreateInfo = { VK_STRUCTURE_TYPE_INSTANCE_CREATE_INFO };
	InstanceCreateInfo.pApplicationInfo = &AppInfo;
	InstanceCreateInfo.enabledLayerCount = 1;
	InstanceCreateInfo.ppEnabledLayerNames = InstanceLayers;
	InstanceCreateInfo.enabledExtensionCount = 0;
	InstanceCreateInfo.ppEnabledExtensionNames = nullptr;

	GS_VK_CHECK(vkCreateInstance(&InstanceCreateInfo, ALLOCATOR, &Instance), "Failed to create Instance!")
}

Vulkan_Instance::~Vulkan_Instance()
{
	vkDestroyInstance(Instance, ALLOCATOR);
}


//  VULKAN DEVICE


VkPhysicalDeviceMemoryProperties MemoryProperties = {};

Vulkan_Device::Vulkan_Device(VkInstance _Instance)
{
	////////////////////////////////////////
	//      DEVICE CREATION/SELECTION     //
	////////////////////////////////////////

	VkPhysicalDeviceFeatures deviceFeatures = {};	//COME BACK TO

	const char* DeviceExtensions[] = { VK_KHR_SWAPCHAIN_EXTENSION_NAME };

	CreatePhysicalDevice(PhysicalDevice, _Instance);

	FVector<QueueInfo> QueueInfos(3);

	QueueInfos[0].QueueFlagBits = VK_QUEUE_GRAPHICS_BIT;
	QueueInfos[0].QueuePriority = 1.0f;
	QueueInfos[1].QueueFlagBits = VK_QUEUE_COMPUTE_BIT;
	QueueInfos[1].QueuePriority = 1.0f;
	QueueInfos[2].QueueFlagBits = VK_QUEUE_TRANSFER_BIT;
	QueueInfos[2].QueuePriority = 1.0f;

	for (uint8 i = 0; i < QueueInfos.length(); i++)
	{
		CreateQueueInfo(QueueInfos[i], PhysicalDevice);
	}


	VkDeviceCreateInfo DeviceCreateInfo = { VK_STRUCTURE_TYPE_DEVICE_CREATE_INFO };
	DeviceCreateInfo.pQueueCreateInfos = &QueueInfos.data()->DeviceQueueCreateInfo;
	DeviceCreateInfo.queueCreateInfoCount = QueueInfos.length();
	DeviceCreateInfo.pEnabledFeatures = &deviceFeatures;
	DeviceCreateInfo.enabledExtensionCount = 1;
	DeviceCreateInfo.ppEnabledExtensionNames = DeviceExtensions;

	GS_VK_CHECK(vkCreateDevice(PhysicalDevice, &DeviceCreateInfo, ALLOCATOR, &Device), "Failed to create logical device!")

	SetVk_Queue(GraphicsQueue, QueueInfos[0].DeviceQueueCreateInfo.queueFamilyIndex);
	SetVk_Queue(ComputeQueue, QueueInfos[1].DeviceQueueCreateInfo.queueFamilyIndex);
	SetVk_Queue(TransferQueue, QueueInfos[2].DeviceQueueCreateInfo.queueFamilyIndex);

	vkGetPhysicalDeviceMemoryProperties(PhysicalDevice, &MemoryProperties);
}

Vulkan_Device::~Vulkan_Device()
{
	vkDestroyDevice(Device, ALLOCATOR);
}

void Vulkan_Device::SetVk_Queue(Vk_Queue& _Queue, const uint32 _QueueFamilyIndex)
{
	vkGetDeviceQueue(Device, _QueueFamilyIndex, 0, &_Queue.GetVkQueue());
}

void Vulkan_Device::CreateQueueInfo(QueueInfo& _QI, VkPhysicalDevice _PD)
{
	_QI.DeviceQueueCreateInfo.sType = VK_STRUCTURE_TYPE_DEVICE_QUEUE_CREATE_INFO;

	uint32_t QueueFamiliesCount = 0;
	vkGetPhysicalDeviceQueueFamilyProperties(_PD, &QueueFamiliesCount, VK_NULL_HANDLE);	//Get the amount of queue families there are in the physical device.
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

void Vulkan_Device::CreatePhysicalDevice(VkPhysicalDevice _PD, VkInstance _Instance)
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

			PhysicalDevicesProperties.erase(i);

			i--;
		}

		i++;
	}

	_PD = PhysicalDevices[i];
}

uint8 Vulkan_Device::GetDeviceTypeScore(VkPhysicalDeviceType _Type)
{
	switch (_Type)
	{
	case VK_PHYSICAL_DEVICE_TYPE_DISCRETE_GPU:		return 255;
	case VK_PHYSICAL_DEVICE_TYPE_INTEGRATED_GPU:	return 254;
	case VK_PHYSICAL_DEVICE_TYPE_CPU:				return 253;
	default:										return 0;
	}
}

uint32 Vulkan_Device::FindMemoryType(uint32 _TypeFilter, uint32 _Properties) const
{
	for (uint32 i = 0; i < MemoryProperties.memoryTypeCount; i++)
	{
		if ((_TypeFilter & (1 << i)) && (MemoryProperties.memoryTypes[i].propertyFlags & _Properties) == _Properties)
		{
			return i;
		}
	}
}