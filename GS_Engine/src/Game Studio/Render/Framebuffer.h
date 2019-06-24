#pragma once

#include "Core.h"

GS_CLASS Framebuffer
{
public:
	virtual ~Framebuffer();

	virtual void AddImage() = 0;
};