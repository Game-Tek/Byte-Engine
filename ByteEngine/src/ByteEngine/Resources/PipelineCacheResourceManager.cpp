#include "PipelineCacheResourceManager.h"

#include "ByteEngine/Application/Application.h"

PipelineCacheResourceManager::PipelineCacheResourceManager() : ResourceManager("PipelineCacheResourceManager")
{
	GTSL::StaticString<256> resources_path;
	resources_path += BE::Application::Get()->GetPathToApplication(); resources_path += "/resources/PipelineCache.bepkg";
	cache.OpenFile(resources_path, (uint8)GTSL::File::AccessMode::READ | (uint8)GTSL::File::AccessMode::WRITE, GTSL::File::OpenMode::LEAVE_CONTENTS);
}

PipelineCacheResourceManager::~PipelineCacheResourceManager()
{
}