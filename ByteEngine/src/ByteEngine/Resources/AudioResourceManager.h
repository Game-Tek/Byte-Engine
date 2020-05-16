#pragma once

#include "ByteEngine/Core.h"

#include "SubResourceManager.h"
#include <AAL/AudioCore.h>
#include <GTSL/FixedVector.hpp>
#include <unordered_map>
#include "ResourceData.h"
#include <GTSL/Id.h>
#include <GTSL/String.hpp>

class AudioResourceManager final : public SubResourceManager
{
public:
	struct AudioResourceData final : ResourceData
	{
		friend AudioResourceManager;

		GTSL::FixedVector<byte> Bytes;
		AAL::AudioChannelCount AudioChannelCount;
		AAL::AudioSampleRate AudioSampleRate;
		AAL::AudioBitDepth AudioBitDepth;
	};
	
private:
	std::unordered_map<GTSL::Id64::HashType, AudioResourceData> resources;
	
public:
	AudioResourceManager() : SubResourceManager("Audio")
	{
	}
	
	~AudioResourceManager() = default;

	AudioResourceData* GetResource(const GTSL::String& resourceName)
	{
		GTSL::ReadLock<GTSL::ReadWriteMutex> lock(resourceMapMutex);
		return &resources.at(GTSL::Id64(resourceName));
	}
	
	AudioResourceData* TryGetResource(const GTSL::String& name);
	
	void ReleaseResource(const GTSL::Id64& resourceName)
	{
		resourceMapMutex.WriteLock();
		if (resources[resourceName].DecrementReferences() == 0) { resources.erase(resourceName); }
		resourceMapMutex.WriteUnlock();
	}
};
