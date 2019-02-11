#pragma once

#include "Core.h"

#include "Resource.h"

#include <string>

#include "FVector.hpp"

GS_CLASS ResourceManager
{
	FVector<Resource *> LoadedResources;

public:
	ResourceManager();
	~ResourceManager();

	template <typename T>
	T * GetResource(const std::string & Path)
	{
		for (uint16 i = 0; i < LoadedResources.length(); i++)
		{
			if (LoadedResources[i]->GetPath() == Path)
			{
				return dynamic_cast<T *>(LoadedResources[i]);
			}
		}

		//SHOULD RETURN NULLPTR IF NOT ALREADY LOADED. THIS IS FOR TESTING PURPOUSES ONLY!

		return LoadAsset<T>(Path);
	}

private:
	template <typename T>
	T * LoadAsset(const std::string & Path)
	{
		T * ptr = new T(Path);

		LoadedResources.push_back(ptr);

		return ptr;
	}
};

