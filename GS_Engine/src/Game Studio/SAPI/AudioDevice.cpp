#include "AudioDevice.h"

#include "Windows/WindowsAudioDevice.h"

AudioDevice::AudioDevice()
{
}

AudioDevice* AudioDevice::CreateAudioDevice()
{
#ifdef GS_PLATFORM_WIN
	return new WindowsAudioDevice();
#endif
}
