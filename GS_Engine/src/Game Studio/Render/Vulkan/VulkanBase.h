#pragma once

#include "Core.h"

#define MAKE_VK_HANDLE(object) typedef struct object##_T* object;

MAKE_VK_HANDLE(VkDevice)

GS_CLASS VulkanObject
{
protected:
	VkDevice m_Device = nullptr;
public:
	explicit VulkanObject(VkDevice _Device) : m_Device(_Device)
	{
	}

	INLINE VkDevice GetVkDevice() const { return m_Device; }
};