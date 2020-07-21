#pragma once

#include "ResourceManager.h"
#include <GTSL/File.h>

class PipelineCacheResourceManager : public ResourceManager
{
public:
	PipelineCacheResourceManager();
	~PipelineCacheResourceManager();

	void DoesCacheExist(bool& doesExist) const { doesExist = cache.GetFileSize(); }
	void GetCacheSize(uint32& size) const { size = static_cast<uint32>(cache.GetFileSize()); }
	void GetCache(GTSL::Buffer& buffer) { cache.ReadFile(buffer); }
	void WriteCache(GTSL::Buffer& buffer);
private:
	GTSL::File cache;
};
