#pragma once

#include "Core.h"

/**
 * \brief Interface for an audio device. Creates and manages an audio device, endpoint and buffer.
 */
class AudioDevice
{
	inline static AudioDevice* audio_device_instance = nullptr;
	
public:
	virtual ~AudioDevice()
	{
		delete audio_device_instance;
	};
	
	static AudioDevice* Get() { return audio_device_instance; }

	AudioDevice();
	
	/**
	 * \brief Initializes the audio device to start receiving audio. Must be called before any other function.
	 */
	virtual void Start() = 0;
	
	/**
	 * \brief Sets the passed variable as the size of the allocated buffer.
	 * \param total_buffer_size_ Pointer to to variable for storing the size of the allocated buffer.
	 */
	virtual void GetBufferSize(uint32* total_buffer_size_) = 0;
	/**
	 * \brief Sets the passed variable as the available size in the allocated buffer.
	 * Should be called to query the available size before filling the audio buffer size it may have some space still occupied since the audio driver may not have consumed it.
	 * \param available_buffer_size_ Pointer to a variable to set as the available buffer size.
	 */
	virtual void GetAvailableBufferSize(uint64* available_buffer_size_) = 0;
	
	/**
	 * \brief Pushes the audio data found in the passed in buffer for the amount of specified samples to the audio buffer, making such data available for the next request from the driver to retrieve audio data.
	 * \param data_ Pointer to the buffer containing the audio data to be used for filling the audio buffer.
	 * \param pushed_samples_ Number of audio samples to copy from the passed pointer to the audio buffer.
	 */
	virtual void PushAudioData(void* data_, uint64 pushed_samples_) = 0;
	
	/**
	 * \brief Shutdowns and destroys the audio device resources. Must be called before destroying the audio device, no other functions shall be called after this.
	 */
	virtual void Stop() = 0;
};