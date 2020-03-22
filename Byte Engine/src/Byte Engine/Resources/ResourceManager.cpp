#include "ResourceManager.h"

#include <ostream>
#include <fstream>
#include "Core/System.h"
#include "Debug/Logger.h"

ResourceReference ResourceManager::TryGetResource(const FString& name, const Id64& type)
{
	auto resource_manager = resourceManagers.find(type);

	BE_ASSERT(resource_manager == resourceManagers.end(), "A resource manager for the specified resource type could not be found! Remember to register all needed resource managers on startup.")

	SubResourceManager::LoadResourceInfo load_resource_info{};
	load_resource_info.ResourceName = Id64(name);
	load_resource_info.ResourcePath += System::GetRunningPath();
	load_resource_info.ResourcePath += "resources/";
	load_resource_info.ResourcePath += name;
	load_resource_info.ResourcePath += '.';
	load_resource_info.ResourcePath += resource_manager->second->GetResourceExtension();

	SubResourceManager::OnResourceLoadInfo on_resource_load_info;
	
	resource_manager->second->LoadResource(load_resource_info, on_resource_load_info);

	return ResourceReference(type, Id64(name));
}

ResourceData* ResourceManager::GetResource(const ResourceReference& resourceReference)
{
	auto resource_manager = resourceManagers.find(resourceReference.resourceType);
	BE_ASSERT(resource_manager == resourceManagers.end(), "A resource manager for the specified resource type could not be found! Remember to register all needed resource managers on startup.")
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
	auto full_path = System::GetRunningPath();
	full_path += "resources/";
	full_path += _ResourceName;

	std::ofstream Outfile(full_path.c_str(), std::ios::out | std::ios::binary);

	if (!Outfile.is_open())
	{
		BE_LOG_WARNING("Could not save file %s.", _ResourceName.c_str())
		Outfile.close();
		return;
	}

	OutStream out_archive(&Outfile);

	Outfile.close();
}