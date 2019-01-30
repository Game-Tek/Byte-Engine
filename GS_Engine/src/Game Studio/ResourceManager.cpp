#include "ResourceManager.h"



ResourceManager::ResourceManager() : LoadedResources(100)
{
}


ResourceManager::~ResourceManager()
{
	for (uint16 i = 0; i < LoadedResources.size(); i++)
	{
		delete LoadedResources[i];
	}
}
