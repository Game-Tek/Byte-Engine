#include "SoundManager.h"
#include "Application/Application.h"

SoundManager::SoundManager()
{
	auto result = GS::Application::Get()->GetResourceManager()->GetResource("sax", "Audio");
	sound = static_cast<AudioResourceManager::AudioResourceData*>(result.ResourceData);

	audioDevice = AudioDevice::CreateAudioDevice();
	audioDevice->Start();

	audioDevice->GetBufferSize(&bufferSize);
	
	buffer = malloc(bufferSize);
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

	audioDevice->PushAudioData();
}
