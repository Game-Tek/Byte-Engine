#pragma once

#include "Core.h"

#include "Native/Vk_Device.h"

#define MAKE_VK_HANDLE(object) typedef struct object##_T* object;

MAKE_VK_HANDLE(VkDevice)

GS_CLASS VulkanObject
{
protected:
	const Vk_Device& m_Device;
public:
	explicit VulkanObject(const Vk_Device& _Device) : m_Device(_Device)
	{
	}
};