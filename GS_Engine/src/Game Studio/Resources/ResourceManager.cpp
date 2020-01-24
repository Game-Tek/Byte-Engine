#include "ResourceManager.h"

#include <ostream>
#include <fstream>
#include "Core/FileSystem.h"
#include "Debug/Logger.h"

void ResourceManager::ReleaseResource(Resource* _Resource) const
{
	_Resource->decrementReferences();

	if (_Resource->getReferenceCount() == 0)
	{
		delete ResourceMap[_Resource->resourceName.GetID()];
	}
}

void ResourceManager::SaveFile(const FString& _ResourceName, FString& fileName, ResourceData& ResourceData_)
{
	auto full_path = FileSystem::GetRunningPath() + "resources/" + fileName;

	std::ofstream Outfile(full_path.c_str(), std::ios::out | std::ios::binary);

	if(!Outfile.is_open())
	{
		GS_LOG_WARNING("Could not save file %s.", _ResourceName.c_str())
		Outfile.close();
		return;
	}

	OutStream out_archive(&Outfile);

	ResourceData_.Write(out_archive);

	Outfile.close();
}

void ResourceManager::LoadResource(const FString& _ResourceName, Resource* _Resource)
{
	const auto FullPath = FileSystem::GetRunningPath() + "resources/" + _ResourceName + "." + _Resource->getResourceTypeExtension();
	LoadResourceData load_resource_data;
	load_resource_data.Caller = this;
	load_resource_data.FullPath = FullPath;
	const auto Result = _Resource->loadResource(load_resource_data);

	if (Result)
	{
		GS_LOG_SUCCESS("Loaded resource %s succesfully!", FullPath.c_str())
	}
	else
	{
		GS_LOG_WARNING("Failed to load %s resource of type %s! Loading fallback resource.", _ResourceName.c_str(), _Resource->getResourceTypeExtension())
		_Resource->loadFallbackResource(FullPath);
	}
}
