#include "ResourceManager.h"

#include "ByteEngine/Application/Application.h"

GTSL::StaticString<512> ResourceManager::GetResourcePath(const GTSL::Range<const utf8*> fileName, const GTSL::Range<const utf8*> extension)
{
	GTSL::StaticString<512> path;
	path += BE::Application::Get()->GetPathToApplication();
	path += "/resources/"; path += fileName; path += '.'; path += extension;
	return path;
}

void ResourceManager::initializePackageFiles(GTSL::Range<const utf8*> path)
{
	for(uint32 i = 0; i < BE::Application::Get()->GetNumberOfThreads(); ++i) {
		packageFiles.EmplaceBack();
		packageFiles.back().OpenFile(path, GTSL::File::AccessMode::READ);
	}
}
