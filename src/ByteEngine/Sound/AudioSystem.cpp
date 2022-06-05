#include "AudioSystem.h"

#include <GTSL/Algorithm.hpp>
#include <GTSL/DataSizes.h>
#include <GTSL/Math/Math.hpp>

#include "ByteEngine/Id.h"
#include "ByteEngine/Application/Application.h"
#include "ByteEngine/Game/ApplicationManager.h"
#include "ByteEngine/Resources/AudioResourceManager.h"

AudioSystem::AudioSystem(const InitializeInfo& initializeInfo) : System(initializeInfo, u8"AudioSystem"), sourceAudioDatas(16, GetPersistentAllocator()), audioBuffer(4 * 2 * 48000, 16, GetPersistentAllocator())
{
	AudioDevice::CreateInfo createInfo;

	bool error = false;

	if (audioDevice.Initialize(createInfo)) {
		auto bits = (uint32)BE::Application::Get()->GetUINTOption(u8"bitDepth");
		bits = GTSL::Math::Clamp(bits, 8u, 32u);
		bits = GTSL::NextPowerOfTwo(bits);

		auto numChannels = (uint32)BE::Application::Get()->GetUINTOption(u8"channels");
		numChannels = GTSL::Math::Clamp(numChannels, 1u, 8u);

		auto samplesPerSecond = (uint32)BE::Application::Get()->GetUINTOption(u8"kHz");
		samplesPerSecond = GTSL::Math::Clamp(samplesPerSecond, 41000u, 96000u);

		if (samplesPerSecond != 41000 && samplesPerSecond != 48000 && samplesPerSecond != 96000 ) {
			BE_LOG_WARNING(u8"User provided ", samplesPerSecond, u8" as audio system sample rate, which is invalid. Defaulting to 48KHz");
			samplesPerSecond = 48000;
		}

		mixFormat.BitsPerSample = static_cast<uint8>(bits);
		mixFormat.NumberOfChannels = static_cast<uint8>(numChannels);
		mixFormat.SamplesPerSecond = samplesPerSecond;

		//onAudioInfoLoadHandle = initializeInfo.ApplicationManager->StoreDynamicTask(u8"onAudioInfoLoad", Task<AudioResourceManager*, AudioResourceManager::AudioInfo>::Create<AudioSystem, &AudioSystem::onAudioInfoLoad>(this), {});
		//onAudioLoadHandle = initializeInfo.ApplicationManager->StoreDynamicTask(u8"onAudioLoad", Task<AudioResourceManager*, AudioResourceManager::AudioInfo, GTSL::Range<const byte*>>::Create<AudioSystem, &AudioSystem::onAudioLoad>(this), {});

		if (audioDevice.IsMixFormatSupported(AAL::StreamShareMode::SHARED, mixFormat)) {
			if (audioDevice.CreateAudioStream(AAL::StreamShareMode::SHARED, mixFormat)) {
				if (audioDevice.Start()) {
					audioBuffer.Allocate(GTSL::Byte(GTSL::MegaByte(1)), mixFormat.GetFrameSize());
					//initializeInfo.ApplicationManager->AddTask(u8"renderAudio", &AudioSystem::render, GTSL::StaticVector<TaskDependency, 1>{ { u8"AudioSystem", AccessTypes::READ_WRITE } }, u8"RenderDo", u8"RenderEnd");

					BE_LOG_MESSAGE(u8"Started WASAPI API\n	Bits per sample: ", (uint32)mixFormat.BitsPerSample, u8"\n	Khz: ", mixFormat.SamplesPerSecond, u8"\n	Channels: ", (uint32)mixFormat.NumberOfChannels)

					if(audioDevice.GetBufferSamplePlacement() == AudioDevice::BufferSamplePlacement::INTERLEAVED) {
						BE_LOG_ERROR(u8"Create audio device requires interleaved sample placment, which isn't supported. Oudio output will be disabled.")
						error = true;
					}

				} else { error = true; }
			} else { error = true; }
		} else { error = true; }
	} else { error = true; }

	if (error)
		BE_LOG_WARNING(u8"Unable to start audio device with requested parameters:\n Stream share mode: Shared\n Bits per sample: ", mixFormat.BitsPerSample, u8"\nNumber of channels: ", mixFormat.NumberOfChannels, u8"\nSamples per second: ", mixFormat.SamplesPerSecond);
}

AudioSystem::~AudioSystem() {
	if (!audioDevice.Stop()) {
		BE_LOG_ERROR(u8"Failed to stop audio device.")
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
	auto* audioResourceManager = GetApplicationManager()->GetSystem<AudioResourceManager>(u8"AudioResourceManager");
	auto& sad = sourceAudioDatas.Emplace(audioToPlay);
	sad.Loaded = false;
	audioEmittersSettings[audioEmitter()].Name = audioToPlay;
}

void AudioSystem::PlayAudio(AudioEmitterHandle audioEmitter)
{
	audioEmittersSettings[audioEmitter()].CurrentSample = 0;
	playingEmitters.EmplaceBack(audioEmitter);
}

void AudioSystem::render(TaskInfo) {
	if(!activeAudioListenerHandle) { return; }
	
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
			
			auto& emitter = audioEmittersSettings[playingEmitters[pe]()];
			emitter.CurrentSample += 0;
			
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
}

void AudioSystem::onAudioInfoLoad(TaskInfo taskInfo, AudioResourceManager* audioResourceManager, AudioResourceManager::AudioInfo audioInfo)
{
	audioResourceManager->LoadAudio(taskInfo.ApplicationManager, audioInfo, onAudioLoadHandle);
}

void AudioSystem::onAudioLoad(TaskInfo taskInfo, AudioResourceManager*, AudioResourceManager::AudioInfo audioInfo, GTSL::Range<const byte*> buffer) {
	GTSL::StaticVector<uint32, 16> toDelete;
	
	auto& sad = sourceAudioDatas[audioInfo.Name];

	sad.Loaded = true;

	for(auto& e : sad.Emitters) {
		playingEmitters.EmplaceBack(e);	
	}
}
