#include "ResourceManager.h"

#include "ByteEngine/Application/Application.h"

GTSL::StaticString<512> ResourceManager::GetResourcePath(const GTSL::Range<const UTF8*> fileName)
{
	GTSL::StaticString<512> path;
	path += BE::Application::Get()->GetPathToApplication();
	path += "/resources/"; path += fileName;
	return path;
}
