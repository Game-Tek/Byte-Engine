#pragma once

#include "Core.h"

#include "Resource.h"

#include <string>

#include "FVector.hpp"

class ResourceManager
{
private:
	static FVector<Resource *> LoadedResources;

public:
	ResourceManager();
	~ResourceManager();

	template <typename T>
	static T * GetResource(const std::string & Path)
	{
		for (uint16 i = 0; i < LoadedResources.size(); i++)
		{
			if (LoadedResources[i]->GetPath() == Path)
			{
				return (T *)LoadedResources[i];
			}
		}

		//SHOULD RETURN NULLPTR IF NOT ALREADY LOADED. THIS IS FOR TESTING PURPOUSES ONLY!

		return LoadAsset<T>(Path);
	}

private:
	template <typename T>
	static T * LoadAsset(const std::string & Path)
	{
		T * ptr = new T(Path);

		LoadedResources.push_back(ptr);

		return ptr;
	}
};

