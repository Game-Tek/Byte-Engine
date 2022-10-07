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

		onAudioInfoLoadHandle = GetApplicationManager()->RegisterTask(this, u8"onAudioInfoLoad", DependencyBlock(TypedDependency<AudioResourceManager>(u8"AudioResourceManager")), &AudioSystem::onAudioInfoLoad);
		onAudioLoadHandle = GetApplicationManager()->RegisterTask(this, u8"onAudioLoad", DependencyBlock(), &AudioSystem::onAudioLoad);

		if (audioDevice.IsMixFormatSupported(AAL::StreamShareMode::SHARED, mixFormat)) {
			if (audioDevice.CreateAudioStream(AAL::StreamShareMode::SHARED, mixFormat)) {
				if (audioDevice.Start()) {
					audioBuffer.Allocate(GTSL::Byte(GTSL::MegaByte(1)).GetCount(), mixFormat.GetFrameSize());
					auto renderTaskHandle = GetApplicationManager()->RegisterTask(this, u8"renderAudio", DependencyBlock(), &AudioSystem::render, u8"RenderDo", u8"RenderEnd");

					GetApplicationManager()->EnqueueScheduledTask(renderTaskHandle);

					BE_LOG_SUCCESS(u8"Started Audio Device\n	Bits per Sample: ", (uint32)mixFormat.BitsPerSample, u8"\n	Khz: ", mixFormat.SamplesPerSecond, u8"\n	Channels: ", (uint32)mixFormat.NumberOfChannels)

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
	auto& sad = sourceAudioDatas.Emplace(GTSL::StringView(audioToPlay));
	sad.Loaded = false;
	audioEmittersSettings[audioEmitter()].Name = audioToPlay;
	audioResourceManager->LoadAudioInfo(audioToPlay, onAudioInfoLoadHandle);
}

void AudioSystem::PlayAudio(AudioEmitterHandle audioEmitter) {
	audioEmittersSettings[audioEmitter()].CurrentSample = 0;
	if(!playingEmitters.Find(audioEmitter)) { // If emitter is already playing, don't add it to list
		playingEmitters.EmplaceBack(audioEmitter);
	}
}

void AudioSystem::render(TaskInfo) {
	if(!activeAudioListenerHandle) { return; }
	
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
			
			auto& sad = sourceAudioDatas[GTSL::StringView(emitter.Name)];
			
			auto audioFrames = sad.FrameCount;
			auto remainingFrames = audioFrames - emitter.CurrentSample;
			auto clampedFrames = GTSL::Math::Limit(availableAudioFrames, remainingFrames);
			auto playedSamples = emitter.CurrentSample;

			const byte* audio = sad.Buffer;

			if (sad.ChannelCount == 1) {
				for (uint32 s = 0; s < clampedFrames; ++s) { //left channel
					auto sample = getSample<int16>(audio, 1, s + playedSamples, 0);
					
					getSample<int16>(buffer, 2, s, AudioDevice::LEFT_CHANNEL) += sample * leftPercentange;
					getSample<int16>(buffer, 2, s, AudioDevice::RIGHT_CHANNEL) += sample * rightPercentage;
				}
			} else {
				for (uint32 s = 0; s < clampedFrames; ++s) { //left channel
					auto lSample = getSample<int16>(audio, 2, s + playedSamples, AudioDevice::LEFT_CHANNEL);
					auto rSample = getSample<int16>(audio, 2, s + playedSamples, AudioDevice::RIGHT_CHANNEL);
			
					getSample<int16>(buffer, 2, s, AudioDevice::LEFT_CHANNEL) += lSample * leftPercentange;
					getSample<int16>(buffer, 2, s, AudioDevice::RIGHT_CHANNEL) += rSample * rightPercentage;
				}
			}
				
			if ((emitter.CurrentSample += clampedFrames) == audioFrames) {
				if (!GetLooping(playingEmitters[pe])) {
					playingEmitters.Pop(pe);
				}
				else {
					emitter.CurrentSample = 0;
				}
			}
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

void AudioSystem::onAudioInfoLoad(TaskInfo taskInfo, AudioResourceManager* audioResourceManager, AudioResourceManager::AudioInfo audioInfo) {
	auto& sad = sourceAudioDatas[GTSL::StringView(audioInfo.Name)];
	GetPersistentAllocator().Allocate(audioInfo.GetAudioSize(), 16, reinterpret_cast<void**>(&sad.Buffer), &sad.Size);
	audioResourceManager->LoadAudio(audioInfo, GTSL::Range<byte*>(sad.Size, sad.Buffer), onAudioLoadHandle);
}

void AudioSystem::onAudioLoad(TaskInfo taskInfo, AudioResourceManager::AudioInfo audioInfo, GTSL::Range<const byte*> buffer) {
	GTSL::StaticVector<uint32, 16> toDelete;
	
	auto& sad = sourceAudioDatas[GTSL::StringView(audioInfo.Name)];

	sad.Loaded = true;
	sad.ChannelCount = audioInfo.ChannelCount;
	sad.FrameCount = audioInfo.Frames;

	for(auto& e : sad.Emitters) {
		playingEmitters.EmplaceBack(e);	
	}
}
