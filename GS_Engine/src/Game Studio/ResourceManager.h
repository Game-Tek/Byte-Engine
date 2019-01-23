#pragma once

#include "Core.h"

#include "Resource.h"

#include <vector>

class ResourceManager
{
public:
	ResourceManager();
	~ResourceManager();

	static void LoadAsset();
private:
	//std::vector<Resource<*> *> LoadedResources[100];
};

