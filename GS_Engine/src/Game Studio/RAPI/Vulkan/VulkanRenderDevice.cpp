#include "Vulkan.h"

#ifdef GS_PLATFORM_WIN
#include "windows.h"
#include "Vulkan/vulkan_win32.h"
#endif // GS_PLATFORM_WIN


#include "VulkanRenderDevice.h"

#include "VulkanRenderContext.h"
#include "VulkanPipelines.h"
#include "VulkanRenderPass.h"
#include "VulkanRenderMesh.h"
#include "VulkanRenderTarget.h"
#include "VulkanUniformBuffer.h"
#include "VulkanTexture.h"

void TransitionImageLayout(VkDevice* device_, VkImage* image_, VkFormat image_format_,
                           VkImageLayout from_image_layout_, VkImageLayout to_image_layout_,
                           VkCommandBuffer* command_buffer_)
{
	VkImageMemoryBarrier barrier = {};
	barrier.sType = VK_STRUCTURE_TYPE_IMAGE_MEMORY_BARRIER;
	barrier.oldLayout = from_image_layout_;
	barrier.newLayout = to_image_layout_;
	barrier.srcQueueFamilyIndex = VK_QUEUE_FAMILY_IGNORED;
	barrier.dstQueueFamilyIndex = VK_QUEUE_FAMILY_IGNORED;
	barrier.image = *image_;
	barrier.subresourceRange.aspectMask = VK_IMAGE_ASPECT_COLOR_BIT;
	barrier.subresourceRange.baseMipLevel = 0;
	barrier.subresourceRange.levelCount = 1;
	barrier.subresourceRange.baseArrayLayer = 0;
	barrier.subresourceRange.layerCount = 1;

	VkPipelineStageFlags sourceStage;
	VkPipelineStageFlags destinationStage;

	if (from_image_layout_ == VK_IMAGE_LAYOUT_UNDEFINED && to_image_layout_ == VK_IMAGE_LAYOUT_TRANSFER_DST_OPTIMAL)
	{
		barrier.srcAccessMask = 0;
		barrier.dstAccessMask = VK_ACCESS_TRANSFER_WRITE_BIT;

		sourceStage = VK_PIPELINE_STAGE_TOP_OF_PIPE_BIT;
		destinationStage = VK_PIPELINE_STAGE_TRANSFER_BIT;
	}
	else if (from_image_layout_ == VK_IMAGE_LAYOUT_TRANSFER_DST_OPTIMAL && to_image_layout_ == VK_IMAGE_LAYOUT_SHADER_READ_ONLY_OPTIMAL)
	{
		barrier.srcAccessMask = VK_ACCESS_TRANSFER_WRITE_BIT;
		barrier.dstAccessMask = VK_ACCESS_SHADER_READ_BIT;

		sourceStage = VK_PIPELINE_STAGE_TRANSFER_BIT;
		destinationStage = VK_PIPELINE_STAGE_FRAGMENT_SHADER_BIT;
	}
	else
	{
		throw std::invalid_argument("unsupported layout transition!");
	}

	vkCmdPipelineBarrier(*command_buffer_, sourceStage, destinationStage, 0, 0, nullptr, 0, nullptr, 1, &barrier);
}

VkFormat VulkanRenderDevice::FindSupportedFormat(const DArray<VkFormat>& formats, VkFormatFeatureFlags formatFeatureFlags, VkImageTiling imageTiling)
{
	VkFormatProperties format_properties;

	bool isSupported = false;

	for (auto& e : formats)
	{
		vkGetPhysicalDeviceFormatProperties(physicalDevice, e, &format_properties);

		switch (imageTiling)
		{
		case VK_IMAGE_TILING_LINEAR:
			isSupported = format_properties.linearTilingFeatures & formatFeatureFlags;
			break;
		case VK_IMAGE_TILING_OPTIMAL:
			isSupported = format_properties.optimalTilingFeatures & formatFeatureFlags;
			break;
		}

		if (isSupported)
		{
			return e;
		}
	}

	return VK_FORMAT_UNDEFINED;
}

uint32 VulkanRenderDevice::FindMemoryType(uint32 memoryType, uint32 memoryFlags) const
{
	for (uint32 i = 0; i < memoryProperties.memoryTypeCount; ++i)
	{
		if (memoryType & (1 << i)) { return i; }
	}

	GS_ASSERT(true, "Failed to find a suitable memory type!")
}

