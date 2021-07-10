#include "PipelineCacheResourceManager.h"

#include "ByteEngine/Application/Application.h"

PipelineCacheResourceManager::PipelineCacheResourceManager() : ResourceManager(u8"PipelineCacheResourceManager")
{
	GTSL::StaticString<256> resources_path;
	resources_path += BE::Application::Get()->GetPathToApplication(); resources_path += u8"/resources/PipelineCache.bepkg";
	switch (cache.Open(resources_path, GTSL::File::READ | GTSL::File::WRITE, true)) {
	default: ;
	}
}

PipelineCacheResourceManager::~PipelineCacheResourceManager()
{
}