#pragma once

#include "Core.h"

#include "Resource.h"

class ResourceManager
{
public:
	ResourceManager();
	~ResourceManager();

	static void LoadAsset();
private:
	Resource * LoadedResources[100];
};

