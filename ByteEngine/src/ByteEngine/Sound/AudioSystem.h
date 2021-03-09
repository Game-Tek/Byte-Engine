#pragma once

#include "ByteEngine/Handle.hpp"

#include "ByteEngine/Game/System.h"
#include <AAL/Platform/Windows/WindowsAudioDevice.h>
#include <GTSL/Array.hpp>
#include <GTSL/Buffer.hpp>
#include <GTSL/Math/Quaternion.h>

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
	
	void SetPosition(AudioEmitterHandle audioEmitterHandle, const GTSL::Vector3 position) { audioEmittersLocation[audioEmitterHandle()] = position; }
	void SetPosition(AudioListenerHandle audioListenerHandle, const GTSL::Vector3 position) { audioListenersLocation[audioListenerHandle()] = position; }
	
	GTSL::Vector3 GetPosition(const AudioListenerHandle audioListenerHandle) const { return audioListenersLocation[audioListenerHandle()]; }
	GTSL::Vector3 GetPosition(const AudioEmitterHandle audioEmitterHandle) const { return audioEmittersLocation[audioEmitterHandle()]; }
	
	void SetOrientation(AudioListenerHandle audioListenerHandle, const GTSL::Quaternion orientation) { audioListenersOrientation[audioListenerHandle()] = orientation; }
	GTSL::Quaternion GetOrientation(AudioListenerHandle audioListenerHandle) const { return audioListenersOrientation[audioListenerHandle()]; }
	
	void SetAudioListener(const AudioListenerHandle audioListenerHandle) { activeAudioListenerHandle = audioListenerHandle; }

	void SetLooping(const AudioEmitterHandle audioEmitterHandle, bool loop) { audioEmittersSettings[audioEmitterHandle()].Loop = loop; }
	bool GetLooping(const AudioEmitterHandle audioEmitterHandle) { return audioEmittersSettings[audioEmitterHandle()].Loop; }

private:
	using AudioDevice = AAL::WindowsAudioDevice;

	static constexpr uint8 WAV_RIGHT_CHANNEL = 0, WAV_LEFT_CHANNEL = 1;
	static constexpr uint8 API_RIGHT_CHANNEL = 1, API_LEFT_CHANNEL = 0;
	
	AudioDevice audioDevice;
	AudioDevice::MixFormat mixFormat;

	GTSL::Array<uint8, 8> audioListeners;
	GTSL::Array<GTSL::Vector3, 8> audioListenersLocation;
	GTSL::Array<GTSL::Quaternion, 8> audioListenersOrientation;
	
	GTSL::Array<GTSL::Vector3, 8> audioEmittersLocation;

	struct AudioEmitterSettings
	{
		bool Loop = false;
	};
	GTSL::Array<AudioEmitterSettings, 8> audioEmittersSettings;

	GTSL::Array<GTSL::Pair<AudioEmitterHandle, Id>, 8> onHoldEmitters;
	
	GTSL::Array<Id, 8> playingEmittersSample;
	GTSL::Array<AudioEmitterHandle, 8> playingEmitters;
	GTSL::Array<uint32, 8> playingEmittersAudio;
	
	GTSL::Array<uint32, 8> playingAudioFilesPlayedFrames;
	GTSL::Array<Id, 8> playingAudioFiles;

	GTSL::Array<Id, 8> lastRequestedAudios;

	GTSL::Buffer<BE::PAR> audioBuffer;
	DynamicTaskHandle<AudioResourceManager*, AudioResourceManager::AudioInfo> onAudioInfoLoadHandle;
	DynamicTaskHandle<AudioResourceManager*, AudioResourceManager::AudioInfo, GTSL::Range<const byte*>> onAudioLoadHandle;

	AudioListenerHandle activeAudioListenerHandle;

	template<typename T>
	auto getSample(byte* buffer, const uint32 availableSamples, const uint32 sample, const uint32 channel) -> T&
	{
		return *reinterpret_cast<T*>(buffer + (channel * availableSamples * mixFormat.GetFrameSize()) + (sample * mixFormat.GetFrameSize()));
	};

	template<typename T>
	auto getIntertwinedSample(byte* buffer, const uint32 availableSamples, const uint32 sample, const uint32 channel) -> T&
	{
		return *(reinterpret_cast<T*>(buffer) + sample * 2 + channel);
	};
	
	void requestAudioStreams();
	void render(TaskInfo);

	void removePlayingSound(uint32 i)
	{
		playingAudioFilesPlayedFrames.Pop(i); playingAudioFiles.Pop(i);
	}

	void removePlayingEmitter(uint32 i)
	{
		playingEmitters.Pop(i); playingEmittersAudio.Pop(i); playingEmittersSample.Pop(i);
	}
	
	void onAudioInfoLoad(TaskInfo taskInfo, AudioResourceManager*, AudioResourceManager::AudioInfo audioInfo);
	void onAudioLoad(TaskInfo taskInfo, AudioResourceManager*, AudioResourceManager::AudioInfo audioInfo, GTSL::Range<const byte*> buffer);
};
