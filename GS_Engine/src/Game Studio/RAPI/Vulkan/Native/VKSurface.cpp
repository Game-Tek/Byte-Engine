#include "VKSurface.h"

#include "RAPI/Vulkan/Vulkan.h"
#include "RAPI/Vulkan/Native/VKInstance.h"
#include "RAPI/Platform/Windows/WindowsWindow.h"

#include "VKDevice.h"

VKSurfaceCreator::VKSurfaceCreator(VKDevice* _Device, const VKInstance& _Instance, const Window& _Window) : VKObjectCreator<VkSurfaceKHR>(_Device), m_Instance(_Instance)
{
	VkWin32SurfaceCreateInfoKHR WCreateInfo = { VK_STRUCTURE_TYPE_WIN32_SURFACE_CREATE_INFO_KHR };
	WCreateInfo.hwnd = SCAST(WindowsWindow&, CCAST(Window&, _Window)).GetWindowObject();
	WCreateInfo.hinstance = SCAST(WindowsWindow&, CCAST(Window&, _Window)).GetHInstance();

	GS_VK_CHECK(vkCreateWin32SurfaceKHR(m_Instance, &WCreateInfo, ALLOCATOR, &Handle), "Failed to create Win32 Surface!");
}

VKSurface::~VKSurface()
{
	vkDestroySurfaceKHR(m_Instance, Handle, ALLOCATOR);
}
