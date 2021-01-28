#pragma once

#include "ResourceManager.h"
#include <GTSL/File.h>

#include <GTSL/Buffer.hpp>

class PipelineCacheResourceManager : public ResourceManager
{
public:
	PipelineCacheResourceManager();
	~PipelineCacheResourceManager();

	void DoesCacheExist(bool& doesExist) const { doesExist = cache.GetFileSize(); }
	void GetCacheSize(uint32& size) const { size = static_cast<uint32>(cache.GetFileSize()); }
	
	template<class ALLOCTOR>
	void GetCache(GTSL::Buffer<ALLOCTOR>& buffer) { cache.ReadFile(cache.GetFileSize(), buffer.GetBufferInterface()); }
	
	template<class ALLOCTOR>
	void WriteCache(GTSL::Buffer<ALLOCTOR>& buffer)
	{
		cache.SetPointer(0, GTSL::File::MoveFrom::BEGIN);
		cache.SetEndOfFile();
		cache.WriteToFile(buffer.GetBufferInterface());
	}
private:
	GTSL::File cache;
};
