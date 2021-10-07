#pragma once

#include "ByteEngine/Core.h"

#include <GTSL/File.h>
#include <GTSL/HashMap.hpp>
#include <GTSL/Buffer.hpp>

#include "ResourceManager.h"
#include "ByteEngine/Game/ApplicationManager.h"

class AudioResourceManager final : public ResourceManager
{
public:
	struct AudioData : Data
	{
		uint32 Frames;
		uint32 SampleRate;
		uint8 ChannelCount;
		uint8 BitDepth;
	};

	struct AudioDataSerialize : DataSerialize<AudioData>
	{
		INSERT_START(AudioDataSerialize)
		{
			INSERT_BODY
			Insert(insertInfo.Frames, buffer);
			Insert(insertInfo.SampleRate, buffer);
			Insert(insertInfo.ChannelCount, buffer);
			Insert(insertInfo.BitDepth, buffer);
		}

		EXTRACT_START(AudioDataSerialize)
		{
			EXTRACT_BODY
			Extract(extractInfo.Frames, buffer);
			Extract(extractInfo.SampleRate, buffer);
			Extract(extractInfo.ChannelCount, buffer);
			Extract(extractInfo.BitDepth, buffer);
		}
	};

	struct AudioInfo : Info<AudioDataSerialize>
	{
		DECL_INFO_CONSTRUCTOR(AudioInfo, Info<AudioDataSerialize>)

		uint32 GetAudioSize()
		{
			return Frames * ChannelCount * (BitDepth / 8);
		}
	};

	void ReleaseAudioAsset(Id asset)
	{
		audioBytes.Remove(asset);
	}
	
	byte* GetAssetPointer(const Id id) { return audioBytes.At(id).GetData(); }
	uint32 GetFrameCount(Id id) const { return audioResourceInfos.At(id).Frames; }
	uint8 GetChannelCount(Id channelName) const { return audioResourceInfos.At(channelName).ChannelCount; }

	AudioResourceManager();

	~AudioResourceManager();

	template<typename... ARGS>
	void LoadAudioInfo(ApplicationManager* gameInstance, Id audioName, DynamicTaskHandle<AudioResourceManager*, AudioInfo, ARGS...> dynamicTaskHandle, ARGS&&... args)
	{
		auto loadAudioInfo = [](TaskInfo taskInfo, AudioResourceManager* resourceManager, Id audioName, decltype(dynamicTaskHandle) dynamicTaskHandle, ARGS&&... args)
		{			
			auto audioInfoSerialize = resourceManager->audioResourceInfos.At(audioName);

			AudioInfo audioInfo(audioName, audioInfoSerialize);

			taskInfo.ApplicationManager->AddStoredDynamicTask(dynamicTaskHandle, GTSL::MoveRef(resourceManager), GTSL::MoveRef(audioInfo), GTSL::ForwardRef<ARGS>(args)...);
		};

		gameInstance->AddDynamicTask(u8"loadAudioInfo", Task<AudioResourceManager*, Id, decltype(dynamicTaskHandle), ARGS...>::Create(loadAudioInfo), {}, this, GTSL::MoveRef(audioName), GTSL::MoveRef(dynamicTaskHandle), GTSL::ForwardRef<ARGS>(args)...);
	}

	//Audio data is aligned to 16 bytes
	template<typename... ARGS>
	void LoadAudio(ApplicationManager* gameInstance, AudioInfo audioInfo, DynamicTaskHandle<AudioResourceManager*, AudioInfo, GTSL::Range<const byte*>, ARGS...> dynamicTaskHandle, ARGS&&... args)
	{
		auto loadAudio = [](TaskInfo taskInfo, AudioResourceManager* resourceManager, AudioInfo audioInfo, decltype(dynamicTaskHandle) dynamicTaskHandle, ARGS&&... args)
		{
			uint32 bytes = audioInfo.GetAudioSize(); const byte* dataPointer = nullptr;

			auto searchResult = resourceManager->audioBytes.TryEmplace(audioInfo.Name, bytes, 16, resourceManager->GetPersistentAllocator());
			
			if (searchResult.State())
			{
				resourceManager->packageFiles[resourceManager->getThread()].SetPointer(audioInfo.ByteOffset);
				auto& buffer = searchResult.Get();
				resourceManager->packageFiles[resourceManager->getThread()].Read(bytes, buffer);
				dataPointer = buffer.GetData();
			}
			else
			{
				dataPointer = searchResult.Get().GetData();
			}

			taskInfo.ApplicationManager->AddStoredDynamicTask(dynamicTaskHandle, GTSL::MoveRef(resourceManager), GTSL::MoveRef(audioInfo), GTSL::Range<const byte*>(bytes, dataPointer), GTSL::ForwardRef<ARGS>(args)...);
		};

		gameInstance->AddDynamicTask(u8"loadAudio", Task<AudioResourceManager*, AudioInfo, decltype(dynamicTaskHandle), ARGS...>::Create(loadAudio), {}, this, GTSL::MoveRef(audioInfo), GTSL::MoveRef(dynamicTaskHandle), GTSL::ForwardRef<ARGS>(args)...);
	}

private:
	GTSL::File indexFile;
	GTSL::HashMap<Id, AudioDataSerialize, BE::PersistentAllocatorReference> audioResourceInfos;
	GTSL::HashMap<Id, GTSL::Buffer<BE::PAR>, BE::PersistentAllocatorReference> audioBytes;

	GTSL::StaticVector<GTSL::File, MAX_THREADS> packageFiles;
};
