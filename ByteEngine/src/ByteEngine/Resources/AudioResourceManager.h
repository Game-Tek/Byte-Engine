#pragma once

#include "ByteEngine/Core.h"

#include <GTSL/File.h>
#include <GTSL/FlatHashMap.h>
#include <GTSL/Vector.hpp>

#include "ResourceManager.h"

class AudioResourceManager final : public ResourceManager
{
public:
	struct AudioResourceInfo final
	{
		uint32 ByteOffset = 0;
		uint8 AudioChannelCount;
		uint8 AudioSampleRate;
		uint8 AudioBitDepth;
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

	~AudioResourceManager();
	
private:
	GTSL::File indexFile, packageFile;
	GTSL::FlatHashMap<AudioAsset> audioAssets;
	GTSL::FlatHashMap<AudioResourceInfo> audioResourceInfos;
};

void Insert(const AudioResourceManager::AudioResourceInfo& audioResourceInfo, GTSL::Buffer& buffer, const GTSL::AllocatorReference& allocatorReference);
void Extract(AudioResourceManager::AudioResourceInfo& audioResourceInfo, GTSL::Buffer& buffer, const GTSL::AllocatorReference& allocatorReference);