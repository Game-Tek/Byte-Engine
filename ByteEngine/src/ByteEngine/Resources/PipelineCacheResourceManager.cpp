#include "PipelineCacheResourceManager.h"

#include "ByteEngine/Application/Application.h"

PipelineCacheResourceManager::PipelineCacheResourceManager() : ResourceManager("PipelineCacheResourceManager")
{
	GTSL::StaticString<256> resources_path;
	resources_path += BE::Application::Get()->GetPathToApplication(); resources_path += "/resources/PipelineCache.bepkg";
	switch (cache.Open(resources_path, GTSL::File::READ | GTSL::File::WRITE)) {
	case GTSL::File::OpenResult::DOES_NOT_EXIST: {
		cache.Create(resources_path, GTSL::File::READ | GTSL::File::WRITE);
		break;
	}
	}
}

PipelineCacheResourceManager::~PipelineCacheResourceManager()
{
}