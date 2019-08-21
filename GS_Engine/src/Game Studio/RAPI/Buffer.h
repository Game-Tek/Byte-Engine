#pragma once

#include "Core.h"
#include "RenderCore.h"

GS_STRUCT BufferCreateInfo
{
	BufferType Type = BufferType::BUFFER_VERTEX;
	void* Data = nullptr;
	size_t Size = 0;
};

GS_CLASS Buffer
{

};