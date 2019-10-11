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
	}
}

void ResourceManager::SaveFile(const FString& _Path, void(* f)(ResourceManager::ResourcePush& _RP))
{
	std::ofstream Outfile(_Path.c_str());

	ResourcePush RP;
	f(RP);

	ResourceHeaderType HeaderCount = RP.GetElementCount();
	Outfile.write(&reinterpret_cast<char&>(HeaderCount), sizeof(ResourceHeaderType));

	for (uint64 i  = 0; i < HeaderCount; ++i)
	{
		uint64 SegmentSize = RP[i].Bytes;
		Outfile.write(&reinterpret_cast<char&>(SegmentSize), sizeof(ResourceHeaderType));
		Outfile.write(reinterpret_cast<char*>(RP[i].Data), sizeof(RP[i].Bytes));
	}

	Outfile.close();
}
