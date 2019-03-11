#pragma once

#include "Core.h"

GS_STRUCT ImageSize
{
	ImageSize() = default;
	ImageSize(const uint16 Width, const uint16 Height);

	uint16 Width = 0;
	uint16 Height = 0;
};

inline ImageSize::ImageSize(const uint16 Width, const uint16 Height) : Width(Width), Height(Height)
{
}