void VulkanRenderDevice::AllocateMemory(VkMemoryRequirements* memoryRequirements,
                                        VkMemoryPropertyFlagBits memoryPropertyFlag, VkDeviceMemory* deviceMemory)
{
	VkMemoryAllocateInfo vk_memory_allocate_info = {VK_STRUCTURE_TYPE_MEMORY_ALLOCATE_INFO};
	vk_memory_allocate_info.allocationSize = memoryRequirements->size;
	vk_memory_allocate_info.memoryTypeIndex = FindMemoryType(memoryRequirements->memoryTypeBits, memoryPropertyFlag);

	VK_CHECK(vkAllocateMemory(device, &vk_memory_allocate_info, ALLOCATOR, deviceMemory), "Failed to allocate memory!");
}

#ifdef GS_DEBUG
inline VKAPI_ATTR VkBool32 VKAPI_CALL debugCallback(VkDebugUtilsMessageSeverityFlagBitsEXT messageSeverity, VkDebugUtilsMessageTypeFlagsEXT messageType, const VkDebugUtilsMessengerCallbackDataEXT* pCallbackData,	void* pUserData)
{
	switch (messageSeverity)
	{
	case VK_DEBUG_UTILS_MESSAGE_SEVERITY_VERBOSE_BIT_EXT: GS_BASIC_LOG_MESSAGE("Vulkan: %s", pCallbackData->pMessage) break;
	case VK_DEBUG_UTILS_MESSAGE_SEVERITY_INFO_BIT_EXT: GS_BASIC_LOG_MESSAGE("Vulkan: %s", pCallbackData->pMessage) break;
	case VK_DEBUG_UTILS_MESSAGE_SEVERITY_WARNING_BIT_EXT: GS_BASIC_LOG_WARNING("Vulkan: %s", pCallbackData->pMessage) break;
	case VK_DEBUG_UTILS_MESSAGE_SEVERITY_ERROR_BIT_EXT: GS_BASIC_LOG_ERROR("Vulkan: %s, %s", pCallbackData->pObjects->pObjectName, pCallbackData->pMessage) break;
	default: break;
	}

	return VK_FALSE;
}
#endif // GS_DEBUG


VulkanRenderDevice::VulkanRenderDevice(const RenderDeviceCreateInfo& renderDeviceCreateInfo) : vulkanQueues(renderDeviceCreateInfo.QueueCreateInfos->getLength())
{
	VkApplicationInfo vk_application_info{ VK_STRUCTURE_TYPE_APPLICATION_INFO };
	vk_application_info.pNext = nullptr;
	vkEnumerateInstanceVersion(&vk_application_info.apiVersion);
	vk_application_info.applicationVersion = VK_MAKE_VERSION(renderDeviceCreateInfo.ApplicationVersion[0], renderDeviceCreateInfo.ApplicationVersion[1], renderDeviceCreateInfo.ApplicationVersion[2]);
	vk_application_info.engineVersion = VK_MAKE_VERSION(0, 0, 1);
	vk_application_info.pApplicationName = renderDeviceCreateInfo.ApplicationName.c_str();
	vk_application_info.pEngineName = "Game-Tek | RAPI";

	Array<const char*, 32, uint8> instance_layers = {
#ifdef GS_DEBUG
		"VK_LAYER_LUNARG_standard_validation",
		"VK_LAYER_LUNARG_parameter_validation",
	};
#else
	};
#endif // GS_DEBUG

	Array<const char*, 32, uint8> extensions = {
#ifdef GS_DEBUG
		VK_EXT_DEBUG_UTILS_EXTENSION_NAME,
#endif // GS_DEBUG

		VK_KHR_SURFACE_EXTENSION_NAME,

#ifdef GS_PLATFORM_WIN
		VK_KHR_WIN32_SURFACE_EXTENSION_NAME,
#endif // GS_PLATFORM_WIN
	};

	VkInstanceCreateInfo vk_instance_create_info{ VK_STRUCTURE_TYPE_INSTANCE_CREATE_INFO };
	vk_instance_create_info.pApplicationInfo = &vk_application_info;
	vk_instance_create_info.enabledLayerCount = instance_layers.getLength();
	vk_instance_create_info.ppEnabledLayerNames = instance_layers.getData();
	vk_instance_create_info.enabledExtensionCount = extensions.getLength();
	vk_instance_create_info.ppEnabledExtensionNames = extensions.getData();

	VK_CHECK(vkCreateInstance(&vk_instance_create_info, GetVkAllocationCallbacks(), &instance))

	VkDebugUtilsMessengerCreateInfoEXT vk_debug_utils_messenger_create_info_EXT{};
	vk_debug_utils_messenger_create_info_EXT.sType = VK_STRUCTURE_TYPE_DEBUG_UTILS_MESSENGER_CREATE_INFO_EXT;
	vk_debug_utils_messenger_create_info_EXT.messageSeverity = VK_DEBUG_UTILS_MESSAGE_SEVERITY_VERBOSE_BIT_EXT | VK_DEBUG_UTILS_MESSAGE_SEVERITY_WARNING_BIT_EXT | VK_DEBUG_UTILS_MESSAGE_SEVERITY_ERROR_BIT_EXT;
	vk_debug_utils_messenger_create_info_EXT.messageType = VK_DEBUG_UTILS_MESSAGE_TYPE_GENERAL_BIT_EXT | VK_DEBUG_UTILS_MESSAGE_TYPE_VALIDATION_BIT_EXT | VK_DEBUG_UTILS_MESSAGE_TYPE_PERFORMANCE_BIT_EXT;
	vk_debug_utils_messenger_create_info_EXT.pfnUserCallback = debugCallback;
	vk_debug_utils_messenger_create_info_EXT.pUserData = nullptr; // Optional

