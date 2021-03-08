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
	audioListenersLocation.EmplaceBack(); audioListenersOrientation.EmplaceBack();
	return AudioListenerHandle(audioListeners.EmplaceBack());
}

AudioEmitterHandle AudioSystem::CreateAudioEmitter()
{
	return AudioEmitterHandle(audioEmittersLocation.EmplaceBack());
}

void AudioSystem::PlayAudio(AudioEmitterHandle audioEmitter, Id audioToPlay)
{
	//playingAudioSources.EmplaceBack(audioEmitter);
	lastRequestedAudios.EmplaceBack(audioToPlay);
	onHoldEmitters.EmplaceBack(audioEmitter, audioToPlay);
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
	GTSL::Array<uint32, 16> emittersToRemoveFromPlaying;
	
	auto* audioResourceManager = BE::Application::Get()->GetResourceManager<AudioResourceManager>("AudioResourceManager");
	
	uint32 availableAudioFrames = 0;
	audioDevice.GetAvailableBufferFrames(availableAudioFrames);
	
	auto* buffer = audioBuffer.GetData();

	GTSL::SetMemory(availableAudioFrames * mixFormat.GetFrameSize(), audioBuffer.GetData(), 0);

	{
		GTSL::Vector3 listenerPosition = GetPosition(activeAudioListenerHandle);
		GTSL::Quaternion listenerRotation = GetOrientation(activeAudioListenerHandle);
		GTSL::Vector3 listenerRightVector = listenerRotation * GTSL::Math::Right;

		for (uint32 pe = 0; pe < playingEmitters.GetLength(); ++pe)
		{
			GTSL::Vector3 emitterPosition = GetPosition(playingEmitters[pe]);

			auto soundDirection = GTSL::Math::DotProduct(GTSL::Math::Normalized(emitterPosition - listenerPosition), listenerRightVector);

			auto reMap = GTSL::Math::MapToRange(soundDirection, -1.0f, 1.0f, 0.0f, 1.0f);
			
			auto leftPercentange = GTSL::Math::InvertRange(reMap, 1.0);
			auto rightPercentage = reMap;

			{
				auto distanceFactor = GTSL::Math::Length(emitterPosition, listenerPosition);
				distanceFactor = GTSL::Math::Clamp(-(distanceFactor / 1500) + 1, 0.0f, 1.0f);
				//auto inDistFact = GTSL::Math::InvertRange(distanceFactor, 1.0f);
				leftPercentange *= distanceFactor; rightPercentage *= distanceFactor;
			}
			
			auto sampleName = playingEmittersSample[pe]; auto sampleIndex = playingEmittersAudio[pe];
			
			byte* audio = audioResourceManager->GetAssetPointer(sampleName);

			auto playedSamples = playingAudioFilesPlayedFrames[sampleIndex];
			
			auto audioFrames = audioResourceManager->GetFrameCount(sampleName);
			auto remainingFrames = audioFrames - playedSamples;
			auto clampedFrames = GTSL::Math::Limit(availableAudioFrames, remainingFrames);
			
			for (uint32 s = 0; s < clampedFrames; ++s) //left channel
			{
				getIntertwinedSample<int16>(buffer, availableAudioFrames, s, API_LEFT_CHANNEL) += getSample<int16>(audio, audioFrames, s + playedSamples, 0) * leftPercentange;
			}

			for (uint32 s = 0; s < clampedFrames; ++s) //right channel
			{
				getIntertwinedSample<int16>(buffer, availableAudioFrames, s, API_RIGHT_CHANNEL) += getSample<int16>(audio, audioFrames, s + playedSamples, 0) * rightPercentage;
			}

			if ((playingAudioFilesPlayedFrames[sampleIndex] += clampedFrames) == audioFrames)
			{
				soundsToRemoveFromPlaying.EmplaceBack(sampleIndex);
				samplesToRemoveFromPlaying.EmplaceBack(playingAudioFiles[sampleIndex]);
				emittersToRemoveFromPlaying.EmplaceBack(pe);
			}
		}
	}

	{
		auto audioDataCopyFunction = [&](uint32 size, void* to)
		{
			GTSL::MemCopy(size, audioBuffer.GetData(), to);
		};
		
		audioDevice.PushAudioData(audioDataCopyFunction, availableAudioFrames);
	}

	for (uint32 i = 0; i < soundsToRemoveFromPlaying.GetLength(); ++i)
	{
		removePlayingSound(soundsToRemoveFromPlaying[i]);
		audioResourceManager->ReleaseAudioAsset(samplesToRemoveFromPlaying[i]);
	}
	
	for (uint32 i = 0; i < emittersToRemoveFromPlaying.GetLength(); ++i)
	{
		removePlayingEmitter(emittersToRemoveFromPlaying[i]);
	}
}

void AudioSystem::onAudioInfoLoad(TaskInfo taskInfo, AudioResourceManager* audioResourceManager, AudioResourceManager::AudioInfo audioInfo)
{
	audioResourceManager->LoadAudio(taskInfo.GameInstance, audioInfo, onAudioLoadHandle);
}

void AudioSystem::onAudioLoad(TaskInfo taskInfo, AudioResourceManager*, AudioResourceManager::AudioInfo audioInfo, GTSL::Range<const byte*> buffer)
{
	auto audioIndex = playingAudioFiles.EmplaceBack(audioInfo.Name);
	playingAudioFilesPlayedFrames.EmplaceBack(0);

	GTSL::Array<uint32, 16> toDelete;
	
	for(uint32 i = 0; i < onHoldEmitters.GetLength(); ++i)
	{
		if(onHoldEmitters[i].Second == audioInfo.Name)
		{
			toDelete.EmplaceBack(i);
			playingEmitters.EmplaceBack(onHoldEmitters[i].First);
			playingEmittersAudio.EmplaceBack(audioIndex);
			playingEmittersSample.EmplaceBack(onHoldEmitters[i].Second);
		}
	}

	for (auto e : toDelete) { onHoldEmitters.Pop(e); }
}
