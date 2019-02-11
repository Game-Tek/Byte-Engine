#include "ResourceManager.h"



ResourceManager::ResourceManager()
{
}


ResourceManager::~ResourceManager()
{
	for (uint16 i = 0; i < LoadedResources.length(); i++)
	{
		delete LoadedResources[i];
	}
}
