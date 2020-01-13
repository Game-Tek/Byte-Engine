#include "ResourceManager.h"

#include <ostream>
#include <fstream>

void ResourceManager::ReleaseResource(Resource* _Resource) const
{
	_Resource->DecrementReferences();

	if (_Resource->GetReferenceCount() == 0)
	{
		//delete ResourceMap[_Resource];
		delete R[R.find(_Resource).Second];
	}
}

void ResourceManager::SaveFile(FString& _ResourceName, FString& _ResourcePath, ResourceData& ResourceData_)
{
	FString full_path = FString("W:/Game Studio/bin/Sandbox/Debug-x64/resources/") + _ResourcePath;

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

void ResourceManager::GetResourceInternal(const FString& _ResourceName, Resource* _Resource) const
{
	const auto FullPath = FString("W:/Game Studio/bin/Sandbox/Debug-x64/resources/") + _ResourceName + "." + _Resource->GetResourceTypeExtension();
	const auto Result = _Resource->LoadResource(FullPath);

	if (Result)
	{
		GS_LOG_SUCCESS("Loaded resource %s succesfully!", FullPath.c_str())
	}
	else
	{
		GS_LOG_WARNING("Failed to load %s resource of type %s! Loading fallback resource.", _ResourceName.c_str(), _Resource->GetResourceTypeExtension())
		_Resource->LoadFallbackResource(FullPath);
	}

	_Resource->IncrementReferences();
	//this->ResourceMap.emplace(resource, resource);
	this->R.emplace_back(_Resource);
}
