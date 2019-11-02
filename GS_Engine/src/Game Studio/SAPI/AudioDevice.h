#pragma once

class AudioDevice
{
	inline static AudioDevice* audio_device_instance = nullptr;

public:
	static AudioDevice* Get() { return audio_device_instance; }

	AudioDevice();
	
	void PushAudioData();
};