#include "Vk_Surface.h"

#include "RAPI/Vulkan/Vulkan.h"
#include "RAPI/Vulkan/Native/Vk_Instance.h"
#include "RAPI/Platform/Windows/WindowsWindow.h"

#include "Vk_PhysicalDevice.h"
#include "Vk_Device.h"

Vk_Surface::Vk_Surface(const Vk_Device& _Device, const Vk_Instance& _Instance, const Vk_PhysicalDevice& _PD, const Window& _Window) : VulkanObject(_Device), m_Instance(_Instance)
{
	VkWin32SurfaceCreateInfoKHR WCreateInfo = { VK_STRUCTURE_TYPE_WIN32_SURFACE_CREATE_INFO_KHR };
	WCreateInfo.hwnd = SCAST(WindowsWindow&, CCAST(Window&, _Window)).GetWindowObject();
	WCreateInfo.hinstance = SCAST(WindowsWindow&, CCAST(Window&, _Window)).GetHInstance();

	GS_VK_CHECK(vkCreateWin32SurfaceKHR(m_Instance, &WCreateInfo, ALLOCATOR, &Surface), "Failed to create Win32 Surface!");

	VkSurfaceCapabilitiesKHR Capabilities;
	auto CapResult = vkGetPhysicalDeviceSurfaceCapabilitiesKHR(_PD, Surface, &Capabilities);

	VkBool32 Supports = 0;
	auto SupResult = vkGetPhysicalDeviceSurfaceSupportKHR(_PD, _Device.GetGraphicsQueue().GetQueueIndex(), Surface, &Supports);
}

Vk_Surface::~Vk_Surface()
{
	vkDestroySurfaceKHR(m_Instance, Surface, ALLOCATOR);
}