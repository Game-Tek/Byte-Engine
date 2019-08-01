#include "Vk_Instance.h"

#include "RAPI/Vulkan/Vulkan.h"

#include <iostream>

static VKAPI_ATTR VkBool32 VKAPI_CALL Callback(VkDebugUtilsMessageSeverityFlagBitsEXT messageSeverity,	VkDebugUtilsMessageTypeFlagsEXT messageType,	const VkDebugUtilsMessengerCallbackDataEXT* pCallbackData,	void* pUserData)
{
	std::cerr << "validation layer: " << pCallbackData->pMessage << std::endl;

	return VK_FALSE;
}

Vk_Instance::Vk_Instance(const char* _AppName)
{
	VkApplicationInfo AppInfo = { VK_STRUCTURE_TYPE_APPLICATION_INFO };
	AppInfo.pNext = nullptr;
	AppInfo.apiVersion = VK_API_VERSION_1_1;	//Should check if version is available vi vkEnumerateInstanceVersion().
	AppInfo.applicationVersion = VK_MAKE_VERSION(1, 0, 0);
	AppInfo.engineVersion = VK_MAKE_VERSION(1, 0, 0);
	AppInfo.pApplicationName = _AppName;
	AppInfo.pEngineName = "Game Studio";

#ifdef GS_DEBUG
	const char* InstanceLayers[] = { "VK_LAYER_LUNARG_standard_validation" };
#else
	const char* InstanceLayers[] = nullptr;
#endif // GS_DEBUG

	const char* Extensions[] = { VK_KHR_SURFACE_EXTENSION_NAME, VK_KHR_WIN32_SURFACE_EXTENSION_NAME };

	VkInstanceCreateInfo InstanceCreateInfo = { VK_STRUCTURE_TYPE_INSTANCE_CREATE_INFO };
	InstanceCreateInfo.pApplicationInfo = &AppInfo;
	InstanceCreateInfo.enabledLayerCount = 1;
	InstanceCreateInfo.ppEnabledLayerNames = InstanceLayers;
	InstanceCreateInfo.enabledExtensionCount = 2;
	InstanceCreateInfo.ppEnabledExtensionNames = Extensions;

	GS_VK_CHECK(vkCreateInstance(&InstanceCreateInfo, ALLOCATOR, &Instance), "Failed to create Instance!")
}

Vk_Instance::~Vk_Instance()
{
	vkDestroyInstance(Instance, ALLOCATOR);
}