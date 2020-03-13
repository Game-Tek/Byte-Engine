#include "MaterialResourceManager.h"

#include <fstream>
#include "Stream.h"

void MaterialResourceManager::ReleaseResource(const Id& resourceName)
{
	if(resources[resourceName].DecrementReferences() == 0) { resources.erase(resourceName);	}
}

bool MaterialResourceManager::LoadResource(const LoadResourceInfo& loadResourceInfo, OnResourceLoadInfo& onResourceLoadInfo)
{
	std::ifstream Input(loadResourceInfo.ResourcePath.c_str(), std::ios::in); //Open file as binary

	MaterialResourceData data;
	
	if (Input.is_open()) //If file is valid
	{
		Input.seekg(0, std::ios::end); //Search for end
		uint64 FileLength = Input.tellg(); //Get file length
		Input.seekg(0, std::ios::beg); //Move file pointer back to beginning

		InStream in_archive(&Input);

		//in_archive >> data.VertexShaderCode;
		//in_archive >> data.FragmentShaderCode;
	}
	else
	{
		Input.close();
		return false;
	}

	Input.close();
	
	resources.insert({ loadResourceInfo.ResourceName, data });

	return true;
}

void MaterialResourceManager::LoadFallback(const LoadResourceInfo& loadResourceInfo, OnResourceLoadInfo& onResourceLoadInfo)
{
}
