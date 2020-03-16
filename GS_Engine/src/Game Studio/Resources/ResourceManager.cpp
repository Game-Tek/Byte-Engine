#include "ResourceManager.h"

#include <ostream>
#include <fstream>
#include "Core/System.h"
#include "Debug/Logger.h"

ResourceReference ResourceManager::TryGetResource(const FString& name, const Id64& type)
{
	auto resource_manager = resourceManagers.find(type);

	GS_ASSERT(resource_manager == resourceManagers.end(), "A resource manager for the specified resource type could not be found! Remember to register all needed resource managers on startup.")

	SubResourceManager::LoadResourceInfo load_resource_info;
	load_resource_info.ResourceName = Id64(name);
	load_resource_info.ResourcePath = System::GetRunningPath() + "resources/" + name + "." + resource_manager->second->GetResourceExtension();

	SubResourceManager::OnResourceLoadInfo on_resource_load_info;
	
	resource_manager->second->LoadResource(load_resource_info, on_resource_load_info);

	return ResourceReference(type, Id64(name), on_resource_load_info.ResourceData);
}

ResourceData* ResourceManager::GetResource(const ResourceReference& resourceReference)
{
	auto resource_manager = resourceManagers.find(resourceReference.resourceType);
	GS_ASSERT(resource_manager == resourceManagers.end(), "A resource manager for the specified resource type could not be found! Remember to register all needed resource managers on startup.")
	return resource_manager->second->GetResource(resourceReference.resourceName);
}

void ResourceManager::ReleaseResource(const ResourceReference& resourceReference) const
{
	resourceManagers.at(resourceReference.resourceType)->ReleaseResource(resourceReference.resourceName);
}

void ResourceManager::ReleaseResource(const Id64& resourceType, const Id64& resourceName)
{
	resourceManagers[resourceType]->ReleaseResource(resourceName);
}

void ResourceManager::SaveFile(const FString& _ResourceName, FString& fileName, ResourceData& ResourceData_)
{
	auto full_path = System::GetRunningPath() + "resources/" + fileName;

	std::ofstream Outfile(full_path.c_str(), std::ios::out | std::ios::binary);

	if (!Outfile.is_open())
	{
		GS_LOG_WARNING("Could not save file %s.", _ResourceName.c_str())
		Outfile.close();
		return;
	}

	OutStream out_archive(&Outfile);

	Outfile.close();
}

void ResourceManager::LoadResource(const FString& _ResourceName, Resource* _Resource)
{
	const auto FullPath = System::GetRunningPath() + "resources/" + _ResourceName + "." + _Resource->
		getResourceTypeExtension();
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
		GS_LOG_WARNING("Failed to load %s resource of type %s! Loading fallback resource.", _ResourceName.c_str(),
		               _Resource->getResourceTypeExtension())
		_Resource->loadFallbackResource(FullPath);
	}
}
