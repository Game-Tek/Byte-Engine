#pragma once

#include "Core.h"

enum class BufferType : uint8
{
	BUFFER_VERTEX,
	BUFFER_INDEX,
	BUFFER_UNIFORM
};

GS_STRUCT BufferCreateInfo
{
	BufferType Type;
};

GS_CLASS Buffer
{

};