#pragma once

#include "Core.h"

#define MAKE_VK_HANDLE(object) typedef struct object##_T* object;

#define VK_NULL_HANDLE 0

class VKDevice;

template<typename T>
GS_STRUCT VKObjectCreator
{
	VKObjectCreator(const VKDevice& _Device) : m_Device(_Device)
	{
	}

	const VKDevice& m_Device;
	T Handle = VK_NULL_HANDLE;
};

template <typename T>
GS_CLASS VKObject
{
protected:
	const VKDevice& m_Device;
	T Handle = VK_NULL_HANDLE;

public:
	explicit VKObject(const VKObjectCreator<T>& _VKOC) : m_Device(_VKOC), Handle(_VKOC.Handle)
	{
	}

	INLINE T GetHandle() const { return Handle; }

	INLINE operator T() { return Handle; }
};