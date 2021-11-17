#pragma once

#include "ResourceManager.h"
#include <GTSL/File.h>

#include <GTSL/Buffer.hpp>

class PipelineCacheResourceManager : public ResourceManager
{
public:
	PipelineCacheResourceManager(const InitializeInfo&);
	~PipelineCacheResourceManager();

	void DoesCacheExist(bool& doesExist) const { doesExist = cache.GetSize(); }
	void GetCacheSize(uint32& size) const { size = static_cast<uint32>(cache.GetSize()); }
	
	template<class ALLOCTOR>
	void GetCache(GTSL::Buffer<ALLOCTOR>& buffer) { cache.Read(cache.GetSize(), buffer); }
	
	template<class ALLOCTOR>
	void WriteCache(GTSL::Buffer<ALLOCTOR>& buffer)
	{
		cache.SetPointer(0);
		cache.Write(buffer);
	}
private:
	GTSL::File cache;
};
