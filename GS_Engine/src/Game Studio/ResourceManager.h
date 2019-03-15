#pragma once

#include "Core.h"

#include "Resource.h"

#include "String.h"

#include "FVector.hpp"

#include "Logger.h"

#include "StaticMeshResource.h"

GS_CLASS ResourceManager
{
public:
	ResourceManager();
	virtual ~ResourceManager();

	StaticMeshResource * GetResource(const String & Path)
	{
		for (uint16 i = 0; i < LoadedResources.length(); i++)
		{
			if (LoadedResources[i]->GetPath() == Path)
			{
				return LoadedResources[i];
				GS_LOG_MESSAGE("Returned found")
			}
		}

		//SHOULD RETURN NULLPTR IF NOT ALREADY LOADED. THIS IS FOR TESTING PURPOUSES ONLY!
		GS_LOG_MESSAGE("Loading")
		return LoadAsset(Path);
	}

protected:
	FVector<StaticMeshResource *> LoadedResources;


	StaticMeshResource * LoadAsset(const String & Path)
	{
		StaticMeshResource * ptr = new StaticMeshResource(Path);

		GS_LOG_MESSAGE("PushBack")
		LoadedResources.push_back(ptr);

		return ptr;
	}
};

