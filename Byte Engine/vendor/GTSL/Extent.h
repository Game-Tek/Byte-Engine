#pragma once

#include "Core.h"

struct Extent2D
{
	Extent2D() = default;
	Extent2D(const uint16 width, const uint16 height) : Width(width), Height(height)
	{
	}

	uint16 Width = 0;
	uint16 Height = 0;
};

struct Extent3D
{
	uint16 Width = 0;
	uint16 Height = 0;
	uint16 Depth = 0;
};
