#include "AudioSystem.h"

#include <GTSL/Algorithm.hpp>
#include <GTSL/DataSizes.h>
#include <GTSL/Math/Math.hpp>

#include "ByteEngine/Id.h"
#include "ByteEngine/Application/Application.h"
#include "ByteEngine/Game/ApplicationManager.h"
#include "ByteEngine/Resources/AudioResourceManager.h"

AudioSystem::AudioSystem(const InitializeInfo& initializeInfo) : System(initializeInfo, u8"AudioSystem"), audioBuffer(200000, 16, GetPersistentAllocator()),
	loadedSounds(16, GetPersistentAllocator())
{
	AudioDevice::CreateInfo createInfo;

	bool error = false;

	if (audioDevice.Initialize(createInfo)) {
		mixFormat.BitsPerSample = 16;
		mixFormat.NumberOfChannels = 2;
		mixFormat.SamplesPerSecond = 48000;

		//onAudioInfoLoadHandle = initializeInfo.ApplicationManager->StoreDynamicTask(u8"onAudioInfoLoad", Task<AudioResourceManager*, AudioResourceManager::AudioInfo>::Create<AudioSystem, &AudioSystem::onAudioInfoLoad>(this), {});
		//onAudioLoadHandle = initializeInfo.ApplicationManager->StoreDynamicTask(u8"onAudioLoad", Task<AudioResourceManager*, AudioResourceManager::AudioInfo, GTSL::Range<const byte*>>::Create<AudioSystem, &AudioSystem::onAudioLoad>(this), {});

		if (audioDevice.IsMixFormatSupported(AAL::StreamShareMode::SHARED, mixFormat)) {
			if (audioDevice.CreateAudioStream(AAL::StreamShareMode::SHARED, mixFormat)) {
				if (audioDevice.Start()) {
					audioBuffer.Allocate(GTSL::Byte(GTSL::MegaByte(1)), mixFormat.GetFrameSize());
					//initializeInfo.ApplicationManager->AddTask(u8"renderAudio", &AudioSystem::render, GTSL::StaticVector<TaskDependency, 1>{ { u8"AudioSystem", AccessTypes::READ_WRITE } }, u8"RenderDo", u8"RenderEnd");

					BE_LOG_MESSAGE(u8"Started WASAPI API\n	Bits per sample: ", (uint32)mixFormat.BitsPerSample, u8"\n	Khz: ", mixFormat.SamplesPerSecond, u8"\n	Channels: ", (uint32)mixFormat.NumberOfChannels)

						BE_ASSERT(audioDevice.GetBufferSamplePlacement() == AudioDevice::BufferSamplePlacement::INTERLEAVED, u8"Unsupported");
				} else { error = true; }
			} else { error = true; }
		} else { error = true; }
	} else { error = true; }

	if (error)
		BE_LOG_WARNING(u8"Unable to start audio device with requested parameters:\n Stream share mode: Shared\n Bits per sample: ", mixFormat.BitsPerSample, u8"\nNumber of channels: ", mixFormat.NumberOfChannels, u8"\nSamples per second: ", mixFormat.SamplesPerSecond);
}

AudioSystem::~AudioSystem() {
	if (!audioDevice.Stop()) {

	}

	audioDevice.Destroy();
}

AudioListenerHandle AudioSystem::CreateAudioListener() {
	audioListenersLocation.EmplaceBack(); audioListenersOrientation.EmplaceBack();
	return AudioListenerHandle(audioListeners.EmplaceBack());
}

AudioEmitterHandle AudioSystem::CreateAudioEmitter() {
	auto index = audioEmittersSettings.GetLength();
	audioEmittersSettings.EmplaceBack();
	audioEmittersLocation.EmplaceBack();
	return AudioEmitterHandle(index);
}

void AudioSystem::BindAudio(AudioEmitterHandle audioEmitter, Id audioToPlay) {
	lastRequestedAudios.EmplaceBack(audioToPlay);
	audioEmittersSettings[audioEmitter()].Name = audioToPlay;
}

void AudioSystem::PlayAudio(AudioEmitterHandle audioEmitter)
{
	if ((!onHoldEmitters.Find(audioEmitter).State())) {
		if(const auto res = playingEmitters.Find(audioEmitter); res.State()) {
			audioEmittersSettings[audioEmitter()].Samples = 0;
		}
		else {
			onHoldEmitters.EmplaceBack(audioEmitter);
		}
	}
}

void AudioSystem::requestAudioStreams() {
	//auto* audioResourceManager = BE::Application::Get()->GetResourceManager<AudioResourceManager>(u8"AudioResourceManager");
	//
	//for(uint8 i = 0; i < lastRequestedAudios.GetLength(); ++i)
	//{
	//	audioResourceManager->LoadAudioInfo(BE::Application::Get()->GetGameInstance(), lastRequestedAudios[i], onAudioInfoLoadHandle);
	//}
	//
	//lastRequestedAudios.Resize(0);
}

