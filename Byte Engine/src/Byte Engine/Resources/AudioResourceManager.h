#pragma once

#include "Core.h"

#include "SubResourceManager.h"
#include "SAPI/AudioCore.h"
#include <GTSL/FixedVector.hpp>
#include <unordered_map>
#include "ResourceData.h"

class AudioResourceManager final : public SubResourceManager
{
public:
	struct AudioResourceData final : ResourceData
	{
		friend AudioResourceManager;

		GTSL::FixedVector<byte> Bytes;
		AudioChannelCount AudioChannelCount;
		AudioSampleRate AudioSampleRate;
		AudioBitDepth AudioBitDepth;
	};
	
private:
	std::unordered_map<GTSL::Id64::HashType, AudioResourceData> resources;
	
public:
	inline static constexpr GTSL::Id64 type{ "Audio" };
	
	AudioResourceManager() : SubResourceManager("Audio")
	{
	}
	
	~AudioResourceManager() = default;

	AudioResourceData* GetResource(const GTSL::String& resourceName)
	{
		ReadLock<ReadWriteMutex> lock(resourceMapMutex);
		return &resources[GTSL::Id64(resourceName)];
	}
	
	AudioResourceData* TryGetResource(const GTSL::String& name);
	
	void ReleaseResource(const GTSL::Id64& resourceName)
	{
		resourceMapMutex.WriteLock();
		if (resources[resourceName].DecrementReferences() == 0) { resources.erase(resourceName); }
		resourceMapMutex.WriteUnlock();
	}
};
