#pragma once

#include "ByteEngine/Game/System.h"
#include <AAL/Platform/Windows/WindowsAudioDevice.h>

class Sound;

class AudioSystem : public System
{
public:
	AudioSystem();
	~AudioSystem();
	
	void Initialize(const InitializeInfo& initializeInfo) override;
	void Shutdown(const ShutdownInfo& shutdownInfo) override;

private:
	using AudioDevice = AAL::WindowsAudioDevice;
	
	AudioDevice audioDevice;
};
