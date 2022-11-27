#pragma once

#include "ByteEngine/Handle.hpp"

#include "ByteEngine/Game/System.hpp"

#include "AAL/AudioDevice.h"
#if BE_PLATFORM_WINDOWS
#include "AAL/Platform/Windows/WindowsAudioDevice.h"
#endif

#include <GTSL/Vector.hpp>
#include <GTSL/Buffer.hpp>
#include <GTSL/Math/Quaternion.h>

#include "ByteEngine/Id.h"
#include "ByteEngine/Game/Tasks.h"
#include "ByteEngine/Resources/AudioResourceManager.h"

class Sound;

MAKE_HANDLE(uint32, AudioListener)
MAKE_HANDLE(uint32, AudioEmitter)

class AudioSystem : public BE::System {
public:
	AudioSystem(const InitializeInfo& initializeInfo);
	~AudioSystem();

	AudioListenerHandle CreateAudioListener();
	AudioEmitterHandle CreateAudioEmitter();

	void BindAudio(AudioEmitterHandle audioEmitter, Id audioToPlay);
	void PlayAudio(AudioEmitterHandle audioEmitter);
	
	void SetPosition(AudioEmitterHandle audioEmitterHandle, const GTSL::Vector3 position) { audioEmittersLocation[audioEmitterHandle()] = position; }
	void SetPosition(AudioListenerHandle audioListenerHandle, const GTSL::Vector3 position) { audioListenersLocation[audioListenerHandle()] = position; }
	
	GTSL::Vector3 GetPosition(const AudioListenerHandle audioListenerHandle) const { return audioListenersLocation[audioListenerHandle()]; }
	GTSL::Vector3 GetPosition(const AudioEmitterHandle audioEmitterHandle) const { return audioEmittersLocation[audioEmitterHandle()]; }
	
	void SetOrientation(AudioListenerHandle audioListenerHandle, const GTSL::Quaternion orientation) { audioListenersOrientation[audioListenerHandle()] = orientation; }
	GTSL::Quaternion GetOrientation(AudioListenerHandle audioListenerHandle) const { return audioListenersOrientation[audioListenerHandle()]; }
	
	void SetAudioListener(const AudioListenerHandle audioListenerHandle) { activeAudioListenerHandle = audioListenerHandle; }

	void SetLooping(const AudioEmitterHandle audioEmitterHandle, bool loop) { audioEmittersSettings[audioEmitterHandle()].Loop = loop; }
	bool GetLooping(const AudioEmitterHandle audioEmitterHandle) { return audioEmittersSettings[audioEmitterHandle()].Loop; }
	
	MAKE_HANDLE(uint8, Channel);
	MAKE_HANDLE(uint8, SoundSource);

private:
	using AudioDevice = AAL::AudioDevice;
#if BE_PLATFORM_WINDOWS
	using AudioDevice = AAL::WindowsAudioDevice;
#elif BE_PLATFORM_LINUX
#endif
	
	AudioDevice audioDevice;
	AudioDevice::MixFormat mixFormat;

	struct SoundSource {
		byte* Data;
	};
	GTSL::StaticVector<SoundSource, 16> soundSources;
	
	struct MixerChannel {
		struct Effect {
			
		};		
		GTSL::StaticVector<Effect, 8> Effects;

		GTSL::StaticVector<SoundSourceHandle, 16> SoundSources;
		
		float32 Volume = 0.0f;
	};
	GTSL::StaticVector<MixerChannel, 16> channels;
	
	GTSL::StaticVector<uint8, 8> audioListeners;
	GTSL::StaticVector<GTSL::Vector3, 8> audioListenersLocation;
	GTSL::StaticVector<GTSL::Quaternion, 8> audioListenersOrientation;
	
	GTSL::StaticVector<GTSL::Vector3, 8> audioEmittersLocation;

	MAKE_HANDLE(uint32, PrivateSound);
	
	struct AudioEmitterSettings
	{
		bool Loop = false;
		//PrivateSoundHandle PrivateSoundHandle;
		Id Name;
		// Indicates current playing sample
		uint32 CurrentSample = 0;

	};
	GTSL::StaticVector<AudioEmitterSettings, 8> audioEmittersSettings;
	
	GTSL::StaticVector<AudioEmitterHandle, 8> playingEmitters;

	struct SourceAudioData {
		
		/**
		 * \brief A list of emitters currently playing this source.
		 */
		GTSL::StaticVector<AudioEmitterHandle, 8> Emitters;

		bool Loaded;

		uint32 ChannelCount = 0, FrameCount = 0;

		byte* Buffer = nullptr;
		uint64 Size = 0;
	};
	GTSL::HashMap<GTSL::StringView, SourceAudioData, BE::PAR> sourceAudioDatas;
	
	GTSL::Buffer<BE::PAR> audioBuffer;
	TaskHandle<AudioResourceManager::AudioInfo> onAudioInfoLoadHandle;
	TaskHandle<AudioResourceManager::AudioInfo, GTSL::Range<const byte*>> onAudioLoadHandle;

	AudioListenerHandle activeAudioListenerHandle;

	template<typename T>
	static auto getSample(const byte* buffer, const uint8 channelCount, const uint32 sample, const uint32 channel) -> T {
		return *(reinterpret_cast<const T*>(buffer) + sample * channelCount + channel);
	}

	template<typename T>
	static auto getSample(byte* buffer, const uint8 channelCount, const uint32 sample, const uint32 channel) -> T& {
		return *(reinterpret_cast<T*>(buffer) + sample * channelCount + channel);
	}

	void render(TaskInfo);

	void removePlayingEmitter(uint32 i)
	{
		audioEmittersSettings[playingEmitters[i]()].CurrentSample = 0;
		playingEmitters.Pop(i);
	}
	
	void onAudioInfoLoad(TaskInfo taskInfo, AudioResourceManager*, AudioResourceManager::AudioInfo audioInfo);
	void onAudioLoad(TaskInfo taskInfo, AudioResourceManager::AudioInfo audioInfo, GTSL::Range<const byte*> buffer);

	bool shouldStream(const uint32 samples) {
		return samples >= samplesToLoad(); //if audio is longer than 10 seconds, stream it
	}

	uint32 samplesToLoad() {
		return 48000 * 10;
	}
};
