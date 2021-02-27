#include "AudioSystem.h"

#include <GTSL/Algorithm.h>
#include <GTSL/DataSizes.h>
#include <GTSL/Math/Math.hpp>

#include "ByteEngine/Id.h"
#include "ByteEngine/Application/Application.h"
#include "ByteEngine/Game/GameInstance.h"
#include "ByteEngine/Resources/AudioResourceManager.h"

AudioSystem::AudioSystem() : System("AudioSystem")
{
}

AudioSystem::~AudioSystem()
{
}

void AudioSystem::Initialize(const InitializeInfo& initializeInfo)
{
	AudioDevice::CreateInfo createInfo;
	audioDevice.Initialize(createInfo);
	
	mixFormat.BitsPerSample = 16;
	mixFormat.NumberOfChannels = 2;
	mixFormat.SamplesPerSecond = 48000;

	onAudioInfoLoadHandle = initializeInfo.GameInstance->StoreDynamicTask("onAudioInfoLoad", Task<AudioResourceManager*, AudioResourceManager::AudioInfo>::Create<AudioSystem, &AudioSystem::onAudioInfoLoad>(this), {});
	onAudioLoadHandle = initializeInfo.GameInstance->StoreDynamicTask("onAudioLoad", Task<AudioResourceManager*, AudioResourceManager::AudioInfo, GTSL::Range<const byte*>>::Create<AudioSystem, &AudioSystem::onAudioLoad>(this), {});
	
	if (audioDevice.IsMixFormatSupported(AAL::StreamShareMode::SHARED, mixFormat))
	{
		audioDevice.CreateAudioStream(AAL::StreamShareMode::SHARED, mixFormat);
		audioDevice.Start();
		audioBuffer.Allocate(GTSL::Byte(GTSL::MegaByte(1)), mixFormat.GetFrameSize(), GetPersistentAllocator());
		initializeInfo.GameInstance->AddTask("renderAudio", Task<>::Create<AudioSystem, &AudioSystem::render>(this), GTSL::Array<TaskDependency, 1>{ { "AudioSystem", AccessTypes::READ_WRITE } }, "RenderDo", "RenderEnd");
	}
	else
	{
		BE_LOG_WARNING("Unable to start audio device with requested parameters:\n	Stream share mode: Shared\n	Bits per sample: ", mixFormat.BitsPerSample, "\n	Number of channels: ", mixFormat.NumberOfChannels, "\n	Samples per second: ", mixFormat.SamplesPerSecond);
	}
}

void AudioSystem::Shutdown(const ShutdownInfo& shutdownInfo)
{
	audioDevice.Stop();
	audioDevice.Destroy();
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
		audioResourceManager->LoadAudioInfo(BE::Application::Get()->GetGameInstance(), lastRequestedAudios[i], onAudioInfoLoadHandle);
	}

	lastRequestedAudios.Resize(0);
}

void AudioSystem::render(TaskInfo)
{
	requestAudioStreams();
	
	GTSL::Array<uint32, 16> soundsToRemoveFromPlaying;
	GTSL::Array<Id, 16> samplesToRemoveFromPlaying;
	
	auto* audioResourceManager = BE::Application::Get()->GetResourceManager<AudioResourceManager>("AudioResourceManager");
	
	uint32 availableAudioFrames = 0;
	audioDevice.GetAvailableBufferFrames(availableAudioFrames);
	
	auto* buffer = audioBuffer.GetData();

	GTSL::SetMemory(audioBuffer.GetCapacity(), audioBuffer.GetData(), 0);
	
	for(uint32 i = 0; i < playingAudioFiles.GetLength(); ++i)
	{
		byte* audio = audioResourceManager->GetAssetPointer(playingAudioFiles[i]);

		auto audioFrames = audioResourceManager->GetFrameCount(playingAudioFiles[i]);
		auto remainingFrames = audioFrames - playingAudioFilesPlayedFrames[i];
		auto clampedFrames = GTSL::Math::Limit(availableAudioFrames, remainingFrames);
		
		audio += mixFormat.GetFrameSize() * playingAudioFilesPlayedFrames[i];

		GTSL::MemCopy(mixFormat.GetFrameSize() * clampedFrames, audio, buffer);

		if((playingAudioFilesPlayedFrames[i] += clampedFrames) == audioFrames)
		{
			soundsToRemoveFromPlaying.EmplaceBack(i);
			samplesToRemoveFromPlaying.EmplaceBack(playingAudioFiles[i]);
		}
	}

	audioDevice.PushAudioData([&](uint32 size, void* to) { GTSL::MemCopy(size, audioBuffer.GetData(), to); }, availableAudioFrames);

	for (uint32 i = 0; i < soundsToRemoveFromPlaying.GetLength(); ++i)
	{
		//removePlayingSound(soundsToRemoveFromPlaying[i]);
		playingAudioFiles.Pop(soundsToRemoveFromPlaying[i]);
		playingAudioFilesPlayedFrames.Pop(soundsToRemoveFromPlaying[i]);
		audioResourceManager->ReleaseAudioAsset(samplesToRemoveFromPlaying[i]);
	}
}

void AudioSystem::onAudioInfoLoad(TaskInfo taskInfo, AudioResourceManager* audioResourceManager, AudioResourceManager::AudioInfo audioInfo)
{
	uint32 a = 0;
	audioResourceManager->LoadAudio(taskInfo.GameInstance, audioInfo, onAudioLoadHandle);
}

void AudioSystem::onAudioLoad(TaskInfo taskInfo, AudioResourceManager*, AudioResourceManager::AudioInfo audioInfo, GTSL::Range<const byte*> buffer)
{
	playingAudioFiles.EmplaceBack(audioInfo.Name);
	playingAudioFilesPlayedFrames.EmplaceBack(0);
}
