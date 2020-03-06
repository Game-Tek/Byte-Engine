#pragma once

#include "Core.h"

#define MAKE_VK_HANDLE(object) typedef struct object##_T* object;

#define VK_NULL_HANDLE 0

class VkDevice;

template <typename T>
struct VKObjectCreator
{
	VKObjectCreator(VkDevice* _Device) : m_Device(_Device)
	{
	}

	VkDevice* m_Device = nullptr;
	T Handle = VK_NULL_HANDLE;
};

template <typename T>
class VKObject
{
protected:
	VkDevice* m_Device = nullptr;
	T Handle = VK_NULL_HANDLE;

public:
	explicit VKObject(const VKObjectCreator<T>& _VKOC) : m_Device(_VKOC.m_Device), Handle(_VKOC.Handle)
	{
	}

	INLINE T GetHandle() const { return Handle; }

	VKObject& operator=(const VKObject<T>& _Other) = default;

	INLINE operator T() { return Handle; }
};
