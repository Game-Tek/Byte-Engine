#pragma once

#include "Core.h"

#include "RAPI/Vulkan/VulkanBase.h"

MAKE_VK_HANDLE(VkSurfaceKHR)

class Window;

enum VkSurfaceFormatKHR;

GS_CLASS Vk_Surface final : public VulkanObject
{
	static VkSurfaceFormatKHR PickBestFormat(const Vk_PhysicalDevice& _PD, const Vk_Surface& _Surface);

	const Vk_Instance& m_Instance;
	VkSurfaceKHR Surface = nullptr;

public:
	Vk_Surface(const Vk_Device& _Device, const Vk_Instance& _Instance, const Vk_PhysicalDevice& _PD, const Window& _Window);
	~Vk_Surface();

	INLINE VkSurfaceKHR GetVkSurface() const { return Surface; }
	INLINE operator VkSurfaceKHR() const { return Surface; }
};