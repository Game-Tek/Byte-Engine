#pragma once

#include "Core.h"

#include "SAPI/AudioDevice.h"

#include <mmdeviceapi.h>
#include <Audioclient.h>

class WindowsAudioDevice : public AudioDevice
{
	IMMDeviceEnumerator* enumerator = NULL;
	IMMDevice* endPoint = NULL;
	IAudioClient* audioClient = NULL;
	IAudioRenderClient* renderClient = NULL;
	WAVEFORMATEX* pwfx = NULL;

	uint32 bufferFrameCount = 0;
	void* data = nullptr;
public:
	WindowsAudioDevice();
	virtual ~WindowsAudioDevice();

	void Start() override;
	void GetAvailableBufferSize(uint64* available_buffer_size_) override;
	void GetBufferSize(uint32* total_buffer_size_) override;
	void PushAudioData(void* data_, uint64 pushed_samples_) override;
	void Stop() override;
};
