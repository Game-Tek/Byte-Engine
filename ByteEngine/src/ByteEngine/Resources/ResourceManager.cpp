#include "ResourceManager.h"

#include "ByteEngine/Application/Application.h"

ResourceManager::ResourceManager() : resourceManagers(32, GetPersistentAllocator())
{
	
}

GTSL::StaticString<256> ResourceManager::GetResourcePath() const
{
	GTSL::StaticString<256> path;
	path += BE::Application::Get()->GetSystemApplication()->GetPathToExecutable();
	path.Drop(path.FindLast('/'));
	return path;
}
