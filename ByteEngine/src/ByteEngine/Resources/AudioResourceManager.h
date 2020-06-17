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
	struct AudioResourceData final : ResourceHandle
	{
		friend AudioResourceManager;

		GTSL::FixedVector<byte> Bytes;
		AAL::AudioChannelCount AudioChannelCount;
		AAL::AudioSampleRate AudioSampleRate;
		AAL::AudioBitDepth AudioBitDepth;
	};
	
public:
	AudioResourceManager() : SubResourceManager("Audio")
	{
	}
	
	~AudioResourceManager() = default;
	
private:
};
