#pragma once

#include "Core.h"

GS_STRUCT UniformBufferCreateInfo
{
	void* Data = nullptr;
	size_t Size = 0;
};

GS_STRUCT UniformBufferUpdateInfo
{
	void* Data = nullptr;
	size_t Size = 0;
};

GS_CLASS UniformBuffer
{
public:
	virtual void UpdateBuffer(const UniformBufferUpdateInfo& _BUI) const = 0;
};