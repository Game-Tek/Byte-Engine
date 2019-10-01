#pragma once

#include "Core.h"

struct GS_API UniformBufferCreateInfo
{
	size_t Size = 0;
};

struct GS_API UniformBufferUpdateInfo
{
	void* Data = nullptr;
	size_t Size = 0;
};

class GS_API UniformBuffer
{
public:
	virtual void UpdateBuffer(const UniformBufferUpdateInfo& _BUI) const = 0;
};