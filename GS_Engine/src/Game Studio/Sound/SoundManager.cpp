#include "SoundManager.h"
#include "Application/Application.h"

SoundManager::SoundManager()
{
	auto result = GS::Application::Get()->GetResourceManager()->TryGetResource("sax", "Audio");
	sound = static_cast<AudioResourceManager::AudioResourceData*>(result.ResourceData);

	struct AudioDevice::AudioDeviceCreateInfo audio_device_create_info { StreamShareMode::SHARED };
	audioDevice = AudioDevice::CreateAudioDevice(audio_device_create_info);
	audioDevice->Start();

	audioDevice->GetBufferSize(&bufferSize);
	
	buffer = static_cast<byte*>(malloc(bufferSize));
}

SoundManager::~SoundManager()
{
	audioDevice->Stop();
	free(buffer);
}

void SoundManager::Update()
{
	uint64 buffer_size = 0;
	audioDevice->GetAvailableBufferSize(&buffer_size);

	GS::Application::Get()->GetClock().GetElapsedTime();
	
	//buffer[]
	
	//audioDevice->PushAudioData();
}
