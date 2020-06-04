#include "MaterialResourceManager.h"

#include <fstream>
#include <GTSL/Stream.h>
#include <GTSL/System.h>

MaterialResourceData* MaterialResourceManager::TryGetResource(const GTSL::String& name)
{
	const GTSL::Id64 hashed_name(name);

	{
		resourceMapMutex.ReadLock();
		if (resources.contains(hashed_name))
		{
			resourceMapMutex.ReadUnlock();
			resourceMapMutex.WriteLock();
			auto& res = resources.at(hashed_name);
			res.IncrementReferences();
			resourceMapMutex.WriteUnlock();
			return &res;
		}
		resourceMapMutex.ReadUnlock();
	}

	GTSL::String path(255, &transientAllocator);
	GTSL::System::GetRunningPath(path);
	path += "resources/";
	path += name;
	path += '.';
	path += "gsmat";
	
	std::ifstream input(path.c_str(), std::ios::in);

	MaterialResourceData data;

	if (input.is_open()) //If file is valid
	{
		GTSL::InStream in_archive(&input);

		//in_archive >> data.VertexShaderCode;
		//in_archive >> data.FragmentShaderCode;
		//

		resourceMapMutex.WriteLock();
		resources.emplace(hashed_name, GTSL::MakeTransferReference(data)).first->second.IncrementReferences();
		resourceMapMutex.WriteUnlock();
	}

	input.close();
	return nullptr;
}
