#include "MaterialResourceManager.h"

#include <fstream>
#include "Stream.h"

void MaterialResourceManager::ReleaseResource(const Id& resourceName) { if (resources[resourceName].DecrementReferences() == 0) { resources.erase(resourceName); } }

ResourceData* MaterialResourceManager::GetResource(const Id& name) { return &resources[name]; }

bool MaterialResourceManager::LoadResource(const LoadResourceInfo& loadResourceInfo, OnResourceLoadInfo& onResourceLoadInfo)
{
	std::ifstream input(loadResourceInfo.ResourcePath.c_str(), std::ios::in); //Open file as binary

	MaterialResourceData data;
	
	if (input.is_open()) //If file is valid
	{
		input.seekg(0, std::ios::end); //Search for end
		uint64 FileLength = input.tellg(); //Get file length
		input.seekg(0, std::ios::beg); //Move file pointer back to beginning

		InStream in_archive(&input);

		//in_archive >> data.VertexShaderCode;
		//in_archive >> data.FragmentShaderCode;
	}
	else
	{
		input.close();
		return false;
	}

	input.close();
	
	resources.insert({ loadResourceInfo.ResourceName, data });

	return true;
}

void MaterialResourceManager::LoadFallback(const LoadResourceInfo& loadResourceInfo, OnResourceLoadInfo& onResourceLoadInfo)
{
}
