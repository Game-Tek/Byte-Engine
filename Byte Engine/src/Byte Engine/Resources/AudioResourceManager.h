#pragma once

#include "Core.h"

#include "SubResourceManager.h"
#include <map>
#include "SAPI/AudioCore.h"
#include <GTSL/FixedVector.hpp>

class AudioResourceManager final : public SubResourceManager
{
public:
	struct AudioResourceData final : ResourceData
	{
	protected:
		friend AudioResourceManager;

		GTSL::FixedVector<byte> Bytes;
		AudioChannelCount AudioChannelCount;
		AudioSampleRate AudioSampleRate;
		AudioBitDepth AudioBitDepth;

	public:
	};
	
private:
	std::map<uint64, AudioResourceData> resources;
	
public:
	~AudioResourceManager() = default;
	
	bool LoadResource(const LoadResourceInfo& loadResourceInfo, OnResourceLoadInfo& onResourceLoadInfo) override;
	void LoadFallback(const LoadResourceInfo& loadResourceInfo, OnResourceLoadInfo& onResourceLoadInfo) override;

	[[nodiscard]] GTSL::Id64 GetResourceType() const override { return "Audio"; }
};
