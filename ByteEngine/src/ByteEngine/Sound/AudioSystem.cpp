#include "AudioSystem.h"

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
	audioDevice.Initialize(create_info);
	audioDevice.Start();
}

void AudioSystem::Shutdown(const ShutdownInfo& shutdownInfo)
{
	audioDevice.Stop();
	audioDevice.Destroy();
}