void AudioSystem::render(TaskInfo) {
	requestAudioStreams();

	if(!activeAudioListenerHandle) { return; }
	
	{
		GTSL::StaticVector<uint32, 16> emittersToRemove;
		for (uint32 i = 0; i < onHoldEmitters.GetLength(); ++i) {
			if (loadedSounds.Find(audioEmittersSettings[onHoldEmitters[i]()].Name).State()) {
				emittersToRemove.EmplaceBack(i);
			}
		}
		
		for (auto e : emittersToRemove) {
			playingEmitters.EmplaceBack(onHoldEmitters[e]);
			onHoldEmitters.Pop(e);
		}
	}
	
	GTSL::StaticVector<uint32, 16> emittersToStop;
	
	//auto* audioResourceManager = BE::Application::Get()->GetResourceManager<AudioResourceManager>(u8"AudioResourceManager");
	
	uint32 availableAudioFrames = 0;
	if(!audioDevice.GetAvailableBufferFrames(availableAudioFrames)) {
		BE_LOG_ERROR(u8"Failed to acquire audio buffer size.")
		//TODO: disable audio
	}
	
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
				auto distanceFactor = GTSL::Math::Distance(emitterPosition, listenerPosition);
				distanceFactor = GTSL::Math::Clamp(-(distanceFactor / 15) + 1, 0.0f, 1.0f);
				//leftPercentange = GTSL::Math::InvertRange(leftPercentange * distanceFactor, 1.0f); rightPercentage = GTSL::Math::InvertRange(rightPercentage * distanceFactor, 1.0f);
				//leftPercentange *= distanceFactor; rightPercentage *= distanceFactor;
			}
			
			auto& emmitter = audioEmittersSettings[playingEmitters[pe]()];
			auto playedSamples = emmitter.Samples;
			
			//byte* audio = audioResourceManager->GetAssetPointer(emmitter.Name);
			//
			//auto audioFrames = audioResourceManager->GetFrameCount(emmitter.Name);
			//auto remainingFrames = audioFrames - playedSamples;
			//auto clampedFrames = GTSL::Math::Limit(availableAudioFrames, remainingFrames);
			//
			//if (audioResourceManager->GetChannelCount(emmitter.Name) == 1) {
			//	for (uint32 s = 0; s < clampedFrames; ++s) { //left channel
			//		auto sample = getSample<int16>(audio, 1, s + playedSamples, 0);
			//		
			//		getSample<int16>(buffer, 2, s, AudioDevice::LEFT_CHANNEL) += sample * leftPercentange;
			//		getSample<int16>(buffer, 2, s, AudioDevice::RIGHT_CHANNEL) += sample * rightPercentage;
			//	}
			//} else {
			//	for (uint32 s = 0; s < clampedFrames; ++s) { //left channel
			//		auto lSample = getSample<int16>(audio, 2, s + playedSamples, AudioDevice::LEFT_CHANNEL);
			//		auto rSample = getSample<int16>(audio, 2, s + playedSamples, AudioDevice::RIGHT_CHANNEL);
			//
			//		getSample<int16>(buffer, 2, s, AudioDevice::LEFT_CHANNEL) += lSample * leftPercentange;
			//		getSample<int16>(buffer, 2, s, AudioDevice::RIGHT_CHANNEL) += rSample * rightPercentage;
			//	}
			//}
			//	
			//if ((emmitter.Samples += clampedFrames) == audioFrames) {
			//	if (!GetLooping(playingEmitters[pe])) {
			//		emittersToStop.EmplaceBack(pe);
			//	}
			//	else {
			//		emmitter.Samples = 0;
			//	}
			//}
		}
	}

	{
		auto audioDataCopyFunction = [&](uint32 size, void* to)
		{
			GTSL::MemCopy(size, audioBuffer.GetData(), to);
		};
		
		if(!audioDevice.PushAudioData(audioDataCopyFunction, availableAudioFrames)) {
			BE_LOG_ERROR(u8"Failed to push audio data to driver.")
			//TODO: disable audio
		}
	}
	
	for (uint32 i = 0; i < emittersToStop.GetLength(); ++i) {
		removePlayingEmitter(i);
	}
}

void AudioSystem::onAudioInfoLoad(TaskInfo taskInfo, AudioResourceManager* audioResourceManager, AudioResourceManager::AudioInfo audioInfo)
{
	audioResourceManager->LoadAudio(taskInfo.ApplicationManager, audioInfo, onAudioLoadHandle);
}

void AudioSystem::onAudioLoad(TaskInfo taskInfo, AudioResourceManager*, AudioResourceManager::AudioInfo audioInfo, GTSL::Range<const byte*> buffer)
{
	loadedSounds.EmplaceBack(audioInfo.Name);
	GTSL::StaticVector<uint32, 16> toDelete;
	
	for(uint32 i = 0; i < onHoldEmitters.GetLength(); ++i)
	{
		if(audioEmittersSettings[onHoldEmitters[i]()].Name == audioInfo.Name)
		{
			toDelete.EmplaceBack(i);
			playingEmitters.EmplaceBack(onHoldEmitters[i]);
			//audioEmittersSettings[onHoldEmitters[i]()].PrivateSoundHandle = soundHandle;
		}
	}

	for (auto e : toDelete) { onHoldEmitters.Pop(e); }

}
