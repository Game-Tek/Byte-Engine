#include "ResourceManager.h"
#include "ByteEngine/Application/Application.h"

GTSL::StaticString<512> ResourceManager::GetUserResourcePath(const GTSL::Range<const char8_t*>& fileWithExtension)
{
	GTSL::StaticString<512> path;
	path += BE::Application::Get()->GetPathToApplication();
	path += u8"/user/"; path += fileWithExtension;
	return path;
}

GTSL::StaticString<512> ResourceManager::GetUserResourcePath(const GTSL::Range<const char8_t*>& fileName, const GTSL::Range<const char8_t*>& extensions)
{
	GTSL::StaticString<512> path;
	path += BE::Application::Get()->GetPathToApplication();
	path += u8"/user/"; path += fileName; path += u8'.'; path += extensions;
	return path;
}

GTSL::StaticString<512> ResourceManager::GetResourcePath(const GTSL::Range<const char8_t*>& fileName, const GTSL::Range<const char8_t*>& extensions)
{
	GTSL::StaticString<512> path;
	path += BE::Application::Get()->GetPathToApplication();
	path += u8"/resources/"; path += fileName; path += u8'.'; path += extensions;
	return path;
}

GTSL::StaticString<512> ResourceManager::GetResourcePath(const GTSL::Range<const char8_t*>& fileWithExtension)
{
	GTSL::StaticString<512> path;
	path += BE::Application::Get()->GetPathToApplication();
	path += u8"/resources/"; path += fileWithExtension;
	return path;
}

void ResourceManager::InitializePackageFiles(GTSL::StaticVector<GTSL::File, MAX_THREADS>& filesPerThread, GTSL::Range<const char8_t*> path)
{
	for (GTSL::uint32 i = 0; i < BE::Application::Get()->GetNumberOfThreads(); ++i) 
	{
		filesPerThread.EmplaceBack();
		switch (filesPerThread.back().Open(path, GTSL::File::READ | GTSL::File::WRITE, true)) {
		case GTSL::File::OpenResult::OK: break;
		case GTSL::File::OpenResult::CREATED: break;
		case GTSL::File::OpenResult::ERROR: break;
		}
	}
}