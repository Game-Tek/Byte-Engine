#pragma once

#include "Core.h"

#include "Resource.h"

#include "String.h"

#include "FVector.hpp"

GS_CLASS ResourceManager
{
public:
	ResourceManager();
	virtual ~ResourceManager();

	template <typename T>
	T * GetResource(String Path)
	{
		for (uint16 i = 0; i < LoadedResources.length(); i++)
		{
			if (Path == LoadedResources[i]->GetPath())
			{
				return dynamic_cast<T *>(LoadedResources[i]);
			}
		}

		//SHOULD RETURN NULLPTR IF NOT ALREADY LOADED. THIS IS FOR TESTING PURPOUSES ONLY!

		return LoadAsset<T>(Path);
	}

protected:
	FVector<Resource *> LoadedResources;


	template <typename T>
	T * LoadAsset(const String & Path)
	{
		T * ptr = new T(Path);

		LoadedResources.push_back(ptr);

		return ptr;
	}
};

