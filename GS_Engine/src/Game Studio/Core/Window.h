#pragma once
#include "Containers/FString.h"
#include "Utility/Extent.h"

class nWindow
{
public:
	struct WindowCreateInfo
	{
		FString Name;
		Extent2D Extent;
		nWindow* ParentWindow = nullptr;
		void* PlatformData = nullptr;
	};
	nWindow(const WindowCreateInfo& windowCreateInfo);
};
