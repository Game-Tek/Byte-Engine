#pragma once

#include "ByteEngine/Handle.hpp"

#include "ByteEngine/Game/System.h"
#include <AAL/Platform/Windows/WindowsAudioDevice.h>
#include <GTSL/Array.hpp>
#include <GTSL/Buffer.hpp>
#include "ByteEngine/Id.h"
#include "ByteEngine/Game/Tasks.h"
#include "ByteEngine/Resources/AudioResourceManager.h"

class Sound;

MAKE_HANDLE(uint32, AudioListener)
MAKE_HANDLE(uint32, AudioEmitter)

class AudioSystem : public System
{
public:
	AudioSystem();
	~AudioSystem();
	
	void Initialize(const InitializeInfo& initializeInfo) override;
	void Shutdown(const ShutdownInfo& shutdownInfo) override;

	AudioListenerHandle CreateAudioListener();
	AudioEmitterHandle CreateAudioEmitter();

	void PlayAudio(AudioEmitterHandle audioEmitter, Id audioToPlay);
	
private:
	using AudioDevice = AAL::WindowsAudioDevice;
	
	AudioDevice audioDevice;
	AudioDevice::MixFormat mixFormat;

	GTSL::Array<uint8, 8> audioListeners;
	GTSL::Array<uint8, 8> audioEmitters;

	GTSL::Array<AudioEmitterHandle, 8> playingEmitters;
	GTSL::Array<uint32, 8> playingAudioFilesPlayedFrames;
	GTSL::Array<Id, 8> playingAudioFiles;

	GTSL::Array<Id, 8> lastRequestedAudios;

	GTSL::Buffer<BE::PAR> audioBuffer;
	DynamicTaskHandle<AudioResourceManager*, AudioResourceManager::AudioInfo> onAudioInfoLoadHandle;
	DynamicTaskHandle<AudioResourceManager*, AudioResourceManager::AudioInfo, GTSL::Range<const byte*>> onAudioLoadHandle;

	void requestAudioStreams();
	void render(TaskInfo);

	void removePlayingSound(uint32 i)
	{
		playingEmitters.Pop(i); playingAudioFilesPlayedFrames.Pop(i); playingAudioFiles.Pop(i);
	}

	void onAudioInfoLoad(TaskInfo taskInfo, AudioResourceManager*, AudioResourceManager::AudioInfo audioInfo);
	void onAudioLoad(TaskInfo taskInfo, AudioResourceManager*, AudioResourceManager::AudioInfo audioInfo, GTSL::Range<const byte*> buffer);
};
