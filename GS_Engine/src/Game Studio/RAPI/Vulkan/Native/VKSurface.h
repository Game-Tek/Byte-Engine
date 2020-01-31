#pragma once

#include "Core.h"

#include "RAPI/Vulkan/VulkanBase.h"

namespace RAPI {
	class Window;
}

MAKE_VK_HANDLE(VkSurfaceKHR)

class VKInstance;
class vkPhysicalDevice;
class Window;

struct VKSurfaceCreator final : VKObjectCreator<VkSurfaceKHR>
{
	VKInstance* m_Instance = nullptr;

	VKSurfaceCreator(VKDevice* _Device, VKInstance* _Instance, RAPI::Window* _Window);
};

class VKSurface final : public VKObject<VkSurfaceKHR>
{
	VKInstance* m_Instance = nullptr;

public:
	VKSurface(const VKSurfaceCreator& _VKSC) : VKObject<VkSurfaceKHR>(_VKSC), m_Instance(_VKSC.m_Instance)
	{
	}

	~VKSurface();
};
