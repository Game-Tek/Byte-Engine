#include "ResourceManager.h"

#include "ByteEngine/Application/Application.h"

GTSL::StaticString<512> ResourceManager::GetResourcePath(const GTSL::Range<const utf8*> fileName, const GTSL::Range<const utf8*> extension)
{
	GTSL::StaticString<512> path;
	path += BE::Application::Get()->GetPathToApplication();
	path += "/resources/"; path += fileName; path += '.'; path += extension;
	return path;
}

GTSL::StaticString<512> ResourceManager::GetResourcePath(const GTSL::Range<const utf8*> fileWithExtension)
{
	GTSL::StaticString<512> path;
	path += BE::Application::Get()->GetPathToApplication();
	path += "/resources/"; path += fileWithExtension;
	return path;
}

void ResourceManager::initializePackageFiles(GTSL::Range<const utf8*> path)
{
	for(uint32 i = 0; i < BE::Application::Get()->GetNumberOfThreads(); ++i) {
		packageFiles.EmplaceBack();
		switch (packageFiles.back().Open(path, GTSL::File::AccessMode::READ)) {
			case GTSL::File::OpenResult::OK: break;
			case GTSL::File::OpenResult::ALREADY_EXISTS: break;
			case GTSL::File::OpenResult::DOES_NOT_EXIST: BE_LOG_ERROR("Package file doesn't exist."); break;
			case GTSL::File::OpenResult::ERROR: break;
		}
	}
}
