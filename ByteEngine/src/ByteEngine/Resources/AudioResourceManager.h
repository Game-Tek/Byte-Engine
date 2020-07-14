#pragma once

#include "ByteEngine/Core.h"

#include "SubResourceManager.h"
#include <AAL/AudioCore.h>
#include <GTSL/File.h>
#include <GTSL/FlatHashMap.h>
#include <GTSL/Vector.hpp>

class AudioResourceManager final : public SubResourceManager
{
public:
	struct AudioResourceInfo final
	{
		uint32 ByteOffset = 0;
		AAL::AudioChannelCount AudioChannelCount;
		AAL::AudioSampleRate AudioSampleRate;
		AAL::AudioBitDepth AudioBitDepth;
	};

	struct AudioAsset
	{
		GTSL::Vector<byte> Bytes;
	};

	struct LoadAudioAssetInfo : ResourceLoadInfo
	{
	};
	void LoadAudioAsset(const LoadAudioAssetInfo& loadAudioAssetInfo);

	AudioResourceManager();

	~AudioResourceManager() = default;

	const char* GetName() const override { return "Audio Resource Manager"; }
	
private:
	GTSL::File indexFile, packageFile;
	GTSL::FlatHashMap<AudioAsset> audioAssets;
	GTSL::FlatHashMap<AudioResourceInfo> audioResourceInfos;
};
