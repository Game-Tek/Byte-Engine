#pragma once

#include "Core.h"

GS_CLASS RenderPass
{
public:
	RenderPass() = default;
	~RenderPass() = default;

	virtual void SetAsActive() const = 0;
};