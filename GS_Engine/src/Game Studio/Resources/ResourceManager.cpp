#include "ResourceManager.h"

#include <ostream>
#include <fstream>
#include "Containers/DArray.hpp"

void ResourceManager::ReleaseResource(Resource* _Resource) const
{
	_Resource->DecrementReferences();

	if (_Resource->GetReferenceCount() == 0)
	{
		//delete ResourceMap[_Resource];
		delete R[R.find(_Resource)];
	}
}

void ResourceManager::SaveFile(const FString& _ResourceName, void(* f)(ResourceManager::ResourcePush& _RP))
{
	FString resource_name(_ResourceName.FindLast('.') - 1, _ResourceName.c_str());
	//FString path = FString("W:/Game Studio/bin/Sandbox/Debug-x64/") + GetBaseResourcePath() + _ResourceName;

	std::ofstream Outfile("W:/Game Studio/bin/Sandbox/Debug-x64/resources/M_Base.gsmat");

	if(!Outfile.is_open())
	{
		GS_LOG_WARNING("Could not save file %s.", _ResourceName.c_str())
		Outfile.close();
		return;
	}

	ResourcePush RP;
	f(RP);

	ResourceHeaderType HeaderCount = RP.GetElementCount() + 1 /*File name segment*/;
	Outfile.write(reinterpret_cast<char*>(&HeaderCount), sizeof(ResourceHeaderType));

	ResourceSegmentType SegmentSize = resource_name.GetLength() + 1;

	Outfile.write(reinterpret_cast<char*>(&SegmentSize), sizeof(ResourceSegmentType));
	Outfile.write(resource_name.c_str(), SegmentSize);

	for (uint64 i  = 0; i < HeaderCount - 1 /*File name segment is not written in loop*/; ++i)
	{
		SegmentSize = RP[i].Bytes;
		Outfile.write(reinterpret_cast<char*>(&SegmentSize), sizeof(ResourceSegmentType));
		Outfile.write(reinterpret_cast<char*>(RP[i].Data), SegmentSize);
	}

	Outfile.close();
}
