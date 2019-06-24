#pragma once

#include "Core.h"

GS_CLASS Swapchain
{
public:
	virtual ~Swapchain();

	virtual void Present() = 0;
};