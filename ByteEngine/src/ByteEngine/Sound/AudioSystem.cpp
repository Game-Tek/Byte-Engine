#include "AudioSystem.h"


#include <GTSL/Algorithm.h>
#include <GTSL/DataSizes.h>
#include <GTSL/Math/Math.hpp>



#include "ByteEngine/Id.h"
#include "ByteEngine/Application/Application.h"
#include "ByteEngine/Resources/AudioResourceManager.h"

AudioSystem::AudioSystem() : System("AudioSystem")
{
}

AudioSystem::~AudioSystem()
{
}

void AudioSystem::Initialize(const InitializeInfo& initializeInfo)
{
	AudioDevice::CreateInfo create_info;
	create_info.ShareMode = AAL::StreamShareMode::SHARED;
	create_info.BitDepth = 32;
	create_info.Frequency = 48000;
	audioDevice.Initialize(create_info);
	audioDevice.Start();

	audioBuffer.Allocate(GTSL::Byte(GTSL::MegaByte(1)), 32, GetPersistentAllocator());
}

void AudioSystem::Shutdown(const ShutdownInfo& shutdownInfo)
{
	audioDevice.Stop();
	audioDevice.Destroy();
	
	audioBuffer.Free(32, GetPersistentAllocator());
}

AudioListenerHandle AudioSystem::CreateAudioListener()
{
	return AudioListenerHandle(audioListeners.EmplaceBack());
}

AudioEmitterHandle AudioSystem::CreateAudioEmitter()
{
	return AudioEmitterHandle(audioEmitters.EmplaceBack());
}

void AudioSystem::PlayAudio(AudioEmitterHandle audioEmitter, Id audioToPlay)
{
	//playingAudioSources.EmplaceBack(audioEmitter);
	lastRequestedAudios.EmplaceBack(audioToPlay);
}

void AudioSystem::requestAudioStreams()
{
	auto* audioResourceManager = BE::Application::Get()->GetResourceManager<AudioResourceManager>("AudioResourceManager");

	for(uint8 i = 0; i < lastRequestedAudios.GetLength(); ++i)
	{
		AudioResourceManager::LoadAudioAssetInfo loadAudioAssetInfo;
		loadAudioAssetInfo.GameInstance;
		loadAudioAssetInfo.ActsOn;
		loadAudioAssetInfo.DataBuffer;
		loadAudioAssetInfo.Name = lastRequestedAudios[i];
		loadAudioAssetInfo.UserData;
		audioResourceManager->LoadAudioAsset(loadAudioAssetInfo);
	}

	lastRequestedAudios.Resize(0);
}

void AudioSystem::render()
{
	GTSL::Array<uint32, 16> soundsToRemoveFromPlaying;
	GTSL::Array<Id, 16> samplesToRemoveFromPlaying;
	
	auto* audioResourceManager = BE::Application::Get()->GetResourceManager<AudioResourceManager>("AudioResourceManager");

	auto framesToBytes = [](uint32 samples, AAL::AudioChannelCount audioChannelCount, AAL::AudioBitDepth audioBits)
	{
		return samples * static_cast<GTSL::UnderlyingType<AAL::AudioSampleRate>>(audioChannelCount) * (static_cast<GTSL::UnderlyingType<AAL::AudioBitDepth>>(audioBits) / 8u); //TODO: chek
	};
	
	uint32 availableAudioFrames = 0;
	audioDevice.GetAvailableBufferFrames(availableAudioFrames);
	
	auto* buffer = audioBuffer.GetData();
	
	for(uint32 i = 0; i < playingEmitters.GetLength(); ++i)
	{
		byte* audio = audioResourceManager->GetAssetPointer(playingAudioFiles[i]);

		auto audioFrames = audioResourceManager->GetFrameCount(playingAudioFiles[i]);
		auto remainingFrames = audioFrames - playingAudioFilesPlayedFrames[i];
		auto clampedFrames = GTSL::Math::Limit(availableAudioFrames, remainingFrames);
		
		audio += framesToBytes(playingAudioFilesPlayedFrames[i], AAL::AudioChannelCount::CHANNELS_STEREO, AAL::AudioBitDepth::BIT_DEPTH_32);

		GTSL::MemCopy(framesToBytes(clampedFrames, AAL::AudioChannelCount::CHANNELS_STEREO, AAL::AudioBitDepth::BIT_DEPTH_32) , audio, buffer);

		if((playingAudioFilesPlayedFrames[i] += clampedFrames) == audioFrames)
		{
			soundsToRemoveFromPlaying.EmplaceBack(i);
			samplesToRemoveFromPlaying.EmplaceBack(playingAudioFiles[i]);
		}
	}

	audioDevice.PushAudioData(audioBuffer.GetData(), availableAudioFrames);

	for (uint32 i = 0; i < soundsToRemoveFromPlaying.GetLength(); ++i)
	{
		removePlayingSound(soundsToRemoveFromPlaying[i]);
		audioResourceManager->ReleaseAudioAsset(samplesToRemoveFromPlaying[i]);
	}
}
