#include "ResourceManager.h"

#include "ByteEngine/Application/Application.h"

GTSL::StaticString<512> ResourceManager::GetResourcePath(const GTSL::Range<const utf8*> fileName)
{
	GTSL::StaticString<512> path;
	path += BE::Application::Get()->GetPathToApplication();
	path += "/resources/"; path += fileName;
	return path;
}

void ResourceManager::initializePackageFiles(GTSL::Range<const utf8*> path)
{
	for(uint32 i = 0; i < BE::Application::Get()->GetNumberOfThreads(); ++i) {
		packageFiles[packageFiles.EmplaceBack()].OpenFile(path, GTSL::File::AccessMode::READ);
	}
}
