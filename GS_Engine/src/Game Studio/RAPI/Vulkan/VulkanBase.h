#pragma once

#include "Core.h"

#define MAKE_VK_HANDLE(object) typedef struct object##_T* object;

#define VK_NULL_HANDLE 0

class Vk_Device;

GS_STRUCT VulkanObjectCreateInfo
{
	VulkanObjectCreateInfo(const Vk_Device& _Device) : m_Device(_Device)
	{
	}

	const Vk_Device& m_Device;
};

GS_CLASS VulkanObject
{
protected:
	const Vk_Device& m_Device;
public:
	explicit VulkanObject(const Vk_Device& _Device) : m_Device(_Device)
	{
	}
};