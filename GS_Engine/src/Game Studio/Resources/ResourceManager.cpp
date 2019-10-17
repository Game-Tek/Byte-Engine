#include "ResourceManager.h"

#include <ostream>
#include <fstream>

void ResourceManager::ReleaseResource(Resource* _Resource) const
{
	_Resource->DecrementReferences();

	if (_Resource->GetReferenceCount() == 0)
	{
		//delete ResourceMap[_Resource];
		delete R[R.find(_Resource)];
	}
}

void ResourceManager::SaveFile(const FString& _ResourceName, void(* f)(std::ostream& _RP))
{
	FString resource_name(_ResourceName.FindLast('.') - 1, _ResourceName.c_str());
	FString full_path = FString("W:/Game Studio/bin/Sandbox/Debug-x64/resources/") + _ResourceName;

	std::ofstream Outfile(full_path.c_str(), std::ios::out | std::ios::binary);

	if(!Outfile.is_open())
	{
		GS_LOG_WARNING("Could not save file %s.", _ResourceName.c_str())
		Outfile.close();
		return;
	}

	Outfile << resource_name;

	//ResourcePush RP;
	f(Outfile);


//ResourceHeaderType HeaderCount = RP.GetElementCount() + 1 /*File name segment*/;
//Outfile.write(reinterpret_cast<char*>(&HeaderCount), sizeof(ResourceHeaderType));
//
//ResourceSegmentType SegmentSize = resource_name.GetLength() + 1;
//
//Outfile.write(reinterpret_cast<char*>(&SegmentSize), sizeof(ResourceSegmentType));
//Outfile.write(resource_name.c_str(), SegmentSize);
//
//for (uint64 i  = 0; i < HeaderCount - 1 /*File name segment is not written in loop*/; ++i)
//{
//	SegmentSize = RP[i].Bytes;
//	Outfile.write(reinterpret_cast<char*>(&SegmentSize), sizeof(ResourceSegmentType));
//	Outfile.write(reinterpret_cast<char*>(RP[i].Data), SegmentSize);
//}

	Outfile.close();
}

void ResourceManager::GetResourceInternal(const FString& _ResourceName, Resource* _Resource) const
{
	const auto FullPath = FString("W:/Game Studio/bin/Sandbox/Debug-x64/resources/") + _ResourceName + _Resource->GetResourceTypeExtension();
	const auto Result = _Resource->LoadResource(FullPath);

	if (Result)
	{
		GS_LOG_SUCCESS("Loaded resource %s succesfully!", FullPath.c_str())
	}
	else
	{
		GS_LOG_WARNING("Failed to load %s resource of type %s! Loaded default resource.", FullPath.c_str(), _Resource->GetName())
			_Resource->LoadFallbackResource(FullPath);
	}

	_Resource->IncrementReferences();
	//this->ResourceMap.emplace(resource, resource);
	this->R.emplace_back(_Resource);
}
