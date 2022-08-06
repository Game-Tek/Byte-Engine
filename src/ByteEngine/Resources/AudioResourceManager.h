#pragma once

#include "ByteEngine/Core.h"

#include <GTSL/File.hpp>
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

	AudioResourceManager(const InitializeInfo&);

	~AudioResourceManager();

	template<typename... ARGS>
	void LoadAudioInfo(Id audioName, TaskHandle<AudioInfo, ARGS...> dynamicTaskHandle, ARGS&&... args) {
		GetApplicationManager()->EnqueueTask(GetApplicationManager()->RegisterTask(this, u8"loadAudioInfo", {}, &AudioResourceManager::loadAudioInfo<ARGS...>, {}, {}), GTSL::MoveRef(audioName), GTSL::MoveRef(dynamicTaskHandle), GTSL::ForwardRef<ARGS>(args)...);
	}

	//Audio data is aligned to 16 bytes
	template<typename... ARGS>
	void LoadAudio(AudioInfo audioInfo, GTSL::Range<byte*> buffer, TaskHandle<AudioInfo, GTSL::Range<const byte*>, ARGS...> dynamicTaskHandle, ARGS&&... args) {
		GetApplicationManager()->EnqueueTask(GetApplicationManager()->RegisterTask(this, u8"loadAudio", {}, &AudioResourceManager::loadAudio<ARGS...>, {}, {}), GTSL::MoveRef(audioInfo), GTSL::MoveRef(buffer), GTSL::MoveRef(dynamicTaskHandle), GTSL::ForwardRef<ARGS>(args)...);
	}

private:
	template<typename... ARGS>
	auto loadAudioInfo(TaskInfo taskInfo, Id audioName, TaskHandle<AudioInfo, ARGS...> dynamicTaskHandle, ARGS&&... args) {
		auto audioInfoSerialize = audioResourceInfos.At(audioName);

		AudioInfo audioInfo(audioName, audioInfoSerialize);

		taskInfo.ApplicationManager->EnqueueTask(dynamicTaskHandle, GTSL::MoveRef(audioInfo), GTSL::ForwardRef<ARGS>(args)...);
	}

	template<typename... ARGS>
	auto loadAudio(TaskInfo taskInfo, AudioInfo audioInfo, GTSL::Range<byte*> buffer, TaskHandle<AudioInfo, GTSL::Range<const byte*>, ARGS...> dynamicTaskHandle, ARGS&&... args) {
		uint32 bytes = audioInfo.GetAudioSize();
		
		packageFiles[getThread()].SetPointer(audioInfo.ByteOffset);
		packageFiles[getThread()].ReadRaw(buffer.begin(), bytes);
		
		taskInfo.ApplicationManager->EnqueueTask(dynamicTaskHandle, GTSL::MoveRef(audioInfo), GTSL::Range(bytes, buffer.begin()), GTSL::ForwardRef<ARGS>(args)...);
	}

	GTSL::File indexFile;
	GTSL::HashMap<Id, AudioDataSerialize, BE::PersistentAllocatorReference> audioResourceInfos;

	GTSL::StaticVector<GTSL::File, MAX_THREADS> packageFiles;
};
