#pragma once

#include "Core.h"

#include "Resource.h"

#include "Id.h"

#include "HashMap.hpp"

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
	HashMap<Resource *> LoadedResources;


	Resource * LoadAsset(const Id & Path)
	{
		if (LoadedResources.Find(, Path.GetID()))
		{
			return LoadedResources.Get();
		}
		else
		{
			Resource* ptr = new StaticMeshResource(Path);
			LoadedResources.Insert(ptr, Path.GetID());
			return ptr;
		}

	}
};

