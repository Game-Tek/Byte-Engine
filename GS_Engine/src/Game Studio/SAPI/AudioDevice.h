#pragma once

#include "Core.h"

class AudioDevice
{
	inline static AudioDevice* audio_device_instance = nullptr;
	
public:
	virtual ~AudioDevice() = default;
	static AudioDevice* Get() { return audio_device_instance; }

	AudioDevice();
	
	virtual void Start() = 0;
	virtual void GetBufferSize(uint32* total_buffer_size_) = 0;
	virtual void GetAvailableBufferSize(uint64* available_buffer_size_) = 0;
	virtual void PushAudioData(void* data_, uint64 pushed_samples_) = 0;
	virtual void Stop() = 0;
};