#ifdef GS_DEBUG
	createDebugUtilsFunction = reinterpret_cast<PFN_vkCreateDebugUtilsMessengerEXT>(vkGetInstanceProcAddr(instance, "vkCreateDebugUtilsMessengerEXT"));
	destroyDebugUtilsFunction = reinterpret_cast<PFN_vkDestroyDebugUtilsMessengerEXT>(vkGetInstanceProcAddr(instance, "vkDestroyDebugUtilsMessengerEXT"));

	createDebugUtilsFunction(instance, &vk_debug_utils_messenger_create_info_EXT, GetVkAllocationCallbacks(), &debugMessenger);
#endif

	vkGetPhysicalDeviceProperties(physicalDevice, &deviceProperties);

	VkPhysicalDeviceFeatures vk_physical_device_features{};
	vk_physical_device_features.samplerAnisotropy = VK_TRUE;
	vk_physical_device_features.shaderSampledImageArrayDynamicIndexing = VK_TRUE;

	Array<const char*, 32, uint8> device_extensions = { VK_KHR_SWAPCHAIN_EXTENSION_NAME };

	auto queue_create_infos = renderDeviceCreateInfo.QueueCreateInfos;

	FVector<VkDeviceQueueCreateInfo> vk_device_queue_create_infos(queue_create_infos->getLength(), queue_create_infos->getLength());

	uint32 queue_families_count = 0;
	vkGetPhysicalDeviceQueueFamilyProperties(physicalDevice, &queue_families_count, nullptr);
	//Get the amount of queue families there are in the physical device.
	FVector<VkQueueFamilyProperties> vk_queue_families_properties(queue_families_count);
	vkGetPhysicalDeviceQueueFamilyProperties(physicalDevice, &queue_families_count, vk_queue_families_properties.getData());

	FVector<bool> used_families(queue_families_count);
	FVector<VkQueueFlagBits> vk_queues_flag_bits(queue_families_count, queue_families_count);
	{
		uint8 i = 0;
		for (auto& e : vk_queues_flag_bits)
		{
			e == VkQueueFlagBits(queue_create_infos->at(i).Capabilities);
			++i;
		}
	}

	for (uint8 q = 0; q < queue_create_infos->getLength(); ++q)
	{
		for (uint8 f = 0; f < queue_families_count; ++f)
		{
			if (vk_queue_families_properties[f].queueCount > 0 && vk_queue_families_properties[f].queueFlags & vk_queues_flag_bits[f])
			{
				if (used_families[f])
				{
					++vk_device_queue_create_infos[f].queueCount;
					vk_device_queue_create_infos[f].pQueuePriorities = &queue_create_infos->at(q).QueuePriority;
					break;
				}

				vk_device_queue_create_infos[f].sType = VK_STRUCTURE_TYPE_DEVICE_QUEUE_CREATE_INFO;				used_families[f] = true;
				vk_device_queue_create_infos[f].pNext = nullptr;
				vk_device_queue_create_infos[f].flags = 0;
				vk_device_queue_create_infos[f].queueFamilyIndex = f;
				vk_device_queue_create_infos[f].queueCount = 1;
				break;
			}
		}
	}

	VkDeviceCreateInfo vk_device_create_info{ VK_STRUCTURE_TYPE_DEVICE_CREATE_INFO };
	vk_device_create_info.queueCreateInfoCount = vk_device_queue_create_infos.getLength();
	vk_device_create_info.pQueueCreateInfos = vk_device_queue_create_infos.getData();
	vk_device_create_info.pEnabledFeatures = &vk_physical_device_features;
	vk_device_create_info.enabledExtensionCount = device_extensions.getLength();
	vk_device_create_info.ppEnabledExtensionNames = device_extensions.getData();

	VK_CHECK(vkCreateDevice(physicalDevice, &vk_device_create_info, GetVkAllocationCallbacks(), &device));

	for (uint8 i = 0; i < renderDeviceCreateInfo.QueueCreateInfos->getLength(); ++i)
	{
		for (uint8 j = 0; j < vk_device_queue_create_infos[i].queueCount; ++j)
		{
			VulkanQueue::VulkanQueueCreateInfo vulkan_queue_create_info;
			vulkan_queue_create_info.FamilyIndex = vk_device_queue_create_infos[i].queueFamilyIndex;
			vulkan_queue_create_info.QueueIndex = j;
			vkGetDeviceQueue(device, vk_device_queue_create_infos[i].queueFamilyIndex, j, &vulkan_queue_create_info.Queue);
			vulkanQueues.emplace_back(vulkan_queue_create_info);
			*renderDeviceCreateInfo.QueueCreateInfos->at(i).QueueToSet = &vulkanQueues[i + j];
		}
	}
}

