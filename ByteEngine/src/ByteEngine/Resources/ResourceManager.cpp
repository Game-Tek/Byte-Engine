#include "ResourceManager.h"

#include "ByteEngine/Application/Application.h"

GTSL::StaticString<512> ResourceManager::GetResourcePath(const GTSL::Range<const utf8*> fileName, const GTSL::Range<const utf8*> extension)
{
	GTSL::StaticString<512> path;
	path += BE::Application::Get()->GetPathToApplication();
	path += u8"/resources/"; path += fileName; path += u8'.'; path += extension;
	return path;
}

GTSL::StaticString<512> ResourceManager::GetResourcePath(const GTSL::Range<const utf8*> fileWithExtension)
{
	GTSL::StaticString<512> path;
	path += BE::Application::Get()->GetPathToApplication();
	path += u8"/resources/"; path += fileWithExtension;
	return path;
}

void ResourceManager::initializePackageFiles(GTSL::Array<GTSL::File, MAX_THREADS>& filesPerThread, GTSL::Range<const utf8*> path)
{
	for (uint32 i = 0; i < BE::Application::Get()->GetNumberOfThreads(); ++i) {
		filesPerThread.EmplaceBack();
		switch (filesPerThread.back().Open(path, GTSL::File::READ | GTSL::File::WRITE, true)) {
		case GTSL::File::OpenResult::OK: break;
		case GTSL::File::OpenResult::CREATED: break;
		case GTSL::File::OpenResult::ERROR: break;
		}
	}
}