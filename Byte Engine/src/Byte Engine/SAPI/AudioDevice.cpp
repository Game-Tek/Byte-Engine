#include "AudioDevice.h"

#include "Windows/WindowsAudioDevice.h"

AudioDevice::AudioDevice()
{
}

AudioDevice* AudioDevice::CreateAudioDevice(const AudioDeviceCreateInfo& audioDeviceCreateinfo)
{
#ifdef BE_PLATFORM_WIN
	return new WindowsAudioDevice();
#endif
}