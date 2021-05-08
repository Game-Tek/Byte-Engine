#include "PipelineCacheResourceManager.h"

#include "ByteEngine/Application/Application.h"

PipelineCacheResourceManager::PipelineCacheResourceManager() : ResourceManager("PipelineCacheResourceManager")
{
	GTSL::StaticString<256> resources_path;
	resources_path += BE::Application::Get()->GetPathToApplication(); resources_path += "/resources/PipelineCache.bepkg";
	cache.Open(resources_path, GTSL::File::AccessMode::READ | GTSL::File::AccessMode::WRITE);
}

PipelineCacheResourceManager::~PipelineCacheResourceManager()
{
}