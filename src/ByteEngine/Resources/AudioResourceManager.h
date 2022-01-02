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
	}
	
	byte* GetAssetPointer(const Id id) { return nullptr; }
	uint32 GetFrameCount(Id id) const { return audioResourceInfos.At(id).Frames; }
	uint8 GetChannelCount(Id channelName) const { return audioResourceInfos.At(channelName).ChannelCount; }

	AudioResourceManager(const InitializeInfo&);

	~AudioResourceManager();

	template<typename... ARGS>
	void LoadAudioInfo(ApplicationManager* gameInstance, Id audioName, DynamicTaskHandle<AudioInfo, ARGS...> dynamicTaskHandle, ARGS&&... args) {
		//gameInstance->AddDynamicTask(u8"loadAudioInfo", &AudioResourceManager::loadAudioInfo<ARGS...>, {}, {}, {}, GTSL::MoveRef(audioName), GTSL::MoveRef(dynamicTaskHandle), GTSL::ForwardRef<ARGS>(args)...);
	}

	//Audio data is aligned to 16 bytes
	template<typename... ARGS>
	void LoadAudio(ApplicationManager* gameInstance, AudioInfo audioInfo, DynamicTaskHandle<AudioInfo, GTSL::Range<const byte*>, ARGS...> dynamicTaskHandle, ARGS&&... args) {
		//gameInstance->AddDynamicTask(u8"loadAudio", &AudioResourceManager::loadAudio<ARGS...>, {}, {}, {}, GTSL::MoveRef(audioInfo), GTSL::MoveRef(dynamicTaskHandle), GTSL::ForwardRef<ARGS>(args)...);
	}

	MAKE_HANDLE(uint32, AudioAsset);

	AudioAssetHandle CreateAudioAsset() {
		auto index = liveAudios.Emplace(GetPersistentAllocator());
		auto& liveAudio = liveAudios[index];
		liveAudio.Buffer.Allocate(100000, 32); //32 bytes is avx256 alignment requirement
	}

private:
	template<typename... ARGS>
	auto loadAudioInfo(TaskInfo taskInfo, Id audioName, DynamicTaskHandle<AudioInfo, ARGS...> dynamicTaskHandle, ARGS&&... args) {
		auto audioInfoSerialize = audioResourceInfos.At(audioName);

		AudioInfo audioInfo(audioName, audioInfoSerialize);

		taskInfo.ApplicationManager->AddStoredDynamicTask(dynamicTaskHandle, GTSL::MoveRef(audioInfo), GTSL::ForwardRef<ARGS>(args)...);
	}

	template<typename... ARGS>
	auto loadAudio(TaskInfo taskInfo, AudioInfo audioInfo, DynamicTaskHandle<AudioInfo, GTSL::Range<const byte*>, ARGS...> dynamicTaskHandle, ARGS&&... args) {
		//uint32 bytes = audioInfo.GetAudioSize(); const byte* dataPointer = nullptr;
		//
		//auto searchResult = audioBytes.TryEmplace(audioInfo.Name, bytes, 32, GetPersistentAllocator());
		//
		//if (searchResult.State())
		//{
		//	packageFiles[getThread()].SetPointer(audioInfo.ByteOffset);
		//	auto& buffer = searchResult.Get();
		//	packageFiles[getThread()].Read(bytes, buffer);
		//	dataPointer = buffer.GetData();
		//} else {
		//	dataPointer = searchResult.Get().GetData();
		//}
		//
		//taskInfo.ApplicationManager->AddStoredDynamicTask(dynamicTaskHandle, GTSL::MoveRef(audioInfo), GTSL::Range(bytes, dataPointer), GTSL::ForwardRef<ARGS>(args)...);
	}

	GTSL::File indexFile;
	GTSL::HashMap<Id, AudioDataSerialize, BE::PersistentAllocatorReference> audioResourceInfos;

	GTSL::StaticVector<GTSL::File, MAX_THREADS> packageFiles;

	struct LiveAudioAssetData {
		LiveAudioAssetData(const BE::PAR& allocator) : Buffer(allocator) {}
		GTSL::Buffer<BE::PAR> Buffer;
	};
	GTSL::FixedVector<LiveAudioAssetData, BE::PAR> liveAudios;
};
