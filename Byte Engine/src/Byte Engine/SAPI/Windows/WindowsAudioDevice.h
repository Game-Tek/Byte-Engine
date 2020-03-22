#pragma once

#include "Core.h"

#include "SAPI/AudioDevice.h"

#include <mmdeviceapi.h>
#include <Audioclient.h>

class WindowsAudioDevice : public AudioDevice
{
	IMMDeviceEnumerator* enumerator = nullptr;
	IMMDevice* endPoint = nullptr;
	IAudioClient* audioClient = nullptr;
	IAudioRenderClient* renderClient = nullptr;
	PWAVEFORMATEXTENSIBLE pwfx = nullptr;

	uint32 bufferFrameCount = 0;
	void* data = nullptr;
public:
	WindowsAudioDevice(const AudioDeviceCreateInfo& audioDeviceCreateInfo);
	virtual ~WindowsAudioDevice();

	void Start() override;
	void GetAvailableBufferSize(uint64* availableBufferSize) override;
	void GetBufferSize(uint32* totalBufferSize) override;
	void PushAudioData(void* data_, uint64 pushedSamples) override;
	void Stop() override;
};
