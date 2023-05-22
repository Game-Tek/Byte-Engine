#include "PipelineCacheResourceManager.h"

#include "ByteEngine/Application/Application.h"

PipelineCacheResourceManager::PipelineCacheResourceManager(const InitializeInfo& initialize_info) : ResourceManager(initialize_info, u8"PipelineCacheResourceManager") {
	
	switch (cache.Open(GetResourcePath(u8"PipelineCache", u8"bepkg"), GTSL::File::READ | GTSL::File::WRITE, true)) {
	default: ;
	}
}

PipelineCacheResourceManager::~PipelineCacheResourceManager()
{
}