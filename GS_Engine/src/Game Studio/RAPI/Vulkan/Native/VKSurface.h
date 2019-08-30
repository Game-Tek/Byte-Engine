#pragma once

#include "Core.h"

#include "RAPI/Vulkan/VulkanBase.h"

MAKE_VK_HANDLE(VkSurfaceKHR)

class VKInstance;
class vkPhysicalDevice;
class Window;

GS_STRUCT VKSurfaceCreator final : VKObjectCreator<VkSurfaceKHR>
{
	const VKInstance & m_Instance;

	VKSurfaceCreator(const VKDevice & _Device, const VKInstance & _Instance, const Window & _Window);
};

GS_CLASS VKSurface final : public VKObject<VkSurfaceKHR>
{
	const VKInstance& m_Instance;

public:
	VKSurface(const VKSurfaceCreator& _VKSC) : VKObject<VkSurfaceKHR>(_VKSC), m_Instance(_VKSC.m_Instance)
	{
	}

	~VKSurface();
};