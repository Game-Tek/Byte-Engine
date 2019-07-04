#pragma once

#include "Core.h"

GS_STRUCT RenderContextCreateInfo
{

};

GS_CLASS RenderContext
{
	virtual void Present() = 0;
};