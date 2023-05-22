#pragma once

#include "ResourceManager.h"

#include <GTSL/File.hpp>
#include <GTSL/Buffer.hpp>

//TODO: support multiple APIs

class PipelineCacheResourceManager : public ResourceManager {
public:
	PipelineCacheResourceManager(const InitializeInfo&);
	~PipelineCacheResourceManager();

	void DoesCacheExist(bool& doesExist) const { doesExist = cache.GetSize(); }
	void GetCacheSize(GTSL::uint32& size) const { size = static_cast<GTSL::uint32>(cache.GetSize()); }
	
	template<class ALLOCTOR>
	void GetCache(GTSL::Buffer<ALLOCTOR>& buffer) { }
	
	template<class ALLOCTOR>
	void WriteCache(GTSL::Buffer<ALLOCTOR>& buffer)
	{
		cache.SetPointer(0);
		cache.Write(buffer);
	}
private:
	GTSL::File cache;
};
