#pragma once

#include "Core.h"

GS_CLASS RenderPass
{
public:
	virtual ~RenderPass();

	virtual void AddSubPass() = 0;
};