VulkanRenderDevice::~VulkanRenderDevice()
{
	vkDeviceWaitIdle(device);
	vkDestroyDevice(device, GetVkAllocationCallbacks());
#ifdef GS_DEBUG
	destroyDebugUtilsFunction(instance, debugMessenger, GetVkAllocationCallbacks());
#endif
	vkDestroyInstance(instance, GetVkAllocationCallbacks());
}

bool VulkanRenderDevice::IsVulkanSupported()
{
#ifdef GS_PLATFORM_WIN
	return true;
#endif // GS_PLATFORM_WIN
}

GPUInfo VulkanRenderDevice::GetGPUInfo()
{
	GPUInfo result;

	result.GPUName = deviceProperties.deviceName;
	result.DriverVersion = deviceProperties.vendorID;
	result.APIVersion = deviceProperties.apiVersion;

	return result;
}

RenderMesh* VulkanRenderDevice::CreateRenderMesh(const RenderMesh::RenderMeshCreateInfo& _MCI) { return new VulkanRenderMesh(this, _MCI); }

UniformBuffer* VulkanRenderDevice::CreateUniformBuffer(const UniformBufferCreateInfo& _BCI) { return new VulkanUniformBuffer(this, _BCI); }

RenderTarget* VulkanRenderDevice::CreateRenderTarget(const RenderTarget::RenderTargetCreateInfo& _ICI) { return new VulkanRenderTarget(this, _ICI); }

Texture* VulkanRenderDevice::CreateTexture(const TextureCreateInfo& textureCreateInfo) { return new VulkanTexture(this, textureCreateInfo); }

BindingsPool* VulkanRenderDevice::CreateBindingsPool(const BindingsPoolCreateInfo& bindingsPoolCreateInfo) { return new VulkanBindingsPool(this, bindingsPoolCreateInfo); }

BindingsSet* VulkanRenderDevice::CreateBindingsSet(const BindingsSetCreateInfo& bindingsSetCreateInfo) { return new VulkanBindingsSet(this, bindingsSetCreateInfo); }

GraphicsPipeline* VulkanRenderDevice::CreateGraphicsPipeline(const GraphicsPipelineCreateInfo& _GPCI) { return new VulkanGraphicsPipeline(this, _GPCI); }

RAPI::RenderPass* VulkanRenderDevice::CreateRenderPass(const RenderPassCreateInfo& _RPCI) {	return new VulkanRenderPass(this, _RPCI); }

ComputePipeline* VulkanRenderDevice::CreateComputePipeline(const ComputePipelineCreateInfo& _CPCI) { return new ComputePipeline(); }

Framebuffer* VulkanRenderDevice::CreateFramebuffer(const FramebufferCreateInfo& _FCI) {	return new VulkanFramebuffer(this, _FCI); }

RenderContext* VulkanRenderDevice::CreateRenderContext(const RenderContextCreateInfo& _RCCI) { return new VulkanRenderContext(this, _RCCI); }

VulkanRenderDevice::VulkanQueue::VulkanQueue(const QueueCreateInfo& queueCreateInfo, const VulkanQueueCreateInfo& vulkanQueueCreateInfo) : queue(vulkanQueueCreateInfo.Queue), queueIndex(vulkanQueueCreateInfo.QueueIndex), familyIndex(vulkanQueueCreateInfo.FamilyIndex)
{
}
