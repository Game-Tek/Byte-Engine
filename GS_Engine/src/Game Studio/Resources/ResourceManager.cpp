#include "ResourceManager.h"

#include <ostream>
#include <fstream>

void ResourceManager::ReleaseResource(Resource* _Resource)
{
	_Resource->DecrementReferences();

	if (_Resource->GetReferenceCount() == 0)
	{
		delete ResourceMap[_Resource];
	}
}

void ResourceManager::SaveFile(const FileDescriptor& _FD)
{
	std::ofstream outfile(_FD.DirectoryAndFileNameWithExtension.c_str());

	outfile.close();
}

void ResourceManager::SaveFile(const FString& _Path, void(* f)(OutStream& _OS))
{
	std::ofstream outfile(_Path.c_str());
	f(outfile);
	outfile.close();
}
